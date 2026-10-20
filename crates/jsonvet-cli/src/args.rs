//! The command line.

use std::ffi::OsString;
use std::path::PathBuf;

use lexopt::prelude::*;

use crate::output::Format;
use crate::settings::{DialectPattern, Settings, parse_rule, parse_severity};

pub(crate) const HELP: &str = "\
Check JSON, I-JSON, JSONC, JWCC (HuJSON), JSON5 and JSON Lines files for syntax errors,
duplicate keys and the rules of each dialect.

Usage: jsonvet [OPTIONS] [FILE]...

A FILE of - reads standard input. A directory stands for the files under it with an extension
or name jsonvet checks by default, skipping hidden files, what .gitignore and .ignore name, and
the exclude setting. A file named on the command line is always checked.

Options:
      --dialect DIALECT[:GLOB]  Check files matching GLOB (every file, without one) as DIALECT:
                                json, i-json, jsonc, jwcc, json5, jsonl, or auto for the file
                                extension's default. Also accepted: ijson, hujson (jwcc),
                                ndjson and json-lines (jsonl). Repeatable; the last match wins.
                                Files with any other extension are checked as json5;
                                extensions match in lower case only.
      --strict GLOB             Same as --dialect json:GLOB; --strict '!GLOB' means auto:GLOB.
      --error RULE              Report RULE as an error. Repeatable, as are the next two.
      --warn RULE               Report RULE as a warning.
      --ignore RULE             Do not check RULE.
      --max-warnings N          Fail when there are more than N warnings.
      --format FORMAT           Print problems as text (the default), as one SARIF 2.1.0
                                document, for GitHub code scanning, or as one JSON document
                                (its fields: docs/usage.md in the repository).
      --schema FILE             Also check each file against the JSON Schema in FILE, reported
                                under the rule schema. Limits: docs/usage.md in the repository.
      --stdin-filename PATH     Name standard input PATH in the output, and pick its dialect
                                and settings by PATH. With no FILE, check standard input.
      --config FILE             Read settings from FILE instead of searching for one.
      --no-config               Do not read a settings file.
      --list-rules              List the rules, with their default severities, and exit.
      --explain RULE            Explain RULE, with an example, and exit.
  -h, --help                    Print this help.
  -V, --version                 Print the version.

Rules: syntax, duplicate-key, invalid-number, invalid-code-point, byte-order-mark,
missing-final-newline (--list-rules says more). With --schema, errors also come under the rule
schema; they are always errors, and --error, --warn, --ignore and settings files do not take
schema.

Settings come from jsonvet.toml, or [tool.jsonvet] in pyproject.toml: for each file, the nearest
one in its folder or a parent up to the repository root. Options on the command line win.

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
    /// The name standard input goes by, from `--stdin-filename`.
    pub(crate) stdin_filename: Option<OsString>,
}

#[derive(Debug)]
pub(crate) enum Command {
    Check(Args),
    Help,
    Version,
    ListRules,
    /// `--explain`'s text for the rule.
    Explain(String),
}

