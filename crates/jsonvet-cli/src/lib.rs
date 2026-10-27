//! The jsonvet command line, shared by the `jsonvet` and `check-json5` binaries.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::io::{self, BufWriter, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use jsonvet::{Dialect, Event, EventKind, Report, Severity};

mod args;
mod frame;
mod glob;
mod json;
mod output;
mod rules;
mod sarif;
mod schema;
mod settings;

use args::{Command, STDIN};
use output::{Format, Look, Printer};
use schema::Schema;
use settings::{Resolver, Settings};

/// How deep `--schema` reads a file. Past it, the file has a syntax error. jsonschema validates
/// recursively, so depth costs stack (see [`STACK`]). Its memory for an error deep under a
/// recursive `$ref` grows linearly with depth: 19 MB for a 60 KB file nested 10,000 levels.
const SCHEMA_MAX_DEPTH: usize = 512;

/// The stack every run uses, for `--schema`: jsonschema validates recursively, and building,
/// validating and dropping a value [`SCHEMA_MAX_DEPTH`] levels deep against a recursive schema
/// took up to 2.2 MB in a debug build, more than Windows' 1 MB main thread. Only the pages used
/// are committed, so a plain check costs no more memory.
const STACK: usize = 64 << 20;

/// How much memory the files read at once may take before they are checked and printed.
const CHUNK_BYTES: usize = 64 << 20;

/// How many bytes of memory a byte of file text takes with `--schema`, which holds the file's
/// value one document at a time: 4 to 22 per byte of that document (docs/design.md, "Memory").
/// 20 is near the top of that range: only many small records take more.
const SCHEMA_BYTES: usize = 20;

/// From this size, a JSON Lines file is split at newlines and its parts checked on the cores
/// its chunk leaves free.
const SPLIT_BYTES: usize = 1 << 20;

/// Run the command with the process's arguments and environment, and return its exit code.
pub fn run() -> ExitCode {
    on_big_stack(run_here)
}

/// Run `f` on a thread with a [`STACK`]-byte stack, or here if no thread can start. A panic in
/// `f` carries on here.
fn on_big_stack<T: Send + 'static>(f: fn() -> T) -> T {
    let thread = std::thread::Builder::new().name("jsonvet".into());
    match thread.stack_size(STACK).spawn(f) {
        Ok(thread) => thread
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
        Err(_) => f(),
    }
}

fn run_here() -> ExitCode {
    let annotate = std::env::var_os("GITHUB_ACTIONS").is_some_and(|v| v == "true");
    let stdout = io::stdout().lock();
    let set = |name| std::env::var_os(name).is_some_and(|v| !v.is_empty());
    // NO_COLOR wins over FORCE_COLOR: asking for no color is the safer wish to honour.
    // FORCE_COLOR=0 and =false ask for no color, as Node and chalk read them.
    let color = if set("NO_COLOR") {
        Some(false)
    } else if set("FORCE_COLOR") {
        Some(std::env::var_os("FORCE_COLOR").is_none_or(|v| v != "0" && v != "false"))
    } else {
        None
    };
    let terminal = Terminal {
        tty: stdout.is_terminal(),
        color,
    };
    match run_with(
        std::env::args_os().skip(1),
        annotate,
        terminal,
        BufWriter::new(stdout),
    ) {
        Ok(code) => ExitCode::from(code),
        Err(message) => {
            eprintln!("jsonvet: {message}");
            ExitCode::from(2)
        }
    }
}

/// Where output goes, as far as text output's look depends on it. Public only for
/// `benches/codspeed.rs`; not part of the crate's API.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Terminal {
    /// Standard output is a terminal. A summary and code frames are for people: hooks and CI
    /// read standard output from a pipe and see one line per problem.
    pub tty: bool,
    /// What the environment asks for when `--color` is `auto`: `Some(false)` for `NO_COLOR`,
    /// `Some(true)` for `FORCE_COLOR`.
    pub color: Option<bool>,
}

