//! The jsonvet command line.

use std::cell::Cell;
use std::ffi::{OsStr, OsString};
use std::io::{self, BufWriter, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use ignore::Match;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use jsonvet::{Dialect, Problem, Report, Severity};

mod args;
mod catalog;
mod editorconfig;
mod format;
mod frame;
mod gitlab;
mod glob;
mod json;
mod lsp;
mod messages;
mod output;
mod rules;
mod sarif;
mod schema;
mod settings;
mod write;

use args::{Command, STDIN};
use output::{Format, Look, Printer};
use schema::Schema;
use schema::select::{Job, Schemas, check_own_schema, jobs, with_schema};
use settings::{Resolver, Settings};
use write::write_atomically;

/// The stack every run uses, for `--schema`: jsonschema validates recursively, and building,
/// validating and dropping a value [`schema::SCHEMA_MAX_DEPTH`] levels deep against a recursive schema
/// took up to 2.2 MB in a debug build, more than Windows' 1 MB main thread. Only the pages used
/// are committed, so a plain check costs no more memory.
const STACK: usize = 64 << 20;

/// How much memory the files read at once may take before they are checked and printed.
const CHUNK_BYTES: usize = 64 << 20;

/// How many bytes of memory a byte of file text takes with a schema, which holds the file's value
/// one document at a time. Kept above the most that docs/usage.md ("JSON Schema limits") gives,
/// so a chunk never overcommits.
const SCHEMA_BYTES: usize = 20;

/// From this size, a JSON Lines file is split at newlines and its parts checked on the cores
/// its chunk leaves free.
const SPLIT_BYTES: usize = 1 << 20;

/// Run the command with the process's arguments and environment, and return its exit code.
pub fn run() -> ExitCode {
    // The release profile aborts on a panic, which ends the process with a signal and no way to
    // report it. A hook runs first, so a bug exits 101 with a pointer to the issue tracker.
    std::panic::set_hook(Box::new(|info| {
        // NOTE: `eprintln!` would panic again here when stderr cannot be written, and abort.
        let _ = writeln!(
            io::stderr(),
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
    // Not locked: `jsonvet lsp` writes to it from a thread of its own.
    let stdout = io::stdout();
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
            // Nowhere to say it when stderr is the failure, but the exit code still tells.
            let _ = writeln!(io::stderr(), "jsonvet: {message}");
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

/// Whether `text` loads as the config file `path` (a `pyproject.toml` reads its `[tool.jsonvet]`
/// table), with the error message if not. Public only for the fuzz target `config_file`; not
/// part of the crate's API.
#[doc(hidden)]
pub fn check_config_text(path: &Path, text: &str) -> Result<(), String> {
    settings::load_str(path, text, false, None).map(drop)
}

/// The command, with its inputs passed in: 0 when no file has an error, 1 when one does.
/// On a terminal, text output ends with a count of the problems. A usage error comes back as
/// `Err`, which `run` turns into exit status 2; under `--format json` it is printed as a JSON
/// document and comes back as `Ok(2)`. Public only for `benches/codspeed.rs`, which measures
/// `--schema` in-process; not part of the crate's API.
#[doc(hidden)]
pub fn run_with(
    args: impl IntoIterator<Item = OsString>,
    annotate: bool,
    terminal: Terminal,
    out: impl Write,
) -> Result<u8, String> {
    let args: Vec<OsString> = args.into_iter().collect();
    let wrote = Cell::new(false);
    // `jsonvet fmt` and `lsp` take no `--format`, so their usage errors stay text.
    if args.first().is_some_and(|a| a == "fmt" || a == "lsp") {
        return run_checked(args, annotate, terminal, out, &mut false, &wrote);
    }
    let error_output = args::error_output(&args);
    // A JSON reader gets English, and a document it can parse, from the first byte.
    messages::english_only(true);
    let args::ErrorOutput::Json { stderr, lines } = error_output else {
        return run_checked(args, annotate, terminal, out, &mut false, &wrote);
    };
    let mut stdin_fix = stderr;
    let mut out = Tracked {
        inner: out,
        wrote: &wrote,
    };
    match run_checked(args, annotate, terminal, &mut out, &mut stdin_fix, &wrote) {
        // NOTE: after output has started, the document is cut off: a second one would not parse.
        // JSON Lines takes one more record at any point.
        Err(message) if lines || !wrote.get() => {
            let usage_help = messages::text("error.usage")
                .rsplit("\n\n")
                .next()
                .unwrap_or_default();
            let (message, help) = match message.strip_suffix(usage_help) {
                Some(message) => (message.trim_end(), usage_help),
                None => (
                    message.as_str(),
                    "Fix the file or option the message names, then run again.",
                ),
            };
            let document = if lines {
                json::failure_record(message, help)
            } else {
                json::failure(message, help)
            };
            // With `--fix` on standard input, standard output holds the fixed text.
            let sent = if stdin_fix {
                io::stderr().write_all(document.as_bytes())
            } else {
                out.write_all(document.as_bytes())
                    .and_then(|()| out.flush())
            };
            sent.map_err(|e| messages::fill("error.write", &[("error", &e)]))?;
            Ok(2)
        }
        result => result,
    }
}

/// A writer that notes whether anything went through it.
struct Tracked<'a, W> {
    inner: W,
    wrote: &'a Cell<bool>,
}

impl<W: Write> Write for Tracked<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.wrote.set(self.wrote.get() || !buf.is_empty());
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "the checking pipeline keeps its stages in execution order"
)]
fn run_checked(
    args: impl IntoIterator<Item = OsString>,
    annotate: bool,
    terminal: Terminal,
    out: impl Write,
    stdin_fix: &mut bool,
    wrote: &Cell<bool>,
) -> Result<u8, String> {
    // Usage errors quote arguments and the schema file, which can hold a newline or an escape.
    let args = match args::parse(args)
        .map_err(|e| messages::fill("error.usage", &[("error", &output::printable(&e))]))?
    {
        Command::Help => return print(out, messages::text("help")),
        Command::Version => return print(out, concat!("jsonvet ", env!("CARGO_PKG_VERSION"))),
        Command::ListRules => return print(out, &rules::list()),
        Command::Explain(text) => return print(out, &text),
        Command::Lsp => return lsp::run(),
        Command::Check(args) => *args,
    };
    messages::english_only(args.format != Format::Text);
    // Standard input fixed to standard output leaves only standard error for the problems,
    // including failures while loading configuration or expanding directories.
    *stdin_fix = args.fix && args.files.iter().any(|file| file == STDIN);
    let resolver = Arc::new(Mutex::new(Resolver::new(&args.config, args.settings)?));
    let settings_for = |path: &Path| lock(&resolver).for_file(path);
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let files = expand(args.files, &resolver, threads)?;
    if let Some(check_only) = args.fmt {
        return format::run(&files, check_only, args.stdin_filename, &resolver, out);
    }
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
    let gitlab_paths = (args.format == Format::GitLab)
        .then(|| gitlab::paths(&files, &shown, &lock(&resolver)))
        .transpose()?;
    let report_name = |file: &OsStr| {
        gitlab_paths
            .as_ref()
            .and_then(|paths| paths.get(file))
            .cloned()
            .unwrap_or_else(|| shown(file))
    };
    // Every config file, and every schema one names, is read before any file is checked, so a
    // bad one stops the run at once.
    let files = jobs(files, &shown, &settings_for, args.schema.as_deref(), &load)?;
    let mut catalog = args
        .schema_catalog
        .then(catalog::Catalog::new)
        .transpose()?;
    let look = look(args.format, args.color, terminal);
    let stdin_fix = *stdin_fix;
    let mut out = out;
    let mut stderr = Tracked {
        inner: io::stderr(),
        wrote,
    };
    // Standard error stands in for standard output's problems when that carries a fixed text.
    let sink: &mut dyn Write = if stdin_fix { &mut stderr } else { &mut out };
    let mut printer = Printer::new(sink, args.format, annotate, look, stdin_fix);
    let mut fixed = FixRun::default();
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
            let noted = (schemas.missing_notes().into_iter()).try_for_each(|n| printer.note(n));
            let line = (!files.is_empty()).then(|| counts.summary(files.len()));
            let found = counts.errors + counts.warnings > 0;
            let record = (files.len(), counts.files, counts.errors, counts.warnings);
            let code = verdict(&counts, args.max_warnings);
            break noted
                .and_then(|()| printer.summary_record(record, code))
                .and_then(|()| print_summary(&mut printer, line.as_deref(), terminal.tty, found));
        }
        let shares = shares.get(files.len() - rest.len()..);
        let with_schema =
            |job: &Job, text: &str| args.schema_catalog || with_schema(job, &shown(&job.1), text);
        let mut texts = take_chunk(&mut ahead, rest, shares, threads, &with_schema);
        let chunk;
        (chunk, rest) = rest.split_at(texts.len());
        let known = if args.fix {
            fix_chunk(
                chunk,
                &mut texts,
                &shown,
                args.unsafe_fixes,
                threads,
                &mut fixed,
            )
        } else {
            vec![None; texts.len()]
        };
        let mut selected = None;
        if let Some(catalog) = &mut catalog {
            selected = Some(catalog.select(chunk, &texts, &shown, &schemas)?);
            if let Err(e) = (catalog.take_notes().into_iter()).try_for_each(|n| printer.note(n)) {
                break Err(e);
            }
        }
        let chunk = selected.as_deref().unwrap_or(chunk);
        let jobs: Vec<_> = chunk.iter().zip(&texts).zip(&known).collect();
        let parts = (threads / jobs.len()).max(1);
        let checked = parallel(
            &jobs,
            threads,
            || false,
            |((((settings, schema), file), (text, with)), known)| {
                let name = shown(file);
                let name = name.to_string_lossy();
                text.as_deref().map(|text| match schema {
                    Some(schema) => check(&name, text, settings, Some(schema), parts, None),
                    None if *with => {
                        check_own_schema(&name, text, settings, &schemas, &root, &load, parts)
                    }
                    None => check(&name, text, settings, None, parts, known.as_deref()),
                })
            },
        );
        schemas.over().map_err(|e| output::printable(&e))?;
        let outcome = print_checked(chunk, checked, &report_name, &mut printer, &mut counts);
        if outcome.is_err() {
            break outcome;
        }
    };
    // A structured report on standard error takes the place of the count.
    let count = !stdin_fix || args.format == Format::Text;
    // The fixed text goes out even when the problems could not be written.
    let finished = fixed.finish(count, &mut out);
    let written = written.and(finished);
    conclude(written, &counts, args.max_warnings)
}

/// A file's report and its schema errors.
type Checked<'t> = (Report<'t>, Vec<schema::Diagnostic>);

