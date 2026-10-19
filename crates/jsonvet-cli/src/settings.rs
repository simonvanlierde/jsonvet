//! What to check for each file: the dialect patterns and rule severities from the config file
//! and the command line, in that order, so the command line wins.

use std::fs;
use std::path::{Path, PathBuf};

use jsonvet::{Config, Dialect, Rule, Severity};
use toml::Spanned;
use toml::Table;
use toml::de::{DeTable, DeValue};

use crate::glob;
use crate::output::printable;

/// A dialect for files matching a pattern, or for every file when the pattern is `None`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DialectPattern {
    pub(crate) files: Option<String>,
    /// `None` means `auto`: the default for the file's extension.
    pub(crate) dialect: Option<Dialect>,
    /// The absolute directory of the config file the pattern comes from, which `files` is
    /// relative to. `None` for the command line, whose patterns match the path as given.
    pub(crate) base: Option<String>,
}

impl DialectPattern {
    /// Parse `DIALECT` or `DIALECT:GLOB`, where `DIALECT` may be `auto`.
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        let (name, files) = match value.split_once(':') {
            Some((name, files)) => (name, Some(files.to_owned())),
            None => (value, None),
        };
        Ok(DialectPattern {
            files,
            dialect: parse_dialect(name)?,
            base: None,
        })
    }
}

fn parse_dialect(name: &str) -> Result<Option<Dialect>, String> {
    if name == "auto" {
        return Ok(None);
    }
    name.parse()
        .map(Some)
        .map_err(|e: jsonvet::UnknownDialect| format!("{e}, or auto"))
}

pub(crate) fn parse_severity(name: &str) -> Result<Severity, String> {
    match name {
        "error" => Ok(Severity::Error),
        "warning" | "warn" => Ok(Severity::Warning),
        "ignore" | "off" => Ok(Severity::Off),
        _ => Err(format!(
            "unknown severity `{name}`{}; expected error, warning or ignore",
            jsonvet::did_you_mean(name, ["error", "warning", "warn", "ignore", "off"])
        )),
    }
}

/// A rule id from the command line or a settings file. `schema` is no [`Rule`]: its errors
/// cannot be turned down, so it gets its own message rather than "unknown rule".
pub(crate) fn parse_rule(id: &str) -> Result<Rule, String> {
    if id == crate::sarif::SCHEMA {
        return Err("the `schema` rule is always an error and cannot be set".to_owned());
    }
    id.parse().map_err(|e| format!("{e}"))
}

/// Dialect patterns and rule severities, merged from every source in order.
#[derive(Clone, Debug, Default)]
pub(crate) struct Settings {
    pub(crate) dialects: Vec<DialectPattern>,
    pub(crate) rules: Vec<(Rule, Severity)>,
}

impl Settings {
    /// Add `later` after `self`, so `later` wins where both say something.
    pub(crate) fn overridden_by(mut self, later: Settings) -> Settings {
        self.dialects.extend(later.dialects);
        self.rules.extend(later.rules);
        self
    }

    /// The checking config for one file: the last matching dialect pattern wins.
    pub(crate) fn config_for(&self, path: &str) -> Config {
        let auto = || Dialect::for_path(Path::new(path)).unwrap_or(Dialect::Json5);
        // Found only when a config file's pattern needs it: it asks the system for the working
        // directory each time.
        let absolute = std::cell::OnceCell::new();
        let matches = |rule: &DialectPattern| match (&rule.files, &rule.base) {
            (None, _) => true,
            (Some(pattern), None) => glob::matches(pattern, path),
            // A file outside the config file's directory matches none of its patterns.
            (Some(pattern), Some(base)) => absolute
                .get_or_init(|| std::path::absolute(path).ok())
                .as_deref()
                .map(Path::to_string_lossy)
                .and_then(|absolute| glob::relative(&absolute, base))
                .is_some_and(|relative| glob::matches(pattern, &relative)),
        };
        let dialect = self
            .dialects
            .iter()
            .rev()
            .find(|rule| matches(rule))
            .and_then(|rule| rule.dialect)
            .unwrap_or_else(auto);
        self.config(dialect)
    }

    /// The checking config for `dialect`, with the rule severities set.
    pub(crate) fn config(&self, dialect: Dialect) -> Config {
        self.rules
            .iter()
            .fold(Config::new(dialect), |config, &(rule, severity)| {
                config.rule(rule, severity)
            })
    }
}

/// The config file for a run started in `start`: the first `jsonvet.toml`, or `pyproject.toml`
/// with a `[tool.jsonvet]` table, in `start` and its parents. The search stops at the first
/// directory with a `.git` entry, so a file above the repository is never used.
pub(crate) fn discover(start: &Path) -> Option<PathBuf> {
    for dir in start.ancestors() {
        let own = dir.join("jsonvet.toml");
        if own.is_file() {
            return Some(own);
        }
        let pyproject = dir.join("pyproject.toml");
        if fs::read_to_string(&pyproject).is_ok_and(|text| has_jsonvet_table(&text)) {
            return Some(pyproject);
        }
        if dir.join(".git").exists() {
            return None;
        }
    }
    None
}

