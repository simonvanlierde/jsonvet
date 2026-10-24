//! `--schema` runs in-process with divan, which `.github/workflows/codspeed.yml` measures under
//! CodSpeed on every pull request, for CPU and for memory: the cases `benches/schema.rs` counts
//! under callgrind, minus the process start. jsonvet's known memory limits are in `--schema`
//! (SECURITY.md), so this is where the memory instrument earns its place. Run them with `cargo
//! bench -p jsonvet-cli --bench codspeed`.

#![allow(
    missing_docs,
    clippy::expect_used,
    reason = "divan's macros generate undocumented items, and a broken fixture should fail loudly"
)]

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use divan::{Bencher, black_box};
use jsonvet_cli::Terminal;

fn main() {
    divan::main();
}

/// How many members or elements the generated files hold.
const ITEMS: usize = 2_000;

/// A string with more than 5,000 backtracking steps to match against `^(a|a)*\1b`.
const COSTLY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!";

/// A folder holding `schema` as `schema.json` and `text` as `file.json`.
fn fixture(name: &str, schema: &str, text: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("codspeed-schema-{name}"));
    fs::create_dir_all(&dir).expect("create the fixture folder");
    fs::write(dir.join("schema.json"), schema).expect("write the schema");
    fs::write(dir.join("file.json"), text).expect("write the file");
    dir
}

/// One object member or array element of the generated files.
fn row(i: usize) -> String {
    format!("{{\"x\": {i}, \"y\": [{i}, \"s\"], \"t\": \"text\"}}")
}

/// The fixture folder for each case, and the exit code jsonvet gives on it.
fn case(name: &str) -> (PathBuf, u8) {
    let rows = || (0..ITEMS).map(row);
    match name {
        // The largest valid file SchemaStore has for a popular schema.
        "dependabot" => {
            let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/schemastore/dependabot-2.0");
            let file =
                fs::read_to_string(dir.join("valid/example.json")).expect("the dependabot fixture");
            let schema = fs::read_to_string(dir.join("schema.json")).expect("its schema");
            (fixture(name, &schema, &file), 0)
        }
        // Checked a member at a time.
        "valid_object" => {
            let members: Vec<String> = rows()
                .enumerate()
                .map(|(i, row)| format!("\"k{i}\": {row}"))
                .collect();
            let schema = r#"{"additionalProperties": {"type": "object", "properties": {"x": {"type": "integer"}}}}"#;
            (
                fixture(name, schema, &format!("{{{}}}", members.join(", "))),
                0,
            )
        }
        // Checked an element at a time.
        "valid_array" => {
            let items: Vec<String> = rows().collect();
            let schema =
                r#"{"items": {"type": "object", "properties": {"x": {"type": "integer"}}}}"#;
            (fixture(name, schema, &format!("[{}]", items.join(", "))), 0)
        }
        // Every element fails, so the run stops at the 100-error cap.
        "failing_array" => {
            let items: Vec<String> = rows().collect();
            let schema =
                r#"{"items": {"type": "object", "properties": {"x": {"type": "string"}}}}"#;
            (fixture(name, schema, &format!("[{}]", items.join(", "))), 1)
        }
        // Every key is too costly for the pattern, so each is checked under both outcomes.
        "costly_keys" => {
            let keys: Vec<String> = (0..ITEMS)
                .map(|i| format!("\"{COSTLY}{i}\": {i}"))
                .collect();
            let schema = r#"{"patternProperties": {"^(a|a)*\\1b": false}}"#;
            (
                fixture(name, schema, &format!("{{{}}}", keys.join(", "))),
                1,
            )
        }
        _ => unreachable!("no case {name}"),
    }
}

#[divan::bench(args = ["dependabot", "valid_object", "valid_array", "failing_array", "costly_keys"])]
fn schema(bencher: Bencher, name: &str) {
    let (dir, code) = case(name);
    let path = |file: &str| OsString::from(dir.join(file));
    let args = vec![
        OsString::from("--no-config"),
        OsString::from("--schema"),
        path("schema.json"),
        path("file.json"),
    ];
    // The exit code checks the case, once, outside the measured runs.
    assert_eq!(
        jsonvet_cli::run_with(args.clone(), false, Terminal::default(), std::io::sink()),
        Ok(code),
        "{name}"
    );
    bencher.bench(|| {
        jsonvet_cli::run_with(
            black_box(args.clone()),
            false,
            Terminal::default(),
            std::io::sink(),
        )
    });
}
