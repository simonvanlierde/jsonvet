//! Which schema each file is checked against, and whether its errors may quote it: the one
//! `--schema` gives, else the one a config file names, else the one a file's own `$schema`
//! names (ADR 0012).

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use jsonvet::{Dialect, Event, EventKind, Report, Rule};

use super::Schema;
use super::limits::{PATTERN_BYTES, RUN_PATTERNS};
use crate::settings::{self, Settings};
use crate::{check, output};

/// Whether the file of `job`, shown as `name`, with `text`, is checked against a schema: the one
/// given, or one its own `$schema` may name.
pub(crate) fn with_schema(((settings, schema), _): &Job, name: &OsStr, text: &str) -> bool {
    schema.is_some()
        || may_name_a_path(text)
            && settings.config_for(&name.to_string_lossy()).dialect() != Dialect::JsonLines
}

/// A file to check, with its settings and the schema it is checked against, if any.
pub(crate) type Job = ((Arc<Settings>, Option<Arc<Schema>>), OsString);

/// Each file, shown by the name `shown` gives it, with its settings and schema: the one at
/// `given`, from `--schema`, or else the one its config file names, through `load`.
pub(crate) fn jobs(
    files: Vec<OsString>,
    shown: &impl Fn(&OsStr) -> OsString,
    settings_for: &impl Fn(&Path) -> Result<Arc<Settings>, String>,
    given: Option<&Path>,
    load: &impl Fn(&Path, bool) -> Loaded,
) -> Result<Vec<Job>, String> {
    // `--schema` is the user's own choice, so it may be anywhere and its errors quote it.
    let given = given
        .map(|path| load(path, false))
        .transpose()
        .map_err(|e| output::printable(&e))?;
    // Settings are shared between files of a folder, so each set is walked once.
    let mut walked = std::collections::HashSet::new();
    files
        .into_iter()
        .map(|file| {
            let name = shown(&file);
            let settings = settings_for(Path::new(&name))?;
            // A schema the config names is read even if no file matches its glob, so a missing
            // one is never passed over. `--schema` replaces them all.
            if given.is_none() && walked.insert(Arc::as_ptr(&settings)) {
                for entry in &settings.schemas {
                    load(&entry.schema, entry.confined).map_err(|e| output::printable(&e))?;
                }
            }
            let schema = match &given {
                Some(schema) => Some(Arc::clone(schema)),
                None => settings
                    .schema_for(&name.to_string_lossy())
                    .map(|entry| load(&entry.schema, entry.confined))
                    .transpose()
                    .map_err(|e| output::printable(&e))?,
            };
            Ok(((settings, schema), file))
        })
        .collect()
}

/// A schema, or why it cannot be used.
pub(crate) type Loaded = Result<Arc<Schema>, String>;

/// The schemas `--schema`, config files and `$schema` keys name, each read once, by canonical
/// path and whether its errors are quiet.
#[derive(Default)]
pub(crate) struct Schemas {
    loaded: Mutex<HashMap<(PathBuf, bool), Loaded>>,
    /// The regexes of the schemas read, as [`RUN_PATTERNS`] counts them, also of those that
    /// failed after they were counted. Changed only while `loaded` is held.
    patterns: AtomicUsize,
    /// Why the run stops: a schema took it past [`RUN_PATTERNS`].
    over: OnceLock<String>,
    /// What each `$schema` path, from each folder of the files that hold one, leads to.
    own: Mutex<HashMap<(PathBuf, String), Own>>,
}

/// The schema a file's own `$schema` names: none if it is not there, or why it cannot be used.
type Own = Result<Option<Arc<Schema>>, String>;

