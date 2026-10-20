//! The jsonvet command line, shared by the `jsonvet` and `check-json5` binaries.

use std::ffi::{OsStr, OsString};
use std::io::{self, BufWriter, IsTerminal, Read, Write};
use std::path::Path;
use std::process::ExitCode;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use jsonvet::{Dialect, Report, Severity};

mod args;
mod glob;
mod json;
mod output;
mod rules;
mod sarif;
mod schema;
mod settings;

use args::{Command, STDIN};
use output::Printer;
use schema::Schema;
use settings::{Resolver, Settings};

/// How deep `--schema` reads a file. Past it, the file has a syntax error. jsonschema 0.58 takes
/// memory quadratic in depth for an error deep under a recursive `$ref`: 1.3 GB for a 50 KB file
/// nested 10,000 levels.
const SCHEMA_MAX_DEPTH: usize = 512;

/// The stack every run uses, for `--schema`: jsonschema validates recursively, and building,
/// validating and dropping a value [`SCHEMA_MAX_DEPTH`] levels deep against a recursive schema
/// took up to 2.2 MB in a debug build, more than Windows' 1 MB main thread. Only the pages used
/// are committed, so a plain check costs no more memory.
const STACK: usize = 64 << 20;

/// How much memory the files read at once may take before they are checked and printed.
const CHUNK_BYTES: usize = 64 << 20;

/// How many bytes of memory a byte of file text takes with `--schema`, which holds the file's
/// events and value: 14 to 27 (docs/design.md, "Memory").
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
    // A summary is for people: hooks and CI read stdout from a pipe and see the same output.
    let summary = stdout.is_terminal();
    match run_with(
        std::env::args_os().skip(1),
        annotate,
        summary,
        BufWriter::new(stdout),
    ) {
        Ok(code) => ExitCode::from(code),
        Err(message) => {
            eprintln!("jsonvet: {message}");
            ExitCode::from(2)
        }
    }
}

/// The command, with its inputs passed in: 0 when no file has an error, 1 when one does.
/// With `summary`, text output ends with a count of the problems. A usage error comes back as
/// `Err`, which `run` turns into exit status 2.
fn run_with(
    args: impl IntoIterator<Item = OsString>,
    annotate: bool,
    summary: bool,
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
    let schema = args
        .schema
        .as_deref()
        .map(|path| Schema::load(path, &*settings_for(path)?))
        .transpose()
        .map_err(|e| output::printable(&e))?;

    // Standard input goes by its `--stdin-filename`, for settings and in the output.
    let stdin_name = args.stdin_filename.unwrap_or_else(|| STDIN.into());
    let shown = |file: &OsStr| -> OsString {
        if file == STDIN {
            stdin_name.clone()
        } else {
            file.to_owned()
        }
    };
    // Every config file is read before any file is checked, so a bad one stops the run at once.
    let files = files
        .into_iter()
        .map(|file| Ok((settings_for(Path::new(&shown(&file)))?, file)))
        .collect::<Result<Vec<_>, String>>()?;
    let mut printer = Printer::new(out, args.format, annotate);
    let mut counts = Counts::default();
    // Files are read and checked on every core, a chunk at a time, then printed here in the
    // order given. A chunk ends once its files hold CHUNK_BYTES, so memory stays near that. With
    // `--schema` a file takes 20 times its size, so the chunk is cut from the sizes before any
    // file is read: else every core takes a file at once. Without, sizing every file first made
    // a walk of 10,000 files 7 ms slower.
    // shortcut: each chunk waits for its slowest file before the next starts; hand files out
    // one at a time across chunks if one big file among many small ones holds a run up.
    let mut rest = &files[..];
    let shares = match schema {
        Some(_) => parallel(&files, threads, || false, |(_, file)| schema_share(file)),
        None => Vec::new(),
    };
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
            |((settings, file), text)| {
                let name = shown(file);
                let name = name.to_string_lossy();
                text.as_deref()
                    .map(|text| check(&name, text, settings, schema.as_ref(), parts))
            },
        );
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

/// The memory a file takes while `--schema` checks it, as far as a chunk counts it:
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
        // The filter must own what it reads.
        let resolver = Arc::clone(resolver);
        // A config file that cannot be read excludes nothing here: the run reports it when it
        // looks up the settings of the files under it.
        let walk = ignore::WalkBuilder::new(&arg)
            .filter_entry(move |entry| {
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
        let lines = parallel(
            &ranges,
            parts,
            || false,
            |lines| jsonvet::parse_lines(text, lines.clone(), &config, schema.is_some()),
        );
        let (report, events) = jsonvet::join_lines(text, lines);
        let errors = schema
            .zip(events)
            .map_or_else(Vec::new, |(schema, events)| schema.errors(&events));
        return (report, errors);
    }
    match schema {
        Some(schema) => {
            let (report, events) = jsonvet::parse(text, &config.max_depth(SCHEMA_MAX_DEPTH));
            // `events` is `None` after a syntax error, so such a file gets no schema errors.
            let errors = events.map_or_else(Vec::new, |events| schema.errors(&events));
            (report, errors)
        }
        None => (jsonvet::check(text, &config), Vec::new()),
    }
}

/// Print one file's problems and schema errors, and add them to `counts`, each before it is
/// printed, so a failed write still leaves the verdict.
fn print_file(
    file: &OsStr,
    report: &Report<'_>,
    schema_errors: &[(usize, String, bool)],
    printer: &mut Printer<impl Write>,
    counts: &mut Counts,
) -> io::Result<()> {
    let before = counts.errors + counts.warnings;
    let result = print_problems(file, report, schema_errors, printer, counts);
    counts.files += usize::from(counts.errors + counts.warnings > before);
    result
}

fn print_problems(
    file: &OsStr,
    report: &Report<'_>,
    schema_errors: &[(usize, String, bool)],
    printer: &mut Printer<impl Write>,
    counts: &mut Counts,
) -> io::Result<()> {
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
    // Without a mark, a match always failed as UTF-8 before: a zero byte is U+0000, and outside
    // a string no dialect allows it. JSON5 does allow it raw inside one, so a first byte that
    // opens a string is left alone.
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
        CHUNK_BYTES, SCHEMA_BYTES, SPLIT_BYTES, Schema, Settings, check, chunk_len, parallel,
        schema_share, split,
    };

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
        // Problems in every part: duplicate keys, and strings the schema wants as integers.
        let pad = "x".repeat(500);
        let mut text = String::from("\u{FEFF}");
        for i in 0..4_000 {
            let _ = match i % 7 {
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
        std::fs::write(&path, r#"{"properties": {"a": {"type": "integer"}}}"#).expect("written");
        let settings = Settings::default();
        let schema = Schema::load(&path, &settings).expect("a schema");
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
        assert_eq!(errors.len(), 572);
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
            let code = super::run_with(args, false, summary, &mut out).expect("a check");
            (code, String::from_utf8(out).expect("UTF-8"))
        };
        let (code, out) = run(true, &[&dup, &ok]);
        assert_eq!(code, 1);
        assert!(out.ends_with("\n1 error in 1 of 2 files\n"), "{out}");
        // A file that cannot be read counts as a file with an error.
        let gone = dir.join("gone.json");
        let (_, out) = run(true, &[&dup, &gone, &ok]);
        assert!(out.ends_with("\n2 errors in 2 of 3 files\n"), "{out}");
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
            super::run_with(args, false, true, &mut out).expect("a check");
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