/// Print what checking the files of `chunk` found, in order: the problems of each, or why it
/// could not be read.
fn print_checked(
    chunk: &[Job],
    checked: Vec<Result<Checked<'_>, &String>>,
    shown: &impl Fn(&OsStr) -> OsString,
    printer: &mut Printer<impl Write>,
    counts: &mut Counts,
) -> io::Result<()> {
    chunk
        .iter()
        .zip(checked)
        .try_for_each(|((_, file), checked)| {
            printer.begin_file();
            let file = &shown(file);
            match checked {
                Ok((report, schema_errors)) => {
                    print_file(file, &report, &schema_errors, printer, counts)?;
                }
                Err(message) => {
                    counts.errors += 1;
                    counts.files += 1;
                    printer.unreadable(file, message)?;
                }
            }
            printer.end_file(file)
        })
}

/// The exit code once the output is written: 1 for an error, or for more warnings than
/// `max_warnings` allows.
fn conclude(
    written: io::Result<()>,
    counts: &Counts,
    max_warnings: Option<usize>,
) -> Result<u8, String> {
    // A reader that has gone, as with `| head`, wants no more output: the verdict so far stands.
    match written {
        Err(e) if e.kind() != io::ErrorKind::BrokenPipe => {
            return Err(messages::fill("error.write", &[("error", &e)]));
        }
        _ => {}
    }
    Ok(verdict(counts, max_warnings))
}

