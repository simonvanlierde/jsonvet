//! Replacing a file safely, for `--fix` and `jsonvet fmt`.

use std::io::{self, Write as _};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::messages;

/// Replace the file `path` leads to with `text`: write a temporary file beside it, give it the
/// file's permissions, then rename it over the file, so a crash leaves the old text or the new
/// one and never half of either. A symlink is followed and stays a symlink. A read-only file is
/// refused, as the rename would replace it anyway. The error is a message ready to show.
pub(crate) fn write_atomically(path: &Path, text: &str) -> Result<(), String> {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let failed = |e: io::Error| messages::fill("write.failed", &[("error", &e.to_string())]);
    let target = std::fs::canonicalize(path).map_err(failed)?;
    let permissions = std::fs::metadata(&target).map_err(failed)?.permissions();
    if permissions.readonly() {
        return Err(messages::text("write.read-only").to_owned());
    }
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    // A name taken by another process is never ours to remove: try the next one.
    let mut tries = 0;
    let (mut file, temp) = loop {
        let temp = target.with_file_name(format!(
            ".{name}.jsonvet-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
        {
            Ok(file) => break (file, temp),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && tries < 8 => tries += 1,
            Err(e) => return Err(failed(e)),
        }
    };
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