/// The command, with its inputs passed in: 0 when no file has an error, 1 when one does.
/// On a terminal, text output ends with a count of the problems. A usage error comes back as
/// `Err`, which `run` turns into exit status 2. Public only for `benches/codspeed.rs`, which
/// measures `--schema` in-process; not part of the crate's API.
#[doc(hidden)]
pub fn run_with(
    args: impl IntoIterator<Item = OsString>,
    annotate: bool,
    terminal: Terminal,
    out: impl Write,
) -> Result<u8, String> {
    // Usage errors quote arguments and the schema file, which can hold a newline or an escape.
    let args = match args::parse(args).map_err(|e| {
        format!(
            "{}\n\nRun `jsonvet --help` for usage.",
            output::printable(&e)
        )
    })? {
        Command::Help => return print(out, args::HELP),
        Command::Version => return print(out, concat!("jsonvet ", env!("CARGO_PKG_VERSION"))),
        Command::ListRules => return print(out, &rules::list()),
        Command::Explain(text) => return print(out, &text),
        Command::Check(args) => args,
    };
    let resolver = Arc::new(Mutex::new(Resolver::new(&args.config, args.settings)?));
    let settings_for = |path: &Path| lock(&resolver).for_file(path);
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let files = expand(args.files, &resolver, threads)?;
    let schemas = Schemas::default();
    let load = |path: &Path, quiet| {
        schemas.get(path, quiet, || {
            Schema::load(path, &*settings_for(path)?, quiet)
        })
    };
    let root = |path: &Path| lock(&resolver).root(path);

    // Standard input goes by its `--stdin-filename`, for settings and in the output.
    let stdin_name = args.stdin_filename.unwrap_or_else(|| STDIN.into());
    let shown = |file: &OsStr| -> OsString {
        if file == STDIN {
            stdin_name.clone()
        } else {
            file.to_owned()
        }
    };
    // Every config file, and every schema one names, is read before any file is checked, so a
    // bad one stops the run at once.
    let files = jobs(files, &shown, &settings_for, args.schema.as_deref(), &load)?;
    let summary = terminal.tty;
    let look = look(args.format, args.color, terminal);
    let mut printer = Printer::new(out, args.format, annotate, look);
    let mut counts = Counts::default();
    // Files are read and checked on every core, a chunk at a time, then printed here in the
    // order given. A chunk ends once its files hold CHUNK_BYTES, so memory stays near that. With
    // a schema a file takes SCHEMA_BYTES times its size, so the chunk is cut from the sizes
    // before any file is read: otherwise every core would start on a file at once. Without
    // a schema the files are not sized first, which made a walk of 10,000 files 7 ms slower.
    // NOTE: each chunk waits for its slowest file before the next starts; hand files out
    // one at a time across chunks if one big file among many small ones holds a run up.
    // NOTE: a file's own `$schema` is found only once it is read, so such a file counts its
    // size once; size every file if a run of large files with one runs out of memory.
    let mut rest = &files[..];
    let shares = schema_shares(&files, threads);
    let written = loop {
        if rest.is_empty() {
            let line = (summary && !files.is_empty()).then(|| counts.summary(files.len()));
            break line
                .map_or(Ok(()), |line| printer.text_line(&line))
                .and_then(|()| printer.finish());
        }
        let offered = match shares.get(files.len() - rest.len()..) {
            Some(shares) if !shares.is_empty() => &rest[..chunk_len(shares)],
            _ => rest,
        };
        let texts = read_chunk(offered, threads);
        let chunk;
        (chunk, rest) = rest.split_at(texts.len());
        let jobs: Vec<_> = chunk.iter().zip(&texts).collect();
        let parts = (threads / jobs.len()).max(1);
        let checked = parallel(
            &jobs,
            threads,
            || false,
            |(((settings, schema), file), text)| {
                let name = shown(file);
                let name = name.to_string_lossy();
                text.as_deref().map(|text| match schema {
                    Some(schema) => check(&name, text, settings, Some(schema), parts),
                    None => check_own_schema(&name, text, settings, &root, &load, parts),
                })
            },
        );
        let outcome = chunk
            .iter()
            .zip(checked)
            .try_for_each(|(((settings, _), file), checked)| match checked {
                Ok((report, schema_errors)) => print_file(
                    &shown(file),
                    (&report, json5(settings, &shown(file))),
                    &schema_errors,
                    &mut printer,
                    &mut counts,
                ),
                Err(message) => {
                    counts.errors += 1;
                    counts.files += 1;
                    printer.unreadable(&shown(file), message)
                }
            });
        if outcome.is_err() {
            break outcome;
        }
    };
    // A reader that has gone, as with `| head`, wants no more output: the verdict so far stands.
    match written {
        Err(e) if e.kind() != io::ErrorKind::BrokenPipe => {
            return Err(format!("cannot write output: {e}"));
        }
        _ => {}
    }
    let too_many_warnings = args.max_warnings.is_some_and(|max| counts.warnings > max);
    Ok(u8::from(counts.errors > 0 || too_many_warnings))
}

/// The texts of the first of `files`, read on up to `threads` threads until they hold
/// [`CHUNK_BYTES`], and always the first.
fn read_chunk<S: Sync>(files: &[(S, OsString)], threads: usize) -> Vec<Result<String, String>> {
    let bytes = AtomicUsize::new(0);
    let full = || bytes.load(Ordering::Relaxed) >= CHUNK_BYTES;
    parallel(files, threads, full, |(_, file)| {
        let text = if file == STDIN {
            read_stdin()
        } else {
            read(Path::new(file))
        };
        bytes.fetch_add(text.as_ref().map_or(0, String::len), Ordering::Relaxed);
        text
    })
}

/// The memory a file takes while a schema checks it, as far as a chunk counts it:
/// [`SCHEMA_BYTES`] times its size, capped at [`CHUNK_BYTES`]. Standard input's size is known
/// only once it is read, so it takes a chunk's worth. A file that cannot be read takes nothing:
/// `read` reports why.
fn schema_share(file: &OsStr) -> usize {
    if file == STDIN {
        return CHUNK_BYTES;
    }
    let size = std::fs::metadata(file).map_or(0, |meta| meta.len());
    usize::try_from(size)
        .unwrap_or(usize::MAX)
        .saturating_mul(SCHEMA_BYTES)
        .min(CHUNK_BYTES)
}

/// A file to check, with its settings and the schema it is checked against, if any.
type Job = ((Arc<Settings>, Option<Arc<Schema>>), OsString);