impl Schemas {
    /// The schema at `path`, from `load` the first time it is asked for with `quiet`. Every
    /// spelling of a path, such as `x/../s.json` for `s.json`, reads it once: else a file
    /// could name one large schema many ways and have it read for each. `load` hands the number
    /// of regexes in the schema to its argument before it compiles any: one that takes the run
    /// past [`RUN_PATTERNS`] is an error, and so is every schema after it.
    pub(crate) fn get(
        &self,
        path: &Path,
        quiet: bool,
        load: impl FnOnce(&dyn Fn(usize) -> Result<(), String>) -> Result<Schema, String>,
    ) -> Loaded {
        let key = std::fs::canonicalize(path)
            .or_else(|_| std::path::absolute(path))
            .unwrap_or_else(|_| path.to_owned());
        // A poisoned lock still holds every schema read.
        let mut loaded = self
            .loaded
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loaded
            .entry((key, quiet))
            .or_insert_with(|| {
                let admit = |count: usize| {
                    self.over()?;
                    let patterns = self.patterns.load(Ordering::Relaxed) + count;
                    if patterns > RUN_PATTERNS {
                        let error = format!(
                            "the schema {} takes the schemas this run reads past \
                             {RUN_PATTERNS} regexes between them, each allowed {} MB once \
                             compiled",
                            path.display(),
                            PATTERN_BYTES >> 20
                        );
                        return Err(self.over.get_or_init(|| error).clone());
                    }
                    self.patterns.store(patterns, Ordering::Relaxed);
                    Ok(())
                };
                load(&admit).map(Arc::new)
            })
            .clone()
    }

    /// Why the run must stop, once a schema read took it past [`RUN_PATTERNS`]: a usage error,
    /// whichever file named the schema, so the verdict does not depend on which thread read it
    /// first.
    pub(crate) fn over(&self) -> Result<(), String> {
        self.over.get().map_or(Ok(()), |error| Err(error.clone()))
    }

    /// What the `$schema` `path` in a file in `folder` leads to, from `find` the first time it
    /// is asked for from that folder. Where a path leads, and whether it stays in the project,
    /// depend on the folder and the path alone, so the answer holds for every file there, and
    /// thousands of files that name one schema look it up once.
    fn own(&self, folder: &Path, path: &str, find: impl FnOnce() -> Own) -> Own {
        let key = (folder.to_owned(), path.to_owned());
        // A poisoned lock still holds every answer, and none is held while `find` runs: it
        // looks at the file system. Two threads may then both run it, with the same answer.
        let known = self
            .own
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(own) = known.get(&key) {
            return own.clone();
        }
        drop(known);
        let own = find();
        self.own
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key, own.clone());
        own
    }
}

/// The local schema path a file's `$schema` names, and the offset of its value, found as the
/// file's events come in: a string member of the top-level object that is no URL. jsonvet fetches
/// no schemas, so a URL names none. It keeps no event, so the file's memory stays its text.
#[derive(Default)]
struct OwnSchema {
    /// How many levels under the top-level object the events are, once it opens.
    depth: Option<usize>,
    /// Whether the next event is the value of the top-level `$schema`.
    value: bool,
    /// Whether the first top-level `$schema`, or the end of the object, has been seen.
    done: bool,
    /// What that `$schema` names.
    found: Option<(usize, String)>,
}

impl OwnSchema {
    fn event(&mut self, event: &Event<'_>) {
        if self.done {
            return;
        }
        if self.value {
            self.done = true;
            if let EventKind::String(path) = &event.kind {
                // A fragment points into the schema, as in `s.json#`; the file is the same.
                let path = path.split_once('#').map_or(&**path, |(path, _)| path);
                self.found =
                    (!path.is_empty() && !is_url(path)).then(|| (event.offset, path.to_owned()));
            }
            return;
        }
        let Some(depth) = &mut self.depth else {
            self.depth = Some(0);
            self.done = event.kind != EventKind::ObjectStart;
            return;
        };
        match &event.kind {
            EventKind::ObjectStart | EventKind::ArrayStart => *depth += 1,
            EventKind::End if *depth == 0 => self.done = true,
            EventKind::End => *depth -= 1,
            EventKind::Key(key) if *depth == 0 && **key == *b"$schema" => self.value = true,
            _ => {}
        }
    }
}

/// Whether some `$schema` in `text` may hold a path rather than a URL, so the text is worth
/// parsing into events: they take 10 times its memory and 2.5 times as long as a plain check,
/// and most `$schema` values are URLs. Anything but `$schema`, a quote, `:` and a URL is a maybe.
fn may_name_a_path(text: &str) -> bool {
    text.match_indices("$schema").any(|(at, key)| {
        let rest = text[at + key.len()..].trim_start_matches(['"', '\'']);
        rest.trim_start()
            .strip_prefix(':')
            .is_none_or(|value| !is_url(value.trim_start().trim_start_matches(['"', '\''])))
    })
}

/// Whether `text` starts with a URI scheme: a letter, then letters, digits, `+`, `-` or `.`,
/// then `:`. One letter is a Windows drive, as in `C:\schemas\a.json`, so a scheme takes two.
fn is_url(text: &str) -> bool {
    text.split_once(':').is_some_and(|(scheme, _)| {
        scheme.len() > 1
            && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    })
}

