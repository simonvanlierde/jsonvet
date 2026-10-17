//! The jsonvet command line, shared by the `jsonvet` and `check-json5` binaries.

use std::ffi::{OsStr, OsString};
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::process::ExitCode;

use jsonvet::Severity;

mod args;
mod glob;
mod output;
mod sarif;
#[cfg(feature = "schema")]
mod schema;
mod settings;

/// A build without the `schema` feature, such as the Python wheels, still takes `--schema`, and
/// says why it cannot use it.
#[cfg(not(feature = "schema"))]
mod schema {
    use std::path::Path;

    use jsonvet::Event;

    use crate::settings::Settings;

    const NO_SCHEMA: &str = "--schema is not in this build: the PyPI wheels, which the \
        pre-commit hook and the GitHub Action install, leave it out to stay small. `cargo install \
        jsonvet-cli` builds jsonvet with it.";

    /// No schema can be loaded, so none exists.
    pub(crate) enum Schema {}

    impl Schema {
        pub(crate) fn load(_: &Path, _: &Settings) -> Result<Schema, String> {
            Err(NO_SCHEMA.to_owned())
        }

        pub(crate) fn errors(&self, _: &[Event<'_>]) -> Vec<(usize, String, bool)> {
            match *self {}
        }
    }
}

use args::{Command, ConfigSource};
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

/// Run the command with the process's arguments and environment, and return its exit code.
/// `--schema` needs the `schema` feature, on by default; without it, `--schema` exits with
/// status 2.
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
    match run_with(
        std::env::args_os().skip(1),
        annotate,
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
/// A usage error comes back as `Err`, which `run` turns into exit status 2.
fn run_with(
    args: impl IntoIterator<Item = OsString>,
    annotate: bool,
    out: impl Write,
) -> Result<u8, String> {
    let args =
        match args::parse(args).map_err(|e| format!("{e}\n\nRun `jsonvet --help` for usage."))? {
            Command::Help => return print(out, args::HELP),
            Command::Version => return print(out, concat!("jsonvet ", env!("CARGO_PKG_VERSION"))),
            Command::Check(args) => args,
        };
    let from_file = match &args.config {
        ConfigSource::None => Settings::default(),
        ConfigSource::File(path) => settings::load(path)?,
        ConfigSource::Search => {
            let cwd = std::env::current_dir()
                .map_err(|e| format!("cannot read the working directory: {e}"))?;
            match settings::discover(&cwd) {
                Some(path) => settings::load(&path)?,
                None => Settings::default(),
            }
        }
    };
    let settings = from_file.overridden_by(args.settings);
    let schema = args
        .schema
        .as_deref()
        .map(|path| Schema::load(path, &settings))
        .transpose()?;

    let mut printer = Printer::new(out, args.format, annotate);
    let mut counts = Counts::default();
    let written = args
        .files
        .iter()
        .try_for_each(|file| {
            check_file(file, &settings, schema.as_ref(), &mut printer, &mut counts)
        })
        .and_then(|()| printer.finish());
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

#[derive(Default)]
struct Counts {
    errors: usize,
    warnings: usize,
}

/// Check one file, and against `schema` if there is one, and add its problems to `counts`,
/// each before it is printed, so a failed write still leaves the verdict. The name is opened as
/// given; its lossy UTF-8 form is only for output and glob matching.
fn check_file(
    file: &OsStr,
    settings: &Settings,
    schema: Option<&Schema>,
    printer: &mut Printer<impl Write>,
    counts: &mut Counts,
) -> io::Result<()> {
    let text = read(Path::new(file));
    let name = file.to_string_lossy();
    let text = match text {
        Ok(text) => text,
        Err(message) => {
            counts.errors += 1;
            return printer.unreadable(file, &message);
        }
    };
    let config = settings.config_for(&name);
    let (report, events) = match schema {
        Some(_) => jsonvet::parse(&text, &config.max_depth(SCHEMA_MAX_DEPTH)),
        None => (jsonvet::check(&text, &config), None),
    };
    for problem in report.problems() {
        match problem.severity() {
            Severity::Error => counts.errors += 1,
            Severity::Warning => counts.warnings += 1,
            _ => {}
        }
        printer.problem(file, &report, problem)?;
    }
    // `events` is `None` after a syntax error, so such a file gets no schema errors.
    if let (Some(schema), Some(events)) = (schema, events) {
        for (offset, message, summary) in schema.errors(&events) {
            counts.errors += usize::from(!summary);
            printer.schema_error(file, &report, offset, &message, summary)?;
        }
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
        Ok(Some(bytes)) => String::from_utf8(bytes)
            .map_err(|e| format!("not valid UTF-8 (byte {})", e.utf8_error().valid_up_to())),
        Ok(None) => Err("not a regular file".to_owned()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Err("no such file".to_owned()),
        Err(e) => Err(format!("cannot read: {e}")),
    }
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