/// Each file, shown by the name `shown` gives it, with its settings and schema: the one at
/// `given`, from `--schema`, or else the one its config file names, through `load`.
fn jobs(
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

/// Each file's [`schema_share`], or 0 for a file with no schema; nothing if no file has one.
fn schema_shares(files: &[Job], threads: usize) -> Vec<usize> {
    if files.iter().all(|((_, schema), _)| schema.is_none()) {
        return Vec::new();
    }
    parallel(
        files,
        threads,
        || false,
        |((_, schema), file)| schema.as_ref().map_or(0, |_| schema_share(file)),
    )
}

/// How many of the files with these shares the next chunk takes: up to and including the one
/// that fills [`CHUNK_BYTES`], so always at least one.
fn chunk_len(shares: &[usize]) -> usize {
    let mut total = 0;
    shares
        .iter()
        .position(|share| {
            total += share;
            total >= CHUNK_BYTES
        })
        .map_or(shares.len(), |last| last + 1)
}

/// The files to check: each argument as given, except that a directory stands for the files under
/// it that jsonvet checks by default, in sorted order. The walk skips hidden files and what
/// `.gitignore`, `.ignore` and the `exclude` setting of each folder's settings name, as ripgrep
/// would.
fn expand(
    args: Vec<OsString>,
    resolver: &Arc<Mutex<Resolver>>,
    threads: usize,
) -> Result<Vec<OsString>, String> {
    let mut files = Vec::with_capacity(args.len());
    // Each argument costs a call to the system, so thousands of them are asked about in parallel.
    let dirs = parallel(
        &args,
        threads,
        || false,
        |arg| arg != STDIN && Path::new(arg).is_dir(),
    );
    for (arg, dir) in args.into_iter().zip(dirs) {
        if !dir {
            files.push(arg);
            continue;
        }
        let start = files.len();
        // The walk reads the ignore files of the folder, and of every folder above it when it
        // can find them, before the filter sees anything.
        let above = std::fs::canonicalize(&arg).ok();
        if let Some(problem) = std::iter::once(Path::new(&arg))
            .chain(above.iter().flat_map(|dir| dir.ancestors()))
            .find_map(unsafe_ignore_file)
        {
            return Err(problem);
        }
        let skipped = Arc::new(Mutex::new(None));
        let skip = Arc::clone(&skipped);
        // The filter must own what it reads.
        let resolver = Arc::clone(resolver);
        // A config file that cannot be read excludes nothing here: the run reports it when it
        // looks up the settings of the files under it.
        let walk = ignore::WalkBuilder::new(&arg)
            .filter_entry(move |entry| {
                // The walk reads a folder's ignore files once the filter lets the folder in.
                if entry.file_type().is_some_and(|t| t.is_dir())
                    && let Some(problem) = unsafe_ignore_file(entry.path())
                {
                    // A poisoned lock still holds the message.
                    skip.lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .get_or_insert(problem);
                    return false;
                }
                let settings = lock(&resolver).for_file(entry.path());
                !settings.is_ok_and(|settings| settings.excluded(entry.path()))
            })
            .build();
        for entry in walk {
            let entry = entry.map_err(|e| output::printable(&format!("cannot walk: {e}")))?;
            if entry.file_type().is_some_and(|t| t.is_file())
                && Dialect::for_path(entry.path()).is_some()
            {
                files.push(entry.into_path().into_os_string());
            }
        }
        if let Some(problem) = skipped
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            return Err(problem);
        }
        files[start..].sort_unstable();
    }
    Ok(files)
}

/// The resolver, even if a panic elsewhere left its lock poisoned: it holds only a cache.
fn lock(resolver: &Mutex<Resolver>) -> std::sync::MutexGuard<'_, Resolver> {
    resolver
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A schema, or why it cannot be used.
type Loaded = Result<Arc<Schema>, String>;

/// The schemas `--schema`, config files and `$schema` keys name, each read once, by canonical
/// path and whether its errors are quiet.
#[derive(Default)]
struct Schemas(Mutex<HashMap<(PathBuf, bool), Loaded>>);

impl Schemas {
    /// The schema at `path`, from `load` the first time it is asked for with `quiet`. Every
    /// spelling of a path, such as `x/../s.json` for `s.json`, reads it once: else a file
    /// could name one large schema many ways and have it read for each.
    fn get(
        &self,
        path: &Path,
        quiet: bool,
        load: impl FnOnce() -> Result<Schema, String>,
    ) -> Loaded {
        let key = std::fs::canonicalize(path)
            .or_else(|_| std::path::absolute(path))
            .unwrap_or_else(|_| path.to_owned());
        // A poisoned lock still holds every schema read.
        let mut loaded = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loaded
            .entry((key, quiet))
            .or_insert_with(|| load().map(Arc::new))
            .clone()
    }
}

