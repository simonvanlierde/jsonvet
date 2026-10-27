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
        // A missing comma points just after the value before it, past any comment.
        (Json, "{\n  \"a\": 1\n  \"b\": 2\n}"),
        (Jsonc, "[1 /* c */ 2]"),
        (Json5, "{a: 1 b: 2}"),
        (Json, "[1 x]"),
        // An object or array still open at the end names its bracket.
        (Json, "{\"a\": [1, {\"b\": 2"),
        (Json, "[1,"),
        (Json, "{\"a\""),
        (Json, "[1e"),
        // A word shows whole, with a hint where it looks like another language's literal.
        (Json, r#"{"a": True}"#),
        (Json, "[False, None, NULL, undefined, nil]"),
        (Json5, r#"{"a": None}"#),
        (Json5, "[Nope]"),
        (Json, "[nulL]"),
        (Json, r#"{"a" True}"#),
        (Json5, "01"),
        (Json, "[-007]"),
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
    // The parse the CLI uses for --schema places problems the same way.
    let (report, _) = jsonvet::parse("{a: 1,\u{2028} a: 2}", &Config::new(Json5));
    assert_eq!(report.location(report.problems()[0].offset()).line, 2);
    // So does the streaming parse it uses for large files.
    let report = jsonvet::parse_with("{a: 1,\u{2028} a: 2}", &Config::new(Json5), &mut |_| {});
    assert_eq!(report.location(report.problems()[0].offset()).line, 2);
}

#[test]
fn noncharacters_are_found_after_long_plain_runs() {
    let plain = "a".repeat(20);
    let text = format!("[\"{plain}é{plain}\u{FFFF}{plain}\u{FDD0}\"]");
    let report = check(&text, &Config::new(Dialect::IJson));
    let offsets: Vec<usize> = report
        .problems()
        .iter()
        .map(jsonvet::Problem::offset)
        .collect();
    assert_eq!(offsets, [44, 67]);
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
    let report = check(r#"{"x": 0, "a": 1, "a": 2}"#, &Config::new(Dialect::Json));
    #[expect(deprecated, reason = "kept working until 4.0")]
    let offset = report.problems()[0].related_offset();
    assert_eq!(offset, Some(9));
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

/// A word shows whole only where it starts and ends at a token boundary; otherwise the message
/// names the one character, as it would without the word scan.
#[test]
fn words_show_whole_only_between_token_boundaries() {
    let first = |dialect, text: &str| check(text, &Config::new(dialect)).problems()[0].to_string();
    assert_eq!(
        first(Dialect::Json, "[trueish]"),
        "expected `,` or `]`, found `i`"
    );
    assert_eq!(
        first(Dialect::Json, "[Trué]"),
        "expected a value, found `T`"
    );
    // 10 MB of letters: one bounded look, not a scan of the whole run.
    let long = format!("[{}]", "a".repeat(10_000_000));
    assert_eq!(first(Dialect::Json, &long), "expected a value, found `a`");
}

/// After a sign only a number can follow, so a word there gets no literal hint: `-null` is no
/// more valid than `-None`.
#[test]
fn a_word_after_a_sign_gets_no_literal_hint() {
    for dialect in [Dialect::Json, Dialect::Json5] {
        let report = check("[-None]", &Config::new(dialect));
        let message = report.problems()[0].to_string();
        assert!(!message.contains("did you mean"), "{dialect}: {message}");
    }
}

/// A hint to use a larger dialect is given only where that dialect reads the token whole: not
/// for a word that runs on, a number cut short, or a string or comment that never ends.
#[test]
fn a_dialect_hint_needs_the_whole_token() {
    let first = |dialect, text: &str| check(text, &Config::new(dialect)).problems()[0].to_string();
    let hinted = [
        (Dialect::Json, "[NaN]"),
        (Dialect::Json, "[NaN-1]"),
        (Dialect::Json, "[-Infinity]"),
        (Dialect::Json, "[0x1F]"),
        (Dialect::Json, "[+1]"),
        (Dialect::Json, "[+NaN]"),
        (Dialect::Json, "[.5]"),
        (Dialect::Json, "{a: 1}"),
        (Dialect::Json, "{a-: 1}"),
        (Dialect::Jwcc, "['a']"),
        (Dialect::Json, "{'a': 1}"),
        (Dialect::Json, "/* c */ 1"),
        (Dialect::Json, "// c"),
    ];
    for (dialect, text) in hinted {
        let message = first(dialect, text);
        assert!(
            message.ends_with("dialect allows this"),
            "{text:?}: {message}"
        );
    }
    let bare = [
        ("[NaNx]", "NaN and Infinity are not allowed in json"),
        ("[Infinityy]", "NaN and Infinity are not allowed in json"),
        ("[-Infinity1]", "NaN and Infinity are not allowed in json"),
        ("[NaN$]", "NaN and Infinity are not allowed in json"),
        ("[NaN\\u0061]", "NaN and Infinity are not allowed in json"),
        ("[NaN\u{301}]", "NaN and Infinity are not allowed in json"),
        ("[0x]", "this number form is not allowed in json"),
        ("[0x1g]", "this number form is not allowed in json"),
        ("[+]", "this number form is not allowed in json"),
        ("[+NaNx]", "this number form is not allowed in json"),
        ("[.5x]", "this number form is not allowed in json"),
        ("[5.e]", "this number form is not allowed in json"),
        ("{a\\x: 1}", "unquoted keys are not allowed in json"),
        ("['a]", "single-quoted strings are not allowed in json"),
        ("{'a: 1}", "single-quoted strings are not allowed in json"),
        ("/* c", "comments are not allowed in json"),
    ];
    for (text, message) in bare {
        let report = check(text, &Config::new(Dialect::Json));
        assert_eq!(report.problems()[0].to_string(), message, "{text:?}");
        assert_eq!(
            report.problems()[0].offset(),
            usize::from(text.starts_with('[') || text.starts_with('{')),
            "{text:?}"
        );
    }
}

/// Each problem spans the text it is about, from `offset` to `end`, or no text at a place
/// between characters.
#[test]
fn each_problem_spans_its_text() {
    use Dialect::{IJson, Json, Json5, JsonLines};
    fn spans(dialect: Dialect, text: &str) -> Vec<(usize, &str)> {
        let report = check(text, &Config::new(dialect));
        report
            .problems()
            .iter()
            .map(|p| (p.offset(), &text[p.offset()..p.end()]))
            .collect()
    }
    assert_eq!(
        spans(Json, r#"{"port": 1, "port": 2}"#),
        [(12, r#""port""#)]
    );
    assert_eq!(spans(Json, "[1, True]"), [(4, "True")]);
    assert_eq!(spans(Json, "[1 2]"), [(2, "")]);
    assert_eq!(spans(Json, "[1, @]"), [(4, "@")]);
    assert_eq!(spans(Json, "[1,"), [(3, "")]);
    assert_eq!(spans(Json, "[\"a\nb\"]"), [(3, "")]);
    assert_eq!(spans(Json, "\u{FEFF}[]"), [(0, "\u{FEFF}")]);
    assert_eq!(
        spans(IJson, "[1e999, 9007199254740993]"),
        [(1, "1e999"), (8, "9007199254740993")]
    );
    assert_eq!(
        spans(IJson, "[\"\\uD800\", \"\u{FFFF}\"]"),
        [(2, "\\uD800"), (12, "\u{FFFF}")]
    );
    assert_eq!(spans(Json5, "{a: 1, a: 2}"), [(7, "a")]);
    assert_eq!(spans(JsonLines, "1\n2"), [(3, "")]);
}

/// A JSON5 key ends on a later line than it starts when a line continuation or a raw U+2028 or
/// U+2029 is inside it, as `Problem::end` says.
#[test]
fn a_json5_key_with_a_line_end_inside_ends_on_a_later_line() {
    for key in ["a\\\nb", "a\u{2028}b", "a\u{2029}b"] {
        let text = format!("{{\"{key}\":1,\"{key}\":2}}");
        let report = check(&text, &Config::new(Dialect::Json5));
        let problem = &report.problems()[0];
        let (start, end) = (
            report.location(problem.offset()),
            report.location(problem.end()),
        );
        assert!(end.line > start.line, "{text:?}: {start:?} to {end:?}");
    }
}

/// JSON5 allows other control characters in a string, but a raw line break still ends it as an
/// error, short or long, in either quote.
#[test]
fn a_raw_line_break_in_a_json5_string_is_an_error() {
    for quote in ['"', '\''] {
        for line_break in ['\r', '\n'] {
            for before in [String::new(), "a\t".repeat(20)] {
                let text = format!("[{quote}{before}{line_break}{quote}]");
                let report = check(&text, &Config::new(Dialect::Json5));
                let messages: Vec<String> =
                    report.problems().iter().map(ToString::to_string).collect();
                assert_eq!(messages, ["line break inside a string"], "{text:?}");
                assert_eq!(report.problems()[0].offset(), 2 + before.len(), "{text:?}");
            }
        }
    }
}

/// Every byte the string scan stops at needs a match arm in the string loop: one without lands on
/// the loop's `debug_assert`. Each character goes in short and long strings, in every dialect.
#[test]
fn every_character_in_a_string_has_a_rule() {
    let extra = ['é', '\u{FFFE}', '\u{10FFFF}'];
    for c in (0..0x80u8).map(char::from).chain(extra) {
        for &dialect in Dialect::ALL {
            for quote in ['"', '\''] {
                for pad in [String::new(), "a".repeat(20)] {
                    check(
                        &format!("[{quote}{pad}{c}{pad}{quote}]"),
                        &Config::new(dialect),
                    );
                }
            }
        }
    }
}
