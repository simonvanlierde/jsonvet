//! The formatter's size guards, its measuring of columns, and the rarer characters in its input,
//! each pinned at the exact number or character where its behaviour changes.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "a broken test setup should fail the test loudly"
)]

use jsonvet::{Config, Dialect, FormatError, FormatOptions, Indent, format};

fn fmt(dialect: Dialect, options: &FormatOptions, text: &str) -> String {
    match format(text, &Config::new(dialect), options) {
        Ok(out) => out,
        Err(FormatError::Syntax(report)) => format!("syntax error: {}", report.problems()[0]),
        Err(other) => other.to_string(),
    }
}

fn width(width: usize) -> FormatOptions {
    FormatOptions {
        width,
        ..FormatOptions::default()
    }
}

/// Arrays `depth` deep around one string, which lines up one per line, so the output is large
/// from a small text. Runs on a big stack, as the command does.
fn deep(depth: usize, inner: usize, pad: usize, options: FormatOptions) -> Result<String, String> {
    let text = format!(
        "{}\"{}\"{}{}",
        "[".repeat(depth),
        "x".repeat(inner),
        "]".repeat(depth),
        " ".repeat(pad)
    );
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || {
            format(&text, &Config::new(Dialect::Json), &options).map_err(|e| e.to_string())
        })
        .unwrap()
        .join()
        .unwrap()
}

const LIMIT: usize = 64 << 20;

#[test]
fn an_output_of_exactly_64_mib_is_kept_and_one_byte_more_is_refused() {
    // 5,790 levels with a two-character string take 67,071,363 bytes.
    let inner = LIMIT - 67_071_363;
    let out = deep(5_790, inner, 0, FormatOptions::default()).unwrap();
    assert_eq!(out.len(), LIMIT);
    assert_eq!(
        deep(5_790, inner + 1, 0, FormatOptions::default()),
        Err("not formatted: the output would be 67.1 MB".to_owned())
    );
}

#[test]
fn the_limit_is_16_times_the_text_when_that_is_more_than_64_mib() {
    // 6,000 levels take about 72 MB from a text padded to 5 MiB, so 80 MiB is the limit.
    let out = deep(6_000, 0, (5 << 20) - 12_002, FormatOptions::default()).unwrap();
    assert!(out.len() > LIMIT, "{}", out.len());
    // The same document in a text of 1 MiB has 64 MiB.
    assert!(deep(6_000, 0, (1 << 20) - 12_002, FormatOptions::default()).is_err());
}

#[test]
fn a_size_over_a_gigabyte_is_given_in_gigabytes_rounded_to_a_tenth() {
    let options = FormatOptions {
        indent: Indent::Spaces(20),
        ..FormatOptions::default()
    };
    // 1,620,036,003 bytes.
    assert_eq!(
        deep(9_000, 0, 0, options),
        Err("not formatted: the output would be 1.6 GB".to_owned())
    );
}

#[test]
fn json5_line_separators_count_as_line_breaks_and_only_there() {
    // A comment keeps the object on several lines, where a blank line shows.
    let blank = "{\"a\": 1, /* x */\n\n\"b\": 2}";
    let expected = fmt(Dialect::Json5, &width(80), blank);
    assert!(expected.contains("\n\n"), "{expected:?}");
    for sep in ['\u{2028}', '\u{2029}'] {
        let text = format!("{{\"a\": 1, /* x */{sep}{sep}\"b\": 2}}");
        assert_eq!(fmt(Dialect::Json5, &width(80), &text), expected, "{sep:?}");
    }
}

#[test]
fn a_json5_line_comment_ends_at_a_line_separator_and_no_other_line_comment_does() {
    let options = width(80);
    for sep in ['\u{2028}', '\u{2029}'] {
        let text = format!("[1, // c{sep}2]");
        assert_eq!(
            fmt(Dialect::Json5, &options, &text),
            "[\n  1, // c\n  2\n]\n",
            "{sep:?}"
        );
    }
    let text = "[1, // c\u{2028}x\n2]";
    assert_eq!(
        fmt(Dialect::Jsonc, &options, text),
        "[\n  1, // c\u{2028}x\n  2\n]\n"
    );
}

#[test]
fn white_space_at_the_end_of_a_line_comment_is_dropped() {
    assert_eq!(
        fmt(Dialect::Jsonc, &width(80), "[1, // c \t \n2]"),
        "[\n  1, // c\n  2\n]\n"
    );
}