/// The local schema path a file's `$schema` names, and the offset of its value: a string member
/// of the top-level object that is no URL. jsonvet fetches no schemas, so a URL names none.
fn own_schema<'e>(events: &'e [Event<'_>]) -> Option<(usize, &'e str)> {
    if events.first()?.kind != EventKind::ObjectStart {
        return None;
    }
    let mut depth = 0;
    let mut events = events[1..].iter();
    while let Some(event) = events.next() {
        match &event.kind {
            EventKind::ObjectStart | EventKind::ArrayStart => depth += 1,
            EventKind::End => depth -= 1,
            EventKind::Key(key) if depth == 0 && **key == *b"$schema" => {
                let value = events.next()?;
                let EventKind::String(path) = &value.kind else {
                    return None;
                };
                // A fragment points into the schema, as in `s.json#`; the file is the same.
                let path = path.split_once('#').map_or(&**path, |(path, _)| path);
                return (!path.is_empty() && !is_url(path)).then_some((value.offset, path));
            }
            _ => {}
        }
    }
    None
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

#[derive(Default)]
struct Counts {
    errors: usize,
    warnings: usize,
    /// Files with at least one problem, or that could not be read.
    files: usize,
}

impl Counts {
    /// Such as `3 errors and 1 warning in 2 of 5 files`.
    fn summary(&self, checked: usize) -> String {
        let n = |count: usize, what: &str| {
            format!("{count} {what}{}", if count == 1 { "" } else { "s" })
        };
        if self.errors + self.warnings == 0 {
            return format!("no problems in {}", n(checked, "file"));
        }
        let problems = match (self.errors, self.warnings) {
            (e, 0) => n(e, "error"),
            (0, w) => n(w, "warning"),
            (e, w) => format!("{} and {}", n(e, "error"), n(w, "warning")),
        };
        format!("{problems} in {} of {}", self.files, n(checked, "file"))
    }
}

/// `f` on the items, on up to `threads` threads counting this one, each with a [`STACK`]-byte
/// stack, until `stop` says so before an item is taken: the results for the items done, in
/// order, always at least the first. A panic in `f` carries on here.
fn parallel<T: Sync, U: Send>(
    items: &[T],
    threads: usize,
    stop: impl Fn() -> bool + Sync,
    f: impl Fn(&T) -> U + Sync,
) -> Vec<U> {
    let next = AtomicUsize::new(0);
    // Each thread takes the next item in turn, so the items taken are always the first ones,
    // and the results are tagged to put them back in order.
    let work = || {
        let mut done = Vec::new();
        loop {
            if stop() && next.load(Ordering::Relaxed) > 0 {
                break done;
            }
            let i = next.fetch_add(1, Ordering::Relaxed);
            let Some(item) = items.get(i) else { break done };
            done.push((i, f(item)));
        }
    };
    let mut done = std::thread::scope(|scope| {
        // A thread that cannot start leaves its share to the others.
        let helpers: Vec<_> = (1..threads.min(items.len()))
            .filter_map(|_| {
                std::thread::Builder::new()
                    .stack_size(STACK)
                    .spawn_scoped(scope, work)
                    .ok()
            })
            .collect();
        let mut done = work();
        for helper in helpers {
            done.extend(
                helper
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
            );
        }
        done
    });
    done.sort_unstable_by_key(|&(i, _)| i);
    done.into_iter().map(|(_, result)| result).collect()
}

/// The parts to check a text in `dialect` in, on up to `parts` threads: more than one only for
/// JSON Lines of [`SPLIT_BYTES`] or more.
fn split(text: &str, dialect: Dialect, parts: usize) -> Vec<std::ops::Range<usize>> {
    let split = dialect == Dialect::JsonLines && parts > 1 && text.len() >= SPLIT_BYTES;
    jsonvet::line_parts(text, if split { parts } else { 1 })
}

/// Check a file's text with no schema given for it, against the schema its own `$schema` names if
/// any, relative to the file's folder, through `load`. The schema must be inside the file's own
/// project, as `root` gives it, and its errors quote nothing from it (ADR 0012). One that does not
/// exist is skipped; one outside the project or that cannot be used is an error at the `$schema`
/// value. JSON Lines holds many documents, so no one `$schema` applies.
fn check_own_schema<'t>(
    name: &str,
    text: &'t str,
    settings: &Settings,
    root: &impl Fn(&Path) -> PathBuf,
    load: &impl Fn(&Path, bool) -> Loaded,
    parts: usize,
) -> (Report<'t>, Vec<(usize, String, bool)>) {
    let config = settings.config_for(name);
    if config.dialect() == Dialect::JsonLines || !may_name_a_path(text) {
        return check(name, text, settings, None, parts);
    }
    // With the file's own depth limit, so a file with no usable `$schema` gets the report a
    // plain check gives, without a second parse.
    let (report, events) = jsonvet::parse(text, &config);
    let Some((offset, path)) = events.as_deref().and_then(own_schema) else {
        return (report, Vec::new());
    };
    let folder = Path::new(name).parent().unwrap_or(Path::new(""));
    let error = match settings::confined(path, folder, &root(Path::new(name))) {
        // A schema that is not there is skipped like a URL: tools name one under
        // `node_modules` that a CI run may not have installed.
        Ok(false) => return (report, Vec::new()),
        Ok(true) => match load(&folder.join(path), true) {
            // As with `--schema`, so the depth cap applies.
            Ok(schema) => return check(name, text, settings, Some(&schema), parts),
            Err(e) => e,
        },
        Err(e) => e,
    };
    (report, vec![(offset, output::printable(&error), false)])
}

