//! Behaviour across the dialects, as one reviewed snapshot: each case shows every problem with
//! its position, severity, rule and message.

use std::fmt::Write;

use jsonvet::{Config, Dialect, check};

fn render(dialect: Dialect, text: &str) -> String {
    let report = check(text, &Config::new(dialect));
    let mut out = format!("{dialect:<6} {text:?}\n");
    if report.problems().is_empty() {
        out.push_str("       ok\n");
    }
    for problem in report.problems() {
        let at = report.location(problem.offset());
        let _ = write!(
            out,
            "       {}:{} {}[{}] {problem}",
            at.line,
            at.column,
            problem.severity(),
            problem.rule()
        );
        if let Some(related) = problem.related().map(|r| r.offset) {
            let first = report.location(related);
            let _ = write!(out, " (from {}:{})", first.line, first.column);
        }
        out.push('\n');
    }
    out
}

fn snapshot(cases: &[(Dialect, &str)]) -> String {
    cases.iter().map(|(d, t)| render(*d, t)).collect()
}

#[test]
fn syntax_in_every_dialect() {
    use Dialect::{Json, Json5, Jsonc, Jwcc};
    insta::assert_snapshot!(snapshot(&[
        // Empty documents, containers and trailing commas.
        (Json, ""),
        (Json, "   "),
        (Json, r#"{"a": 1}"#),
        (Json, "[1, 2,]"),
        (Jwcc, "[1, 2,]"),
        (Json, r#"{"a": 1,}"#),
        (Json, "[,]"),
        (Json5, "[,]"),
        (Json5, "[1,,]"),
        (Json5, "{,}"),
        // Unquoted keys and single quotes.
        (Json, "{a: 1}"),
        (Json5, "{a: 1, $b: 2, _c: 3, ünï: 4}"),
        // Identifier characters go by general category, as in ECMAScript 5.1, not by Unicode's
        // ID_Start and ID_Continue: U+1885 is a mark, U+00B7 punctuation, U+0E33 and U+2E2F letters.
        (Json5, "{\u{1885}: 1}"),
        (Json5, "{\\u1885: 1}"),
        (Json5, "{a\u{1885}: 1}"),
        (Json5, "{a\u{B7}: 1}"),
        (Json5, "{\u{E33}: 1, \u{2E2F}: 2}"),
        // U+3308C is a CJK Extension J letter, new in Unicode 17.
        (Json5, "{\u{3308C}: 1}"),
        (Json, "['x']"),
        (Json, "{'a': 1}"),
        (Json, "{1: 2}"),
        (Json5, "{abc"),
        (Json5, r"{a\x61: 1}"),
        (Json5, "['x', \"y\"]"),
        // Comments.
        (Json, "// c\n1"),
        (Jsonc, "// c\n1"),
        (Jsonc, "/* open"),
        // Missing separators, trailing text and literals.
        (Json, "[1 2]"),
        (Json, r#"{"a" 1}"#),
        (Json, r#"{"a":1 "b":2}"#),
        (Json, "1 2"),
        (Json, "[1]x"),
        (Json, "nul"),
        (Json, "nux"),
        (Json, "nullx"),
        (Json, "[1}"),
        (Json, r#"{"a": 1]"#),
        // Numbers.
        (Json, "NaN"),
        (Json, "-Infinity"),
        (Json5, "[NaN, +Infinity, -Infinity, +NaN]"),
        (Json, "01"),
        (Json, "1."),
        (Json, ".5"),
        (Json, "+1"),
        (Json, "0x1F"),
        (Json5, "[0x1F, -0x10, .5, 5., +1, 1e3, 1E-3]"),
        (Json5, "0x"),
        (Json5, "1e"),
        (Json5, "."),
        (Json, "-"),
        // Strings and escapes.
        (Json, "\"abc"),
        (Json, r#""\" \\ \/ \b \f \n \r \t \u0041""#),
        (Json, "\"a\nb\""),
        (Json, "\"a\tb\""),
        (Json5, "\"a\tb\""),
        (Json, r#""\q""#),
        (Json5, r#""\q""#),
        (Json5, r#""\1""#),
        (Json5, r#""\9""#),
        (Json5, r#""\01""#),
        (Json5, r#""\0""#),
        (Json5, r#""\x4""#),
        (Json, r#""\u12x4""#),
        (Json5, "\"a\\\nb\""),
        (Json5, "\"a\\\r\nb\""),
        // JSON5's extra line terminators and whitespace.
        (Json5, "\"\u{2028}\""),
        (Json5, "[1, // c\u{2028} 2]"),
        (Jsonc, "[1, // c\u{2028} 2]"),
        (Jsonc, "[1, // c\r2]"),
        (Jwcc, "[1, // c\r2]"),
        (Jwcc, "[1, // c\r\n2]"),
        (Jwcc, "[1] // c\r"),
        (Jwcc, "[1, // c\u{2028} 2]"),
        (Json5, "\u{A0}[\u{3000}1\u{FEFF}]\u{2029}"),
        (Json, "[\u{A0}1]"),
        (Json5, "[\u{B}1\u{C}]"),
        (Json, "[\u{B}1]"),
    ]));
}

#[test]
fn duplicate_keys() {
    use Dialect::{IJson, Json, Json5};
    insta::assert_snapshot!(snapshot(&[
        (Json, r#"{"a": 1, "a": 2}"#),
        (
            Json,
            "{\n  \"x\": {\"a\": 1, \"\\u0061\": 2},\n  \"l\": [{\"k\": 1}, {\"k\": 2, \"k\": 3}],\n  \"x\": 0\n}"
        ),
        (Json5, "{a: 1, 'a': 2, \"b c\": {d: 1, d: 2}, \\u0061: 3}"),
        (Json5, "{\"ünicode\": 1, \"ünicode\": 2}"),
        // Lone surrogates stay distinct keys, as in Python; U+FFFD is not one of them.
        (
            Json,
            r#"{"\ud800": 1, "\udbff": 2, "\ufffd": 3, "a\udc00": 4}"#
        ),
        (
            Json,
            r#"{"\ud800": 1, "\uD800": 2, "\ud83d\ude00": 3, "😀": 4}"#
        ),
        (Json, r#"{"\ud800": 1, "\ud801": 2}"#),
        (
            Json,
            r#"{"\u0041\udc00": 1, "A\udc00": 2, "\ud800\u0041": 3, "\ud800A": 4}"#
        ),
        (Json, r#"{"\u00e9": 1, "é": 2, "\u4e2d": 3, "中": 4}"#),
        (Json, r#"{"\n": 1, "n": 2, "\/": 3, "/": 4}"#),
        (
            Json5,
            r#"{"\v": 1, "v": 2, "\0": 3, "0": 4, "\'": 5, "'": 6}"#
        ),
        (Json5, r#"{"\é": 1, "é": 2, "\q": 3, "q": 4}"#),
        (Json5, r#"{"\x41b": 1, "Ab": 2}"#),
        (
            Json5,
            "{\"a\\\nb\": 1, \"a\\\r\nb\": 2, \"a\\\u{2028}b\": 3, \"ab\": 4}"
        ),
        (IJson, r#"{"a": 1, "a": 2}"#),
        (Json, r#"[{"a": 1, "a": 2}, {"\x": 0}]"#),
    ]));
}

#[test]
fn i_json_and_json_lines() {
    use Dialect::{IJson, Json, JsonLines};
    insta::assert_snapshot!(snapshot(&[
        (
            IJson,
            "[9007199254740991, 9007199254740992, -9007199254740993, 1e400, 1.5]"
        ),
        (Json, "[9007199254740992, 1e400]"),
        (IJson, r#"["\ud800", "\ud83d\ude00", "\ufdd0", "﷐"]"#),
        (IJson, r#"["\uffff", "\ud83f\udfff", "\u00e8", "\ufffd"]"#),
        (IJson, "\u{FEFF}{}"),
        (Json, "\u{FEFF}{}"),
        (JsonLines, ""),
        (JsonLines, "{\"a\": 1}\n[2]\n3\n"),
        (JsonLines, "{\"a\": 1}\n[2]"),
        (JsonLines, "{\"a\": 1}\n\n[2]\n"),
        (JsonLines, "{\"a\": 1}\n   \n"),
        (JsonLines, "{\"a\":\n1}\n"),
        (JsonLines, "1 2\n"),
        (JsonLines, "{\"a\": 1, \"a\": 2}\r\n[3]\r\n"),
        (JsonLines, "\u{FEFF}{}\n"),
        // A lone CR is white space, not a line ending, though positions count it as one.
        (JsonLines, "[1,\r2]\n"),
        (JsonLines, "1\r"),
        (JsonLines, "1\r\n\r\n"),
    ]));
}

#[test]
fn json_lines_positions_do_not_depend_on_the_line_ending() {
    let problems = |text: &str| -> Vec<String> {
        let report = check(text, &Config::new(Dialect::JsonLines));
        report
            .problems()
            .iter()
            .map(|p| {
                let at = report.location(p.offset());
                format!("{}:{} {p}", at.line, at.column)
            })
            .collect()
    };
    for lf in [
        "[1,\n2]\n",
        "{\"a\":\n1}\n",
        "1 \n\n2\n",
        "{\"a\": 1, \"a\": 2}\n[3 4]\n",
    ] {
        let crlf = lf.replace('\n', "\r\n");
        assert_eq!(problems(&crlf), problems(lf), "{crlf:?}");
    }
}

#[test]
fn positions_follow_json5_line_terminators() {
    use Dialect::{Json, Json5, JsonLines, Jsonc, Jwcc};
    // A raw U+2028 in a string ends a line in JSON5 only, so the duplicate is on line 2
    // elsewhere and on line 4 in JSON5, after both.
    let separator = "{\"s\": \"a\u{2028}b\u{2029}\",\n \"s\": 2}";
    insta::assert_snapshot!(snapshot(&[
        (Json5, "{a: 1,\r\n a: 2}"),
        (Json5, "{a: 1,\r a: 2}"),
        (Json5, "{a: 1,\u{2028} a: 2}"),
        (Json, separator),
        (Jsonc, separator),
        (Jwcc, separator),
        (Json5, separator),
        (
            JsonLines,
            "{\"s\": \"\u{2028}\", \"s\": 2}\n{\"t\": 1, \"t\": 2}\n"
        ),
        (Json5, "{\"é😀\": 1, \"é😀\": 2}"),
    ]));
}

#[test]
fn hex_numbers_overflow_exactly_where_a_double_does() {
    use jsonvet::{Rule, Severity};
    let config = Config::new(Dialect::Json5).rule(Rule::InvalidNumber, Severity::Error);
    let message = |hex: String| {
        let text = format!("0x{hex}");
        let report = check(&text, &config);
        report.problems().first().map(ToString::to_string)
    };
    let overflow = Some("number overflows a double".to_owned());
    let unsafe_integer =
        Some("integer beyond ±(2^53 − 1) cannot be represented exactly".to_owned());
    assert_eq!(
        message(format!("{}C{}", "F".repeat(13), "0".repeat(242))),
        overflow
    );
    assert_eq!(
        message(format!("{}b{}", "f".repeat(13), "f".repeat(242))),
        unsafe_integer
    );
    assert_eq!(message(format!("1{}", "0".repeat(256))), overflow);
    assert_eq!(message(format!("00{}", "F".repeat(14))), unsafe_integer);
    assert_eq!(message("1FFFFFFFFFFFFF".to_owned()), None);
}

#[test]
fn names_and_file_names() {
    use std::path::Path;
    let for_path = |name: &str| Dialect::for_path(Path::new(name)).map(Dialect::name);
    for (name, dialect) in [
        ("a.json", Some("json5")),
        ("dir/.babelrc", Some("json5")),
        ("a.babelrc", None),
        ("x.webmanifest", Some("json5")),
        ("a.code-workspace", Some("jsonc")),
        ("a.hujson", Some("jwcc")),
        ("a.ndjson", Some("jsonl")),
        ("a.txt", None),
        ("json", None),
        ("a.JSON", None),
        ("a.", None),
    ] {
        assert_eq!(for_path(name), dialect, "{name}");
    }
    for (ext, dialect) in Dialect::EXTENSIONS {
        let name = format!("dir/a.{ext}");
        assert_eq!(
            Dialect::for_path(Path::new(&name)),
            Some(*dialect),
            "{name}"
        );
    }
    for (name, dialect) in Dialect::FILE_NAMES {
        assert_eq!(Dialect::for_path(Path::new(name)), Some(*dialect), "{name}");
    }
    for (alias, dialect) in [
        ("ndjson", Dialect::JsonLines),
        ("json-lines", Dialect::JsonLines),
        ("hujson", Dialect::Jwcc),
        ("ijson", Dialect::IJson),
        ("jwcc", Dialect::Jwcc),
    ] {
        assert_eq!(alias.parse(), Ok(dialect), "{alias}");
    }
    let unknown = "yaml".parse::<Dialect>().expect_err("not a dialect");
    assert_eq!(unknown.name(), "yaml");
    assert!(
        unknown
            .to_string()
            .starts_with("unknown dialect `yaml`; expected one of json,")
    );
    for rule in jsonvet::Rule::ALL {
        assert!(!rule.description().is_empty(), "{rule}");
    }
    assert_eq!(
        jsonvet::Rule::DuplicateKey.description(),
        "a key repeated in one object"
    );
    let unknown = "tabs".parse::<jsonvet::Rule>().expect_err("not a rule");
    assert_eq!(unknown.id(), "tabs");
    assert!(
        unknown
            .to_string()
            .starts_with("unknown rule `tabs`; expected one of syntax,")
    );
}

#[test]
fn warnings_alone_pass() {
    let config = Config::new(Dialect::Json);
    let report = check("\u{FEFF}{}", &config);
    assert_eq!(report.problems().len(), 1);
    assert!(report.passed());
    assert!(!check("{", &config).passed());
}

#[test]
fn nesting_stops_at_the_depth_limit() {
    let nested = |depth: usize| format!("{}{}", "[".repeat(depth), "]".repeat(depth));
    let config = Config::new(Dialect::Json);
    assert!(check(&nested(Config::DEFAULT_MAX_DEPTH), &config).passed());
    let too_deep = nested(Config::DEFAULT_MAX_DEPTH + 1);
    let report = check(&too_deep, &config);
    let problem = &report.problems()[0];
    assert_eq!(problem.offset(), Config::DEFAULT_MAX_DEPTH);
    assert_eq!(problem.to_string(), "nested deeper than 10000 levels");
    let shallow = Config::new(Dialect::Json5).max_depth(2);
    assert!(check("{a: [1]}", &shallow).passed());
    assert!(!check("{a: [{}]}", &shallow).passed());
}

#[test]
fn related_offsets_say_what_they_point_at() {
    use jsonvet::RelatedKind;
    let related = |dialect, text| {
        let report = check(text, &Config::new(dialect));
        report.problems()[0].related().map(|r| (r.offset, r.kind))
    };
    assert_eq!(
        related(Dialect::Json, r#"{"a": 1, "a": 2}"#),
        Some((1, RelatedKind::FirstOccurrence))
    );
    assert_eq!(
        related(Dialect::Json, r#"[1, "abc"#),
        Some((4, RelatedKind::Start))
    );
    assert_eq!(
        related(Dialect::Jsonc, "1 /* open"),
        Some((2, RelatedKind::Start))
    );
    assert_eq!(related(Dialect::Json, "[1 2]"), None);
}

#[test]
fn duplicate_key_paths_stay_short_however_deep_the_key() {
    let depth = 9_000;
    let members = vec![r#""a":0"#; 20_000].join(",");
    let deep = format!("{}{{{members}}}{}", "[".repeat(depth), "]".repeat(depth));
    let every_level = format!(
        "{}1{}",
        r#"{"abc":1,"abc":"#.repeat(depth),
        "}".repeat(depth)
    );
    for text in [deep, every_level] {
        let report = check(&text, &Config::new(Dialect::Json));
        assert!(report.problems().len() >= depth);
        for problem in report.problems() {
            // 256 bytes and the three of `…`.
            let path = problem.path().unwrap_or_default();
            assert!(path.len() <= 259, "{} bytes", path.len());
        }
    }
    let text = format!("{}{{\"a\":0,\"a\":0}}{}", "[".repeat(100), "]".repeat(100));
    let report = check(&text, &Config::new(Dialect::Json));
    let message = report.problems()[0].to_string();
    let levels = "[0]".repeat(42);
    assert_eq!(message, format!("duplicate key ${levels}…{levels}.a"));
}

/// Printed as they are, bidi controls would reorder how the rest of the message displays.
#[test]
fn messages_show_bidi_controls_as_code_points() {
    let config = Config::new(Dialect::Json);
    for c in [
        '\u{61C}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202E}', '\u{2066}', '\u{2069}',
    ] {
        let text = format!("[{c}]");
        let report = check(&text, &config);
        assert_eq!(
            report.problems()[0].to_string(),
            format!("expected a value, found U+{:04X}", u32::from(c))
        );
    }
    let report = check("[\u{200D}]", &config);
    assert_eq!(
        report.problems()[0].to_string(),
        "expected a value, found `\u{200D}`"
    );
}

/// JWCC ends a `//` comment at LF only, JSONC also at a lone CR, and JSON5 also at U+2028 and
/// U+2029. Where the comment does not end, it swallows the `2]` after it.
#[test]
fn line_comments_end_where_each_dialect_says() {
    use Dialect::{Json5, Jsonc, Jwcc};
    for (end, dialects) in [
        ("\n", &[Jwcc, Jsonc, Json5][..]),
        ("\r\n", &[Jwcc, Jsonc, Json5]),
        ("\r", &[Jsonc, Json5]),
        ("\u{2028}", &[Json5]),
        ("\u{2029}", &[Json5]),
    ] {
        for dialect in [Jwcc, Jsonc, Json5] {
            let text = format!("[1, // c{end}2]");
            let passed = check(&text, &Config::new(dialect)).passed();
            assert_eq!(passed, dialects.contains(&dialect), "{dialect} {text:?}");
        }
    }
}
