//! The jsonvet command line, shared by the `jsonvet` and `check-json5` binaries.

use std::ffi::{OsStr, OsString};
use std::io::{self, BufWriter, IsTerminal, Read, Write};
use std::path::Path;
use std::process::ExitCode;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use jsonvet::{Dialect, Report, Severity};

mod args;
mod frame;
mod glob;
mod json;
mod messages;
mod output;
mod rules;
mod sarif;
mod schema;
mod settings;

use args::{Command, STDIN};
use output::{Format, Look, Printer};
use schema::Schema;
use schema::select::{Job, Schemas, check_own_schema, jobs, with_schema};
use settings::{Resolver, Settings};

/// The stack every run uses, for `--schema`: jsonschema validates recursively, and building,
/// validating and dropping a value [`schema::SCHEMA_MAX_DEPTH`] levels deep against a recursive schema
/// took up to 2.2 MB in a debug build, more than Windows' 1 MB main thread. Only the pages used
/// are committed, so a plain check costs no more memory.
const STACK: usize = 64 << 20;

/// How much memory the files read at once may take before they are checked and printed.
const CHUNK_BYTES: usize = 64 << 20;

/// How many bytes of memory a byte of file text takes with a schema, which holds the file's value
/// one document at a time: 4 to 22 per byte of that document (docs/usage.md, "JSON Schema
/// limits"). 20 is near the top of that range: only many small records take more.
const SCHEMA_BYTES: usize = 20;

/// From this size, a JSON Lines file is split at newlines and its parts checked on the cores
/// its chunk leaves free.
const SPLIT_BYTES: usize = 1 << 20;

