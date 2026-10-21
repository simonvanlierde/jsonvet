//! Paths that were once quadratic stay linear. Each test counts the instructions for an input of
//! size n and of size 10n: linear work gives a ratio near 10, quadratic work near 100. Work
//! that stops growing past a cap gives a ratio near 1. Valgrind
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

/// Count the instructions of `run` at `n` and `10 * n`, and fail unless they stay within twice
/// each other: the work does not grow with the input past `n`.
fn assert_flat(n: usize, run: impl Fn(usize) -> u64) {
    let (small, large) = (run(n), run(10 * n));
    eprintln!("n = {n}: {small} instructions; 10n: {large}");
    assert!(
        large < 2 * small,
        "10 times the input took {large} instructions, against {small}"
    );
}

/// Run jsonvet with `--schema schema.json` on `text` as `file`.
fn with_schema(name: &str, schema: &str, text: &str) -> u64 {
    let dir = workdir(name, &[("schema.json", schema), ("file.json", text)]);
    instructions(
        Path::new(env!("CARGO_BIN_EXE_jsonvet")),
        &["--schema", "schema.json", "file.json"],
        &dir,
    )
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
#[ignore = "needs valgrind: just scaling"]
fn schema_errors_past_the_first_100() {
    // Once 100 errors are kept, jsonschema is not asked for the rest, which each cost 2^10 checks.
    let defs: Vec<String> = (0..10)
        .map(|i| {
            let next = format!("{{\"$ref\": \"#/$defs/l{}\"}}", i + 1);
            format!("\"l{i}\": {{\"anyOf\": [{next}, {next}]}}")
        })
        .collect();
    let schema = format!(
        "{{\"$defs\": {{{}, \"l10\": {{\"type\": \"string\"}}}}, \"items\": {{\"$ref\": \"#/$defs/l0\"}}}}",
        defs.join(", ")
    );
    assert_flat(200, |n| {
        with_schema("errors", &schema, &format!("[{}]", vec!["0"; n].join(",")))
    });
}

#[test]
#[ignore = "needs valgrind: just scaling"]
fn schema_refs_to_many_ids() {
    // Each `$ref` is looked up by its absolute URI, not compared with every `$id`.
    assert_linear(300, |n| {
        let defs: Vec<String> = (0..n)
            .map(|i| format!("\"d{i}\": {{\"$id\": \"https://x/u{i}\", \"type\": \"object\"}}"))
            .collect();
        let refs: Vec<String> = (0..n)
            .map(|i| format!("{{\"$ref\": \"https://x/u{}\"}}", n - 1 - i))
            .collect();
        let schema = format!(
            "{{\"$defs\": {{{}}}, \"allOf\": [{}]}}",
            defs.join(", "),
            refs.join(", ")
        );
        with_schema("ids", &schema, "{}")
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

/// Walk a folder of `files` files under a config with `patterns` exclude and `patterns` dialect
/// patterns, none of which match.
fn walk_with_patterns(patterns: usize, files: usize) -> u64 {
    let globs: Vec<String> = (0..patterns)
        .map(|i| format!("\"nomatch{i}/*.json\""))
        .collect();
    let dialects: Vec<String> = (0..patterns)
        .map(|i| format!("{{ files = \"nomatch{i}/*.json\", dialect = \"json\" }}"))
        .collect();
    let config = format!(
        "exclude = [{}]\ndialects = [{}]\n",
        globs.join(", "),
        dialects.join(", ")
    );
    let names: Vec<String> = (0..files).map(|i| format!("f{i}.json")).collect();
    let mut tree: Vec<(&str, &str)> = names.iter().map(|name| (name.as_str(), "{}")).collect();
    tree.push(("jsonvet.toml", &config));
    let dir = workdir("patterns", &tree);
    instructions(Path::new(env!("CARGO_BIN_EXE_jsonvet")), &["."], &dir)
}

#[test]
#[ignore = "needs valgrind: just scaling"]
fn exclude_and_dialect_patterns_on_many_files() {
    // Each pattern is split into characters and each base normalized once, when the config is
    // read, and each file is made absolute and relative once: not once for each pattern.
    assert_linear(500, |n| walk_with_patterns(100, n));
    let (files, patterns) = (1_000, 200);
    let none = walk_with_patterns(0, files);
    let each = (walk_with_patterns(patterns, files) - none) / (files * patterns) as u64;
    eprintln!("{each} instructions for each pattern and file");
    // A file is matched against each pattern twice, once to exclude it and once for its dialect.
    assert!(
        each < 1_000,
        "a pattern costs {each} instructions on each file"
    );
}