fn parse_format(name: &str) -> Result<Format, String> {
    match name {
        "text" => Ok(Format::Text),
        "sarif" => Ok(Format::Sarif),
        "json" => Ok(Format::Json),
        _ => {
            let hint = jsonvet::did_you_mean(name, ["text", "sarif", "json"]);
            Err(format!(
                "--format takes text, sarif or json, not `{name}`{hint}"
            ))
        }
    }
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
    let mut stdin_filename = None;
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
                let rule = parse_rule(&value(&mut parser)?)?;
                settings.rules.push((rule, severity));
            }
            Long("max-warnings") => {
                let n = value(&mut parser)?;
                max_warnings = Some(
                    n.parse()
                        .map_err(|_| format!("--max-warnings takes a number, not `{n}`"))?,
                );
            }
            Long("format") => format = parse_format(&value(&mut parser)?)?,
            Long("config") => config = ConfigSource::File(PathBuf::from(value(&mut parser)?)),
            Long("no-config") => config = ConfigSource::None,
            Long("list-rules") => return Ok(Command::ListRules),
            Long("explain") => {
                return crate::rules::explain(&value(&mut parser)?).map(Command::Explain);
            }
            Long("schema") => schema = Some(PathBuf::from(value(&mut parser)?)),
            Long("stdin-filename") => {
                stdin_filename = Some(parser.value().map_err(|e| e.to_string())?);
            }
            // pre-commit puts file names after the options with no `--`, so an existing file
            // named like `-x.json`, `-h.json` or `--y.json` is a file, not an option.
            arg => {
                let (mut file, separator) = match arg {
                    Short(c) => (OsString::from(format!("-{c}")), ""),
                    Long(name) => (OsString::from(format!("--{name}")), "="),
                    Value(file) => {
                        files.push(file);
                        continue;
                    }
                };
                let rest = rest(&mut parser);
                if rest.is_empty() {
                    match file.to_str() {
                        Some("-h" | "--help") => return Ok(Command::Help),
                        Some("-V" | "--version") => return Ok(Command::Version),
                        _ => {}
                    }
                } else {
                    file.push(separator);
                    file.push(rest);
                }
                files.push(on_disk(file)?);
            }
        }
    }
    if files.iter().filter(|file| *file == STDIN).count() > 1 {
        return Err("`-` (standard input) can be given only once".to_owned());
    }
    if files.is_empty() && stdin_filename.is_some() {
        files.push(STDIN.into());
    }
    Ok(Command::Check(Args {
        files,
        settings,
        max_warnings,
        config,
        format,
        schema,
        stdin_filename,
    }))
}

/// The file name that stands for standard input.
pub(crate) const STDIN: &str = "-";

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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

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
        let wide: Vec<&str> = HELP.lines().filter(|l| l.chars().count() > 96).collect();
        assert!(wide.is_empty(), "help lines over 96 columns: {wide:?}");
    }

    /// usage.md's option list names the options `--help` does, and the parser knows each one.
    /// Their wording differs, so only the names are compared. `--help` and `--version` need no
    /// line in usage.md.
    #[test]
    fn usage_md_lists_the_options_help_does() {
        let usage = include_str!("../../../docs/usage.md").replace("\r\n", "\n");
        let block = |text: &str, start: &str, end: &str| {
            let (_, rest) = text.split_once(start).expect(start);
            rest.split_once(end).expect(end).0.to_owned()
        };
        let options = |text: &str| -> BTreeSet<String> {
            text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                .filter(|word| word.len() > 2 && word.starts_with("--"))
                .map(ToOwned::to_owned)
                .collect()
        };
        let mut help = options(&block(HELP, "\nOptions:\n", "\n\nRules:"));
        for option in &help {
            let error = parse([option.as_str()].iter().map(Into::into)).err();
            assert_ne!(error, Some(format!("unknown option `{option}`")));
        }
        help.retain(|option| option != "--help" && option != "--version");
        let docs = options(&block(&usage, "## Options\n\n```text\n", "```"));
        assert_eq!(help, docs, "options in --help and in docs/usage.md");
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
        assert_eq!(check(&["--format=text", "x.json"]).format, Format::Text);
        assert_eq!(check(&["--format", "json", "x.json"]).format, Format::Json);
        assert!(parse(["--format", "jsonl"].iter().map(Into::into)).is_err());
        // Value names are case-sensitive, as in clap and ruff, but a wrong case gets a hint.
        assert_eq!(
            parse(["--format", "SARIF"].iter().map(Into::into)).err(),
            Some(
                "--format takes text, sarif or json, not `SARIF` (did you mean `sarif`?)"
                    .to_owned()
            )
        );
        assert!(parse(["--error", "nope"].iter().map(Into::into)).is_err());
        assert!(parse(["--dialect", "yaml"].iter().map(Into::into)).is_err());
        assert!(parse(["--strict"].iter().map(Into::into)).is_err());
    }
}