/// 1 for an error, or for more warnings than `max_warnings` allows, and 0 otherwise.
fn verdict(counts: &Counts, max_warnings: Option<usize>) -> u8 {
    let too_many_warnings = max_warnings.is_some_and(|max| counts.warnings > max);
    u8::from(counts.errors > 0 || too_many_warnings)
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
        schema_bytes(size)
    } else {
        size
    }
}

/// [`SCHEMA_BYTES`] times `size`, capped at [`CHUNK_BYTES`].
fn schema_bytes(size: usize) -> usize {
    size.saturating_mul(SCHEMA_BYTES).min(CHUNK_BYTES)
}

/// The memory a file takes while a schema checks it, as far as a chunk counts it:
/// [`schema_bytes`] of its size. Standard input's size is known
/// only once it is read, so it takes a chunk's worth. A file that cannot be read takes nothing:
/// `read` reports why.
fn schema_share(file: &OsStr) -> usize {
    if file == STDIN {
        return CHUNK_BYTES;
    }
    let size = std::fs::metadata(file).map_or(0, |meta| meta.len());
    schema_bytes(usize::try_from(size).unwrap_or(usize::MAX))
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
    // Each argument costs a call to the system, so thousands of them are asked about in parallel:
    // whether it is a folder, and which file it is, for `unique`.
    let found = parallel(
        &args,
        threads,
        || false,
        |arg| {
            let meta = std::fs::metadata(arg);
            let dir = arg != STDIN && meta.as_ref().is_ok_and(std::fs::Metadata::is_dir);
            (dir, identity(arg, &meta))
        },
    );
    // The identity of each of `files`, or `None` for a file a walk found.
    let mut ids = Vec::with_capacity(args.len());
    // Folders that share no file need no check for repeats: the walk follows no symlinks.
    let mut roots = Vec::new();
    let mut distinct = true;
    // The user's global git ignore file, read once for every folder.
    let mut global = None;
    for (arg, (dir, id)) in args.into_iter().zip(found) {
        if !dir {
            distinct = false;
            files.push(arg);
            ids.push(Some(id));
            continue;
        }
        let start = files.len();
        let above = std::fs::canonicalize(&arg).ok();
        match &above {
            Some(root) => roots.push(root.clone()),
            None => distinct = false,
        }
        let global = global.get_or_insert_with(|| Arc::new(Gitignore::global().0));
        walk(&arg, above, global, resolver, &mut files)?;
        ids.resize(files.len(), None);
        files[start..].sort_unstable();
    }
    // Checking costs a call to the system for each file, 12% of a run over 10,000 small files.
    let overlap = |i: usize, root: &PathBuf| {
        roots[..i]
            .iter()
            .any(|r| r.starts_with(root) || root.starts_with(r))
    };
    if distinct && !roots.iter().enumerate().any(|(i, root)| overlap(i, root)) {
        return Ok(files);
    }
    Ok(unique(files, ids, threads))
}

