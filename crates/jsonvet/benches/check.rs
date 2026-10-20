//! Instruction counts for `check` on the inputs `scripts/bench.py` times, scaled down so callgrind
//! finishes in seconds. Needs valgrind and `gungraun-runner` at the gungraun version
//! (`cargo bench -p jsonvet --bench check`); ci.yml's `bench` job runs it.

#![allow(
    missing_docs,
    reason = "gungraun's macros generate undocumented public items"
)]

use std::fmt::Write as _;
use std::hint::black_box;

use gungraun::{library_benchmark, library_benchmark_group, main};
use jsonvet::{Config, Dialect, check};

/// Rows in the generated documents: about 13 KB of JSON.
const ROWS: usize = 200;

/// A small tsconfig-like file, the common case for a pre-commit hook.
fn config_file() -> String {
    let options: Vec<String> = (0..30).map(|i| format!("    \"opt{i}\": true")).collect();
    format!(
        "{{\n  \"compilerOptions\": {{\n{}\n  }},\n  \"include\": [\"src\"]\n}}\n",
        options.join(",\n")
    )
}

/// An object of `ROWS` rows, each a small array of every JSON type, as bench.py's `doc`.
fn document() -> String {
    let mut text = String::from("{\n");
    for i in 0..ROWS {
        let comma = if i + 1 < ROWS { "," } else { "" };
        let _ = writeln!(
            text,
            "  \"k{i}\": [1, 2.5, \"text\", true, null, {{\"x\": {i}, \"y\": [{i}, \"s\"]}}]{comma}"
        );
    }
    text.push_str("}\n");
    text
}

/// [`document`] with a duplicate of key `k5` at the end.
fn document_with_duplicate() -> String {
    let mut text = document();
    text.truncate(text.len() - 3);
    text.push_str(",\n  \"k5\": 0\n}\n");
    text
}

/// JSON5 that strict JSON rejects, as bench.py's `json5_doc`.
fn json5_document() -> String {
    let mut text = String::from("{\n");
    for i in 0..ROWS {
        let _ = writeln!(
            text,
            "  k{i}: [1, 0x1F, .5, 'text', true, null, {{x: {i}, y: [{i}, 's',],}},], // row {i}"
        );
    }
    text.push_str("}\n");
    text
}

#[library_benchmark]
#[bench::config_file(config_file(), Dialect::Json)]
#[bench::document(document(), Dialect::Json)]
#[bench::duplicate_key(document_with_duplicate(), Dialect::Json)]
#[bench::json5(json5_document(), Dialect::Json5)]
fn bench_check(text: String, dialect: Dialect) -> usize {
    black_box(
        check(black_box(&text), &Config::new(dialect))
            .problems()
            .len(),
    )
}

library_benchmark_group!(name = checks, benchmarks = bench_check);

main!(library_benchmark_groups = checks);
