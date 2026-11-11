//! How many heap blocks one in-process run allocates on a large generated file, counted by dhat's
//! global allocator in its own test binary. A run that allocates per row or per container shows
//! up as a count that grows with the file; each ceiling here holds at any file size.

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

// The workspace forbids unsafe code, and a global allocator needs it, so a crate counts.
#[global_allocator]
static ALLOCATOR: dhat::Alloc = dhat::Alloc;

/// The number of rows in each generated file.
const ROWS: usize = 100_000;

/// Held by each test while dhat runs: only one profiler may run at a time, and another test's
/// allocations would add to the count.
static COUNTING: Mutex<()> = Mutex::new(());

fn counting() -> MutexGuard<'static, ()> {
    COUNTING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A fresh folder for the test `name`, marked as a repository so the project ends there.
fn folder(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("allocations-{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join(".git")).expect("create the directory");
    dir
}

/// `count` small records of numbers, strings and arrays, each written by `row` from its index,
/// after `open` and joined by `between`, then `close`.
fn rows(
    count: usize,
    open: &str,
    between: &str,
    close: &str,
    row: impl Fn(&mut String, usize),
) -> String {
    let mut text = String::from(open);
    for i in 0..count {
        if i > 0 {
            text.push_str(between);
        }
        row(&mut text, i);
    }
    text.push_str(close);
    text
}

/// What one run with `args` allocates, with its exit code and output. The run has a 64 MB stack,
/// as the binary does. The caller holds [`counting`].
fn heap(args: Vec<OsString>) -> (dhat::HeapStats, Result<u8, String>, String) {
    let profiler = dhat::Profiler::builder().testing().build();
    let (code, out) = std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || {
            let mut out = Vec::new();
            let code = jsonvet_cli::run_with(args, false, Terminal::default(), &mut out);
            (code, out)
        })
        .expect("a thread")
        .join()
        .expect("no panic");
    let stats = dhat::HeapStats::get();
    drop(profiler);
    eprintln!(
        "allocated {} blocks, {} bytes, peak {}",
        stats.total_blocks, stats.total_bytes, stats.max_bytes
    );
    (stats, code, String::from_utf8_lossy(&out).into_owned())
}

fn assert_under(blocks: u64, max: u64) {
    assert!(blocks <= max, "allocated {blocks} blocks, over {max}");
}

/// `fmt` keeps the members of every open container on one stack, and the tree in a few vectors,
/// so the count does not grow with the containers. About 100 blocks here; a vector of members per
/// container took 700,095. Each comment still takes a few blocks, so the rows have none.
#[test]
fn fmt_allocates_a_fixed_number_of_blocks() {
    let _counting = counting();
    let dir = folder("fmt");
    // About 8 MB, 7 containers a row.
    let text = rows(ROWS, "[\n", ",\n", "\n]\n", |text, i| {
        write!(
            text,
            "{{\"x\":{i},\"y\":[{i},\"s\",{{\"a\":[1,2,3]}}],\"t\":\"text\",\"u\":[[{i}],[{i}]]}}"
        )
        .expect("write to a string");
    });
    let file = dir.join("rows.jsonc");
    fs::write(&file, text).expect("write the file");
    let (stats, code, out) = heap(vec![
        OsString::from("fmt"),
        OsString::from("--no-config"),
        OsString::from("--check"),
        file.into_os_string(),
    ]);
    assert_eq!(code, Ok(1), "{out}");
    assert_under(stats.total_blocks, 1_000);
    // The member stack is freed once the tree is parsed, before the output is built. Kept, the
    // peak is 68,909,531 bytes here; freed, 66,812,379. Both are exact on every run.
    assert!(
        stats.max_bytes <= 67_900_000,
        "peaked at {} bytes, over 67,900,000: is the member stack freed after parsing?",
        stats.max_bytes
    );
}

/// Each line of JSON Lines builds its tree in the vectors of the line before. About 8,600 blocks
/// here on 1 thread and 8,900 on 12; a tree in new vectors for each line took 7 blocks a line,
/// 708,746 in all.
#[test]
fn schema_on_json_lines_allocates_a_fixed_number_of_blocks() {
    let _counting = counting();
    let dir = folder("lines");
    let text = rows(ROWS, "", "\n", "\n", |text, i| {
        write!(text, "{{\"x\": {i}, \"y\": [{i}, \"s\"], \"t\": \"text\"}}")
            .expect("write to a string");
    });
    let file = dir.join("file.jsonl");
    fs::write(&file, text).expect("write the file");
    let schema = dir.join("schema.json");
    fs::write(
        &schema,
        r#"{"type": "object", "required": ["x"], "properties": {"y": {"type": "array"}}}"#,
    )
    .expect("write the schema");
    let (stats, code, out) = heap(vec![
        OsString::from("--no-config"),
        OsString::from("--schema"),
        schema.into_os_string(),
        file.into_os_string(),
    ]);
    assert_eq!(code, Ok(0), "{out}");
    assert_under(stats.total_blocks, 20_000);
}

/// What `--fix` allocates on `count` rows that each end in a trailing comma.
fn fix_stats(name: &str, count: usize) -> dhat::HeapStats {
    let dir = folder(name);
    let text = rows(count, "[\n", ",\n", "\n]\n", |text, i| {
        write!(
            text,
            "{{\"x\": {i}, \"y\": [{i}, \"s\"], \"t\": \"text\",}}"
        )
        .expect("write to a string");
    });
    let file = dir.join("rows.json");
    fs::write(&file, text).expect("write the file");
    let (stats, code, out) = heap(vec![
        OsString::from("--no-config"),
        OsString::from("--dialect"),
        OsString::from("json"),
        OsString::from("--fix"),
        file.clone().into_os_string(),
    ]);
    assert_eq!(code, Ok(0), "{out}");
    assert!(!fs::read_to_string(&file).expect("read").contains(",}"));
    stats
}

/// `--fix` on a trailing comma in each row. Each fix holds its edits, so a comma takes 2 blocks
/// (200,156 in all at [`ROWS`]). The count is the slope between two sizes, so a fixed cost in
/// the run does not move it. The bytes catch a scan that keeps each member when only
/// `--unsafe-fixes` reads them: 679 bytes a comma, against 930 with them.
#[test]
fn fix_allocates_two_blocks_a_comma() {
    let _counting = counting();
    let full = fix_stats("fix", ROWS);
    let half = fix_stats("fix-half", ROWS / 2);
    let extra = (ROWS - ROWS / 2) as u64;
    let blocks = full.total_blocks.saturating_sub(half.total_blocks);
    assert!(
        blocks <= 2 * extra + 100,
        "allocated {blocks} blocks for {extra} more commas ({} at {ROWS} rows, {} at {}), over 2 per comma",
        full.total_blocks,
        half.total_blocks,
        ROWS / 2,
    );
    assert!(
        full.total_bytes <= 800 * ROWS as u64,
        "allocated {} bytes, over 800 per comma",
        full.total_bytes
    );
}
