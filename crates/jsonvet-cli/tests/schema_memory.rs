//! The heap `--schema` takes at its peak on a generated 10 MB file, counted by a global allocator
//! in its own test binary. Each test holds `COUNTING` from before it builds its files, so no
//! other test allocates while one counts, under `cargo test` threads as well as nextest processes.

#![allow(
    clippy::expect_used,
    reason = "a broken test setup should fail the test loudly"
)]

use std::ffi::OsString;
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use jsonvet_cli::Terminal;
use peak_alloc::PeakAlloc;

// The workspace forbids unsafe code, and a global allocator needs it, so a crate counts.
#[global_allocator]
static ALLOCATOR: PeakAlloc = PeakAlloc;

/// The size of the generated file.
const FILE_BYTES: usize = 10 << 20;

/// The most heap one run may take at its peak, per byte of the file, beyond what was in use
/// before it. Folding the parser's events as they come takes 18 bytes per byte here: the file's
/// text and its values. Holding every event would take 34, so a return to that fails.
const MAX_BYTES_PER_BYTE: usize = 24;

/// The same for a JSON Lines file, split across threads, each line folded as it is read. That
/// takes 1 byte per byte, the file's text. Parts that kept their events would take 17.
const MAX_JSON_LINES_BYTES_PER_BYTE: usize = 4;

/// Held by each test from its start, so tests on other threads do not add to its peak.
static COUNTING: Mutex<()> = Mutex::new(());

/// Take `COUNTING`. A test that panicked holding it freed what it held, so the count is still
/// sound.
fn counting() -> MutexGuard<'static, ()> {
    COUNTING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A fresh folder for the test `name`, marked as a repository so the project ends there.
