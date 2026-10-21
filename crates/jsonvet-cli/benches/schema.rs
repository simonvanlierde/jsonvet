//! Instruction counts for `jsonvet --schema`, the whole run of the binary: a real SchemaStore file,
//! then generated files for the paths `schema/mod.rs` treats apart. Needs valgrind and
//! `gungraun-runner` at the gungraun version (`cargo bench -p jsonvet-cli --bench schema`); ci.yml's
//! `bench` job runs it.

#![allow(
    missing_docs,
    clippy::expect_used,
    reason = "gungraun's macros generate undocumented public items, and a broken fixture should fail loudly"
)]

use std::fs;
use std::path::{Path, PathBuf};

use gungraun::{
    BinaryBenchmarkConfig, Command, ExitWith, Stdio, binary_benchmark, binary_benchmark_group, main,
};

/// How many members or elements the generated files hold.
const ITEMS: usize = 2_000;

/// A string with more than 5,000 backtracking steps to match against `^(a|a)*\1b`.
const COSTLY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!";

/// The repository's `tests/schemastore/`.
fn schemastore() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/schemastore")
}

/// A folder holding `schema` as `schema.json` and `text` as `file.json`.
fn fixture(name: &str, schema: &str, text: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("bench-schema-{name}"));
    fs::create_dir_all(&dir).expect("create the fixture folder");
    fs::write(dir.join("schema.json"), schema).expect("write the schema");
    fs::write(dir.join("file.json"), text).expect("write the file");
    dir
}

/// One object member or array element of the generated files.
fn row(i: usize) -> String {
    format!("{{\"x\": {i}, \"y\": [{i}, \"s\"], \"t\": \"text\"}}")
}

/// The fixture for each case, and the exit code jsonvet gives on it.
fn case(name: &str) -> (PathBuf, i32) {
    let rows = || (0..ITEMS).map(row);
    match name {
        // The largest valid file SchemaStore has for a popular schema, as a pre-commit hook checks it.
        "dependabot" => {
            let dir = schemastore().join("dependabot-2.0");
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

#[binary_benchmark]
#[benches::cases(
    "dependabot",
    "valid_object",
    "valid_array",
    "failing_array",
    "costly_keys"
)]
fn bench_schema(name: &str) -> Command {
    let (dir, code) = case(name);
    Command::new(env!("CARGO_BIN_EXE_jsonvet"))
        .args(["--no-config", "--schema", "schema.json", "file.json"])
        .current_dir(dir)
        .stdout(Stdio::Null)
        .exit_with(ExitWith::Code(code))
        .build()
}

binary_benchmark_group!(name = schema, benchmarks = bench_schema);

main!(
    config = BinaryBenchmarkConfig::default().env_clear(true);
    binary_benchmark_groups = schema
);
