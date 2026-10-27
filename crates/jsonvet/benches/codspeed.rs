//! Wall-clock benchmarks of the library with divan, which `.github/workflows/codspeed.yml` runs
//! under `CodSpeed` on every pull request: `check` in every dialect, `parse` with its events, the
//! JSON Lines split that jsonvet-cli runs on threads, and line and column lookups. Run them with
//! `cargo bench -p jsonvet --bench codspeed`, or as CI does with `cargo codspeed build -p jsonvet
//! --bench codspeed` and then `cargo codspeed run -p jsonvet`.

#![allow(missing_docs, reason = "divan's macros generate undocumented items")]

use std::fmt::Write as _;

use divan::{Bencher, black_box};
use jsonvet::{Config, Dialect, check, join_lines, line_parts, parse, parse_lines};

fn main() {
    divan::main();
}

/// Rows in the generated documents: about 65 KB of JSON.
const ROWS: usize = 1_000;

/// A small tsconfig-like file, the common case for a pre-commit hook.
fn config_file() -> String {
    let options: Vec<String> = (0..30).map(|i| format!("    \"opt{i}\": true")).collect();
    format!(
        "{{\n  \"compilerOptions\": {{\n{}\n  }},\n  \"include\": [\"src\"]\n}}\n",
        options.join(",\n")
    )
}

/// An object of `ROWS` rows, each a small array of every JSON type.
fn document() -> String {
    let mut text = String::from("{\n");
    for i in 0..ROWS {
        let comma = if i + 1 < ROWS { "," } else { "" };
        let _ = writeln!(
            text,
            "  \"k{i}\": [1, 2.5e3, \"text \\u00e9\", true, null, {{\"x\": {i}, \"y\": [{i}, \"s\"]}}]{comma}"
        );
    }
    text.push_str("}\n");
    text
}

/// [`document`] with a duplicate of every tenth key at the end.
fn document_with_duplicates() -> String {
    let mut text = document();
    text.truncate(text.len() - 3);
    for i in (0..ROWS).step_by(10) {
        let _ = write!(text, ",\n  \"k{i}\": 0");
    }
    text.push_str("\n}\n");
    text
}

/// JSONC: [`document`]'s rows with line and block comments.
fn jsonc_document() -> String {
    let mut text = String::from("/* generated */\n{\n");
    for i in 0..ROWS {
        let comma = if i + 1 < ROWS { "," } else { "" };
        let _ = writeln!(
            text,
            "  \"k{i}\": [1, /* two */ 2.5, \"text\", true, null, {{\"x\": {i}}}]{comma} // row {i}"
        );
    }
    text.push_str("}\n");
    text
}

/// JWCC: JSONC with trailing commas.
fn jwcc_document() -> String {
    let mut text = String::from("{\n");
    for i in 0..ROWS {
        let _ = writeln!(
            text,
            "  \"k{i}\": [1, 2.5, \"text\", true, null, {{\"x\": {i}, \"y\": [{i}, \"s\",],}},], // row {i}"
        );
    }
    text.push_str("}\n");
    text
}

/// JSON5 that strict JSON rejects.
fn json5_document() -> String {
    let mut text = String::from("{\n");
    for i in 0..ROWS {
        let _ = writeln!(
            text,
            "  k{i}: [1, 0x1F, .5, +Infinity, 'text', true, null, {{x: {i}, y: [{i}, 's',],}},], // row {i}"
        );
    }
    text.push_str("}\n");
    text
}

/// JSON Lines: `ROWS` records, one per line.
fn json_lines_document() -> String {
    let mut text = String::new();
    for i in 0..ROWS {
        let _ = writeln!(
            text,
            "{{\"id\": {i}, \"name\": \"event {i}\", \"ok\": true, \"tags\": [\"a\", \"b\"], \"v\": 1.5}}"
        );
    }
    text
}

/// Deeply nested arrays and objects.
fn nested_document() -> String {
    const DEPTH: usize = 100;
    let mut text = String::new();
    for i in 0..DEPTH {
        let _ = write!(text, "{{\"level{i}\": [");
    }
    text.push_str("null");
    for _ in 0..DEPTH {
        text.push_str("]}");
    }
    text
}

