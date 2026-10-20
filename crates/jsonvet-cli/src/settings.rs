//! What to check for each file: the dialect patterns and rule severities from the config file
//! and the command line, in that order, so the command line wins.

use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use jsonvet::{Config, Dialect, Rule, Severity};
use toml::Spanned;
use toml::Table;
use toml::de::{DeTable, DeValue};

use crate::args::ConfigSource;
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

/// Dialect patterns, rule severities and excluded paths, merged from every source in order.
#[derive(Clone, Debug, Default)]
pub(crate) struct Settings {
    pub(crate) dialects: Vec<DialectPattern>,
    pub(crate) rules: Vec<(Rule, Severity)>,
    /// Patterns for paths a directory walk skips, each with the absolute directory of the
    /// config file it comes from, which it is relative to.
    pub(crate) exclude: Vec<(String, String)>,
}

impl Settings {
    /// Add `later` after `self`, so `later` wins where both say something.
    pub(crate) fn overridden_by(mut self, later: Settings) -> Settings {
        self.dialects.extend(later.dialects);
        self.rules.extend(later.rules);
        self.exclude.extend(later.exclude);
        self
    }

    /// Whether a directory walk skips `path`, a file or a folder.
    pub(crate) fn excluded(&self, path: &Path) -> bool {
        let absolute = std::cell::OnceCell::new();
        self.exclude
            .iter()
            .any(|(pattern, base)| in_base(pattern, base, &path.to_string_lossy(), &absolute))
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
            (Some(pattern), Some(base)) => in_base(pattern, base, path, &absolute),
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

/// Whether `pattern` matches `path` relative to the directory `base`. A path outside `base`
/// matches none of its patterns. `absolute` caches `path` made absolute, which asks the system
/// for the working directory.
fn in_base(
    pattern: &str,
    base: &str,
    path: &str,
    absolute: &std::cell::OnceCell<Option<PathBuf>>,
) -> bool {
    absolute
        .get_or_init(|| std::path::absolute(path).ok())
        .as_deref()
        .map(Path::to_string_lossy)
        .and_then(|absolute| glob::relative(&absolute, base))
        .is_some_and(|relative| glob::matches(pattern, &relative))
}

/// The settings for each file: the nearest config file above it, or the one `--config` names,
/// with the command line's settings on top.
pub(crate) struct Resolver {
    /// The command line's settings: all there is when no config file applies.
    cli: Arc<Settings>,
    /// The settings for every file, with `--config` or `--no-config`: then nothing is searched.
    fixed: Option<Arc<Settings>>,
    /// The settings found for each folder searched so far, by its absolute path.
    found: HashMap<PathBuf, Arc<Settings>>,
    /// The working directory, asked for once rather than for each file. Empty unless searching.
    cwd: PathBuf,
    /// The nearest folder at or above the working directory with a `.git` entry, if any.
    repo: Option<PathBuf>,
    /// The settings of the user's own config file, read the first time a search finds no
    /// project config file: `None` until then.
    user: Option<Arc<Settings>>,
    /// The folder of the last file looked up, as given, and its settings. Files come a folder at
    /// a time, so this skips most lookups, which took 0.5 µs a file.
    last: Option<(PathBuf, Arc<Settings>)>,
}

impl Resolver {
    /// Read the file `--config` names, if any, so a bad one fails before any file is checked.
    pub(crate) fn new(config: &ConfigSource, cli: Settings) -> Result<Resolver, String> {
        let (fixed, cwd) = match config {
            ConfigSource::Search => (
                None,
                std::env::current_dir()
                    .map_err(|e| format!("cannot read the working directory: {e}"))?,
            ),
            ConfigSource::None => (Some(Arc::new(cli.clone())), PathBuf::new()),
            ConfigSource::File(path) => (
                Some(Arc::new(load(path)?.overridden_by(cli.clone()))),
                PathBuf::new(),
            ),
        };
        let repo = cwd
            .ancestors()
            .find(|dir| dir.join(".git").exists())
            .map(Path::to_owned);
        Ok(Resolver {
            cli: Arc::new(cli),
            fixed,
            found: HashMap::new(),
            cwd,
            repo,
            user: None,
            last: None,
        })
    }

    /// The settings for the file at `path`, which need not exist: those of the first
    /// `jsonvet.toml`, or `pyproject.toml` with a `[tool.jsonvet]` table, in its folder or a
    /// folder above. The search stops at the first folder with a `.git` entry, so a file above
    /// the repository is never used. With no `.git` above the file, it stops at the working
    /// directory, or for a file outside it, at the file's own folder: a file planted above the
    /// job's folder on a shared machine never applies. With no project config file, the user's
    /// own applies, if any. `-`, standard input, is searched from the working directory.
    pub(crate) fn for_file(&mut self, path: &Path) -> Result<Arc<Settings>, String> {
        if let Some(fixed) = &self.fixed {
            return Ok(Arc::clone(fixed));
        }
        let folder = path.parent().unwrap_or(path);
        if let Some((last, settings)) = &self.last
            && last.as_os_str() == folder.as_os_str()
        {
            return Ok(Arc::clone(settings));
        }
        let settings = self.search(path)?;
        self.last = Some((folder.to_owned(), Arc::clone(&settings)));
        Ok(settings)
    }

    fn search(&mut self, path: &Path) -> Result<Arc<Settings>, String> {
        let path = lexical(&self.cwd.join(path));
        let start = path.parent().unwrap_or(&path);
        // Where the search stops if it meets no `.git` first.
        let last = if self
            .repo
            .as_ref()
            .is_some_and(|repo| start.starts_with(repo))
        {
            None
        } else if start.starts_with(&self.cwd) {
            Some(self.cwd.as_path())
        } else if start.ancestors().any(|dir| dir.join(".git").exists()) {
            None
        } else {
            Some(start)
        };
        let mut searched = Vec::new();
        let mut settings = None;
        for dir in start.ancestors() {
            if let Some(known) = self.found.get(dir) {
                settings = Some(Arc::clone(known));
                break;
            }
            searched.push(dir);
            if let Some(config) = config_in(dir) {
                let own = load(&config)?.overridden_by(Settings::clone(&self.cli));
                settings = Some(Arc::new(own));
                break;
            }
            if Some(dir) == last || dir.join(".git").exists() {
                break;
            }
        }
        let settings = match settings {
            Some(settings) => settings,
            None => self.user()?,
        };
        for dir in searched {
            self.found.insert(dir.to_owned(), Arc::clone(&settings));
        }
        Ok(settings)
    }

    /// The settings of the user's own `jsonvet.toml`, as ruff has, with the command line's on
    /// top; the command line's alone without one.
    fn user(&mut self) -> Result<Arc<Settings>, String> {
        if let Some(user) = &self.user {
            return Ok(Arc::clone(user));
        }
        let user = match user_config() {
            Some(path) => {
                let mut own = load(&path)?;
                // Its globs are relative to the working directory, as in ruff: it sits in no
                // project.
                let cwd = self.cwd.to_string_lossy().into_owned();
                for pattern in &mut own.dialects {
                    pattern.base = Some(cwd.clone());
                }
                for (_, base) in &mut own.exclude {
                    base.clone_from(&cwd);
                }
                Arc::new(own.overridden_by(Settings::clone(&self.cli)))
            }
            None => Arc::clone(&self.cli),
        };
        self.user = Some(Arc::clone(&user));
        Ok(user)
    }
}

/// The user's own config file, if there is one: `jsonvet/jsonvet.toml` in `%APPDATA%` on
/// Windows, and elsewhere in `$XDG_CONFIG_HOME` or `~/.config`, as ruff looks for its own on
/// Linux and macOS. A relative `XDG_CONFIG_HOME` is ignored, as the XDG spec says.
fn user_config() -> Option<PathBuf> {
    let absolute = |name| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
    };
    let dir = if cfg!(windows) {
        absolute("APPDATA")
    } else {
        absolute("XDG_CONFIG_HOME").or_else(|| absolute("HOME").map(|home| home.join(".config")))
    };
    let path = dir?.join("jsonvet").join("jsonvet.toml");
    path.is_file().then_some(path)
}

/// The absolute `path` with `..` folded away by name, as the globs fold it, so the search climbs
/// the folders as written: from `a/../b` it goes to the parent of `b`, not to `a`. `components`
/// already drops each `.` after the first part.
fn lexical(path: &Path) -> PathBuf {
    let mut folded = PathBuf::new();
    for part in path.components() {
        if part == Component::ParentDir {
            folded.pop();
        } else {
            folded.push(part);
        }
    }
    folded
}

/// The config file in `dir`: its `jsonvet.toml`, or its `pyproject.toml` with a
/// `[tool.jsonvet]` table.
fn config_in(dir: &Path) -> Option<PathBuf> {
    let own = dir.join("jsonvet.toml");
    if own.is_file() {
        return Some(own);
    }
    let pyproject = dir.join("pyproject.toml");
    // One that is not a regular file is skipped, as a `jsonvet.toml` is; one that is too large
    // is left for `load` to report.
    match crate::read_small(&pyproject, crate::MAX_CONFIG_BYTES) {
        Ok(text) => has_jsonvet_table(&text).then_some(pyproject),
        Err(e) if e.kind() == std::io::ErrorKind::Other => pyproject.is_file().then_some(pyproject),
        Err(_) => None,
    }
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
/// `files` glob, relative to the config file's directory, and a `dialect`; and `exclude`, an
/// array of globs, relative to the same directory, for paths a directory walk skips. Any other
/// key is an error, so a typo is never ignored.
pub(crate) fn load(path: &Path) -> Result<Settings, String> {
    // A key or value from the file shows escaped, so it cannot start a line of its own on stderr.
    let in_file = |e: &dyn std::fmt::Display| printable(&format!("{}: {e}", path.display()));
    let text = crate::read_small(path, crate::MAX_CONFIG_BYTES).map_err(|e| in_file(&e))?;
    // A symlink can lead to any file on the machine, such as a secret that a pull request links
    // to, so an error in one names the place only, never a key or value from it.
    let linked = fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink());
    let hidden = "invalid setting, not shown because the file is a symlink";
    // The same, with the line and column of the byte offset `at` after the name.
    let at = |at: usize, e: &dyn std::fmt::Display| {
        let before = &text[..at];
        let line = before.matches('\n').count() + 1;
        let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
        let e: &dyn std::fmt::Display = if linked { &hidden } else { e };
        printable(&format!("{}:{line}:{column}: {e}", path.display()))
    };
    let table = DeTable::parse(&text)
        .map_err(|e| at(e.span().map_or(0, |span| span.start), &e.message()))?;
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
            ("exclude", DeValue::Array(patterns)) => {
                for pattern in patterns {
                    let text = pattern.get_ref().as_str().ok_or_else(|| {
                        at(
                            pattern.span().start,
                            &"`exclude` must be an array of strings",
                        )
                    })?;
                    // Without a base there is nothing for the pattern to be relative to.
                    if let Some(base) = &base {
                        settings.exclude.push((text.to_owned(), base.clone()));
                    }
                }
            }
            ("exclude", _) => return Err(at(here, &"`exclude` must be an array of strings")),
            ("rules", _) => return Err(at(here, &"`rules` must be a table")),
            ("dialects", _) => return Err(at(here, &"`dialects` must be an array of tables")),
            (key_text, _) => {
                let hint = jsonvet::did_you_mean(key_text, ["rules", "dialects", "exclude"]);
                return Err(at(
                    key.span().start,
                    &format!("unknown key `{key_text}`{hint}; expected rules, dialects or exclude"),
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