/// Add to `files` the files under the folder `arg` that jsonvet checks by default, in the order
/// the walk finds them. `above` is the folder as a canonical path, if it has one, and `global`
/// the rules of the user's global git ignore file.
fn walk(
    arg: &OsStr,
    above: Option<PathBuf>,
    global: &Arc<Gitignore>,
    resolver: &Arc<Mutex<Resolver>>,
    files: &mut Vec<OsString>,
) -> Result<(), String> {
    let global = Arc::clone(global);
    // The rules of the folders above the walk, from `/` down, then of the walk's own.
    let mut rules = None;
    let mut ups: Vec<_> = above
        .iter()
        .flat_map(|dir| dir.ancestors().skip(1))
        .collect();
    ups.reverse();
    for dir in ups {
        rules = folder_rules(rules.as_ref(), dir, true)?;
    }
    let root = PathBuf::from(arg);
    let rules = folder_rules(rules.as_ref(), &root, false)?;
    let absolute = above.unwrap_or_default();
    // The rules of each folder the walk has let in, by its path as walked.
    let mut folders = std::collections::HashMap::new();
    folders.insert(root.clone(), rules);
    let folders = Mutex::new(folders);
    let skipped = Arc::new(Mutex::new(None));
    let skip = Arc::clone(&skipped);
    // The filter must own what it reads.
    let resolver = Arc::clone(resolver);
    // A config file that cannot be read excludes nothing here: the run reports it when it
    // looks up the settings of the files under it. The `ignore` crate's own ignore-file
    // readers stay off: they open a FIFO or `/dev/zero` unchecked.
    let walk = ignore::WalkBuilder::new(arg)
        .standard_filters(false)
        .filter_entry(move |entry| {
            let path = entry.path();
            let dir = entry.file_type().is_some_and(|t| t.is_dir());
            let mut folders = folders
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let rules = path
                .parent()
                .and_then(|up| folders.get(up))
                .cloned()
                .flatten();
            let matched = rules.as_ref().map_or(ignore::Match::None, |rules| {
                let absolute = || absolute.join(path.strip_prefix(&root).unwrap_or(path));
                rules.matched(path, dir, absolute, &global)
            });
            if matched.is_ignore() || (matched.is_none() && hidden(entry)) {
                return false;
            }
            let settings = lock(&resolver).for_file(path);
            if settings.is_ok_and(|settings| settings.excluded(path)) {
                return false;
            }
            if dir {
                match folder_rules(rules.as_ref(), path, false) {
                    Ok(rules) => {
                        folders.insert(path.to_path_buf(), rules);
                    }
                    Err(problem) => {
                        // A poisoned lock still holds the message.
                        skip.lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .get_or_insert(problem);
                        return false;
                    }
                }
            }
            true
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
    Ok(())
}

/// What names a file apart from its spelling.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Identity {
    #[cfg(unix)]
    Inode(u64, u64),
    Path(PathBuf),
}

/// The identity of `file`, from `meta`, what `std::fs::metadata` said about it.
fn identity(file: &OsStr, meta: &io::Result<std::fs::Metadata>) -> Identity {
    #[cfg(unix)]
    if let Ok(meta) = meta {
        use std::os::unix::fs::MetadataExt;
        return Identity::Inode(meta.dev(), meta.ino());
    }
    // Elsewhere the metadata has no stable file id, so the canonical path stands in.
    #[cfg(not(unix))]
    let _ = meta;
    // A file that cannot be found stays as spelled, so it is reported once per spelling.
    Identity::Path(std::fs::canonicalize(file).unwrap_or_else(|_| file.into()))
}

/// `files` without any that is the same file as an earlier one, so that each is checked once,
/// under its first spelling. Standard input can be given only once, as the arguments check.
/// `ids` holds each file's identity where it is known already.
fn unique(files: Vec<OsString>, ids: Vec<Option<Identity>>, threads: usize) -> Vec<OsString> {
    let ids: Vec<Identity> = if ids.iter().all(Option::is_some) {
        ids.into_iter().flatten().collect()
    } else {
        let files: Vec<_> = files.iter().zip(ids).collect();
        parallel(
            &files,
            threads,
            || false,
            |(file, id)| {
                id.clone()
                    .unwrap_or_else(|| identity(file, &std::fs::metadata(file)))
            },
        )
    };
    let mut seen = std::collections::HashSet::with_capacity(files.len());
    files
        .into_iter()
        .zip(ids)
        .filter_map(|(file, id)| seen.insert(id).then_some(file))
        .collect()
}

/// The resolver, even if a panic elsewhere left its lock poisoned: it holds only a cache.
fn lock(resolver: &Mutex<Resolver>) -> std::sync::MutexGuard<'_, Resolver> {
    resolver
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What `--fix` did over the whole run.
#[derive(Default)]
struct FixRun {
    problems: usize,
    files: usize,
    /// Standard input as it is after the fixes, for standard output.
    stdin: Option<String>,
}

impl FixRun {
    /// Print the count to standard error when `count` is set, then standard input's fixed text to
    /// `out`, which is written even if the count was not.
    fn finish(&self, count: bool, out: &mut impl Write) -> io::Result<()> {
        let counted = match self.summary() {
            Some(summary) if count => writeln!(io::stderr(), "{summary}"),
            _ => Ok(()),
        };
        let text = match &self.stdin {
            Some(text) => out.write_all(text.as_bytes()).and_then(|()| out.flush()),
            None => Ok(()),
        };
        counted.and(text)
    }

    /// Such as `fixed 3 problems in 2 files`, or `None` when nothing was fixed.
    fn summary(&self) -> Option<String> {
        (self.problems > 0).then(|| {
            messages::fill(
                "summary.fixed",
                &[
                    (
                        "problems",
                        &messages::count("summary.problems", self.problems),
                    ),
                    ("files", &messages::count("summary.files", self.files)),
                ],
            )
        })
    }
}

/// Fix the `texts` of `chunk` in place, and write each changed file, so the check that follows
/// reports what is left. A file that cannot be written gets the reason in place of its text.
fn fix_chunk(
    chunk: &[Job],
    texts: &mut [(Result<String, String>, bool)],
    shown: &(impl Fn(&OsStr) -> OsString + Sync),
    unsafe_fixes: bool,
    threads: usize,
    run: &mut FixRun,
) -> Vec<Option<Vec<Problem>>> {
    let jobs: Vec<_> = chunk.iter().zip(texts.iter()).collect();
    let done = parallel(
        &jobs,
        threads,
        || false,
        |(((settings, _), file), (text, _))| {
            let text = text.as_ref().ok()?;
            let name = shown(file);
            let config = settings.config_for(&name.to_string_lossy(), text);
            let fixed = match jsonvet::try_fix(text, &config, unsafe_fixes, None) {
                Ok(fixed) => fixed,
                Err(problems) => return Some(Err(problems)),
            };
            let saved = if file == STDIN {
                Ok(())
            } else {
                write_atomically(Path::new(file), &fixed.text)
            };
            Some(Ok((fixed, saved)))
        },
    );
    drop(jobs);
    let mut known = Vec::with_capacity(done.len());
    for (((_, file), entry), done) in chunk.iter().zip(texts.iter_mut()).zip(done) {
        // The scan's problems stand for the check only while the text is unchanged.
        known.push(match &done {
            Some(Err(problems)) => problems.clone(),
            _ => None,
        });
        let done = done.and_then(Result::ok);
        match done {
            Some((fixed, Ok(()))) => {
                run.problems += fixed.fixes.len();
                run.files += 1;
                entry.0 = Ok(fixed.text);
            }
            Some((_, Err(error))) => {
                entry.0 = Err(error);
            }
            None => {}
        }
        if file == STDIN
            && let Ok(text) = &entry.0
        {
            run.stdin = Some(text.clone());
        }
    }
    known
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
    known: Option<&[Problem]>,
) -> Checked<'t> {
    let config = settings.config_for(name, text);
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
        // The problems `--fix` found on its way, when it changed nothing.
        None => match known {
            Some(problems) => (
                jsonvet::report(text, problems.to_vec(), &config),
                Vec::new(),
            ),
            None => (jsonvet::check(text, &config), Vec::new()),
        },
    }
}

