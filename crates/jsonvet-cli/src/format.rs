//! `jsonvet fmt`: format files in place, or with `--check` list the ones that would change.

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::Path;
use std::sync::Mutex;

use jsonvet::{Config, FormatOptions, Rule};

use crate::args::STDIN;
use crate::editorconfig;
use crate::messages;
use crate::output::printable;
use crate::settings::{FormatSettings, Resolver};
use crate::write::write_atomically;
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

/// The dialect and rules, and the layout, that `jsonvet fmt` uses for the file at `path`.
pub(crate) fn setup(
    path: &Path,
    resolver: &Mutex<Resolver>,
    styles: &editorconfig::Cache,
) -> Result<(Config, FormatOptions), String> {
    let (settings, dirs) = {
        let mut resolver = lock(resolver);
        (resolver.for_file(path)?, resolver.search_dirs(path))
    };
    let config = settings.config_for(&path.to_string_lossy());
    let style = styles.style(path, &dirs);
    Ok((
        config,
        options(
            &settings.format_for(&path.to_string_lossy()),
            settings.user,
            style,
        ),
    ))
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

/// The catalog text for a file the formatter refused. Sizes are in decimal units,
/// rounded to a tenth.
fn refusal(error: &jsonvet::FormatError<'_>) -> String {
    match error {
        jsonvet::FormatError::InputTooLarge => messages::text("fmt.input-too-large").to_owned(),
        jsonvet::FormatError::SuppressionTargets => {
            messages::text("fmt.suppression-targets").to_owned()
        }
        jsonvet::FormatError::OutputTooLarge { bytes } => {
            let (unit, tenths) = if *bytes >= 1_000_000_000 {
                ("GB", (bytes + 50_000_000) / 100_000_000)
            } else {
                ("MB", (bytes + 50_000) / 100_000)
            };
            let size = format!("{}.{} {unit}", tenths / 10, tenths % 10);
            messages::fill("fmt.output-too-large", &[("size", &size)])
        }
        other => other.to_string(),
    }
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
    let (config, options) = setup(path, resolver, styles)?;
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
                    printable(&messages::problem(problem))
                ));
            }
            done.failed = true;
            return Ok(done);
        }
        Err(other) => {
            done.problems.push(format!("{name}: {}", refusal(&other)));
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
        } else if let Err(message) = write_atomically(path, &formatted) {
            done.problems.push(format!("{name}: {message}"));
            done.failed = true;
        }
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::{options, refusal};
    use crate::editorconfig::Style;
    use crate::settings::FormatSettings;
    use jsonvet::{FormatOptions, Indent};

    #[test]
    fn suppression_refusal_uses_the_translation() {
        crate::messages::use_translation(
            "\"fmt.suppression-targets\" = \"suppression coverage would change\"",
        );
        assert_eq!(
            refusal(&jsonvet::FormatError::SuppressionTargets),
            "suppression coverage would change"
        );
    }

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
}
