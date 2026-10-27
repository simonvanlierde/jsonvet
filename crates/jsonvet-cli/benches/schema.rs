//! Instruction counts for `jsonvet --schema`, the whole run of the binary: a real SchemaStore file,
//! then generated files for the paths `schema/mod.rs` treats apart. Needs valgrind and
//! `gungraun-runner` at the gungraun version (`cargo bench -p jsonvet-cli --bench schema`); ci.yml's
//! `bench` job runs it.

#![allow(
    missing_docs,
    clippy::expect_used,
    reason = "gungraun's macros generate undocumented public items, and a broken fixture should fail loudly"
)]

use gungraun::{
    BinaryBenchmarkConfig, Command, ExitWith, Stdio, binary_benchmark, binary_benchmark_group, main,
};

mod common;

#[binary_benchmark]
#[benches::cases(
    "dependabot",
    "valid_object",
    "valid_array",
    "failing_array",
    "costly_keys"
)]
fn bench_schema(name: &str) -> Command {
    let (dir, code) = common::case("bench-schema", name);
    Command::new(env!("CARGO_BIN_EXE_jsonvet"))
        .args(["--no-config", "--schema", "schema.json", "file.json"])
        .current_dir(dir)
        .stdout(Stdio::Null)
        .exit_with(ExitWith::Code(i32::from(code)))
        .build()
}

binary_benchmark_group!(name = schema, benchmarks = bench_schema);

main!(
    config = BinaryBenchmarkConfig::default().env_clear(true);
    binary_benchmark_groups = schema
);
