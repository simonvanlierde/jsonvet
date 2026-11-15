//! Replacing a file safely, for `--fix` and `jsonvet fmt`.

use std::fs::{File, Permissions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::messages;

/// Replace the file `path` leads to with `text`: write a temporary file beside it, give it the
/// file's permissions, then rename it over the file, so a crash leaves the old text or the new
/// one and never half of either. A symlink is followed and stays a symlink. A read-only file is
/// refused, as the rename would replace it anyway. The error is a message ready to show.
pub(crate) fn write_atomically(path: &Path, text: &str) -> Result<(), String> {
    let failed = |e: io::Error| messages::fill("write.failed", &[("error", &e.to_string())]);
    let target = std::fs::canonicalize(path).map_err(failed)?;
    let permissions = std::fs::metadata(&target).map_err(failed)?.permissions();
    if permissions.readonly() {
        return Err(messages::text("write.read-only").to_owned());
    }
    let (mut file, temp) = temp_beside(&target, &permissions).map_err(failed)?;
    let write = || -> io::Result<()> {
        file.write_all(text.as_bytes())?;
        file.set_permissions(permissions)?;
        file.sync_all()?;
        std::fs::rename(&temp, &target)
    };
    write().map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        failed(e)
    })
}

/// A new, empty file beside `target`, and its path. On Unix it is created with `permissions`
/// already, so no one else can read the new text before they are set.
fn temp_beside(target: &Path, permissions: &Permissions) -> io::Result<(File, PathBuf)> {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    // The umask can narrow this; `set_permissions` later restores the rest.
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(
        &mut options,
        std::os::unix::fs::PermissionsExt::mode(permissions) & 0o777,
    );
    #[cfg(not(unix))]
    let _ = permissions;
    // A name taken by another process is never ours to remove: try the next one.
    let mut tries = 0;
    loop {
        let temp = target.with_file_name(format!(
            ".{name}.jsonvet-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match options.open(&temp) {
            Ok(file) => return Ok((file, temp)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && tries < 8 => tries += 1,
            Err(e) => return Err(e),
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::temp_beside;

    /// A file only its owner may read gets a temporary file only its owner may read, from the
    /// start, so its new text is never readable by others.
    #[test]
    fn the_temporary_file_starts_with_the_files_permissions() {
        let dir = std::env::temp_dir().join(format!("jsonvet-write-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temporary directory");
        let target = dir.join("secret.json");
        std::fs::write(&target, "{}").expect("written");
        let private = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&target, private.clone()).expect("permissions set");
        let (file, temp) = temp_beside(&target, &private).expect("a temporary file");
        let mode = file.metadata().expect("metadata").permissions().mode();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(mode & 0o777, 0o600);
        assert!(temp.starts_with(&dir));
    }
}
