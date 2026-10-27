//! `--schema` runs in-process with divan, which `.github/workflows/codspeed.yml` measures under
//! CodSpeed on every pull request, for CPU and for memory: the cases `benches/schema.rs` counts
//! under callgrind, minus the process start. jsonvet's known memory limits are in `--schema`
//! (SECURITY.md), so these are the runs CodSpeed measures memory on. Run them with `cargo
//! bench -p jsonvet-cli --bench codspeed`.

#![allow(
    missing_docs,
    clippy::expect_used,
    reason = "divan's macros generate undocumented items, and a broken fixture should fail loudly"
)]

use divan::{Bencher, black_box};
use jsonvet_cli::Terminal;
use std::ffi::OsString;

mod common;

fn main() {
    divan::main();
}

#[divan::bench(args = ["dependabot", "valid_object", "valid_array", "failing_array", "costly_keys"])]
fn schema(bencher: Bencher, name: &str) {
    let (dir, code) = common::case("codspeed-schema", name);
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
