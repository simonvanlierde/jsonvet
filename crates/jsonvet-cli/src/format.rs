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
use crate::{lock, parallel, read, read_stdin};

/// The config's setting, or the editorconfig's where it has none; the other way round for the
/// user's own config, which yields to the editorconfig.
fn pick<T>(own: Option<T>, editor: Option<T>, user: bool) -> Option<T> {
    if user { editor.or(own) } else { own.or(editor) }
}

/// The layout settings for one file: the defaults, then the user's own config file, then its
/// `.editorconfig` files, then the `[format]` table of its project's config file. A setting
/// from the project wins everywhere, so `--check` agrees on every machine.
fn options(settings: &FormatSettings, user: bool, style: editorconfig::Style) -> FormatOptions {
    let defaults = FormatOptions::default();
    FormatOptions {
        indent: pick(settings.indent, style.indent, user).unwrap_or(defaults.indent),
        width: pick(settings.width, style.width, user).unwrap_or(defaults.width),
        normalize_json5: settings.normalize_json5.unwrap_or(defaults.normalize_json5),
        bracket_spacing: settings.bracket_spacing.unwrap_or(defaults.bracket_spacing),
    }
}

/// What formatting one file gave: the lines for standard error, the text for `out`, and
/// whether the file failed or would change.
#[derive(Default)]
struct Done {
    problems: Vec<String>,
    out: String,
    failed: bool,
    changed: bool,
}

/// Format each of `files` on all the cores, naming standard input `stdin_name`.
/// Standard input goes to `out`, and with `check_only` nothing is written but the names of the
/// files that would change. Problems go to standard error. Everything is printed in the order
/// of `files`. Exit status 1 for a file that cannot be read or has a syntax error, and with
/// `check_only` for a file that would change.
pub(crate) fn run(
    files: &[OsString],
    check_only: bool,
    stdin_name: Option<OsString>,
    resolver: &Mutex<Resolver>,
    mut out: impl Write,
) -> Result<u8, String> {
    let stdin_name = stdin_name.unwrap_or_else(|| STDIN.into());
    let write_error = |e: io::Error| messages::fill("error.write", &[("error", &e)]);
    let styles = editorconfig::Cache::default();
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let done = parallel(
        files,
        threads,
        || false,
        |file| one(file, &stdin_name, check_only, resolver, &styles),
    );
    let (mut failed, mut changed) = (false, false);
    for done in done {
        let done = done?;
        for problem in &done.problems {
            eprintln!("{problem}");
        }
        out.write_all(done.out.as_bytes()).map_err(write_error)?;
        failed |= done.failed;
        changed |= done.changed;
    }
    out.flush().map_err(write_error)?;
    Ok(u8::from(failed || (check_only && changed)))
}

/// Format one of the files; an `Err` is a problem with the config, which ends the run.
fn one(
    file: &OsString,
    stdin_name: &OsString,
    check_only: bool,
    resolver: &Mutex<Resolver>,
    styles: &editorconfig::Cache,
) -> Result<Done, String> {
    let mut done = Done::default();
    let stdin = file == STDIN;
    let shown = if stdin { stdin_name } else { file };
    let name = printable(&shown.to_string_lossy());
    let text = if stdin {
        read_stdin()
    } else {
        read(Path::new(file))
    };
    let text = match text {
        Ok(text) => text,
        Err(message) => {
            done.problems.push(format!("{name}: {message}"));
            done.failed = true;
            return Ok(done);
        }
    };
    let path = Path::new(shown);
    let (settings, dirs) = {
        let mut resolver = lock(resolver);
        (resolver.for_file(path)?, resolver.search_dirs(path))
    };
    let config = settings.config_for(&shown.to_string_lossy());
    let style = styles.style(path, &dirs);
    let options = options(&settings.format, settings.user, style);
    let formatted = match jsonvet::format(&text, &config, &options) {
        Ok(formatted) => formatted,
        Err(jsonvet::FormatError::Syntax(report)) => {
            for problem in report
                .problems()
                .iter()
                .filter(|p| p.rule() == Rule::Syntax)
            {
                let at = report.location(problem.offset());
                done.problems.push(format!(
                    "{name}:{}:{}: {}",
                    at.line,
                    at.column,
                    printable(&problem.to_string())
                ));
            }
            done.failed = true;
            return Ok(done);
        }
        Err(other) => {
            done.problems.push(format!("{name}: {other}"));
            done.failed = true;
            return Ok(done);
        }
    };
    if stdin && !check_only {
        done.out = formatted;
    } else if formatted != text {
        drop(text);
        done.changed = true;
        if check_only {
            done.out = format!("{name}\n");
        } else if let Err(message) = write_atomic(path, &formatted) {
            done.problems.push(format!("{name}: {message}"));
            done.failed = true;
        }
    }
    Ok(done)
}

/// Replace the file at `path` with `text` through a temporary file in the same folder and a
/// rename, so a crash leaves the old file or the new one. A symlink is followed, so the link
/// stays; the file keeps its permissions. The temporary file is removed only when this call
/// made it: a name already taken is never touched, and the next name is tried.
fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let cannot = |e: io::Error| format!("cannot write: {e}");
    let real = fs::canonicalize(path).map_err(cannot)?;
    let permissions = fs::metadata(&real).map_err(cannot)?.permissions();
    // The rename would replace a file its owner made read-only, so say no first.
    if permissions.readonly() {
        return Err("the file is read-only".to_owned());
    }
    let dir = real.parent().unwrap_or(Path::new("."));
    let name = real.file_name().unwrap_or_default().to_string_lossy();
    for n in 0..1000 {
        let tmp = dir.join(format!(".{name}.jsonvet-{}-{n}.tmp", std::process::id()));
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(cannot(e)),
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
        return done.map_err(cannot);
    }
    Err("cannot write: no free temporary file name".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{options, write_atomic};
    use crate::editorconfig::Style;
    use crate::settings::FormatSettings;
    use jsonvet::{FormatOptions, Indent};

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
            bracket_spacing: None,
        };
        let got = options(&settings, false, style);
        assert_eq!((got.indent, got.width), (Indent::Tab, 60));
        // The user's config yields to the editorconfig.
        let got = options(&settings, true, style);
        assert_eq!((got.indent, got.width), (Indent::Tab, 100));
        // Settings the config leaves out fall back to the defaults, whoever asks.
        assert_eq!(
            got.bracket_spacing,
            FormatOptions::default().bracket_spacing
        );
        assert!(got.normalize_json5);
        let got = options(&FormatSettings::default(), false, Style::default());
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
