//! The command line.

use std::ffi::OsString;
use std::path::PathBuf;

use jsonvet::Rule;
use lexopt::prelude::*;

use crate::output::Format;
use crate::settings::{DialectPattern, Settings, parse_severity};

pub(crate) const HELP: &str = "\
Check JSON, I-JSON, JSONC, JWCC (HuJSON), JSON5 and JSON Lines files for syntax errors,
duplicate keys and the rules of each dialect.

Usage: jsonvet [OPTIONS] [FILE]...

Options:
      --dialect DIALECT[:GLOB]  Check files matching GLOB (every file, without one) as DIALECT:
                                json, i-json, jsonc, jwcc, json5, jsonl, or auto for the file
                                extension's default. Repeatable; the last match wins.
                                Files with any other extension are checked as json5;
                                extensions match in lower case only.
      --strict GLOB             Same as --dialect json:GLOB; --strict '!GLOB' means auto:GLOB.
      --error RULE              Report RULE as an error. Repeatable, as are the next two.
      --warn RULE               Report RULE as a warning.
      --ignore RULE             Do not check RULE.
      --max-warnings N          Fail when there are more than N warnings.
      --format FORMAT           Print problems as text (the default) or as one SARIF 2.1.0
                                document, for GitHub code scanning.
      --schema FILE             Also check each file against the JSON Schema in FILE, reported
                                under the rule schema. Not in the PyPI wheels; `cargo install
                                jsonvet-cli` has it. Limits: see the README.
      --config FILE             Read settings from FILE instead of searching for one.
      --no-config               Do not read a settings file.
  -h, --help                    Print this help.
  -V, --version                 Print the version.

Rules: syntax, duplicate-key, invalid-number, invalid-code-point, byte-order-mark,
missing-final-newline. With --schema, errors also come under the rule schema; they are always
errors, and --error, --warn, --ignore and settings files do not take schema.

Settings come from jsonvet.toml, or [tool.jsonvet] in pyproject.toml, found in the working
directory or a parent up to the repository root. Options on the command line win.

Exit status: 0 when no file has an error, 1 when one does, 2 for a usage error, such as a
schema that cannot be read or used.";

/// Where settings come from besides the command line.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ConfigSource {
    Search,
    File(PathBuf),
    None,
}

#[derive(Debug)]
pub(crate) struct Args {
    pub(crate) files: Vec<OsString>,
    pub(crate) settings: Settings,
    pub(crate) max_warnings: Option<usize>,
    pub(crate) config: ConfigSource,
    pub(crate) format: Format,
    pub(crate) schema: Option<PathBuf>,
}

#[derive(Debug)]
pub(crate) enum Command {
    Check(Args),
    Help,
    Version,
}

pub(crate) fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let mut parser = lexopt::Parser::from_args(args);
    // Otherwise `-x=a.json` would come back as `-x` and `a.json`, losing the `=`.
    parser.set_short_equals(false);
    let mut files = Vec::new();
    let mut settings = Settings::default();
    let mut max_warnings = None;
    let mut config = ConfigSource::Search;
    let mut format = Format::Text;
    let mut schema = None;
    loop {
        // lexopt reads a non-UTF-8 argument that starts with `-` lossily. No option has such a
        // name, so it is a file, kept as given. An existing file named like `--dialect=json`
        // is a file too, as pre-commit puts file names after the options with no `--`.
        if let Some(file) = parser.try_raw_args().and_then(|mut raw| {
            raw.next_if(|arg| match arg.to_str() {
                None => true,
                Some(arg) => arg.starts_with("--") && arg.contains('=') && exists(arg),
            })
        }) {
            files.push(file);
            continue;
        }
        let Some(arg) = parser.next().map_err(|e| e.to_string())? else {
            break;
        };
        match arg {
            Long("dialect") => settings
                .dialects
                .push(DialectPattern::parse(&value(&mut parser)?)?),
            Long("strict") => {
                let glob = value(&mut parser)?;
                settings.dialects.push(match glob.strip_prefix('!') {
                    Some(glob) => DialectPattern::parse(&format!("auto:{glob}"))?,
                    None => DialectPattern::parse(&format!("json:{glob}"))?,
                });
            }
            Long(severity @ ("error" | "warn" | "ignore")) => {
                let severity = parse_severity(severity)?;
                let rule: Rule = value(&mut parser)?.parse().map_err(|e| format!("{e}"))?;
                settings.rules.push((rule, severity));
            }
            Long("max-warnings") => {
                let n = value(&mut parser)?;
                max_warnings = Some(
                    n.parse()
                        .map_err(|_| format!("--max-warnings takes a number, not `{n}`"))?,
                );
            }
            Long("format") => {
                format = match value(&mut parser)?.as_str() {
                    "text" => Format::Text,
                    "sarif" => Format::Sarif,
                    other => return Err(format!("--format takes text or sarif, not `{other}`")),
                }
            }
            Long("config") => config = ConfigSource::File(PathBuf::from(value(&mut parser)?)),
            Long("no-config") => config = ConfigSource::None,
            Long("schema") => schema = Some(PathBuf::from(value(&mut parser)?)),
            Value(file) => files.push(file),
            // pre-commit puts file names after the options with no `--`, so an existing file
            // named like `-x.json`, `-h.json` or `--y.json` is a file, not an option.
            Short(c) => {
                let rest = rest(&mut parser);
                let file = match (c, rest.is_empty()) {
                    ('h', true) => return Ok(Command::Help),
                    ('V', true) => return Ok(Command::Version),
                    _ => joined(&format!("-{c}"), "", &rest),
                };
                files.push(on_disk(file)?);
            }
            Long(name) => {
                let name = name.to_owned();
                let rest = rest(&mut parser);
                let file = match (name.as_str(), rest.is_empty()) {
                    ("help", true) => return Ok(Command::Help),
                    ("version", true) => return Ok(Command::Version),
                    (_, true) => format!("--{name}").into(),
                    _ => joined(&format!("--{name}"), "=", &rest),
                };
                files.push(on_disk(file)?);
            }
        }
    }
    Ok(Command::Check(Args {
        files,
        settings,
        max_warnings,
        config,
        format,
        schema,
    }))
}