/// Check a file's text, named by its lossy UTF-8 form for glob matching, and against `schema`
/// if there is one, on up to `parts` threads: the report and the schema errors.
fn check<'t>(
    name: &str,
    text: &'t str,
    settings: &Settings,
    schema: Option<&Schema>,
    parts: usize,
) -> (Report<'t>, Vec<(usize, String, bool)>) {
    let config = settings.config_for(name);
    let ranges = split(text, config.dialect(), parts);
    if ranges.len() > 1 {
        let config = match schema {
            Some(_) => config.max_depth(SCHEMA_MAX_DEPTH),
            None => config,
        };
        // Each line is a document of its own, and the error cap counts per document, so each
        // part's schema errors are the ones a single pass finds in its lines.
        let (lines, errors): (Vec<_>, Vec<_>) = parallel(
            &ranges,
            parts,
            || false,
            |lines| match schema {
                Some(schema) => schema.check_lines(text, lines.clone(), &config),
                None => (
                    jsonvet::parse_lines(text, lines.clone(), &config, None),
                    Vec::new(),
                ),
            },
        )
        .into_iter()
        .unzip();
        let (report, syntax_error) = jsonvet::join_lines(text, lines);
        // A file with a syntax error in any part gets no schema errors.
        let errors = if syntax_error {
            Vec::new()
        } else {
            errors.into_iter().flatten().collect()
        };
        return (report, errors);
    }
    match schema {
        Some(schema) => {
            // A file with a syntax error gets no schema errors.
            schema.check(text, &config.max_depth(SCHEMA_MAX_DEPTH))
        }
        None => (jsonvet::check(text, &config), Vec::new()),
    }
}

/// How text output looks: code frames on a terminal and lines otherwise, so tools reading a pipe
/// always get `file:line:col`, and in color as `--color` and the environment ask.
fn look(format: Format, color: Option<bool>, terminal: Terminal) -> Look {
    let text = format == Format::Text;
    Look {
        frames: text && terminal.tty,
        color: text && color.or(terminal.color).unwrap_or(terminal.tty),
    }
}

/// Whether `file`'s report counts U+2028 and U+2029 as line ends: only when it was checked as
/// JSON5, as [`check`] does.
fn json5(settings: &Settings, file: &OsStr) -> bool {
    settings.config_for(&file.to_string_lossy()).dialect() == Dialect::Json5
}

