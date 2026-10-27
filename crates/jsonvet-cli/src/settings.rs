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
use crate::glob::{self, Glob, Target};
use crate::output::printable;

/// A dialect for files matching a pattern, or for every file when the pattern is `None`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DialectPattern {
    pub(crate) files: Option<Glob>,
    /// `None` means `auto`: the default for the file's extension.
    pub(crate) dialect: Option<Dialect>,
    /// The absolute directory of the config file the pattern comes from, normalized, which
    /// `files` is relative to. `None` for the command line, whose patterns match the path as
    /// given.
    pub(crate) base: Option<String>,
}

impl DialectPattern {
    /// Parse `DIALECT` or `DIALECT:GLOB`, where `DIALECT` may be `auto`.
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        let (name, files) = match value.split_once(':') {
            Some((name, files)) => (name, Some(Glob::new(files))),
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
    /// config file it comes from, normalized, which it is relative to.
    pub(crate) exclude: Vec<(Glob, String)>,
    /// Schemas for the files matching a pattern, from a config file's `schemas`.
    pub(crate) schemas: Vec<SchemaPattern>,
}

/// A schema for the files matching a glob, from a config file.
#[derive(Clone, Debug)]
pub(crate) struct SchemaPattern {
    pub(crate) files: Glob,
    /// The schema file, as the config file's own folder joined with the path it gives.
    pub(crate) schema: PathBuf,
    /// The absolute directory `files` is relative to, normalized, as for [`DialectPattern`].
    pub(crate) base: String,
    /// From a project config: then the schema sits inside the project, and an error in it
    /// quotes nothing from it ([`confined`]). The user's own config is not confined.
    pub(crate) confined: bool,
}

impl Settings {
    /// Add `later` after `self`, so `later` wins where both say something.
    pub(crate) fn overridden_by(mut self, later: Settings) -> Settings {
        self.dialects.extend(later.dialects);
        self.rules.extend(later.rules);
        self.exclude.extend(later.exclude);
        self.schemas.extend(later.schemas);
        self
    }

    /// The schema the config file names for `path`, if any: the last matching entry wins, as
    /// for dialects.
    pub(crate) fn schema_for(&self, path: &str) -> Option<&SchemaPattern> {
        let mut located = Located::new(path);
        self.schemas.iter().rev().find(|entry| {
            located
                .in_base(&entry.base)
                .is_some_and(|path| entry.files.matches(path))
        })
    }

    /// Whether a directory walk skips `path`, a file or a folder.
    pub(crate) fn excluded(&self, path: &Path) -> bool {
        let path = path.to_string_lossy();
        let mut located = Located::new(&path);
        self.exclude
            .iter()
            .any(|(glob, base)| located.in_base(base).is_some_and(|path| glob.matches(path)))
    }