/// Run the command with the process's arguments and environment, and return its exit code.
pub fn run() -> ExitCode {
    // The release profile aborts on a panic, which ends the process with a signal and no way to
    // report it. A hook runs first, so a bug exits 101 with a pointer to the issue tracker.
    std::panic::set_hook(Box::new(|info| {
        eprintln!(
            "jsonvet: internal error: {info}\n\nThis is a bug in jsonvet. Please report it at \
             https://github.com/simonvanlierde/jsonvet/issues with the command you ran and, if \
             you can share it, the file that caused it."
        );
        std::process::exit(101);
    }));
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
    // NOTE: a hidden trigger for tests/cli.rs, so the panic hook runs without a real bug. Debug
    // builds only, so release binaries cannot be made to panic on purpose.
    #[cfg(debug_assertions)]
    assert!(
        std::env::var_os("JSONVET_TEST_PANIC").is_none(),
        "test panic"
    );
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

/// The end of a run: the `line` of counts as text on a terminal (`text`), and as an annotation
/// when `found` problems, then whatever the format closes with.
fn print_summary(
    printer: &mut Printer<impl Write>,
    line: Option<&str>,
    text: bool,
    found: bool,
) -> io::Result<()> {
    if let Some(line) = line {
        if text {
            printer.text_line(line)?;
        }
        if found {
            printer.summary_annotation(line)?;
        }
    }
    printer.finish()
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
    let args = match args::parse(args)
        .map_err(|e| messages::fill("error.usage", &[("error", &output::printable(&e))]))?
    {
        Command::Help => return print(out, messages::text("help")),
        Command::Version => return print(out, concat!("jsonvet ", env!("CARGO_PKG_VERSION"))),
        Command::ListRules => return print(out, &rules::list()),
        Command::Explain(text) => return print(out, &text),
        Command::Check(args) => args,
    };
    messages::english_only(args.format != Format::Text);
    let resolver = Arc::new(Mutex::new(Resolver::new(&args.config, args.settings)?));
    let settings_for = |path: &Path| lock(&resolver).for_file(path);
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let files = expand(args.files, &resolver, threads)?;
    let schemas = Schemas::default();
    let load = |path: &Path, quiet| {
        schemas.get(path, quiet, |admit| {
            Schema::load(path, &*settings_for(path)?, quiet, admit)
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
    // A file whose own `$schema` may name a schema is known only once read: it counts the same,
    // and the texts read past the end of the chunk are kept, in `ahead`, for the next one.
    // NOTE: each chunk waits for its slowest file before the next starts; hand files out
    // one at a time across chunks if one big file among many small ones holds a run up.
    let mut rest = &files[..];
    let shares = schema_shares(&files, threads);
    let mut ahead = Vec::new();
    let written = loop {
        if rest.is_empty() {
            let line = (!files.is_empty()).then(|| counts.summary(files.len()));
            let found = counts.errors + counts.warnings > 0;
            break print_summary(&mut printer, line.as_deref(), summary, found);
        }
        let shares = shares.get(files.len() - rest.len()..);
        let with_schema = |job: &Job, text: &str| with_schema(job, &shown(&job.1), text);
        let texts = take_chunk(&mut ahead, rest, shares, threads, &with_schema);
        let chunk;
        (chunk, rest) = rest.split_at(texts.len());
        let jobs: Vec<_> = chunk.iter().zip(&texts).collect();
        let parts = (threads / jobs.len()).max(1);
        let checked = parallel(
            &jobs,
            threads,
            || false,
            |(((settings, schema), file), (text, with))| {
                let name = shown(file);
                let name = name.to_string_lossy();
                text.as_deref().map(|text| match schema {
                    Some(schema) => check(&name, text, settings, Some(schema), parts),
                    None if *with => {
                        check_own_schema(&name, text, settings, &schemas, &root, &load, parts)
                    }
                    None => check(&name, text, settings, None, parts),
                })
            },
        );
        schemas.over().map_err(|e| output::printable(&e))?;
        let outcome =
            chunk
                .iter()
                .zip(checked)
                .try_for_each(|((_, file), checked)| match checked {
                    Ok((report, schema_errors)) => print_file(
                        &shown(file),
                        &report,
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
            return Err(messages::fill("error.write", &[("error", &e)]));
        }
        _ => {}
    }
    schemas.note_missing();
    let too_many_warnings = args.max_warnings.is_some_and(|max| counts.warnings > max);
    Ok(u8::from(counts.errors > 0 || too_many_warnings))
}

/// The texts of the first of `files`, read on up to `threads` threads until their [`weight`]s
/// reach [`CHUNK_BYTES`], and always the first, each with whether `with_schema` says it is
/// checked against a schema.
fn read_chunk<S: Sync>(
    files: &[(S, OsString)],
    threads: usize,
    with_schema: &(impl Fn(&(S, OsString), &str) -> bool + Sync),
) -> Vec<(Result<String, String>, bool)> {
    let bytes = AtomicUsize::new(0);
    let full = || bytes.load(Ordering::Relaxed) >= CHUNK_BYTES;
    parallel(files, threads, full, |file| {
        let text = if file.1 == STDIN {
            read_stdin()
        } else {
            read(Path::new(&file.1))
        };
        let with = text.as_ref().is_ok_and(|text| with_schema(file, text));
        bytes.fetch_add(weight(&text, with), Ordering::Relaxed);
        (text, with)
    })
}

/// The texts of the next chunk of `rest`, the files left, each with whether it is checked
/// against a schema: up to and including the one whose [`weight`] fills [`CHUNK_BYTES`], from
/// `ahead`, the texts of the first of them read already, or if none are, from those read now.
/// `shares` are the files' [`schema_share`]s, if any.
fn take_chunk(
    ahead: &mut Vec<(Result<String, String>, bool)>,
    rest: &[Job],
    shares: Option<&[usize]>,
    threads: usize,
    with_schema: &(impl Fn(&Job, &str) -> bool + Sync),
) -> Vec<(Result<String, String>, bool)> {
    if ahead.is_empty() {
        let offered = match shares {
            Some(shares) if !shares.is_empty() => &rest[..chunk_len(shares)],
            _ => rest,
        };
        *ahead = read_chunk(offered, threads, with_schema);
    }
    let weights: Vec<usize> = ahead
        .iter()
        .map(|(text, with)| weight(text, *with))
        .collect();
    ahead.drain(..chunk_len(&weights)).collect()
}

/// The memory a text read takes while it is checked, as far as a chunk counts it: its size, or
/// with a schema, [`SCHEMA_BYTES`] times that, capped at [`CHUNK_BYTES`].
fn weight(text: &Result<String, String>, with_schema: bool) -> usize {
    let size = text.as_ref().map_or(0, String::len);
    if with_schema {
        size.saturating_mul(SCHEMA_BYTES).min(CHUNK_BYTES)
    } else {
        size
    }
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
            let entry = entry
                .map_err(|e| output::printable(&messages::fill("error.walk", &[("error", &e)])))?;
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
        let files = messages::count("summary.files", checked);
        if self.errors + self.warnings == 0 {
            return messages::fill("summary.none", &[("files", &files)]);
        }
        let errors = || messages::count("summary.errors", self.errors);
        let warnings = || messages::count("summary.warnings", self.warnings);
        let problems = match (self.errors, self.warnings) {
            (_, 0) => errors(),
            (0, _) => warnings(),
            _ => messages::fill(
                "summary.both",
                &[("errors", &errors()), ("warnings", &warnings())],
            ),
        };
        messages::fill(
            "summary.some",
            &[
                ("problems", &problems),
                ("failed", &self.files),
                ("files", &files),
            ],
        )
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
        // A file with a syntax error gets no schema errors.
        Some(schema) => schema.check(text, &config),
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

/// Print one file's problems and schema errors, and add them to `counts`, each before it is
/// printed, so a failed write still leaves the verdict. The file count is added last: only the
/// summary reads it, and a failed write ends the run before the summary.
fn print_file(
    file: &OsStr,
    report: &Report<'_>,
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
        printer.problem(file, report, problem)?;
    }
    for (offset, message, summary) in schema_errors {
        counts.errors += usize::from(!summary);
        printer.schema_error(file, report, *offset, message, *summary)?;
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
        Ok(None) => Err(messages::text("read.not-regular").to_owned()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            Err(messages::text("read.missing").to_owned())
        }
        Err(e) => Err(messages::fill("read.error", &[("error", &e)])),
    }
}

/// Standard input's text, or why it has none. Empty input is an empty file.
fn read_stdin() -> Result<String, String> {
    let mut bytes = Vec::new();
    match io::stdin().lock().read_to_end(&mut bytes) {
        Ok(_) => decode(bytes),
        Err(e) => Err(messages::fill("read.error", &[("error", &e)])),
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
        return Err(messages::fill("read.encoding", &[("encoding", &encoding)]));
    }
    String::from_utf8(bytes).map_err(|e| {
        let byte = e.utf8_error().valid_up_to();
        messages::fill("read.not-utf8", &[("byte", &byte)])
    })
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
        return Err(io::Error::other(messages::text("read.not-regular")));
    }
    let mut text = String::new();
    // The cap holds even for a file that grows after the check.
    std::fs::File::open(path)?
        .take(max + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > max {
        let mb = max >> 20;
        return Err(io::Error::other(messages::fill(
            "read.too-large",
            &[("mb", &mb)],
        )));
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
            messages::text("read.not-regular").to_owned()
        } else if meta.len() > MAX_CONFIG_BYTES {
            messages::fill("read.too-large", &[("mb", &(MAX_CONFIG_BYTES >> 20))])
        } else {
            return None;
        };
        let error = format!("{}: {problem}", path.display());
        Some(output::printable(&messages::fill(
            "error.walk",
            &[("error", &error)],
        )))
    })
}

fn print(mut out: impl Write, text: &str) -> Result<u8, String> {
    match writeln!(out, "{text}").and_then(|()| out.flush()) {
        Err(e) if e.kind() != io::ErrorKind::BrokenPipe => {
            Err(messages::fill("error.write", &[("error", &e)]))
        }
        _ => Ok(0),
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use jsonvet::Dialect;

    use super::{
        CHUNK_BYTES, MAX_CONFIG_BYTES, SCHEMA_BYTES, SPLIT_BYTES, STDIN, Schema, Settings, check,
        chunk_len, parallel, read_small, schema_share, schema_shares, split, unsafe_ignore_file,
    };
    use std::sync::Arc;

    /// A schema that takes any value, in a fresh folder named for `test`, which also holds an
    /// empty folder `x`.
    fn any_schema(test: &str) -> (std::path::PathBuf, Schema) {
        let dir = std::env::temp_dir().join(format!("jsonvet-{test}-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("x")).expect("a temporary directory");
        std::fs::write(dir.join("s.json"), "{}").expect("written");
        let schema = Schema::load(&dir.join("s.json"), &Settings::default(), false, |_| Ok(()));
        (dir, schema.expect("a schema"))
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
        let schema = Schema::load(&path, &settings, false, |_| Ok(())).expect("a schema");
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

    /// Text output takes a translation; SARIF and JSON stay English with one in place.
    #[test]
    fn a_translation_reaches_text_output_only() {
        let dir = std::env::temp_dir().join(format!("jsonvet-locale-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let dup = dir.join("dup.json");
        std::fs::write(&dup, "{\"a\": 1, \"a\": 2}").expect("write");
        crate::messages::use_translation(
            "\"duplicate-key\" = \"XX {path}\"\n\"related.first-at\" = \"XX {line}:{column}\"\n",
        );
        let run = |format: &str| {
            let args = ["--no-config", "--format", format]
                .map(std::ffi::OsString::from)
                .into_iter()
                .chain([dup.clone().into_os_string()]);
            let mut out = Vec::new();
            super::run_with(args, false, super::Terminal::default(), &mut out).expect("a check");
            String::from_utf8(out).expect("UTF-8")
        };
        let text = run("text");
        assert!(text.contains("XX $.a (XX 1:2)"), "{text}");
        for format in ["json", "sarif"] {
            let out = run(format);
            assert!(out.contains("duplicate key $.a (first at 1:2)"), "{out}");
            assert!(!out.contains("XX"), "{out}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A JSON Lines file and a `--schema` error get the same code frames on a terminal as any
    /// other problem, at the line they are on.
    #[test]
    fn json_lines_and_schema_errors_get_code_frames_on_a_terminal() {
        let dir = std::env::temp_dir().join(format!("jsonvet-frames-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let lines = dir.join("l.jsonl");
        let schema = dir.join("s.json");
        let data = dir.join("d.json");
        std::fs::write(&lines, "{\"a\": 1}\n{\"a\": True}\n").expect("write");
        std::fs::write(&schema, r#"{"properties": {"a": {"type": "string"}}}"#).expect("write");
        std::fs::write(&data, "{\n  \"a\": 1\n}\n").expect("write");
        let run = |args: &[&std::ffi::OsStr]| {
            let args = ["--no-config".into()]
                .into_iter()
                .chain(args.iter().map(|a| (*a).to_owned()));
            let terminal = super::Terminal {
                tty: true,
                color: Some(false),
            };
            let mut out = Vec::new();
            let code = super::run_with(args, false, terminal, &mut out).expect("a check");
            let out = String::from_utf8(out).expect("UTF-8");
            (
                code,
                out.replace(
                    &format!("{}{}", dir.display(), std::path::MAIN_SEPARATOR),
                    "",
                ),
            )
        };
        assert_eq!(
            run(&[lines.as_os_str()]),
            (
                1,
                "error[syntax]: expected a value, found `True`; did you mean `true`?\n \
                 --> l.jsonl:2:7\n  |\n2 | {\"a\": True}\n  |       ^^^^\n  |\n  = help: https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md#syntax\n\n\
                 1 error in 1 of 1 file\n"
                    .to_owned()
            )
        );
        assert_eq!(
            run(&["--schema".as_ref(), schema.as_os_str(), data.as_os_str()]),
            (
                1,
                "error[schema]: 1 is not of type \"string\" (at $.a)\n \
                 --> d.json:2:8\n  |\n2 |   \"a\": 1\n  |        ^\n  |\n  = help: https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md#schema\n\n\
                 1 error in 1 of 1 file\n"
                    .to_owned()
            )
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_summary_names_errors_and_warnings_together() {
        let counts = |errors, warnings| super::Counts {
            errors,
            warnings,
            files: 1,
        };
        assert_eq!(
            counts(1, 1).summary(2),
            "1 error and 1 warning in 1 of 2 files"
        );
        assert_eq!(counts(0, 2).summary(1), "2 warnings in 1 of 1 file");
    }

    #[test]
    fn a_line_separator_ends_a_frame_line_only_in_a_json5_file() {
        let dir = std::env::temp_dir().join(format!("jsonvet-u2028-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let frame = |name: &str| {
            let file = dir.join(name);
            std::fs::write(&file, "{\"a\": \"x\u{2028}y\", \"a\": 2}\n").expect("write");
            let mut out = Vec::new();
            let args = ["--no-config".into(), file.into_os_string()];
            let terminal = super::Terminal {
                tty: true,
                color: Some(false),
            };
            super::run_with(args, false, terminal, &mut out).expect("a check");
            String::from_utf8(out).expect("UTF-8")
        };
        let five = frame("a.json5");
        // Both the duplicate and the first copy, whose line ends at the separator.
        assert!(five.contains("\n2 | y\", \"a\": 2}\n"), "{five}");
        assert!(five.contains("\n1 | {\"a\": \"x\n"), "{five}");
        let other = frame("a.jsonc");
        // The frame shows U+2028 as a replacement character, on the one line.
        assert!(
            other
                .lines()
                .any(|l| l.starts_with("1 | {\"a\": \"x\u{FFFD}y\", \"a\": 2}")),
            "{other}"
        );
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
