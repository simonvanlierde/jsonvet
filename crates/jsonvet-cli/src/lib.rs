//! The jsonvet command line, shared by the `jsonvet` and `check-json5` binaries.

use std::ffi::{OsStr, OsString};
use std::io::{self, BufWriter, IsTerminal, Read, Write};
use std::path::Path;
use std::process::ExitCode;

use std::sync::atomic::{AtomicUsize, Ordering};

use jsonvet::{Report, Severity};

mod args;
mod glob;
mod output;
mod rules;
mod sarif;
mod schema;
mod settings;

use args::{Command, ConfigSource, STDIN};
use output::Printer;
use schema::Schema;
use settings::Settings;

/// How deep `--schema` reads a file. Past it, the file has a syntax error. jsonschema 0.58 takes
/// memory quadratic in depth for an error deep under a recursive `$ref`: 1.3 GB for a 50 KB file
/// nested 10,000 levels.
const SCHEMA_MAX_DEPTH: usize = 512;

/// The stack every run uses, for `--schema`: jsonschema validates recursively, and building,
/// validating and dropping a value [`SCHEMA_MAX_DEPTH`] levels deep against a recursive schema
/// took up to 2.2 MB in a debug build, more than Windows' 1 MB main thread. Only the pages used
/// are committed, so a plain check costs no more memory.
const STACK: usize = 64 << 20;

/// How many bytes of file text a run reads before it checks and prints them.
const CHUNK_BYTES: usize = 64 << 20;

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
    let settings = from_file(&args.config)?.overridden_by(args.settings);
    let schema = args
        .schema
        .as_deref()
        .map(|path| Schema::load(path, &settings))
        .transpose()
        .map_err(|e| output::printable(&e))?;

    if args.files.is_empty() && stdin_is_piped() {
        eprintln!("jsonvet: no files given; to check standard input, pass `-` as the file");
    }
    // Standard input goes by its `--stdin-filename`, for settings and in the output.
    let stdin_name = args.stdin_filename.unwrap_or_else(|| STDIN.into());
    let shown = |file: &OsStr| -> OsString {
        if file == STDIN {
            stdin_name.clone()
        } else {
            file.to_owned()
        }
    };
    let mut printer = Printer::new(out, args.format, annotate);
    let mut counts = Counts::default();
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    // Files are read and checked on every core, a chunk at a time, then printed here in the
    // order given. A chunk ends once its files hold CHUNK_BYTES, so memory stays near that.
    // shortcut: each chunk waits for its slowest file before the next starts; hand files out
    // one at a time across chunks if one big file among many small ones holds a run up.
    let mut rest = &args.files[..];
    let written = loop {
        if rest.is_empty() {
            let line =
                (summary && !args.files.is_empty()).then(|| counts.summary(args.files.len()));
            break line
                .map_or(Ok(()), |line| printer.text_line(&line))
                .and_then(|()| printer.finish());
        }
        let bytes = AtomicUsize::new(0);
        let full = || bytes.load(Ordering::Relaxed) >= CHUNK_BYTES;
        let texts = parallel(rest, threads, full, |file| {
            let text = if file == STDIN {
                read_stdin()
            } else {
                read(Path::new(file))
            };
            bytes.fetch_add(text.as_ref().map_or(0, String::len), Ordering::Relaxed);
            text
        });
        let chunk;
        (chunk, rest) = rest.split_at(texts.len());
        let jobs: Vec<_> = chunk.iter().zip(&texts).collect();
        let checked = parallel(
            &jobs,
            threads,
            || false,
            |(file, text)| {
                let name = shown(file);
                let name = name.to_string_lossy();
                text.as_deref()
                    .map(|text| check(&name, text, &settings, schema.as_ref()))
            },
        );
        let outcome = chunk
            .iter()
            .zip(checked)
            .try_for_each(|(file, checked)| match checked {
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

/// The settings from the config file `config` names, or none.
fn from_file(config: &ConfigSource) -> Result<Settings, String> {
    let path = match config {
        ConfigSource::None => None,
        ConfigSource::File(path) => Some(path.clone()),
        ConfigSource::Search => {
            let cwd = std::env::current_dir()
                .map_err(|e| format!("cannot read the working directory: {e}"))?;
            settings::discover(&cwd)
        }
    };
    path.map_or_else(|| Ok(Settings::default()), |path| settings::load(&path))
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

/// Check a file's text, named by its lossy UTF-8 form for glob matching, and against `schema`
/// if there is one: the report and the schema errors.
fn check<'t>(
    name: &str,
    text: &'t str,
    settings: &Settings,
    schema: Option<&Schema>,
) -> (Report<'t>, Vec<(usize, String, bool)>) {
    let config = settings.config_for(name);
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
        Ok(Some(bytes)) => utf8(bytes),
        Ok(None) => Err("not a regular file".to_owned()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Err("no such file".to_owned()),
        Err(e) => Err(format!("cannot read: {e}")),
    }
}

/// Standard input's text, or why it has none. Empty input is an empty file.
fn read_stdin() -> Result<String, String> {
    let mut bytes = Vec::new();
    match io::stdin().lock().read_to_end(&mut bytes) {
        Ok(_) => utf8(bytes),
        Err(e) => Err(format!("cannot read: {e}")),
    }
}

fn utf8(bytes: Vec<u8>) -> Result<String, String> {
    String::from_utf8(bytes)
        .map_err(|e| format!("not valid UTF-8 (byte {})", e.utf8_error().valid_up_to()))
}

/// Whether standard input is a pipe or a file, as in `echo '{}' | jsonvet`: not a terminal,
/// and on Unix not a device such as `/dev/null`, which CI jobs and hooks often leave it as.
fn stdin_is_piped() -> bool {
    let stdin = io::stdin();
    if stdin.is_terminal() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::fd::AsFd;
        use std::os::unix::fs::FileTypeExt;
        let meta = stdin
            .as_fd()
            .try_clone_to_owned()
            .and_then(|fd| std::fs::File::from(fd).metadata());
        meta.is_ok_and(|meta| meta.is_file() || meta.file_type().is_fifo())
    }
    #[cfg(not(unix))]
    true
}

fn print(mut out: impl Write, text: &str) -> Result<u8, String> {
    match writeln!(out, "{text}").and_then(|()| out.flush()) {
        Err(e) if e.kind() != io::ErrorKind::BrokenPipe => Err(format!("cannot write output: {e}")),
        _ => Ok(0),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::parallel;

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
        assert_eq!(run(true, &[&ok]).1, "no problems in 1 file\n");
        // No summary off a terminal, nor for no files.
        assert!(!run(false, &[&dup]).1.contains(" in 1 of 1 file"));
        assert_eq!(run(true, &[]).1, "");
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