    /// The checking config for one file: the last matching dialect pattern wins.
    pub(crate) fn config_for(&self, path: &str) -> Config {
        let auto = || Dialect::for_path(Path::new(path)).unwrap_or(Dialect::Json5);
        let given = std::cell::OnceCell::new();
        let mut located = Located::new(path);
        let dialect = self
            .dialects
            .iter()
            .rev()
            .find(|rule| match (&rule.files, &rule.base) {
                (None, _) => true,
                (Some(glob), None) => glob.matches(given.get_or_init(|| Target::new(path))),
                (Some(glob), Some(base)) => {
                    located.in_base(base).is_some_and(|path| glob.matches(path))
                }
            })
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

/// A path, made absolute and relative to a config file's folder only when a pattern needs it,
/// and once for each folder rather than once for each pattern: a walk matches every pattern
/// against every entry.
struct Located<'p> {
    path: &'p str,
    /// The path made absolute and normalized, once a pattern needs it.
    absolute: std::cell::OnceCell<Option<String>>,
    /// The last folder asked for, and the path relative to it, if inside it.
    relative: Option<(&'p str, Option<Target>)>,
}

impl<'p> Located<'p> {
    fn new(path: &'p str) -> Located<'p> {
        Located {
            path,
            absolute: std::cell::OnceCell::new(),
            relative: None,
        }
    }

    /// The path relative to the normalized folder `base`, or `None` when it is outside it,
    /// which matches none of its patterns.
    fn in_base(&mut self, base: &'p str) -> Option<&Target> {
        if self.relative.as_ref().is_none_or(|(last, _)| *last != base) {
            let absolute = self.absolute.get_or_init(|| absolute(self.path));
            let relative = absolute
                .as_deref()
                .and_then(|absolute| glob::relative(absolute, base))
                .map(Target::new);
            self.relative = Some((base, relative));
        }
        self.relative
            .as_ref()
            .and_then(|(_, relative)| relative.as_ref())
    }
}

/// `path` made absolute and normalized, or `None` for an empty path, as `std::path::absolute`
/// gives. The working directory is asked for once rather than for each path: jsonvet never
/// changes it.
fn absolute(path: &str) -> Option<String> {
    static CWD: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    let path = Path::new(path);
    let absolute = match CWD.get_or_init(|| std::env::current_dir().ok()) {
        Some(cwd) if path.is_relative() && !path.as_os_str().is_empty() => {
            std::path::absolute(cwd.join(path))
        }
        _ => std::path::absolute(path),
    };
    absolute
        .ok()
        .map(|absolute| glob::normalize(&absolute.to_string_lossy()))
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
    /// The working directory, asked for once rather than for each file.
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
        let here = std::env::current_dir()
            .map_err(|e| format!("cannot read the working directory: {e}"))?;
        let repo = here
            .ancestors()
            .find(|dir| dir.join(".git").exists())
            .map(Path::to_owned);
        let fixed = match config {
            ConfigSource::Search => None,
            ConfigSource::None => Some(Arc::new(cli.clone())),
            // The person running jsonvet named it, so like `--schema` it is not confined.
            ConfigSource::File(path) => {
                Some(Arc::new(load(path, None)?.overridden_by(cli.clone())))
            }
        };
        Ok(Resolver {
            cli: Arc::new(cli),
            fixed,
            found: HashMap::new(),
            cwd: here,
            repo,
            user: None,
            last: None,
        })
    }

    /// The project the file at `path` sits in, made canonical: where [`Resolver::for_file`]'s
    /// search for it stops. Its config file's schema paths and its own `$schema` stay in it.
    pub(crate) fn root(&self, path: &Path) -> PathBuf {
        let path = lexical(&self.cwd.join(path));
        let start = path.parent().unwrap_or(&path);
        let last = self.last(start);
        let root = start
            .ancestors()
            .find(|dir| Some(*dir) == last || dir.join(".git").exists())
            .unwrap_or(start);
        fs::canonicalize(root).unwrap_or_else(|_| root.to_owned())
    }

    /// Where the search from the folder `start` stops if it meets no `.git` first.
    fn last<'s>(&'s self, start: &'s Path) -> Option<&'s Path> {
        if self
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
        }
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
        let last = self.last(start);
        let mut searched = Vec::new();
        let mut settings = None;
        for dir in start.ancestors() {
            if let Some(known) = self.found.get(dir) {
                settings = Some(Arc::clone(known));
                break;
            }
            searched.push(dir);
            if let Some(config) = config_in(dir) {
                let own = load(&config, Some(&self.root(&path)))?
                    .overridden_by(Settings::clone(&self.cli));
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
                let mut own = load(&path, None)?;
                // Its globs are relative to the working directory, as in ruff: it sits in no
                // project. Normalized, as `load` does for a file's own folder.
                let cwd = glob::normalize(&self.cwd.to_string_lossy());
                for pattern in &mut own.dialects {
                    pattern.base = Some(cwd.clone());
                }
                for (_, base) in &mut own.exclude {
                    base.clone_from(&cwd);
                }
                // Its schema paths stay relative to its own folder, where they sit.
                for entry in &mut own.schemas {
                    entry.base.clone_from(&cwd);
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

/// Whether the schema at `path`, relative to the folder `base`, as a file's `$schema` or a
/// project config names it, exists: an error if it is outside `root`, the canonical project
/// root (ADR 0012). A pull request picks these paths, and an error in a schema would show what
/// the file holds.
pub(crate) fn confined(path: &str, base: &Path, root: &Path) -> Result<bool, String> {
    let outside = || format!("the schema {path} is outside the project, so it is not read");
    let given = Path::new(path);
    // Before the path is touched: on Windows, opening `\\host\share\s.json` connects to the host.
    // A drive or UNC path is refused on every OS, so a file gets the same verdict everywhere.
    let drive = matches!(path.as_bytes(), [letter, b':', ..] if letter.is_ascii_alphabetic());
    if drive
        || path.starts_with("\\\\")
        || given.has_root()
        || matches!(given.components().next(), Some(Component::Prefix(_)))
    {
        return Err(outside());
    }
    let base = if base.as_os_str().is_empty() {
        Path::new(".")
    } else {
        base
    };
    let joined = base.join(given);
    // A missing file is judged by where it would be, so a path outside is refused the same
    // whether or not anything is there, and nothing outside can be probed.
    let (exists, real) = match fs::canonicalize(&joined) {
        Ok(real) => (true, Some(real)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (false, leads_to(joined)),
        Err(e) => return Err(format!("cannot read the schema {path}: {e}")),
    };
    match real {
        Some(real) if real.starts_with(root) => Ok(exists),
        _ => Err(outside()),
    }
}

/// Where the missing file `path` would be: its deepest part that exists, made canonical, with
/// the rest added by name. A dangling symlink on the way is followed by hand. `None` if that
/// cannot be told, such as for a loop of symlinks.
fn leads_to(mut path: PathBuf) -> Option<PathBuf> {
    // As many symlinks as Linux follows in one path.
    for _ in 0..40 {
        let (dir, rest) = path.ancestors().find_map(|dir| {
            fs::symlink_metadata(dir).ok()?;
            Some((dir, path.strip_prefix(dir).ok()?))
        })?;
        match fs::canonicalize(dir) {
            Ok(real) => return Some(lexical(&real.join(rest))),
            Err(_) => path = dir.parent()?.join(fs::read_link(dir).ok()?).join(rest),
        }
    }
    None
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
/// array of globs, relative to the same directory, for paths a directory walk skips; and
/// `schemas`, an array of tables with a `files` glob and a `schema` path, both relative to that
/// directory. Any other key is an error, so a typo is never ignored. With a `root`, the file is
/// a project's, and each schema must be inside `root` ([`confined`]).
pub(crate) fn load(path: &Path, root: Option<&Path>) -> Result<Settings, String> {
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
    let absolute = std::path::absolute(path).map_err(|e| in_file(&e))?;
    let folder = absolute.parent();
    let base = folder.map(|dir| glob::normalize(&dir.to_string_lossy()));
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
                        settings.exclude.push((Glob::new(text), base.clone()));
                    }
                }
            }
            ("schemas", DeValue::Array(entries)) => {
                for entry in entries {
                    let pattern = schema_entry(entry, folder, root);
                    let pattern = pattern.map_err(|(offset, e)| at(offset, &e))?;
                    if let Some(mut pattern) = pattern
                        && let Some(base) = &base
                    {
                        pattern.base.clone_from(base);
                        settings.schemas.push(pattern);
                    }
                }
            }
            ("exclude", _) => return Err(at(here, &"`exclude` must be an array of strings")),
            ("schemas", _) => return Err(at(here, &"`schemas` must be an array of tables")),
            ("rules", _) => return Err(at(here, &"`rules` must be a table")),
            ("dialects", _) => return Err(at(here, &"`dialects` must be an array of tables")),
            (key_text, _) => {
                let hint =
                    jsonvet::did_you_mean(key_text, ["rules", "dialects", "exclude", "schemas"]);
                return Err(at(
                    key.span().start,
                    &format!(
                        "unknown key `{key_text}`{hint}; expected rules, dialects, exclude or schemas"
                    ),
                ));
            }
        }
    }
    Ok(settings)
}

/// One `dialects` entry: a table with exactly a `files` string and a `dialect` string. An error
/// comes with the offset it is about.
fn dialect_entry(entry: &Spanned<DeValue<'_>>) -> Result<DialectPattern, (usize, String)> {
    let [(_, files), (dialect_at, dialect)] =
        entry_strings(entry, "dialects", ["files", "dialect"])?;
    Ok(DialectPattern {
        files: Some(Glob::new(files)),
        dialect: parse_dialect(dialect).map_err(|e| (dialect_at, e))?,
        base: None,
    })
}

/// One entry of `schemas`, with no base yet, for a config file in `folder`: `None` without a
/// folder, which nothing is relative to. With a `root`, the schema must be inside it.
fn schema_entry(
    entry: &Spanned<DeValue<'_>>,
    folder: Option<&Path>,
    root: Option<&Path>,
) -> Result<Option<SchemaPattern>, (usize, String)> {
    let [(_, files), (schema_at, schema)] = entry_strings(entry, "schemas", ["files", "schema"])?;
    let Some(folder) = folder else {
        return Ok(None);
    };
    // A missing schema is reported when it is read, which `--schema` skips.
    if let Some(root) = root {
        confined(schema, folder, root).map_err(|e| (schema_at, e))?;
    }
    Ok(Some(SchemaPattern {
        files: Glob::new(files),
        schema: folder.join(schema),
        base: String::new(),
        confined: root.is_some(),
    }))
}

/// The two strings of an entry in the array `array`, a table with exactly the keys `keys`, each
/// with its offset. An error comes with the offset it is about.
fn entry_strings<'e>(
    entry: &'e Spanned<DeValue<'_>>,
    array: &str,
    keys: [&str; 2],
) -> Result<[(usize, &'e str); 2], (usize, String)> {
    let start = entry.span().start;
    let DeValue::Table(table) = entry.get_ref() else {
        return Err((start, format!("each `{array}` entry must be a table")));
    };
    if let Some(key) = table.keys().find(|k| !keys.contains(&k.get_ref().as_ref())) {
        let name = key.get_ref();
        let hint = jsonvet::did_you_mean(name, keys);
        return Err((
            key.span().start,
            format!(
                "unknown key `{name}`{hint} in a `{array}` entry; expected {} and {}",
                keys[0], keys[1]
            ),
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
                    format!("each `{array}` entry needs a `{key}` string"),
                )
            })
    };
    Ok([string(keys[0])?, string(keys[1])?])
}

#[cfg(test)]
mod tests {
    use super::{Glob, Settings, absolute, confined};

    #[test]
    fn a_schema_path_with_a_root_drive_or_prefix_is_refused_before_it_is_opened() {
        let base = std::path::Path::new(".");
        let root = std::fs::canonicalize(base).expect("the working directory");
        let root = root.as_path();
        let mut paths = vec![
            "/etc/passwd",
            "//host/share/s.json",
            "C:\\s.json",
            "C:/s.json",
            "c:s.json",
            "\\\\host\\share\\s.json",
        ];
        if cfg!(windows) {
            paths.push("\\s.json");
        }
        for path in paths {
            let refused = confined(path, base, root).expect_err(path);
            assert!(refused.contains("outside the project"), "{path}: {refused}");
        }
        assert_eq!(confined("missing-schema.json", base, root), Ok(false));
    }

    #[test]
    fn each_pattern_matches_relative_to_its_own_base() {
        // Bases from the working directory, so they are absolute on Windows too.
        let (r1, r2) = (
            absolute("r1").expect("a base"),
            absolute("r2").expect("a base"),
        );
        let settings = Settings {
            exclude: vec![
                (Glob::new("a.json"), r1.clone()),
                (Glob::new("b.json"), r2.clone()),
            ],
            ..Settings::default()
        };
        let path = |base: &str, name: &str| std::path::PathBuf::from(format!("{base}/{name}"));
        assert!(settings.excluded(&path(&r1, "a.json")));
        assert!(settings.excluded(&path(&r2, "b.json")));
        assert!(!settings.excluded(&path(&r1, "b.json")));
    }

    #[test]
    fn paths_are_made_absolute_from_the_working_directory() {
        let cwd = std::env::current_dir().expect("a working directory");
        let cwd = cwd.to_string_lossy().replace('\\', "/");
        assert_eq!(absolute("a/./b.json"), Some(format!("{cwd}/a/b.json")));
        assert_eq!(absolute(""), None);
        #[cfg(unix)]
        assert_eq!(absolute("/x/../y.json").as_deref(), Some("/y.json"));
    }
}