/// Check `text` as a run checks the file at `path`: with its settings and its schema, if any.
/// `Err` is what ends such a run: a config file or schema that cannot be used.
/// `on_schema` receives local schema dependencies, including missing ones and failed loads.
fn check_file<'t>(
    path: &Path,
    text: &'t str,
    resolver: Resolver,
    on_schema: &impl Fn(&Path),
) -> Result<Checked<'t>, String> {
    let resolver = Mutex::new(resolver);
    let settings_for = |path: &Path| lock(&resolver).for_file(path);
    let schemas = Schemas::default();
    let load = |path: &Path, quiet| {
        on_schema(path);
        schemas.get(path, quiet, |admit| {
            Schema::load(path, &*settings_for(path)?, quiet, admit)
        })
    };
    let root = |path: &Path| lock(&resolver).root(path);
    let jobs = jobs(
        vec![path.into()],
        &OsStr::to_owned,
        &settings_for,
        None,
        &load,
    )?;
    // One file in, one job out.
    let Some(job @ ((settings, schema), _)) = jobs.first() else {
        return Err(String::new());
    };
    let name = path.to_string_lossy();
    let checked = match schema {
        Some(schema) => check(&name, text, settings, Some(schema), 1, None),
        None if with_schema(job, path.as_os_str(), text) => {
            check_own_schema(&name, text, settings, &schemas, &root, &load, 1)
        }
        None => check(&name, text, settings, None, 1, None),
    };
    schemas.observe_missing(on_schema);
    schemas.over()?;
    Ok(checked)
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
    schema_errors: &[schema::Diagnostic],
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
    for finding in schema_errors {
        counts.errors += usize::from(!finding.summary);
        printer.schema_error(file, report, finding)?;
    }
    counts.files += usize::from(counts.errors + counts.warnings > before);
    Ok(())
}

/// `path` opened for reading, with its size, if it is a regular file or a symlink to one. The
/// type is checked on the open file, not the path, so a path swapped for a FIFO or a device after
/// a check is still refused, and on Unix opening a FIFO does not wait for a writer.
fn open_regular(path: &Path) -> io::Result<Option<(std::fs::File, u64)>> {
    // shortcut: elsewhere the path is checked before the open, which a swap can race; Windows
    // has no FIFO to swap in at a file path.
    #[cfg(not(unix))]
    if !std::fs::metadata(path)?.is_file() {
        return Ok(None);
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    // A regular file reads the same without blocking.
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(&mut options, libc::O_NONBLOCK);
    let file = options.open(path)?;
    let meta = file.metadata()?;
    Ok(meta.is_file().then_some((file, meta.len())))
}

/// A file's text, or why it has none. Only regular files are read, so a FIFO cannot hang the
/// run and a device such as `/dev/zero` cannot fill memory. A symlink to a regular file is
/// followed, as `cat` would.
fn read(path: &Path) -> Result<String, String> {
    let read = open_regular(path).and_then(|opened| {
        if let Some((file, len)) = opened {
            // The size is known already: `fs::read`, and `File`'s own `read_to_end`, would ask
            // the system for it again. `take` reads to the end without asking. A file too big for
            // memory fails alone, as an error, rather than aborting the run, but only where an
            // oversized reservation fails: Linux's default heuristic overcommit
            // (`vm.overcommit_memory=0`). Under mode 1 or on macOS it succeeds and the read can
            // fill memory. NOTE: no cap against available memory, as std has no portable query.
            let mut bytes = Vec::new();
            bytes.try_reserve_exact(usize::try_from(len).unwrap_or(usize::MAX))?;
            file.take(u64::MAX)
                .read_to_end(&mut bytes)
                .map(|_| Some(bytes))
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
    String::from_utf8(read_small_bytes(path, max)?).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "stream did not contain valid UTF-8",
        )
    })
}

/// [`read_small`], as bytes.
fn read_small_bytes(path: &Path, max: u64) -> io::Result<Vec<u8>> {
    let Some((file, _)) = open_regular(path)? else {
        return Err(io::Error::other(messages::text("read.not-regular")));
    };
    let mut bytes = Vec::new();
    // The cap holds even for a file that grows after the check.
    file.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        let mb = max >> 20;
        return Err(io::Error::other(messages::fill(
            "read.too-large",
            &[("mb", &mb)],
        )));
    }
    Ok(bytes)
}