/// Print one file's problems and schema errors, and add them to `counts`, each before it is
/// printed, so a failed write still leaves the verdict. The file count is added last: only the
/// summary reads it, and a failed write ends the run before the summary.
fn print_file(
    file: &OsStr,
    (report, json5): (&Report<'_>, bool),
    schema_errors: &[(usize, String, bool)],
    printer: &mut Printer<impl Write>,
    counts: &mut Counts,
) -> io::Result<()> {
    let before = counts.errors + counts.warnings;
    for problem in report.problems() {
        match problem.severity() {
            Severity::Error => counts.errors += 1,
            Severity::Warning => counts.warnings += 1,
            _ => {}
        }
        printer.problem(file, report, json5, problem)?;
    }
    for (offset, message, summary) in schema_errors {
        counts.errors += usize::from(!summary);
        printer.schema_error(file, report, json5, *offset, message, *summary)?;
    }
    counts.files += usize::from(counts.errors + counts.warnings > before);
    Ok(())
}

/// A file's text, or why it has none. Only regular files are read, so a FIFO cannot hang the
/// run and a device such as `/dev/zero` cannot fill memory. A symlink to a regular file is
/// followed, as `cat` would.
fn read(path: &Path) -> Result<String, String> {
    let read = std::fs::metadata(path).and_then(|meta| {
        if meta.is_file() {
            std::fs::read(path).map(Some)
        } else {
            Ok(None)
        }
    });
    match read {
        Ok(Some(bytes)) => decode(bytes),
        Ok(None) => Err("not a regular file".to_owned()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Err("no such file".to_owned()),
        Err(e) => Err(format!("cannot read: {e}")),
    }
}

/// Standard input's text, or why it has none. Empty input is an empty file.
fn read_stdin() -> Result<String, String> {
    let mut bytes = Vec::new();
    match io::stdin().lock().read_to_end(&mut bytes) {
        Ok(_) => decode(bytes),
        Err(e) => Err(format!("cannot read: {e}")),
    }
}

/// A file's bytes as UTF-8 text. A UTF-16 or UTF-32 file says so, rather than that its first
/// byte is not UTF-8: by its byte order mark, or without one by where the zero bytes of its first
/// two characters fall, as RFC 4627 section 3 tells the encodings apart.
fn decode(bytes: Vec<u8>) -> Result<String, String> {
    // Without a mark, a text these patterns match would fail the check anyway: a zero byte is
    // U+0000, and outside a string no dialect allows it. JSON5 does allow it raw inside one, so
    // a first byte that opens a string is left alone.
    let opens_string = |b: u8| b == b'"' || b == b'\'';
    // UTF-32 LE first: its mark starts with UTF-16 LE's.
    let encoding = match *bytes.as_slice() {
        [0xFF, 0xFE, 0, 0, ..] => Some("UTF-32 (LE)"),
        [0, 0, 0xFE, 0xFF, ..] => Some("UTF-32 (BE)"),
        [0xFF, 0xFE, ..] => Some("UTF-16 (LE)"),
        [0xFE, 0xFF, ..] => Some("UTF-16 (BE)"),
        [0, 0, 0, a, ..] if a != 0 => Some("UTF-32 (BE)"),
        [0, a, 0, b, ..] if a != 0 && b != 0 => Some("UTF-16 (BE)"),
        [a, 0, 0, 0, ..] if a != 0 && !opens_string(a) => Some("UTF-32 (LE)"),
        [a, 0, b, 0, ..] if a != 0 && b != 0 && !opens_string(a) => Some("UTF-16 (LE)"),
        _ => None,
    };
    if let Some(encoding) = encoding {
        return Err(format!("file is {encoding}; jsonvet reads UTF-8"));
    }
    String::from_utf8(bytes)
        .map_err(|e| format!("not valid UTF-8 (byte {})", e.utf8_error().valid_up_to()))
}

/// The most a config file or an ignore file may hold: real ones take a few kilobytes.
pub(crate) const MAX_CONFIG_BYTES: u64 = 16 << 20;

/// The most a `--schema` file may hold: 40 times the largest schema on schemastore.org
/// (1.6 MB), and room for the 5,000,000 objects and arrays a schema may hold at 2 to 3 bytes each.
pub(crate) const MAX_SCHEMA_BYTES: u64 = 64 << 20;

/// The text of a config or schema file of at most `max` bytes. As in [`read`], only a regular
/// file, or a symlink to one, is opened, so a FIFO cannot hang the run and `/dev/zero` cannot
/// fill memory.
pub(crate) fn read_small(path: &Path, max: u64) -> io::Result<String> {
    if !std::fs::metadata(path)?.is_file() {
        return Err(io::Error::other("not a regular file"));
    }
    let mut text = String::new();
    // The cap holds even for a file that grows after the check.
    std::fs::File::open(path)?
        .take(max + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > max {
        return Err(io::Error::other(format!("larger than {} MB", max >> 20)));
    }
    Ok(text)
}

/// Why a folder walk must not read the `.gitignore` or `.ignore` in `dir`, if it must not. The
/// `ignore` crate reads them to the end with no check of what they are, so a link to
/// `/dev/zero` takes memory until the run is killed and a FIFO hangs it. A file that cannot be
/// looked at is no problem: the crate skips it too.
fn unsafe_ignore_file(dir: &Path) -> Option<String> {
    [".gitignore", ".ignore"].into_iter().find_map(|name| {
        let path = dir.join(name);
        let meta = std::fs::metadata(&path).ok()?;
        let problem = if !meta.is_file() {
            "not a regular file".to_owned()
        } else if meta.len() > MAX_CONFIG_BYTES {
            format!("larger than {} MB", MAX_CONFIG_BYTES >> 20)
        } else {
            return None;
        };
        Some(output::printable(&format!(
            "cannot walk: {}: {problem}",
            path.display()
        )))
    })
}

fn print(mut out: impl Write, text: &str) -> Result<u8, String> {
    match writeln!(out, "{text}").and_then(|()| out.flush()) {
        Err(e) if e.kind() != io::ErrorKind::BrokenPipe => Err(format!("cannot write output: {e}")),
        _ => Ok(0),
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use jsonvet::Dialect;

    use super::{
        CHUNK_BYTES, MAX_CONFIG_BYTES, SCHEMA_BYTES, SPLIT_BYTES, STDIN, Schema, Schemas, Settings,
        check, chunk_len, parallel, read_small, schema_share, schema_shares, split,
        unsafe_ignore_file,
    };
    use std::sync::Arc;

    /// A schema that takes any value, in a fresh folder named for `test`, which also holds an
    /// empty folder `x`.
    fn any_schema(test: &str) -> (std::path::PathBuf, Schema) {
        let dir = std::env::temp_dir().join(format!("jsonvet-{test}-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("x")).expect("a temporary directory");
        std::fs::write(dir.join("s.json"), "{}").expect("written");
        let schema = Schema::load(&dir.join("s.json"), &Settings::default(), false);
        (dir, schema.expect("a schema"))
    }

    #[test]
    fn every_spelling_of_a_schema_path_reads_it_once() {
        let (dir, _) = any_schema("spellings");
        let schemas = Schemas::default();
        let reads = AtomicUsize::new(0);
        let read = || {
            reads.fetch_add(1, Ordering::Relaxed);
            Schema::load(&dir.join("s.json"), &Settings::default(), false)
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
    fn only_files_with_a_schema_take_a_share() {
        let (dir, schema) = any_schema("shares");
        let _ = std::fs::remove_dir_all(&dir);
        let schema = Arc::new(schema);
        let settings = Arc::new(Settings::default());
        let job =
            |schema: Option<&Arc<Schema>>| ((Arc::clone(&settings), schema.cloned()), STDIN.into());
        assert_eq!(
            schema_shares(&[job(None), job(None)], 2),
            Vec::<usize>::new()
        );
        assert_eq!(
            schema_shares(&[job(None), job(Some(&schema)), job(None)], 2),
            [0, CHUNK_BYTES, 0]
        );
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

    #[test]
    fn small_files_are_read_up_to_their_limit_and_no_further() {
        let dir = std::env::temp_dir().join(format!("jsonvet-small-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temporary directory");
        let path = dir.join("f");
        std::fs::write(&path, "0123456789").expect("written");
        assert_eq!(read_small(&path, 10).expect("ten bytes"), "0123456789");
        assert!(read_small(&path, 9).is_err());
        // An ignore file is judged by its size alone; sparse files, so nothing is written.
        let sized = |len: u64| {
            std::fs::File::create(dir.join(".gitignore"))
                .and_then(|file| file.set_len(len))
                .expect("a large file");
        };
        sized(MAX_CONFIG_BYTES);
        assert_eq!(unsafe_ignore_file(&dir), None);
        sized(MAX_CONFIG_BYTES + 1);
        assert!(unsafe_ignore_file(&dir).is_some_and(|e| e.ends_with("larger than 16 MB")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_json_lines_of_the_split_size_is_split_one_part_per_thread() {
        let line = "[1]\n".repeat(SPLIT_BYTES / 4);
        assert_eq!(split(&line, Dialect::JsonLines, 8).len(), 8);
        assert_eq!(split(&line, Dialect::JsonLines, 1).len(), 1);
        assert_eq!(split(&line, Dialect::Json, 8).len(), 1);
        assert_eq!(split(&line[4..], Dialect::JsonLines, 8).len(), 1);
    }

    #[test]
    fn a_large_json_lines_file_checked_in_parts_reports_what_one_pass_does() {
        // Problems in every part: duplicate keys, and strings the schema wants as integers. Every
        // thousandth line has more schema errors than a document reports, and two in each
        // thousand have values the schema cannot check.
        let pad = "x".repeat(500);
        let wide: Vec<String> = (0..150).map(|k| format!("\"k{k}\": \"s\"")).collect();
        let wide = format!("{{{}}}", wide.join(", "));
        let mut text = String::from("\u{FEFF}");
        for i in 0..4_000 {
            let _ = match i % 7 {
                _ if i % 1_000 == 999 => writeln!(text, "{wide}"),
                _ if i % 1_000 == 500 => writeln!(text, "{{\"a\": 1e400}}"),
                _ if i % 1_000 == 501 => writeln!(text, r#"{{"\ud800": 1, "\ud801": 2}}"#),
                0 => writeln!(text, "{{\"a\": {i}, \"a\": 1}}"),
                1 => write!(text, "{{\"a\": \"{i}\", \"p\": \"{pad}\"}}\r\n"),
                _ => writeln!(text, "{{\"a\": {i}, \"p\": \"{pad}\"}}"),
            };
        }
        assert!(text.len() > SPLIT_BYTES);
        let deep = format!("{}{}\n", "[".repeat(600), "]".repeat(600));
        let texts = [
            text.clone(),
            format!("{text}{{\"a\": 1\n{text}"),
            format!("{text}{deep}{text}"),
        ];
        let dir = std::env::temp_dir().join(format!("jsonvet-split-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temporary directory");
        let path = dir.join("schema.json");
        let schema = concat!(
            r#"{"properties": {"a": {"type": "integer"}}, "#,
            r#""patternProperties": {"^k": {"type": "integer"}}}"#,
        );
        std::fs::write(&path, schema).expect("written");
        let settings = Settings::default();
        let schema = Schema::load(&path, &settings, false).expect("a schema");
        let _ = std::fs::remove_dir_all(&dir);
        for text in &texts {
            for schema in [None, Some(&schema)] {
                let (whole, whole_errors) = check("x.jsonl", text, &settings, schema, 1);
                let (split, split_errors) = check("x.jsonl", text, &settings, schema, 8);
                assert_ne!(whole.problems(), &[]);
                assert_eq!(split.problems(), whole.problems());
                assert_eq!(split_errors, whole_errors);
            }
        }
        let (_, errors) = check("x.jsonl", &texts[0], &settings, Some(&schema), 8);
        // 570 strings, 100 and a summary for each wide line, and one for each line not checked.
        assert_eq!(errors.len(), 570 + 4 * 101 + 8);
        let unchecked = errors.iter().filter(|(_, m, _)| m.contains("not checked"));
        assert_eq!(unchecked.count(), 8);
    }

    #[test]
    fn parallel_keeps_the_order_and_stops_after_a_prefix() {
        let items: Vec<usize> = (0..1000).collect();
        for threads in [1, 3, 16] {
            let squares = parallel(&items, threads, || false, |i| i * i);
            assert_eq!(squares, items.iter().map(|i| i * i).collect::<Vec<_>>());
            // Stopped from the start, it still does the first item.
            assert_eq!(parallel(&items, threads, || true, |&i| i), [0]);
            let done = AtomicUsize::new(0);
            let some = parallel(
                &items,
                threads,
                || done.load(Ordering::Relaxed) >= 100,
                |&i| {
                    done.fetch_add(1, Ordering::Relaxed);
                    i
                },
            );
            // At least 100, plus at most one per thread already under way.
            assert!((100..100 + threads).contains(&some.len()), "{}", some.len());
            assert_eq!(some, (0..some.len()).collect::<Vec<_>>());
        }
        assert_eq!(
            parallel(&[] as &[u8], 4, || false, |&b| b),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn a_chunk_ends_with_the_file_that_fills_it() {
        assert_eq!(chunk_len(&[]), 0);
        assert_eq!(chunk_len(&[1, 2, 3]), 3);
        assert_eq!(chunk_len(&[CHUNK_BYTES, 1]), 1);
        assert_eq!(chunk_len(&[CHUNK_BYTES - 1, 1, 1]), 2);
        assert_eq!(chunk_len(&[CHUNK_BYTES - 2, 1, 1]), 3);
        assert_eq!(chunk_len(&[0, CHUNK_BYTES - 1, 0, 2, 5]), 4);
    }

    #[test]
    fn a_file_counts_its_size_twenty_times_with_a_schema() {
        let dir = std::env::temp_dir().join(format!("jsonvet-share-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let path = dir.join("a.json");
        std::fs::write(&path, " ".repeat(1000)).expect("write");
        let path = path.as_os_str();
        assert_eq!(schema_share(path), 1000 * SCHEMA_BYTES);
        std::fs::write(path, " ".repeat(CHUNK_BYTES / SCHEMA_BYTES + 1)).expect("write");
        assert_eq!(schema_share(path), CHUNK_BYTES, "capped");
        assert_eq!(schema_share(dir.join("gone.json").as_os_str()), 0);
        assert_eq!(schema_share(std::ffi::OsStr::new("-")), CHUNK_BYTES);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn frames_show_on_a_terminal_and_color_follows_the_option_then_the_environment() {
        use super::{Format, Look, Terminal, look};
        let terminal = |tty, color| Terminal { tty, color };
        // Off a terminal, `color_forced_off_a_terminal_keeps_one_line_per_problem` in
        // tests/cli.rs covers each case end to end.
        let frames = |color| Look {
            frames: true,
            color,
        };
        let lines = Look {
            frames: false,
            color: false,
        };
        let cases = [
            (Format::Text, None, terminal(true, None), frames(true)),
            (
                Format::Text,
                None,
                terminal(true, Some(false)),
                frames(false),
            ),
            (
                Format::Text,
                Some(false),
                terminal(true, Some(true)),
                frames(false),
            ),
            (Format::Sarif, Some(true), terminal(true, None), lines),
            (Format::Json, Some(true), terminal(true, None), lines),
        ];
        for (format, color, terminal, want) in cases {
            assert_eq!(
                look(format, color, terminal),
                want,
                "{format:?} {color:?} {terminal:?}"
            );
        }
    }

    #[test]
    fn the_summary_counts_problems_and_files() {
        let dir = std::env::temp_dir().join(format!("jsonvet-summary-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let dup = dir.join("dup.json");
        let ok = dir.join("ok.json");
        std::fs::write(&dup, "{\"a\": 1, \"a\": 2}").expect("write");
        std::fs::write(&ok, "{}").expect("write");
        let run = |summary: bool, files: &[&std::path::Path]| {
            let mut out = Vec::new();
            let args = ["--no-config".into()]
                .into_iter()
                .chain(files.iter().map(|f| f.as_os_str().to_owned()));
            let terminal = super::Terminal {
                tty: summary,
                color: Some(false),
            };
            let code = super::run_with(args, false, terminal, &mut out).expect("a check");
            (code, String::from_utf8(out).expect("UTF-8"))
        };
        let (code, out) = run(true, &[&dup, &ok]);
        assert_eq!(code, 1);
        assert!(out.ends_with("\n1 error in 1 of 2 files\n"), "{out}");
        // A file that cannot be read counts as a file with an error.
        let gone = dir.join("gone.json");
        // The clean file last, so counting it instead of a file with problems shows.
        let (_, out) = run(true, &[&dup, &gone, &dup, &ok]);
        assert!(out.ends_with("\n3 errors in 3 of 4 files\n"), "{out}");
        assert_eq!(run(true, &[&ok]).1, "no problems in 1 file\n");
        // No summary off a terminal, nor for no files.
        assert!(!run(false, &[&dup]).1.contains(" in 1 of 1 file"));
        assert_eq!(run(true, &[]).1, "");
        // SARIF and JSON stay one document on a terminal too.
        for format in ["sarif", "json"] {
            let args = ["--no-config", "--format", format]
                .map(std::ffi::OsString::from)
                .into_iter()
                .chain([dup.clone().into_os_string()]);
            let mut out = Vec::new();
            let terminal = super::Terminal {
                tty: true,
                color: None,
            };
            super::run_with(args, false, terminal, &mut out).expect("a check");
            let out = String::from_utf8(out).expect("UTF-8");
            assert!(
                serde_json::from_str::<serde_json::Value>(&out).is_ok(),
                "{out}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_panic_on_the_big_stack_carries_on_in_the_caller() {
        static RUNS: AtomicUsize = AtomicUsize::new(0);
        let panic = std::panic::catch_unwind(|| {
            super::on_big_stack(|| -> u8 {
                RUNS.fetch_add(1, Ordering::Relaxed);
                panic!("on the big stack")
            })
        })
        .expect_err("the panic carries on");
        assert_eq!(panic.downcast_ref::<&str>(), Some(&"on the big stack"));
        assert_eq!(RUNS.load(Ordering::Relaxed), 1);
    }
}