fn folder(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("schema-memory-{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join(".git")).expect("create the directory");
    dir
}

/// Rows as the CodSpeed cases write them, small records of numbers, strings and arrays, after
/// `open` and joined by `between`, to at least `bytes` bytes, then `close`.
fn rows(bytes: usize, open: &str, between: &str, close: &str) -> String {
    let mut text = String::from(open);
    let mut i = 0;
    while text.len() < bytes {
        if i > 0 {
            text.push_str(between);
        }
        write!(text, "{{\"x\": {i}, \"y\": [{i}, \"s\"], \"t\": \"text\"}}")
            .expect("write to a string");
        i += 1;
    }
    text.push_str(close);
    text
}

/// The peak heap of one `--schema` run against `schema` on a generated 10 MB array, or JSON Lines
/// file with `lines`, beyond what was in use before it, with its exit code and output.
fn peak(name: &str, schema: &str, lines: bool) -> (usize, Result<u8, String>, String) {
    let _counting = counting();
    let dir = folder(name);
    let text = if lines {
        rows(FILE_BYTES, "", "\n", "\n")
    } else {
        rows(FILE_BYTES, "[", ", ", "]")
    };
    let file = dir.join(if lines { "file.jsonl" } else { "file.json" });
    fs::write(&file, &text).expect("write the file");
    drop(text);
    fs::write(dir.join("schema.json"), schema).expect("write the schema");
    run(&[
        OsString::from("--no-config"),
        OsString::from("--schema"),
        OsString::from(dir.join("schema.json")),
        OsString::from(file),
    ])
}

/// The peak heap of one run with `args`, beyond what was in use before it, with its exit code and
/// output. The caller holds [`counting`].
fn run(args: &[OsString]) -> (usize, Result<u8, String>, String) {
    let before = ALLOCATOR.current_usage();
    ALLOCATOR.reset_peak_usage();
    let mut out = Vec::new();
    let code = jsonvet_cli::run_with(args.to_vec(), false, Terminal::default(), &mut out);
    let peak = ALLOCATOR.peak_usage() - before;
    eprintln!(
        "peak heap: {peak} bytes, {} per byte of the file",
        peak / FILE_BYTES
    );
    (peak, code, String::from_utf8_lossy(&out).into_owned())
}

fn assert_under(peak: usize, per_byte: usize) {
    assert!(
        peak <= per_byte * FILE_BYTES,
        "peak heap {peak} bytes is over {per_byte} per byte of the {FILE_BYTES}-byte file"
    );
}

// Neither schema is one jsonvet splits, so the whole document is checked in one pass.

#[test]
fn schema_memory_stays_under_its_bound_on_a_valid_10_mb_file() {
    let schema =
        r#"{"type": "array", "minItems": 1, "items": {"type": "object", "required": ["x"]}}"#;
    let (peak, code, out) = peak("valid", schema, false);
    assert_eq!(code, Ok(0), "{out}");
    assert_under(peak, MAX_BYTES_PER_BYTE);
}

/// An error sends jsonvet back over the file to place it.
#[test]
fn schema_memory_stays_under_its_bound_placing_an_error_in_a_10_mb_file() {
    let schema = r#"{"minItems": 1, "items": {"properties": {"x": {"not": {"const": 7}}}}}"#;
    let (peak, code, out) = peak("error", schema, false);
    assert_eq!(code, Ok(1), "{out}");
    assert!(out.contains("(at $[7].x)"), "{out}");
    assert_under(peak, MAX_BYTES_PER_BYTE);
}

/// A JSON Lines file this size is split across threads, one part per thread, and an error on
/// line 8 sends its part back over its lines to place it.
#[test]
fn schema_memory_stays_under_its_bound_on_a_split_10_mb_json_lines_file() {
    let schema = r#"{"type": "object", "properties": {"x": {"not": {"const": 7}}}}"#;
    let (peak, code, out) = peak("lines", schema, true);
    assert_eq!(code, Ok(1), "{out}");
    assert!(out.contains(":8:7: error[schema]"), "{out}");
    assert_under(peak, MAX_JSON_LINES_BYTES_PER_BYTE);
}

/// A schema for files `{"$schema": ..., "rows": [...]}` that each row passes.
const ROWS: &str = r#"{"properties": {"rows": {"items": {"type": "object", "required": ["x"]}}}}"#;

/// `count` files of `bytes` each whose `$schema` names `own`, in a fresh folder with [`ROWS`] as
/// `s.json`: the arguments that check them against their own `$schema`, and against `--schema`.
fn own_schema_files(
    name: &str,
    own: &str,
    count: usize,
    bytes: usize,
) -> (Vec<OsString>, Vec<OsString>) {
    let dir = folder(name);
    fs::write(dir.join("s.json"), ROWS).expect("write the schema");
    let text = rows(
        bytes,
        &format!("{{\"$schema\": \"{own}\", \"rows\": ["),
        ", ",
        "]}",
    );
    let files: Vec<OsString> = (0..count)
        .map(|i| {
            let file = dir.join(format!("f{i}.json"));
            fs::write(&file, &text).expect("write a file");
            file.into_os_string()
        })
        .collect();
    let own = ["--no-config".into()]
        .into_iter()
        .chain(files.clone())
        .collect();
    let given = [
        "--no-config".into(),
        "--schema".into(),
        dir.join("s.json").into_os_string(),
    ]
    .into_iter()
    .chain(files)
    .collect();
    (own, given)
}

/// A file's own `$schema` is found as the file is read, with no event kept, so the file is then
/// checked as `--schema` checks it, or, for a schema that is not there, as a plain check does.
#[test]
fn a_local_dollar_schema_takes_what_schema_or_a_plain_check_does() {
    let _counting = counting();
    let (own, given) = own_schema_files("own", "s.json", 1, FILE_BYTES);
    let (with_schema, code, out) = run(&given);
    assert_eq!(code, Ok(0), "{out}");
    let (peak, code, out) = run(&own);
    assert_eq!(code, Ok(0), "{out}");
    assert!(
        peak <= with_schema + with_schema / 4,
        "a local `$schema` took {peak} bytes, `--schema` {with_schema}"
    );
    let (missing, _) = own_schema_files("own-missing", "node_modules/s.json", 1, FILE_BYTES);
    let (peak, code, out) = run(&missing);
    assert_eq!(code, Ok(0), "{out}");
    assert_under(peak, MAX_JSON_LINES_BYTES_PER_BYTE);
}

/// Files with a local `$schema` count toward a chunk as files with `--schema` do, so no more of
/// them are checked at once. 12 files of 2 MB fit one chunk by their size alone.
#[test]
fn files_with_a_local_dollar_schema_are_checked_as_few_at_a_time_as_with_schema() {
    let _counting = counting();
    let (count, bytes) = (12, 2 << 20);
    let (own, given) = own_schema_files("own-many", "s.json", count, bytes);
    let (with_schema, code, out) = run(&given);
    assert_eq!(code, Ok(0), "{out}");
    let (peak, code, out) = run(&own);
    assert_eq!(code, Ok(0), "{out}");
    // A `$schema` is found only once its file is read, so the texts the other threads read in
    // the meantime are held for the next chunk: at most every file's.
    assert!(
        peak <= with_schema + count * bytes,
        "local `$schema`s took {peak} bytes, `--schema` {with_schema}"
    );
}

/// A schema with a regex only fancy-regex compiles keeps its regexes compiled once, as one the
/// `regex` crate compiles does: the copies that name a pattern in an error are compiled only
/// when such an error comes. Compiled when the schema was, they took half as much again.
#[test]
fn a_regex_with_lookaround_compiles_the_schema_once() {
    let _counting = counting();
    let schema = |first: &str| {
        let regexes: Vec<String> = (0..100)
            .map(|i| format!("\"{first}(?:[a-z]{{20}}){{100}}N{i}\": {{}}"))
            .collect();
        format!("{{\"patternProperties\": {{{}}}}}", regexes.join(", "))
    };
    let dir = folder("lookaround");
    fs::write(dir.join("file.json"), "{}").expect("write the file");
    let mut peaks = Vec::new();
    for (name, first) in [("linear.json", ""), ("fancy.json", "(?=a)")] {
        fs::write(dir.join(name), schema(first)).expect("write the schema");
        let (peak, code, out) = run(&[
            OsString::from("--no-config"),
            OsString::from("--schema"),
            OsString::from(dir.join(name)),
            OsString::from(dir.join("file.json")),
        ]);
        assert_eq!(code, Ok(0), "{out}");
        peaks.push(peak);
    }
    let (linear, fancy) = (peaks[0], peaks[1]);
    assert!(
        fancy <= linear + linear / 5,
        "with lookaround {fancy} bytes, without {linear}"
    );
}