/// The rules of one folder's ignore files, and of the folders above it that have any, applied
/// as the `ignore` crate applies them by default.
struct IgnoreRules {
    up: Option<Arc<IgnoreRules>>,
    /// A folder above the walk's own, whose rules match absolute paths.
    above: bool,
    /// The folder holds a `.git` or a `.jj`, so it is a repository's root.
    repo: bool,
    ignore: Gitignore,
    git: Gitignore,
    exclude: Gitignore,
}

impl IgnoreRules {
    /// What the rules say of `path`, as walked; `absolute` gives it as an absolute path, for the
    /// folders above the walk. `.ignore` wins over `.gitignore`, over `info/exclude`, over the
    /// user's `global` file, and the nearest folder wins within each. The git files apply only in
    /// a repository, and not across a nested repository's root.
    fn matched<'a>(
        &'a self,
        path: &Path,
        dir: bool,
        absolute: impl Fn() -> PathBuf,
        global: &'a Gitignore,
    ) -> ignore::Match<&'a ignore::gitignore::Glob> {
        let chain = || std::iter::successors(Some(self), |rules| rules.up.as_deref());
        let in_repo = chain().any(|rules| rules.repo);
        let (mut ignore, mut git, mut exclude) = (Match::None, Match::None, Match::None);
        let mut past_repo = false;
        let mut whole = None;
        for rules in chain() {
            let path = if rules.above {
                whole.get_or_insert_with(&absolute).as_path()
            } else {
                path
            };
            if ignore.is_none() {
                ignore = rules.ignore.matched(path, dir);
            }
            if in_repo && !past_repo {
                if git.is_none() {
                    git = rules.git.matched(path, dir);
                }
                if exclude.is_none() {
                    exclude = rules.exclude.matched(path, dir);
                }
            }
            past_repo |= rules.repo;
        }
        let global = if in_repo {
            global.matched(path, dir)
        } else {
            Match::None
        };
        ignore.or(git).or(exclude).or(global)
    }
}

/// The ignore rules for `dir`, under those of the folders above it, `up`: `up` itself when
/// `dir` adds none. Each ignore file is read as a config file is, so a link to `/dev/zero`
/// or a FIFO stops the walk with an error instead of filling memory or hanging it. A file that
/// is missing or cannot be opened adds no rules, as in git.
fn folder_rules(
    up: Option<&Arc<IgnoreRules>>,
    dir: &Path,
    above: bool,
) -> Result<Option<Arc<IgnoreRules>>, String> {
    let dot_git = std::fs::metadata(dir.join(".git")).ok();
    let repo = dot_git.is_some() || dir.join(".jj").exists();
    let ignore = ignore_rules(dir, Some(&dir.join(".ignore")))?;
    let git = ignore_rules(dir, Some(&dir.join(".gitignore")))?;
    let git_dir = match dot_git {
        Some(meta) if meta.is_file() => common_dir(dir)?,
        Some(_) => Some(dir.join(".git")),
        None => None,
    };
    let exclude = ignore_rules(dir, git_dir.map(|d| d.join("info/exclude")).as_deref())?;
    if !repo && ignore.is_empty() && git.is_empty() && exclude.is_empty() {
        return Ok(up.cloned());
    }
    Ok(Some(Arc::new(IgnoreRules {
        up: up.cloned(),
        above,
        repo,
        ignore,
        git,
        exclude,
    })))
}

/// The folder a worktree's `.git` file leads to, which holds its `info/exclude`, if it has one.
fn common_dir(dir: &Path) -> Result<Option<PathBuf>, String> {
    let first_line = |path: &Path| match read_small(path, MAX_CONFIG_BYTES) {
        Ok(text) => Ok(text.lines().next().map(str::to_owned)),
        Err(e) if e.kind() == io::ErrorKind::Other => Err(walk_error(path, &e)),
        Err(_) => Ok(None),
    };
    let Some(git_dir) = first_line(&dir.join(".git"))?
        .and_then(|line| line.strip_prefix("gitdir: ").map(|d| dir.join(d)))
    else {
        return Ok(None);
    };
    // A submodule's `.git` file leads to a folder with no `commondir`, and git reads no
    // `info/exclude` for it.
    let Some(common) = first_line(&git_dir.join("commondir"))? else {
        return Ok(None);
    };
    Ok(Some(git_dir.join(common)))
}

