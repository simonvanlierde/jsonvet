//! The heap `jsonvet fmt` takes at its peak on a generated 8 MB file, counted by a global
//! allocator in its own test binary, and that it refuses a file whose output would be huge
//! without building that output.

#![allow(
    clippy::expect_used,
    reason = "a broken test setup should fail the test loudly"
)]

use std::ffi::OsString;
use std::fmt::Write;
use std::fs;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use jsonvet_cli::Terminal;
use peak_alloc::PeakAlloc;

// The workspace forbids unsafe code, and a global allocator needs it, so a crate counts.
#[global_allocator]
static ALLOCATOR: PeakAlloc = PeakAlloc;

/// The size of the generated file.
const FILE_BYTES: usize = 8 << 20;

/// The most heap one run may take at its peak, per byte of the file, beyond what was in use
/// before it. The text and the formatted output take 1 byte each; the rest is the tree. A tree of
/// owned strings and boxed nodes took 30 bytes per byte in all.
const MAX_BYTES_PER_BYTE: usize = 8;

static COUNTING: Mutex<()> = Mutex::new(());

fn counting() -> MutexGuard<'static, ()> {
    COUNTING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Rows as the CodSpeed cases write them, small records of numbers, strings and arrays, with a
/// comment before every tenth, to at least `bytes` bytes.
fn rows(bytes: usize) -> String {
    let mut text = String::from("[\n");
    let mut i = 0;
    while text.len() < bytes {
        if i > 0 {
            text.push_str(",\n");
        }
        if i % 10 == 9 {
            text.push_str("// row\n");
        }
        write!(text, "{{\"x\":{i},\"y\":[{i},\"s\"],\"t\":\"text\"}}").expect("write to a string");
        i += 1;
    }
    text.push_str("\n]\n");
    text
}

/// The peak heap of one `jsonvet fmt` run on `file`, beyond what was in use before it, with its
/// exit code and output. The caller holds [`counting`].
fn run(dir: &Path, file: &Path, size: usize) -> (usize, Result<u8, String>, String) {
    let args = [
        OsString::from("fmt"),
        OsString::from("--no-config"),
        OsString::from("--check"),
        file.as_os_str().to_owned(),
    ];
    let _ = dir;
    let before = ALLOCATOR.current_usage();
    ALLOCATOR.reset_peak_usage();
    // The binary runs on a 64 MB stack; so does this.
    let (code, out) = std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || {
            let mut out = Vec::new();
            let code = jsonvet_cli::run_with(args.to_vec(), false, Terminal::default(), &mut out);
            (code, out)
        })
        .expect("a thread")
        .join()
        .expect("no panic");
    let peak = ALLOCATOR.peak_usage() - before;
    eprintln!(
        "peak heap: {peak} bytes, {} per byte of the file",
        peak / size
    );
    (peak, code, String::from_utf8_lossy(&out).into_owned())
}

fn folder(name: &str) -> std::path::PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("format-memory-{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join(".git")).expect("create the directory");
    dir
}

#[test]
fn format_memory_stays_under_its_bound_on_an_8_mb_file() {
    let _counting = counting();
    let dir = folder("rows");
    let text = rows(FILE_BYTES);
    let size = text.len();
    let file = dir.join("rows.jsonc");
    fs::write(&file, &text).expect("write the file");
    drop(text);
    let (peak, code, out) = run(&dir, &file, size);
    assert_eq!(code, Ok(1), "{out}");
    assert!(
        peak <= MAX_BYTES_PER_BYTE * size,
        "peak heap {peak} bytes is over {MAX_BYTES_PER_BYTE} per byte of the {size}-byte file"
    );
}

/// 9,000 levels deep is valid, and formatted with indentation it takes 160 MB from 18 KB.
#[test]
fn a_file_whose_output_would_be_huge_is_refused_without_building_it() {
    let _counting = counting();
    let dir = folder("deep");
    let depth = 9_000;
    let text = format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
    let file = dir.join("deep.json");
    fs::write(&file, &text).expect("write the file");
    let (peak, code, _) = run(&dir, &file, text.len());
    assert_eq!(code, Ok(1));
    assert!(peak < (64 << 20) / 2, "built {peak} bytes before refusing");
    assert_eq!(fs::read_to_string(&file).expect("read"), text);
}
