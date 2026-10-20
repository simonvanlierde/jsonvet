//! Paths that were once quadratic stay linear. Each test counts the instructions for an input of
//! size n and of size 10n: linear work gives a ratio near 10, quadratic work near 100. Valgrind
//! counts them, because wall-clock time swings too much on a shared machine, so the tests are
//! ignored by default. `just scaling` runs them, as ci.yml's `scaling` job does.

#![allow(
    clippy::expect_used,
    reason = "a broken test setup should fail the test loudly"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The largest allowed ratio between the counts at 10n and at n.
const MAX_RATIO: u64 = 15;

/// A fresh directory holding `files`, inside the target directory so the config search stops
/// at its `.git`.
fn workdir(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("scaling-{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join(".git")).expect("create the directory");
    for (file, text) in files {
        fs::write(dir.join(file), text).expect("write file");
    }
    dir
}

/// The instructions one run of `program` takes, as valgrind counts them.
fn instructions(program: &Path, args: &[&str], dir: &Path) -> u64 {
    let out = Command::new("valgrind")
        .args([
            "--tool=cachegrind",
            "--cache-sim=no",
            "--cachegrind-out-file=/dev/null",
        ])
        .arg(program)
        .args(args)
        .current_dir(dir)
        .env_remove("GITHUB_ACTIONS")
        .output()
        .expect("run valgrind");
    let stderr = String::from_utf8_lossy(&out.stderr);
    // The summary line reads `==pid== I refs:  1,234,567`.
    let count = stderr
        .lines()
        .find_map(|line| line.split_once("refs:"))
        .map(|(_, count)| {
            count
                .chars()
                .filter(char::is_ascii_digit)
                .collect::<String>()
        });
    count
        .and_then(|count| count.parse().ok())
        .unwrap_or_else(|| panic!("no instruction count from valgrind:\n{stderr}"))
}

/// Count the instructions of `run` at `n` and `10 * n`, and fail when they grow faster than
/// linearly.
fn assert_linear(n: usize, run: impl Fn(usize) -> u64) {
    let (small, large) = (run(n), run(10 * n));
    let tenths = large * 10 / small;
    let ratio = format!("{}.{}", tenths / 10, tenths % 10);
    eprintln!("n = {n}: {small} instructions; 10n: {large}; ratio {ratio}");
    assert!(
        large < MAX_RATIO * small,
        "10 times the input took {ratio} times the instructions ({small} to {large})"
    );
}

/// Run jsonvet on `text` as `file`, with `args` before it.
fn jsonvet(file: &str, text: &str, args: &[&str]) -> u64 {
    let dir = workdir(
        file,
        &[(file, text), ("schema.json", r#"{"type": "object"}"#)],
    );
    let mut args = args.to_vec();
    args.push(file);
    instructions(Path::new(env!("CARGO_BIN_EXE_jsonvet")), &args, &dir)
}

#[test]
#[ignore = "needs valgrind: just scaling"]
fn duplicate_keys_on_many_lines() {
    // Each duplicate's line comes from the line starts, found once for the whole file.
    assert_linear(5_000, |n| {
        let keys: Vec<String> = (0..n).map(|i| format!("\"k\": {i}")).collect();
        jsonvet("lines.json", &format!("{{\n{}\n}}", keys.join(",\n")), &[])
    });
}

#[test]
#[ignore = "needs valgrind: just scaling"]
fn duplicate_keys_on_one_long_line() {
    // The `é` makes columns a count of characters, which goes on from the last duplicate found.
    assert_linear(5_000, |n| {
        let keys: Vec<String> = (0..n).map(|i| format!("\"k\": {i}")).collect();
        jsonvet(
            "line.json",
            &format!("{{\"é\": 0, {}}}", keys.join(", ")),
            &[],
        )
    });
}

#[test]
#[ignore = "needs valgrind: just scaling"]
fn schema_on_keys_with_lone_surrogates() {
    // Each key holding U+FFFD is looked up among the earlier ones, not compared with each.
    assert_linear(1_000, |n| {
        let keys: Vec<String> = (0..n).map(|i| format!("\"\\ud800{i}\": {i}")).collect();
        jsonvet(
            "keys.json",
            &format!("{{{}}}", keys.join(",")),
            &["--schema", "schema.json"],
        )
    });
}

#[test]
#[ignore = "needs valgrind and the diff_json5 fuzz target: just scaling"]
fn json5_hex_numbers_in_the_diff_json5_target() {
    // json5 before 1.3.2 found the line and column of every hex digit after the first, an error
    // it then threw away, by counting from the start of the text.
    let target = std::env::var_os("JSONVET_DIFF_JSON5")
        .expect("JSONVET_DIFF_JSON5 names the diff_json5 binary, built without cargo fuzz");
    assert_linear(5_000, |n| {
        let dir = workdir(
            "hex",
            &[("hex.json5", &format!("[{}]", vec!["0x11"; n].join(",")))],
        );
        instructions(Path::new(&target), &["hex.json5"], &dir)
    });
}