fn exists(path: impl AsRef<std::path::Path>) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// An argument jsonvet has no option for: a file if one has that name, or else a mistyped
/// option, such as `--strcit` or `-q`.
fn on_disk(file: OsString) -> Result<OsString, String> {
    if exists(&file) {
        Ok(file)
    } else {
        Err(format!("unknown option `{}`", file.to_string_lossy()))
    }
}

fn value(parser: &mut lexopt::Parser) -> Result<String, String> {
    parser
        .value()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|v| format!("not valid UTF-8: {}", v.to_string_lossy()))
}

/// The rest of the current argument after an unknown option: the remaining letters of
/// `-dash.json`, or the value of `--name=value`.
fn rest(parser: &mut lexopt::Parser) -> OsString {
    parser.optional_value().unwrap_or_default()
}

fn joined(option: &str, separator: &str, rest: &OsString) -> OsString {
    let mut name = OsString::from(option);
    name.push(separator);
    name.push(rest);
    name
}

#[cfg(test)]
mod tests {
    use super::{Command, ConfigSource, Format, HELP, parse};
    use jsonvet::{Dialect, Rule, Severity};

    #[test]
    fn help_names_every_rule_and_dialect() {
        let words: Vec<&str> = HELP
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .collect();
        let missing: Vec<String> = Rule::ALL
            .iter()
            .map(|rule| rule.id().to_owned())
            .chain(Dialect::ALL.iter().map(ToString::to_string))
            .filter(|name| !words.contains(&name.as_str()))
            .collect();
        assert!(missing.is_empty(), "not in the help: {missing:?}");
    }

    fn check(args: &[&str]) -> super::Args {
        match parse(args.iter().map(Into::into)) {
            Ok(Command::Check(args)) => args,
            other => panic!("expected a check, got {other:?}"),
        }
    }

    #[test]
    fn files_that_look_like_options_stay_files() {
        let args = check(&["--", "-dash.json", "--y.json", "--dialect=json", "a.json"]);
        assert_eq!(
            args.files,
            ["-dash.json", "--y.json", "--dialect=json", "a.json"]
        );
        // An unknown `-x` or `--name`, or a known `--name=value`, is a file only when it exists;
        // tests/cli.rs covers that case.
        for unknown in [
            "--strcit",
            "--help=x",
            "--x.json",
            "-q",
            "-dash.json",
            "-h.json",
            "-x=a.json",
        ] {
            assert_eq!(
                parse(["a.json", unknown].iter().map(Into::into)).err(),
                Some(format!("unknown option `{unknown}`"))
            );
        }
        assert!(matches!(
            parse(["a.json", "-h"].iter().map(Into::into)),
            Ok(Command::Help)
        ));
        assert!(matches!(
            parse(["--version"].iter().map(Into::into)),
            Ok(Command::Version)
        ));
    }

    #[test]
    fn rule_limit_and_config_flags_parse_and_bad_values_fail() {
        let args = check(&[
            "--warn=duplicate-key",
            "--max-warnings",
            "3",
            "--no-config",
            "--format=sarif",
            "x.jsonc",
        ]);
        assert_eq!(
            args.settings.rules,
            [(Rule::DuplicateKey, Severity::Warning)]
        );
        assert_eq!(args.max_warnings, Some(3));
        assert_eq!(args.config, ConfigSource::None);
        assert_eq!(args.format, Format::Sarif);
        assert_eq!(check(&["x.json"]).format, Format::Text);
        assert!(parse(["--format", "json"].iter().map(Into::into)).is_err());
        assert!(parse(["--error", "nope"].iter().map(Into::into)).is_err());
        assert!(parse(["--dialect", "yaml"].iter().map(Into::into)).is_err());
        assert!(parse(["--strict"].iter().map(Into::into)).is_err());
    }
}