/// Whether a `pyproject.toml` has a `[tool.jsonvet]` table, valid or not: `load` then reports
/// what is wrong with it, rather than the search skipping a table with a typo.
fn has_jsonvet_table(text: &str) -> bool {
    match text.parse::<Table>() {
        Ok(table) => table
            .get("tool")
            .and_then(|tool| tool.get("jsonvet"))
            .is_some(),
        Err(_) => text.contains("[tool.jsonvet"),
    }
}

/// Read a config file: a `jsonvet.toml`, or the `[tool.jsonvet]` table of a `pyproject.toml`.
/// It holds `rules`, a table of rule id to severity, and `dialects`, an array of tables with a
/// `files` glob, relative to the config file's directory, and a `dialect`. Any other key is an
/// error, so a typo is never ignored.
pub(crate) fn load(path: &Path) -> Result<Settings, String> {
    // A key or value from the file shows escaped, so it cannot start a line of its own on stderr.
    let in_file = |e: &dyn std::fmt::Display| printable(&format!("{}: {e}", path.display()));
    let text = fs::read_to_string(path).map_err(|e| in_file(&e))?;
    // The same, with the line and column of the byte offset `at` after the name.
    let at = |at: usize, e: &dyn std::fmt::Display| {
        let before = &text[..at];
        let line = before.matches('\n').count() + 1;
        let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
        printable(&format!("{}:{line}:{column}: {e}", path.display()))
    };
    let table = DeTable::parse(&text).map_err(|e| match e.span() {
        Some(span) => at(span.start, &e.message()),
        None => in_file(&e.message()),
    })?;
    let table = table.get_ref();
    let file = if path.file_name().is_some_and(|n| n == "pyproject.toml") {
        match table
            .get("tool")
            .and_then(|tool| tool.get_ref().get("jsonvet"))
        {
            Some(file) => match file.get_ref() {
                DeValue::Table(table) => table,
                _ => return Err(at(file.span().start, &"`tool.jsonvet` must be a table")),
            },
            None => return Err(in_file(&"no [tool.jsonvet] table")),
        }
    } else {
        table
    };
    let base = std::path::absolute(path).map_err(|e| in_file(&e))?;
    let base = base.parent().map(|dir| dir.to_string_lossy().into_owned());
    let mut settings = Settings::default();
    for (key, value) in file {
        let here = value.span().start;
        match (key.get_ref().as_ref(), value.get_ref()) {
            ("rules", DeValue::Table(rules)) => {
                for (id, severity) in rules {
                    let rule = parse_rule(id.get_ref()).map_err(|e| at(id.span().start, &e))?;
                    let severity_at = severity.span().start;
                    let severity = severity.get_ref().as_str().ok_or_else(|| {
                        at(
                            severity_at,
                            &format!("`rules.{}` must be a string", id.get_ref()),
                        )
                    })?;
                    let severity = parse_severity(severity).map_err(|e| at(severity_at, &e))?;
                    settings.rules.push((rule, severity));
                }
            }
            ("dialects", DeValue::Array(entries)) => {
                for entry in entries {
                    let mut pattern = dialect_entry(entry).map_err(|(offset, e)| at(offset, &e))?;
                    pattern.base.clone_from(&base);
                    settings.dialects.push(pattern);
                }
            }
            ("rules", _) => return Err(at(here, &"`rules` must be a table")),
            ("dialects", _) => return Err(at(here, &"`dialects` must be an array of tables")),
            (key_text, _) => {
                let hint = jsonvet::did_you_mean(key_text, ["rules", "dialects"]);
                return Err(at(
                    key.span().start,
                    &format!("unknown key `{key_text}`{hint}; expected rules or dialects"),
                ));
            }
        }
    }
    Ok(settings)
}

/// One `dialects` entry: a table with exactly a `files` string and a `dialect` string. An error
/// comes with the offset it is about.
fn dialect_entry(entry: &Spanned<DeValue<'_>>) -> Result<DialectPattern, (usize, String)> {
    let start = entry.span().start;
    let DeValue::Table(table) = entry.get_ref() else {
        return Err((start, "each `dialects` entry must be a table".to_owned()));
    };
    if let Some(key) = table
        .keys()
        .find(|k| !matches!(k.get_ref().as_ref(), "files" | "dialect"))
    {
        let name = key.get_ref();
        let hint = jsonvet::did_you_mean(name, ["files", "dialect"]);
        return Err((
            key.span().start,
            format!("unknown key `{name}`{hint} in a `dialects` entry; expected files and dialect"),
        ));
    }
    let string = |key: &str| {
        let value = table.get(key);
        value
            .and_then(|v| v.get_ref().as_str().map(|s| (v.span().start, s)))
            .ok_or_else(|| {
                let offset = value.map_or(start, |v| v.span().start);
                (
                    offset,
                    format!("each `dialects` entry needs a `{key}` string"),
                )
            })
    };
    let (_, files) = string("files")?;
    let (dialect_at, dialect) = string("dialect")?;
    Ok(DialectPattern {
        files: Some(files.to_owned()),
        dialect: parse_dialect(dialect).map_err(|e| (dialect_at, e))?,
        base: None,
    })
}
