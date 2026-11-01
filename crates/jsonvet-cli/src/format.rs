//! `jsonvet fmt`: format files in place, or with `--check` list the ones that would change.

use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::Mutex;

use jsonvet::{FormatOptions, Rule};

use crate::args::STDIN;
use crate::editorconfig;
use crate::messages;
use crate::output::printable;
use crate::settings::{FormatSettings, Resolver};
use crate::{lock, read, read_stdin};

/// The layout settings for one file: the defaults, then its `.editorconfig` files, then the
/// `[format]` table of its config file.
fn options(settings: &FormatSettings, style: editorconfig::Style) -> FormatOptions {
    let defaults = FormatOptions::default();
    FormatOptions {
        indent: settings.indent.or(style.indent).unwrap_or(defaults.indent),
        width: settings.width.or(style.width).unwrap_or(defaults.width),
        normalize_json5: settings.normalize_json5.unwrap_or(defaults.normalize_json5),
    }
}

/// Format each of `files`, naming standard input `stdin_name`. Standard input goes to `out`,
/// and with `check_only` nothing is written but the names of the files that would change.
/// Problems go to standard error. Exit status 1 for a file that cannot be read or has a
/// syntax error, and with `check_only` for a file that would change.
pub(crate) fn run(
    files: &[OsString],
    check_only: bool,
    stdin_name: Option<OsString>,
    resolver: &Mutex<Resolver>,
    mut out: impl Write,
) -> Result<u8, String> {
    let stdin_name = stdin_name.unwrap_or_else(|| STDIN.into());
    let write_error = |e: io::Error| messages::fill("error.write", &[("error", &e)]);
    let (mut failed, mut changed) = (false, false);
    for file in files {
        let stdin = file == STDIN;
        let shown = if stdin { &stdin_name } else { file };
        let name = printable(&shown.to_string_lossy());
        let text = if stdin {
            read_stdin()
        } else {
            read(Path::new(file))
        };
        let text = match text {
            Ok(text) => text,
            Err(message) => {
                eprintln!("{name}: {message}");
                failed = true;
                continue;
            }
        };
        let path = Path::new(shown);
        let (settings, dirs) = {
            let mut resolver = lock(resolver);
            (resolver.for_file(path)?, resolver.search_dirs(path))
        };
        let config = settings.config_for(&shown.to_string_lossy());
        let options = options(&settings.format, editorconfig::style(path, &dirs));
        let formatted = match jsonvet::format(&text, &config, &options) {
            Ok(formatted) => formatted,
            Err(report) => {
                for problem in report
                    .problems()
                    .iter()
                    .filter(|p| p.rule() == Rule::Syntax)
                {
                    let at = report.location(problem.offset());
                    eprintln!(
                        "{name}:{}:{}: {}",
                        at.line,
                        at.column,
                        printable(&problem.to_string())
                    );
                }
                failed = true;
                continue;
            }
        };
        if stdin && !check_only {
            out.write_all(formatted.as_bytes()).map_err(write_error)?;
        } else if formatted != text {
            changed = true;
            if check_only {
                writeln!(out, "{name}").map_err(write_error)?;
            } else if let Err(e) = write_atomic(path, &formatted) {
                eprintln!("{name}: cannot write: {e}");
                failed = true;
            }
        }
    }
    out.flush().map_err(write_error)?;
    Ok(u8::from(failed || (check_only && changed)))
}

/// Replace the file at `path` with `text` through a temporary file in the same folder and a
/// rename, so a crash leaves the old file or the new one. A symlink is followed, so the link
/// stays; the file keeps its permissions. The temporary file is removed only when this call
/// made it: a name already taken is never touched, and the next name is tried.
fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    let real = fs::canonicalize(path)?;
    let dir = real.parent().unwrap_or(Path::new("."));
    let name = real.file_name().unwrap_or_default().to_string_lossy();
    let permissions = fs::metadata(&real)?.permissions();
    for n in 0..1000 {
        let tmp = dir.join(format!(".{name}.jsonvet-{}-{n}.tmp", std::process::id()));
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        let written = file
            .write_all(text.as_bytes())
            .and_then(|()| file.sync_all());
        // Closed before the rename, which Windows refuses for an open file.
        drop(file);
        let done = written
            .and_then(|()| fs::set_permissions(&tmp, permissions.clone()))
            .and_then(|()| fs::rename(&tmp, &real));
        if done.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        return done;
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free temporary file name",
    ))
}

#[cfg(test)]
mod tests {
    use super::{options, write_atomic};
    use crate::editorconfig::Style;
    use crate::settings::FormatSettings;
    use jsonvet::Indent;

    #[test]
    fn settings_beat_editorconfig_beat_defaults() {
        let style = Style {
            indent: Some(Indent::Tab),
            width: Some(100),
        };
        let settings = FormatSettings {
            indent: None,
            width: Some(60),
            normalize_json5: Some(true),
        };
        let got = options(&settings, style);
        assert_eq!(
            (got.indent, got.width, got.normalize_json5),
            (Indent::Tab, 60, true)
        );
        let got = options(&FormatSettings::default(), Style::default());
        assert_eq!(
            (got.indent, got.width, got.normalize_json5),
            (Indent::Spaces(2), 80, false)
        );
    }

    #[test]
    fn a_taken_temporary_name_is_left_alone() {
        let dir = std::env::temp_dir().join(format!("jsonvet-fmt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a directory");
        let file = dir.join("a.json");
        std::fs::write(&file, "[1]").expect("written");
        let taken = dir.join(format!(".a.json.jsonvet-{}-0.tmp", std::process::id()));
        std::fs::write(&taken, "mine").expect("written");
        write_atomic(&file, "[2]\n").expect("replaced");
        assert_eq!(std::fs::read_to_string(&file).expect("read"), "[2]\n");
        assert_eq!(std::fs::read_to_string(&taken).expect("read"), "mine");
        let left = std::fs::read_dir(&dir).expect("listing").count();
        assert_eq!(left, 2, "only the file and the taken name remain");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