/// A long string with many escapes and non-ASCII characters.
fn string_heavy_document() -> String {
    let mut text = String::from("[");
    for i in 0..ROWS {
        if i > 0 {
            text.push_str(", ");
        }
        let _ = write!(
            text,
            "\"line {i}\\n\\ttab \\\"quoted\\\" caf\u{e9} \u{65e5}\u{672c} \\ud83d\\ude00 end\""
        );
    }
    text.push_str("]\n");
    text
}

/// `check` on each input in the dialect its file would be checked in.
mod check_dialect {
    use super::{Bencher, Config, Dialect, black_box, check};

    fn run(bencher: Bencher, text: &str, dialect: Dialect) {
        let config = Config::new(dialect);
        bencher.bench(|| check(black_box(text), black_box(&config)).problems().len());
    }

    #[divan::bench]
    fn config_file(bencher: Bencher) {
        run(bencher, &super::config_file(), Dialect::Json);
    }

    #[divan::bench]
    fn json(bencher: Bencher) {
        run(bencher, &super::document(), Dialect::Json);
    }

    #[divan::bench]
    fn i_json(bencher: Bencher) {
        run(bencher, &super::document(), Dialect::IJson);
    }

    #[divan::bench]
    fn duplicate_keys(bencher: Bencher) {
        run(bencher, &super::document_with_duplicates(), Dialect::Json);
    }

    #[divan::bench]
    fn jsonc(bencher: Bencher) {
        run(bencher, &super::jsonc_document(), Dialect::Jsonc);
    }

    #[divan::bench]
    fn jwcc(bencher: Bencher) {
        run(bencher, &super::jwcc_document(), Dialect::Jwcc);
    }

    #[divan::bench]
    fn json5(bencher: Bencher) {
        run(bencher, &super::json5_document(), Dialect::Json5);
    }

    #[divan::bench]
    fn json_lines(bencher: Bencher) {
        run(bencher, &super::json_lines_document(), Dialect::JsonLines);
    }

    #[divan::bench]
    fn nested(bencher: Bencher) {
        run(bencher, &super::nested_document(), Dialect::Json);
    }

    #[divan::bench]
    fn strings(bencher: Bencher) {
        run(bencher, &super::string_heavy_document(), Dialect::IJson);
    }

    /// Checking stops at the first syntax error, here in the last row.
    #[divan::bench]
    fn syntax_error(bencher: Bencher) {
        let mut text = super::document();
        text.truncate(text.len() - 3);
        text.push_str(" \"missing\": }\n");
        run(bencher, &text, Dialect::Json);
    }
}

/// `parse`, which also builds the events that `--schema` checks.
mod parse_events {
    use super::{Bencher, Config, Dialect, black_box, parse};

    fn run(bencher: Bencher, text: &str, dialect: Dialect) {
        let config = Config::new(dialect);
        bencher.bench(|| {
            let (report, events) = parse(black_box(text), black_box(&config));
            (report.problems().len(), events.map(|e| e.len()))
        });
    }

    #[divan::bench]
    fn json(bencher: Bencher) {
        run(bencher, &super::document(), Dialect::Json);
    }

    #[divan::bench]
    fn json5(bencher: Bencher) {
        run(bencher, &super::json5_document(), Dialect::Json5);
    }
}

/// A JSON Lines text split into parts, each checked on its own, then joined, as jsonvet-cli does
/// for large files.
#[divan::bench(args = [1, 4])]
fn json_lines_parts(bencher: Bencher, parts: usize) {
    let text = json_lines_document();
    let config = Config::new(Dialect::JsonLines);
    bencher.bench(|| {
        let text = black_box(text.as_str());
        let mut events = 0;
        let lines = line_parts(text, parts)
            .into_iter()
            .map(|range| parse_lines(text, range, &config, Some(&mut |_| events += 1)))
            .collect();
        let (report, syntax_error) = join_lines(text, lines);
        (report.problems().len(), syntax_error, events)
    });
}

/// Line and column of every problem in a file with many duplicate keys.
#[divan::bench]
fn locations(bencher: Bencher) {
    let text = document_with_duplicates();
    let config = Config::new(Dialect::Json);
    bencher.bench(|| {
        let report = check(black_box(&text), &config);
        report
            .problems()
            .iter()
            .map(|p| report.location(p.offset()).line)
            .sum::<usize>()
    });
}
