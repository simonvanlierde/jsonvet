//! The heap `--schema` takes at its peak on a generated 10 MB file, counted by a global allocator
//! in its own test binary. nextest runs each test in a process of its own, and a lock keeps
//! `cargo test`'s threads from counting at once, so no other test allocates while one counts.

#![allow(
    clippy::expect_used,
    reason = "a broken test setup should fail the test loudly"
)]

use std::ffi::OsString;
use std::fmt::Write;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use jsonvet_cli::Terminal;
use peak_alloc::PeakAlloc;

// The workspace forbids unsafe code, and a global allocator needs it, so a crate counts.
#[global_allocator]
static ALLOCATOR: PeakAlloc = PeakAlloc;

/// The size of the generated file.
const FILE_BYTES: usize = 10 << 20;

/// The most heap one run may take at its peak, per byte of the file, beyond what was in use
/// before it. Holding every parser event took 34 bytes per byte here; folding them as they come
/// takes 18, the file's text and its values.
const MAX_BYTES_PER_BYTE: usize = 24;

/// The same for a JSON Lines file, split across threads, each line folded as it is read. Each part
/// keeping its events took 17 bytes per byte; folding them as they come takes 1, the file's text.
const MAX_JSON_LINES_BYTES_PER_BYTE: usize = 4;

/// Held while a run counts, so runs on other threads do not add to its peak.
static COUNTING: Mutex<()> = Mutex::new(());

/// The peak heap of one `--schema` run against `schema` on a generated 10 MB array, or JSON Lines
/// file with `lines`, beyond what was in use before it, with its exit code and output.
fn peak(name: &str, schema: &str, lines: bool) -> (usize, Result<u8, String>, String) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("schema-memory-{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join(".git")).expect("create the directory");
    // Rows as the CodSpeed cases write them: small records of numbers, strings and arrays.
    let (open, between, close) = if lines {
        ("", "\n", "\n")
    } else {
        ("[", ", ", "]")
    };
    let mut text = String::from(open);
    let mut i = 0;
    while text.len() < FILE_BYTES {
        if i > 0 {
            text.push_str(between);
        }
        write!(text, "{{\"x\": {i}, \"y\": [{i}, \"s\"], \"t\": \"text\"}}")
            .expect("write to a string");
        i += 1;
    }
    text.push_str(close);
    let file = dir.join(if lines { "file.jsonl" } else { "file.json" });
    fs::write(&file, &text).expect("write the file");
    drop(text);
    fs::write(dir.join("schema.json"), schema).expect("write the schema");

    let args = vec![
        OsString::from("--no-config"),
        OsString::from("--schema"),
        OsString::from(dir.join("schema.json")),
        OsString::from(file),
    ];
    let _counting = COUNTING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let before = ALLOCATOR.current_usage();
    ALLOCATOR.reset_peak_usage();
    let mut out = Vec::new();
    let code = jsonvet_cli::run_with(args, false, Terminal::default(), &mut out);
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