#[test]
fn non_ascii_bare_keys_and_literals_keep_their_text() {
    let text = "{ \"ab\": 1, \u{e9}t\u{e9}: 2, \u{df}: 3, \u{3b1}\u{3b2}\u{3b3}: [1, 2] }";
    assert_eq!(
        fmt(Dialect::Json5, &width(30), text),
        "{\n  \"ab\": 1,\n  \u{e9}t\u{e9}: 2,\n  \u{df}: 3,\n  \u{3b1}\u{3b2}\u{3b3}: [1, 2]\n}\n"
    );
}

#[test]
fn a_comment_over_several_lines_leaves_the_column_of_its_last_line() {
    // The comment moves two columns right, so its last line is ` */`: after a space the value
    // starts at column 4, and `[1, 2]` takes 6 more.
    let text = "{\"a\": /*\n*/ [1,2]}";
    assert_eq!(
        fmt(Dialect::Jsonc, &width(10), text),
        "{\n  \"a\": /*\n */ [1, 2]\n}\n"
    );
    assert_eq!(
        fmt(Dialect::Jsonc, &width(9), text),
        "{\n  \"a\": /*\n */ [\n    1,\n    2\n  ]\n}\n"
    );
}

#[test]
fn a_tab_counts_as_four_columns_at_every_depth() {
    let tabs = |width| FormatOptions {
        indent: Indent::Tab,
        width,
        ..FormatOptions::default()
    };
    // `\t"a": ` is 9 columns and `[1, 2]` takes 6 more.
    let text = "{\"a\": [1,2] // c\n}";
    assert_eq!(
        fmt(Dialect::Jsonc, &tabs(15), text),
        "{\n\t\"a\": [1, 2] // c\n}\n"
    );
    assert_eq!(
        fmt(Dialect::Jsonc, &tabs(14), text),
        "{\n\t\"a\": [\n\t\t1,\n\t\t2\n\t] // c\n}\n"
    );
}

#[test]
fn comments_before_the_comma_count_toward_the_width_of_the_value() {
    // `  "a": ` is 7 columns, `[1, 2]` 6, then ` /* c */` 8 and the comma 1.
    let text = "{\"a\": [1,2] /* c */, \"b\": 1}";
    let flat = "{\n  \"a\": [1, 2] /* c */,\n  \"b\": 1\n}\n";
    assert_eq!(fmt(Dialect::Jwcc, &width(22), text), flat);
    assert_eq!(
        fmt(Dialect::Jwcc, &width(21), text),
        "{\n  \"a\": [\n    1,\n    2\n  ] /* c */,\n  \"b\": 1\n}\n"
    );
}

#[test]
fn a_blank_line_before_the_comments_after_the_last_member_is_kept_unless_nothing_precedes() {
    let jsonc = |text| fmt(Dialect::Jsonc, &width(80), text);
    assert_eq!(
        jsonc("{\"a\": 1\n\n// t\n}"),
        "{\n  \"a\": 1\n\n  // t\n}\n"
    );
    assert_eq!(jsonc("{\"a\": 1\n// t\n}"), "{\n  \"a\": 1\n  // t\n}\n");
    assert_eq!(jsonc("{\n\n// t\n}"), "{\n  // t\n}\n");
}

#[test]
fn comments_after_a_colon_go_on_their_own_lines_where_the_text_broke_them() {
    let jsonc = |text| fmt(Dialect::Jsonc, &width(80), text);
    // A line comment ends the line; the next comment starts one level in, and so does the value.
    assert_eq!(
        jsonc("{\"a\": // x\n/* y */ [1, // c\n2]}"),
        "{\n  \"a\": // x\n    /* y */ [\n      1, // c\n      2\n    ]\n}\n"
    );
    // A block comment followed by a line break in the text puts the value on the next line.
    assert_eq!(
        jsonc("{\"a\": /* y */\n[1, // c\n2]}"),
        "{\n  \"a\": /* y */\n    [\n      1, // c\n      2\n    ]\n}\n"
    );
    // Only the last comment decides that; the ones before it stay on the line.
    assert_eq!(
        jsonc("{\"a\": /* x */\n/* y */\n1}"),
        "{\n  \"a\": /* x */ /* y */\n    1\n}\n"
    );
}

#[test]
fn normalizing_json5_unquotes_the_keys_an_identifier_can_be() {
    let options = FormatOptions {
        normalize_json5: true,
        ..FormatOptions::default()
    };
    let text = "{\"a\": 1, '_b': 2, \"$c\": 3, \"d-e\": 4, \"1x\": 5, 'Z9': 6, \"\": 7}";
    assert_eq!(
        fmt(Dialect::Json5, &options, text),
        "{ a: 1, _b: 2, $c: 3, \"d-e\": 4, \"1x\": 5, Z9: 6, \"\": 7 }\n"
    );
}