/// Check a file's text with no schema given for it, which [`with_schema`] says may name one in its
/// own `$schema`, against that schema if any, relative to the file's folder, through `load`. The schema must be inside the file's own
/// project, as `root` gives it, and its errors quote nothing from it (ADR 0012). One that does not
/// exist is skipped; one outside the project or that cannot be used is an error at the `$schema`
/// value. JSON Lines holds many documents, so no one `$schema` applies.
pub(crate) fn check_own_schema<'t>(
    name: &str,
    text: &'t str,
    settings: &Settings,
    schemas: &Schemas,
    root: &impl Fn(&Path) -> PathBuf,
    load: &impl Fn(&Path, bool) -> Loaded,
    parts: usize,
) -> (Report<'t>, Vec<(usize, String, bool)>) {
    let config = settings.config_for(name);
    // With the file's own depth limit, so a file with no usable `$schema` gets the report a
    // plain check gives, without a second parse.
    let mut own = OwnSchema::default();
    let report = jsonvet::parse_with(text, &config, &mut |event| own.event(&event));
    let syntax_error = report.problems().iter().any(|p| p.rule() == Rule::Syntax);
    let Some((offset, path)) = own.found.filter(|_| !syntax_error) else {
        return (report, Vec::new());
    };
    let folder = Path::new(name).parent().unwrap_or(Path::new(""));
    let own = schemas.own(folder, &path, || {
        // A schema that is not there is skipped like a URL: tools name one under
        // `node_modules` that a CI run may not have installed.
        if !settings::confined(&path, folder, &root(Path::new(name)))? {
            return Ok(None);
        }
        load(&folder.join(&path), true).map(Some)
    });
    match own {
        Ok(None) => (report, Vec::new()),
        // As with `--schema`.
        Ok(Some(schema)) => check(name, text, settings, Some(&schema), parts),
        Err(error) => (report, vec![(offset, output::printable(&error), false)]),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{Schema, Schemas, Settings};

    #[test]
    fn every_spelling_of_a_schema_path_reads_it_once() {
        let dir = std::env::temp_dir().join(format!("jsonvet-spellings-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("x")).expect("a temporary directory");
        std::fs::write(dir.join("s.json"), "{}").expect("written");
        let schemas = Schemas::default();
        let reads = AtomicUsize::new(0);
        let read = |admit: &dyn Fn(usize) -> Result<(), String>| {
            reads.fetch_add(1, Ordering::Relaxed);
            Schema::load(&dir.join("s.json"), &Settings::default(), false, admit)
        };
        for path in ["s.json", "x/../s.json", "x/../x/../s.json", "./s.json"] {
            assert!(schemas.get(&dir.join(path), true, read).is_ok(), "{path}");
        }
        assert_eq!(reads.load(Ordering::Relaxed), 1);
        // Quiet and not are two reads, since their errors differ.
        assert!(schemas.get(&dir.join("s.json"), false, read).is_ok());
        assert_eq!(reads.load(Ordering::Relaxed), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_a_dollar_schema_that_may_hold_a_path_is_worth_parsing() {
        for (text, path) in [
            ("{}", false),
            (r#"{"$schema": "https://x/s.json", "a": 1}"#, false),
            ("{$schema: 'https://x/s.json'}", false),
            (
                r#"{"a": {"$schema":"http://x"}, "$schema" : "urn:x"}"#,
                false,
            ),
            (r#"{"$schema": "s.json"}"#, true),
            (r#"{"$schema": "C:\\s.json"}"#, true),
            (r#"{"$schema": /* https://x */ "s.json"}"#, true),
            (r#"{"$schema" /* c */: "https://x"}"#, true),
            (r#"{"$schema": 1}"#, true),
            (r#"{"a": "https://x", "$schema": "s.json"}"#, true),
            ("{\"$schema", true),
        ] {
            assert_eq!(super::may_name_a_path(text), path, "{text}");
        }
    }

    #[test]
    fn is_url_tells_a_scheme_from_a_windows_drive() {
        for (text, url) in [
            ("https://x", true),
            ("ab:x", true),
            ("C:\\a.json", false),
            ("./a.json", false),
            ("a.json", false),
            // A scheme starts with a letter.
            ("1a:x", false),
        ] {
            assert_eq!(super::is_url(text), url, "{text}");
        }
    }
}