/// The rules in the ignore file at `file`, matched relative to `dir`. As in git, a line that is
/// not a valid pattern matches nothing, and reading stops at the first line that is not UTF-8.
fn ignore_rules(dir: &Path, file: Option<&Path>) -> Result<Gitignore, String> {
    let mut builder = GitignoreBuilder::new(dir);
    if let Some(file) = file {
        match read_small_bytes(file, MAX_CONFIG_BYTES) {
            Ok(bytes) => {
                let text = match std::str::from_utf8(&bytes) {
                    Ok(text) => text,
                    Err(e) => {
                        let valid = &bytes[..e.valid_up_to()];
                        let whole = valid
                            .iter()
                            .rposition(|&b| b == b'\n')
                            .map_or(&[][..], |end| &valid[..end]);
                        std::str::from_utf8(whole).unwrap_or_default()
                    }
                };
                let text = text.strip_prefix('\u{feff}').unwrap_or(text);
                for line in text.lines() {
                    let _ = builder.add_line(None, line);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Other => return Err(walk_error(file, &e)),
            Err(_) => {}
        }
    }
    Ok(builder.build().unwrap_or_else(|_| Gitignore::empty()))
}

/// The walk's error for the file at `path`.
fn walk_error(path: &Path, problem: &io::Error) -> String {
    let error = format!("{}: {problem}", path.display());
    output::printable(&messages::fill("error.walk", &[("error", &error)]))
}

/// Whether the walk skips `entry` as hidden: its name starts with a dot, or on Windows the
/// system marks it hidden.
fn hidden(entry: &ignore::DirEntry) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_HIDDEN
        if entry
            .metadata()
            .is_ok_and(|meta| meta.file_attributes() & 0x2 != 0)
        {
            return true;
        }
    }
    entry.file_name().as_encoded_bytes().starts_with(b".")
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
        chunk_len, folder_rules, parallel, read_small, schema_share, schema_shares, split,
        take_chunk,
    };
    use crate::schema::select::Job;
    use std::sync::Arc;

    /// A writer that records what reaches it and fails on the single write of `]`, which closes
    /// a JSON report.
    #[derive(Default)]
    struct Probe {
        written: Vec<u8>,
    }

    impl std::io::Write for Probe {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if buf == b"]" {
                return Err(std::io::Error::other("closed"));
            }
            self.written.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn the_last_format_before_a_double_dash_decides() {
        let json = |args: &[&str]| {
            super::args::error_output(&args.iter().map(Into::into).collect::<Vec<_>>())
                != super::args::ErrorOutput::Text
        };
        assert!(json(&["--format", "json"]));
        assert!(json(&["--format=json", "a.json"]));
        assert!(json(&["--format=jsonl", "a.json"]));
        assert!(!json(&["--format", "json", "--format=text"]));
        assert!(json(&["--format=text", "--format", "json"]));
        assert!(!json(&["--", "--format=json"]));
        assert!(!json(&["a.json"]));
    }

    /// A failure after the first diagnostic leaves the cut-off report alone: no `errors`.
    #[test]
    fn an_error_after_output_has_started_adds_no_document() {
        let dir = std::env::temp_dir().join(format!("jsonvet-cutoff-{}", std::process::id()));
        std::fs::create_dir(&dir).expect("an exclusive temporary directory");
        let file = dir.join("bad.json");
        std::fs::write(&file, "[1,").expect("write");
        let mut probe = Probe::default();
        let args = ["--format".into(), "json".into(), file.into_os_string()];
        let result = super::run_with(args, false, super::Terminal::default(), &mut probe);
        std::fs::remove_dir_all(&dir).expect("clean up");
        let written = String::from_utf8(probe.written).expect("UTF-8");
        assert!(result.is_err(), "{result:?}");
        assert!(written.contains("\"syntax\""), "{written}");
        assert!(!written.contains("errors"), "{written}");
    }

    /// The type is checked on the opened file, and opening a FIFO with no writer returns at
    /// once, so a path swapped for one between a check and the open cannot hang the run.
    #[cfg(unix)]
    #[test]
    fn opening_a_fifo_does_not_wait_for_a_writer() {
        let dir = std::env::temp_dir().join(format!("jsonvet-fifo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temporary directory");
        let fifo = dir.join("fifo.json");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("run mkfifo");
        assert!(made.success());
        let (send, receive) = std::sync::mpsc::channel();
        let path = fifo.clone();
        std::thread::spawn(move || {
            let opened = super::open_regular(&path).map(|o| o.is_none());
            let _ = send.send(opened.map_err(|e| e.to_string()));
        });
        let refused = receive.recv_timeout(std::time::Duration::from_secs(5));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            refused,
            Ok(Ok(true)),
            "the open waited, or did not refuse the FIFO"
        );
    }

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
    fn config_text_reports_a_bad_file() {
        let path = std::path::Path::new("/nonexistent/jsonvet.toml");
        assert_eq!(super::check_config_text(path, "rules = {}"), Ok(()));
        let message = super::check_config_text(path, "rules = [").expect_err("bad toml");
        assert!(
            message.starts_with("/nonexistent/jsonvet.toml:1:"),
            "{message}"
        );
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
        assert!(folder_rules(None, &dir, false).is_ok());
        sized(MAX_CONFIG_BYTES + 1);
        assert!(folder_rules(None, &dir, false).is_err_and(|e| e.ends_with("larger than 16 MB")));
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
                let (whole, whole_errors) = check("x.jsonl", text, &settings, schema, 1, None);
                let (split, split_errors) = check("x.jsonl", text, &settings, schema, 8, None);
                assert_ne!(whole.problems(), &[]);
                assert_eq!(split.problems(), whole.problems());
                assert_eq!(split_errors, whole_errors);
            }
        }
        let (_, errors) = check("x.jsonl", &texts[0], &settings, Some(&schema), 8, None);
        // 570 strings, 100 and a summary for each wide line, and one for each line not checked.
        assert_eq!(errors.len(), 570 + 4 * 101 + 8);
        let unchecked = errors
            .iter()
            .filter(|finding| finding.message.contains("not checked"));
        assert_eq!(unchecked.count(), 8);
    }

    #[test]
    fn parallel_keeps_the_order_and_stops_after_a_prefix() {
        let items: Vec<usize> = (0..1000).collect();
        for threads in [1, 3, 16] {
            let squares = parallel(&items, threads, || false, |i| i * i);
            assert_eq!(squares, items.iter().map(|i| i * i).collect::<Vec<_>>());
            // Stopped from the start, it still does the first item, and a thread that looks
            // before another has taken it may take one more.
            let stopped = parallel(&items, threads, || true, |&i| i);
            assert!((1..=threads).contains(&stopped.len()), "{stopped:?}");
            assert_eq!(stopped, (0..stopped.len()).collect::<Vec<_>>());
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
    fn take_chunk_respects_shares() {
        let dir = std::env::temp_dir().join(format!("jsonvet-chunk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let f1 = dir.join("1.json");
        let f2 = dir.join("2.json");
        std::fs::write(&f1, "{}").expect("write");
        std::fs::write(&f2, "{}").expect("write");
        let settings = Arc::new(Settings::default());
        let job1: Job = ((settings.clone(), None), f1.into_os_string());
        let job2: Job = ((settings.clone(), None), f2.into_os_string());
        let rest = [job1, job2];
        let mut ahead = Vec::new();
        let shares = [CHUNK_BYTES, CHUNK_BYTES];
        let taken = take_chunk(&mut ahead, &rest, Some(&shares), 1, &|_, _| false);
        assert_eq!(taken.len(), 1);
        assert_eq!(ahead, []);
        let _ = std::fs::remove_dir_all(&dir);
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
            (Format::JsonLines, Some(true), terminal(true, None), lines),
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
        let dup2 = dir.join("dup2.json");
        std::fs::write(&dup2, "{\"a\": 1, \"a\": 2}").expect("write");
        let (_, out) = run(true, &[&dup, &gone, &dup2, &ok]);
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

    /// Text output takes a translation; SARIF, JSON and JSON Lines stay English with one in place.
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
        for format in ["json", "jsonl", "sarif"] {
            let out = run(format);
            assert!(out.contains("duplicate key $.a (first at 1:2)"), "{out}");
            assert!(!out.contains("XX"), "{out}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn schema_json_reports_normalized_paths_when_they_name_a_value() {
        let dir = std::env::temp_dir().join(format!("jsonvet-paths-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let schema = dir.join("schema.json");
        let data = dir.join("input.jsonl");
        std::fs::write(
            &schema,
            r#"{"required":["missing"],"properties":{"0":{"items":{"additionalProperties":false}}}}"#,
        )
        .expect("write schema");
        let long = "x".repeat(256);
        std::fs::write(
            &data,
            format!(
                "{{\"0\":[{{\"a'\\\\\\\"\\u000B\\n\\u00e9\":1,\"{long}\":2}}]}}\n{{\"0\":[{{\"repeated\":3,\"repeated\":4}}]}}\n{{\"0\":[{{\"\\ud800\":5}}]}}\n{{\"0\":[{{\"\\ufffd\":6}}]}}\n"
            ),
        )
        .expect("write data");
        let args = [
            "--no-config".into(),
            "--format=json".into(),
            "--schema".into(),
            schema.into(),
            data.into(),
        ];
        let mut out = Vec::new();
        let result = super::run_with(args, false, super::Terminal::default(), &mut out);
        std::fs::remove_dir_all(&dir).expect("clean up");
        assert_eq!(result, Ok(1));
        let report: serde_json::Value = serde_json::from_slice(&out).expect("a JSON report");
        let diagnostics = report["diagnostics"].as_array().expect("diagnostics");
        let paths: Vec<_> = diagnostics
            .iter()
            .map(|finding| (finding["line"].as_u64(), finding["path"].as_str()))
            .collect();
        assert_eq!(
            paths,
            [
                (Some(2), None), // Duplicate-key diagnostic.
                (Some(1), Some("$")),
                (Some(1), Some(r#"$['0'][0]['a\'\\"\u000b\né']"#)),
                (Some(1), None), // Too long.
                (Some(2), Some("$")),
                (Some(2), None), // Duplicate member.
                (Some(3), Some("$")),
                (Some(3), None), // Lone surrogate.
                (Some(4), Some("$")),
                (Some(4), Some("$['0'][0]['�']")),
            ]
        );
        assert!(
            diagnostics[2]["message"]
                .as_str()
                .is_some_and(|message| message.contains(r#"$["0"][0]"#))
        );
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
                 --> l.jsonl:2:7\n  |\n2 | {\"a\": True}\n  |       ^^^^\n  |\n  = help: https://simonvanlierde.com/jsonvet/docs/usage.html#syntax\n\n\
                 1 error in 1 of 1 file\n"
                    .to_owned()
            )
        );
        assert_eq!(
            run(&["--schema".as_ref(), schema.as_os_str(), data.as_os_str()]),
            (
                1,
                "error[schema]: 1 is not of type \"string\" (at $.a)\n \
                 --> d.json:2:8\n  |\n2 |   \"a\": 1\n  |        ^\n  |\n  = help: https://simonvanlierde.com/jsonvet/docs/usage.html#schema\n\n\
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
