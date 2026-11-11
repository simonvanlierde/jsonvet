//! The formatter: reviewed snapshots of what it makes of each dialect and comment placement,
//! width boundaries, and properties over generated documents with comments.

#![allow(
    clippy::expect_used,
    reason = "a generated text that does not parse is a bug in the test"
)]

use std::fmt::Write as _;

use jsonvet::{Config, Dialect, FormatOptions, Indent, Rule, check, format, parse};
use proptest::prelude::*;

#[test]
fn formatting_preserves_hidden_problems() {
    let config = Config::new(Dialect::Jsonc);
    let text = "{\"a\":1,\n// jsonvet-ignore duplicate-key\n\"a\":2,\"b\":1,\"b\":2}\n";
    assert_eq!(check(text, &config).problems(), []);
    let error = format(text, &config, &FormatOptions::default()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "not formatted: formatting would change which problems an inline suppression hides"
    );
    let block = text.replace(
        "// jsonvet-ignore duplicate-key",
        "/* jsonvet-ignore duplicate-key */",
    );
    assert!(matches!(
        format(&block, &config, &FormatOptions::default()),
        Err(jsonvet::FormatError::SuppressionTargets)
    ));
    // This suppression hides nothing before or after, so the tokens it covers may change.
    let before_colon = "{\"a\":0,\"a\" // jsonvet-ignore duplicate-key\n:1,\n\"b\":0,\"c\":0}";
    assert!(format(before_colon, &config, &FormatOptions::default()).is_ok());
    let safe = "{\"a\":1,\n// jsonvet-ignore duplicate-key\n\"a\":2\n}";
    assert!(format(safe, &config, &FormatOptions::default()).is_ok());
    // Normalization changes atom spelling and adds commas, but keeps their identities.
    let json5 = Config::new(Dialect::Json5);
    let safe = "{a:1,\n// jsonvet-ignore duplicate-key\n'a':2\n}";
    assert!(format(safe, &json5, &normalized()).is_ok());
    let root = "// jsonvet-ignore invalid-number\n[1e999,1e999]";
    let numbers = json5
        .clone()
        .rule(Rule::InvalidNumber, jsonvet::Severity::Error);
    assert!(matches!(
        format(root, &numbers, &width(8)),
        Err(jsonvet::FormatError::SuppressionTargets)
    ));
    for name in ["syntax", "schema", "unknown-rule", "unused-suppression", ""] {
        let text = format!("{{\"a\":1,\n// jsonvet-ignore {name}\n\"a\":2,\"b\":1,\"b\":2}}\n");
        assert!(
            format(&text, &config, &FormatOptions::default()).is_ok(),
            "{name}"
        );
    }
}

fn fmt(dialect: Dialect, options: &FormatOptions, text: &str) -> String {
    match format(text, &Config::new(dialect), options) {
        Ok(out) => out,
        Err(jsonvet::FormatError::Syntax(report)) => {
            format!("syntax error: {}", report.problems()[0])
        }
        Err(other) => other.to_string(),
    }
}

fn width(width: usize) -> FormatOptions {
    FormatOptions {
        width,
        ..FormatOptions::default()
    }
}

fn normalized() -> FormatOptions {
    FormatOptions {
        normalize_json5: true,
        ..FormatOptions::default()
    }
}

/// Each case as its input and its output, so a snapshot shows the change.
fn show(dialect: Dialect, options: &FormatOptions, cases: &[&str]) -> String {
    let mut out = String::new();
    for text in cases {
        let formatted = fmt(dialect, options, text);
        let (text, shown) = (
            text.replace('\u{feff}', "<BOM>"),
            formatted.replace('\u{feff}', "<BOM>"),
        );
        let _ = write!(
            out,
            "{dialect} {options:?}\n--- in\n{text}\n--- out\n{shown}\n"
        );
        let _ = writeln!(out, "=== {formatted:?}\n");
    }
    out
}

#[test]
fn jsonc_comment_placement() {
    insta::assert_snapshot!(show(
        Dialect::Jsonc,
        &FormatOptions::default(),
        &[
            "// head\n\n\n// second\n{\"a\":1} // tail\n// last\n",
            "{ // after the brace\n\"a\": 1, // after the comma\n// own line\n\"b\": [1,2,3], /* block after */\n\"c\": 4 // last, no comma\n}",
            "{\"a\": /* after colon */ 2, \"b\" /* before colon */ : 3, \"c\": 4 /* before comma */, \"d\": 5}",
            "{\"a\": // line after colon\n 1}",
            "{ // only\n}",
            "[ /* only */ ]",
            "[\n// own line\n]",
            "[/* a */ 1, /* b */ 2, 3 /* c */]",
            "[1, /* between */ 2]",
            "[\n  1, 2, // two\n  3\n]",
            "{\"a\": {\"b\": [1 /* deep */]}, \"c\": [2]}",
            "{\n\n\n\"a\":1,\n\n\n\n// c\n\n\"b\":2\n\n\n}",
            "[1,\n/* a\n   b */\n2]",
            "{\"a\": [] // empty\n}",
            "[1, 2 /* end */]",
            "{\"a\": 1 /* x */ }",
        ]
    ));
}

#[test]
fn jsonc_whitespace_and_line_ends() {
    let tab = FormatOptions {
        indent: Indent::Tab,
        ..FormatOptions::default()
    };
    let four = FormatOptions {
        indent: Indent::Spaces(4),
        ..FormatOptions::default()
    };
    let mut out = show(
        Dialect::Jsonc,
        &tab,
        &["{\"a\": [1, 2], // c\n\"b\": {\"c\": 3}}"],
    );
    out += &show(
        Dialect::Jsonc,
        &four,
        &["{\"a\": [1, 2], // c\n\"b\": {\"c\": 3}}"],
    );
    out += &show(
        Dialect::Jsonc,
        &FormatOptions::default(),
        &[
            "{\r\n\"a\": 1, // c\r\n\"b\": [2]\r\n}",
            "\u{feff}[1]",
            "[1]\n\n\n",
            "[1]",
            "{}",
            "  \"x\"  ",
            "// head\n[1]",
        ],
    );
    insta::assert_snapshot!(out);
}

#[test]
fn jwcc_keeps_trailing_commas_only_where_lines_break() {
    insta::assert_snapshot!(show(
        Dialect::Jwcc,
        &FormatOptions::default(),
        &[
            "{\"a\": 1, \"b\": [1, 2,],}",
            "{\n\"a\": 1,\n// c\n\"b\": 2,\n}",
            "[\n  1,\n  2, // two\n]",
            "[1 /* x */,]",
        ]
    ));
}

#[test]
fn json5_is_kept_as_written() {
    insta::assert_snapshot!(show(
        Dialect::Json5,
        &FormatOptions::default(),
        &[
            "{a: 'single', \"b\": \"double\", 'c': 0x1F, d: +1, e: .5, f: 5., g: Infinity, h: -Infinity, i: NaN,}",
            "{a: [1, 2,], // c\n b: 'it\\'s',\n}",
            "{\\u0061: 1, 'a-b': 2}",
            "'it\\'s \"quoted\"'",
            "{\"line\": \"one \\\ntwo\"}",
            "[0xDEAD, +.5e-3, 1e+5, -0]",
        ]
    ));
}

#[test]
fn json5_normalized() {
    insta::assert_snapshot!(show(
        Dialect::Json5,
        &normalized(),
        &[
            "{a: 'single', \"b\": \"double\", 'c': 0x1F, d: +1, e: .5,}",
            "{'it\\'s': 'say \"hi\"', \"a-b\": 1, \"\": 2, \"1a\": 3, \"ok_$\": 4, \"\\u0061\": 5}",
            "{a: [1, 2,], b: {c: 1,},}",
            "{a: 1, // c\n b: 2}",
            "[\n1, 2]",
            "{\"aaaaaaaaaaaaaaaaaaa\": [1111111111, 2222222222, 3333333333], \"bbbbbbbbbbbbbbbbbbbbbbbbb\": 1}",
            "[]",
            "{ // only\n}",
        ]
    ));
    // The setting means nothing outside JSON5.
    insta::assert_snapshot!(
        "json5_normalized_other_dialects",
        show(
            Dialect::Jsonc,
            &normalized(),
            &["{\"a\": 1, // c\n \"b\": 2}"]
        )
    );
}

#[test]
fn strict_dialects_use_the_plain_layout() {
    let long = "{\"name\": \"jsonvet\", \"keywords\": [\"json\", \"json5\", \"jsonc\", \"lint\", \"pre-commit\"], \"nested\": {\"a\": [1, 2, {\"b\": null}]}}";
    let mut out = show(
        Dialect::Json,
        &FormatOptions::default(),
        &[long, "[[], {}, [[]]]"],
    );
    out += &show(Dialect::IJson, &FormatOptions::default(), &[long]);
    out += &show(Dialect::Json, &width(40), &[long]);
    out += &show(
        Dialect::Json,
        &FormatOptions::default(),
        &["{\"a\": 1,}", "// c\n1"],
    );
    insta::assert_snapshot!(out);
}

#[test]
fn json_lines_are_compact_whatever_the_width() {
    insta::assert_snapshot!(show(
        Dialect::JsonLines,
        &width(10),
        &[
            "{\"a\":1,\"b\":[1,2,{\"c\":null}]}\n[1,2,3]\n\"x\"\n",
            "{\"a\":1}\n\n   \n{\"b\":2}",
            "  {  \"a\" :  1  }  \r\n[ ]\r\n",
            "",
            "{\"a\":\n1}\n",
        ]
    ));
}

#[test]
fn nested_layout_follows_the_width() {
    insta::assert_snapshot!(show(
        Dialect::Json,
        &width(30),
        &[
            "{\"short\": [1, 2, 3], \"longer\": [\"aaaaaaaaaa\", \"bbbbbbbbbb\", \"cccccccccc\"], \"obj\": {\"k\": \"v\"}}",
            "[[1, 2], [3, 4], [\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"]]",
        ]
    ));
}

#[test]
fn a_syntax_error_is_reported_not_formatted() {
    let err = format(
        "{\"a\": }",
        &Config::new(Dialect::Json),
        &FormatOptions::default(),
    );
    let Err(jsonvet::FormatError::Syntax(report)) = err else {
        panic!("a syntax error");
    };
    assert_eq!(report.problems()[0].rule(), Rule::Syntax);
    assert!(
        format(
            "[1,]",
            &Config::new(Dialect::Json),
            &FormatOptions::default()
        )
        .is_err()
    );
    assert!(
        format(
            "[1,]",
            &Config::new(Dialect::Jwcc),
            &FormatOptions::default()
        )
        .is_ok()
    );
}

/// A flat array whose line is exactly `columns` wide: `["xxx"]`.
fn array_of_width(columns: usize) -> String {
    format!("[\"{}\"]", "x".repeat(columns - 4))
}

#[test]
fn exactly_the_width_fits_and_one_more_breaks() {
    let json = Config::new(Dialect::Json);
    let options = FormatOptions::default();
    let at_80 = array_of_width(80);
    assert_eq!(
        format(&at_80, &json, &options).unwrap(),
        format!("{at_80}\n")
    );
    let at_81 = array_of_width(81);
    assert_eq!(
        format(&at_81, &json, &options).unwrap(),
        format!("[\n  \"{}\"\n]\n", "x".repeat(77))
    );
    // Indentation counts: `  "k": ["x…"],` is 12 columns plus the string.
    let nested = |n: usize| format!("{{\"a\": 1, \"k\": [\"{}\"], \"z\": 2}}", "x".repeat(n));
    let fits = format(&nested(68), &json, &options).unwrap();
    assert!(fits.contains(&format!("  \"k\": [\"{}\"],\n", "x".repeat(68))));
    assert_eq!(fits.lines().map(str::len).max(), Some(80));
    let breaks = format(&nested(69), &json, &options).unwrap();
    assert!(breaks.contains(&format!("  \"k\": [\n    \"{}\"\n  ],\n", "x".repeat(69))));
    // The last member has no comma after it, so the same width fits one column more.
    let last = |n: usize| format!("{{\"a\": 1, \"k\": [\"{}\"]}}", "x".repeat(n));
    let fits = format(&last(69), &json, &options).unwrap();
    assert!(fits.contains(&format!("  \"k\": [\"{}\"]\n", "x".repeat(69))));
    // The whole document on one line when it fits.
    assert_eq!(
        format("{\"a\": [1, 2]}", &json, &width(15)).unwrap(),
        "{ \"a\": [1, 2] }\n"
    );
    assert_eq!(
        format("{\"a\": [1, 2], \"b\": 1}", &json, &width(20)).unwrap(),
        "{\n  \"a\": [1, 2],\n  \"b\": 1\n}\n"
    );
}

// ---- properties ----

/// A tiny deterministic stream, so a failing case is a pair of numbers.
struct Rng(u64);

impl Rng {
    fn next(&mut self, below: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        usize::try_from(self.0 >> 33).unwrap_or(0) % below
    }
}

#[derive(Clone, Debug)]
enum Tree {
    Atom(String),
    Array(Vec<Tree>),
    Object(Vec<(String, Tree)>),
}

/// The number atoms of a dialect: the ones `fill-numbers` packs.
fn number_atoms(dialect: Dialect) -> Vec<String> {
    let mut atoms = vec!["1".to_owned(), "-0.5e3".to_owned(), "123456".to_owned()];
    if dialect == Dialect::Json5 {
        atoms.extend(["0x1F", ".5", "+1", "Infinity", "NaN", "-Infinity"].map(String::from));
    }
    atoms
}

/// A number-only array of up to 40 members, alone or in an object or array.
fn number_array(dialect: Dialect) -> impl Strategy<Value = Tree> {
    let numbers = prop::collection::vec(
        prop::sample::select(number_atoms(dialect)).prop_map(Tree::Atom),
        0..40,
    )
    .prop_map(Tree::Array);
    prop_oneof![
        numbers.clone().prop_map(|a| Tree::Array(vec![a])),
        numbers.prop_map(|a| Tree::Object(vec![("\"k0\"".to_owned(), a)])),
    ]
}

fn tree(dialect: Dialect) -> impl Strategy<Value = Tree> {
    let json5 = dialect == Dialect::Json5;
    let mut atoms = vec![
        "1".to_owned(),
        "-0.5e3".to_owned(),
        "true".to_owned(),
        "null".to_owned(),
        r#""a\"b""#.to_owned(),
        "\"é\"".to_owned(),
        "\"\\u00e9\"".to_owned(),
        "\"a string long enough to push a line past a small width\"".to_owned(),
    ];
    let mut keys = vec!["\"k0\"", "\"k1\"", "\"k2\"", "\"a long key\""];
    if json5 {
        atoms.extend(
            [
                "'single'", "0x1F", ".5", "+1", "Infinity", "NaN", "'it\\'s'",
            ]
            .map(String::from),
        );
        keys.extend(["a", "'b'", "$c", "\"d-e\""]);
    }
    let atom = prop::sample::select(atoms).prop_map(Tree::Atom);
    let key = prop::sample::select(keys).prop_map(str::to_owned);
    atom.prop_recursive(4, 40, 5, move |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..5).prop_map(Tree::Array),
            prop::collection::vec((key.clone(), inner), 0..5).prop_map(Tree::Object),
        ]
    })
}

/// Text for the tree with a random gap at every place a token could be separated from the next:
/// white space, blank lines, and in the dialects that have them, comments with a number each.
fn render(tree: &Tree, dialect: Dialect, seed: u64, trailing: bool, plain: bool) -> String {
    struct Out<'d> {
        text: String,
        rng: Rng,
        dialect: Dialect,
        trailing: bool,
        plain: bool,
        marker: &'d std::cell::Cell<usize>,
    }
    impl Out<'_> {
        fn gap(&mut self) {
            let comments = self.dialect.comments_allowed() && !self.plain;
            let choice = if self.plain {
                2 + self.rng.next(2)
            } else {
                self.rng.next(if comments { 16 } else { 5 })
            };
            let n = self.marker.get();
            self.marker.set(n + 1);
            let piece = match choice {
                0..=1 => String::new(),
                2 => " ".to_owned(),
                3 => "\n".to_owned(),
                4 => "\n\n\n  ".to_owned(),
                5 => format!(" /*#{n}#*/"),
                6 => format!("/*#{n}#*/ "),
                7 => format!(" // #{n}#\n"),
                8 => format!("\n// #{n}#\n"),
                9 => format!("\n\n/* #{n}# */\n"),
                10 => format!(" /* #{n}#\n   more */ "),
                11 => format!("  // #{n}#   \n\n"),
                12 => format!("\n/*#{n}#*/ "),
                _ => format!(" /*#{n}#*/ \n"),
            };
            self.text.push_str(&piece);
        }
        fn value(&mut self, t: &Tree) {
            match t {
                Tree::Atom(a) => self.text.push_str(a),
                Tree::Array(items) => {
                    self.text.push('[');
                    self.gap();
                    for (i, item) in items.iter().enumerate() {
                        self.value(item);
                        self.gap();
                        if i + 1 < items.len() || (self.trailing && self.rng.next(2) == 0) {
                            self.text.push(',');
                            self.gap();
                        }
                    }
                    self.text.push(']');
                }
                Tree::Object(members) => {
                    self.text.push('{');
                    self.gap();
                    for (i, (key, item)) in members.iter().enumerate() {
                        self.text.push_str(key);
                        self.gap();
                        self.text.push(':');
                        self.gap();
                        self.value(item);
                        self.gap();
                        if i + 1 < members.len() || (self.trailing && self.rng.next(2) == 0) {
                            self.text.push(',');
                            self.gap();
                        }
                    }
                    self.text.push('}');
                }
            }
        }
    }
    let marker = std::cell::Cell::new(0);
    let mut out = Out {
        text: String::new(),
        rng: Rng(seed),
        dialect,
        trailing,
        plain,
        marker: &marker,
    };
    out.gap();
    out.value(tree);
    out.gap();
    out.text
}

trait CommentsAllowed {
    fn comments_allowed(self) -> bool;
}

impl CommentsAllowed for Dialect {
    fn comments_allowed(self) -> bool {
        matches!(self, Dialect::Jsonc | Dialect::Jwcc | Dialect::Json5)
    }
}

/// What a check finds, apart from what layout fixes.
fn problems(text: &str, config: &Config) -> Vec<String> {
    let mut found: Vec<String> = check(text, config)
        .problems()
        .iter()
        .filter(|p| p.rule() != Rule::MissingFinalNewline)
        .map(|p| format!("{}: {p}", p.rule()))
        .collect();
    found.sort();
    found
}

fn events(text: &str, config: &Config) -> Vec<String> {
    let (_, events) = parse(text, config);
    events
        .expect("valid")
        .into_iter()
        .map(|e| format!("{:?}", e.kind))
        .collect()
}

fn options(width: usize, indent: usize, normalize: bool, pad: bool, fill: bool) -> FormatOptions {
    FormatOptions {
        indent: if indent == 0 {
            Indent::Tab
        } else {
            Indent::Spaces(indent)
        },
        width,
        normalize_json5: normalize,
        bracket_spacing: pad,
        fill_numbers: fill,
    }
}

/// Everything the formatter promises about the output for one document.
fn check_promises(text: &str, dialect: Dialect, options: &FormatOptions) -> Result<(), String> {
    let config = Config::new(dialect);
    let once = format(text, &config, options)
        .map_err(|_| format!("generated text is invalid: {text:?}"))?;
    let twice = format(&once, &config, options)
        .map_err(|_| format!("output is invalid\nin: {text:?}\nout: {once:?}"))?;
    if once != twice {
        return Err(format!(
            "not idempotent\nin: {text:?}\n1: {once:?}\n2: {twice:?}"
        ));
    }
    if !once.ends_with('\n') || once.ends_with("\n\n") {
        return Err(format!("not exactly one final newline: {once:?}"));
    }
    if problems(text, &config) != problems(&once, &config) {
        return Err(format!("problems changed\nin: {text:?}\nout: {once:?}"));
    }
    if events(text, &config) != events(&once, &config) {
        return Err(format!("values changed\nin: {text:?}\nout: {once:?}"));
    }
    for n in 0..text.matches('#').count() / 2 {
        let marker = format!("#{n}#");
        if once.matches(&marker).count() != text.matches(&marker).count() {
            return Err(format!(
                "comment {marker} lost or copied\nin: {text:?}\nout: {once:?}"
            ));
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn formatting_twice_changes_nothing_and_loses_nothing(
        (dialect, t, seed, trailing, width, indent, normalize, pad, fill) in prop::sample::select(vec![
            Dialect::Json, Dialect::IJson, Dialect::Jsonc, Dialect::Jwcc, Dialect::Json5,
        ])
        .prop_flat_map(|d| (Just(d), tree(d), any::<u64>(), any::<bool>(), prop::sample::select(vec![1, 12, 30, 80]), 0usize..5, any::<bool>(), any::<bool>(), any::<bool>()))
    ) {
        let trailing = trailing && matches!(dialect, Dialect::Jwcc | Dialect::Json5);
        let text = render(&t, dialect, seed, trailing, false);
        if let Err(message) = check_promises(&text, dialect, &options(width, indent, normalize, pad, fill)) {
            prop_assert!(false, "{}", message);
        }
    }

    /// Number-only arrays that really fill: widths near the numbers' size, plain gaps.
    #[test]
    fn filled_number_arrays_are_stable_and_lose_nothing(
        (dialect, t, seed, trailing, width, indent, normalize, pad) in prop::sample::select(vec![
            Dialect::Json, Dialect::IJson, Dialect::Jsonc, Dialect::Jwcc, Dialect::Json5,
        ])
        .prop_flat_map(|d| (Just(d), number_array(d), any::<u64>(), any::<bool>(), 8usize..40, 0usize..5, any::<bool>(), any::<bool>()))
    ) {
        let trailing = trailing && matches!(dialect, Dialect::Jwcc | Dialect::Json5);
        let text = render(&t, dialect, seed, trailing, true);
        let options = options(width, indent, normalize, pad, true);
        if let Err(message) = check_promises(&text, dialect, &options) {
            prop_assert!(false, "{}", message);
        }
        // A row of several numbers never runs past the width, a tab counting four columns.
        let out = format(&text, &Config::new(dialect), &options).unwrap();
        for line in out.lines().filter(|l| !l.contains(['[', '{', ':', ']', '}'])) {
            let cols = line.chars().map(|c| if c == '\t' { 4 } else { 1 }).sum::<usize>();
            prop_assert!(!line.trim().contains(' ') || cols <= width, "{} wide: {:?}", cols, out);
        }
    }

    #[test]
    fn strict_json_keeps_its_value(
        (t, seed, width) in (tree(Dialect::Json), any::<u64>(), prop::sample::select(vec![1, 20, 80]))
    ) {
        let text = render(&t, Dialect::Json, seed, false, false);
        let out = format(&text, &Config::new(Dialect::Json), &self::width(width)).unwrap();
        let before: serde_json::Value = serde_json::from_str(&text).unwrap();
        let after: serde_json::Value = serde_json::from_str(&out).unwrap();
        prop_assert_eq!(before, after);
    }

    #[test]
    fn json_lines_stay_one_record_a_line(
        records in prop::collection::vec((tree(Dialect::Json), any::<u64>()), 0..5),
        crlf in any::<bool>(),
    ) {
        let eol = if crlf { "\r\n" } else { "\n" };
        let text: String = records
            .iter()
            .map(|(t, seed)| render(t, Dialect::Json, *seed, false, false).replace(['\n', '\r'], " ") + eol)
            .collect();
        let config = Config::new(Dialect::JsonLines);
        let once = format(&text, &config, &FormatOptions::default()).unwrap();
        prop_assert_eq!(once.lines().count(), records.len());
        prop_assert_eq!(&format(&once, &config, &FormatOptions::default()).unwrap(), &once);
        prop_assert_eq!(problems(&text, &config), problems(&once, &config));
    }
}

#[test]
fn a_lone_cr_is_the_line_ending_when_it_is_the_first() {
    let jsonc = Config::new(Dialect::Jsonc);
    let out = format("{\r// c\r\"a\": 1\r}\r", &jsonc, &FormatOptions::default()).unwrap();
    assert_eq!(out, "{\r  // c\r  \"a\": 1\r}\r");
    let out = format("{\r\n// c\n\"a\": 1}", &jsonc, &FormatOptions::default()).unwrap();
    assert_eq!(out, "{\r\n  // c\r\n  \"a\": 1\r\n}\r\n");
}

#[test]
fn a_lone_cr_never_ends_a_jwcc_line_comment_in_the_output() {
    let jwcc = Config::new(Dialect::Jwcc);
    for text in ["{\r\"a\": 1 // c\n}", "{\r\"a\": 1, // c\n\"b\": 2}"] {
        let out = format(text, &jwcc, &FormatOptions::default()).unwrap();
        assert!(!out.contains('\r'), "{out:?}");
        // The input mixes endings and the output does not; nothing else changes.
        let mut before = problems(text, &jwcc);
        before.retain(|p| !p.starts_with("mixed-line-endings"));
        assert_eq!(problems(&out, &jwcc), before, "{out:?}");
    }
}

#[test]
fn a_multiline_comment_does_not_change_the_width_of_the_value_before_it() {
    let options = width(26);
    let once = fmt(Dialect::Jsonc, &options, "{\"a\":[1,2] /* x\n z */}");
    assert_eq!(once, "{\n  \"a\": [1, 2] /* x\n    z */\n}\n");
    assert_eq!(fmt(Dialect::Jsonc, &options, &once), once);
}

#[test]
fn trailing_comment_width_counts_characters_and_tabs() {
    for (comment, columns) in [("/*x*/", 5), ("/*é*/", 5), ("/*\t*/", 8)] {
        let text = format!("{{\"a\":[1,2] {comment}}}");
        let fits = width(7 + 6 + 1 + columns);
        let out = fmt(Dialect::Jsonc, &fits, &text);
        assert!(out.contains("\"a\": [1, 2]"), "{out:?}");
        let breaks = width(fits.width - 1);
        let out = fmt(Dialect::Jsonc, &breaks, &text);
        assert!(out.contains("\"a\": [\n"), "{out:?}");
    }
}

#[test]
fn braces_are_padded_by_default_and_the_padding_counts_toward_the_width() {
    let json = Config::new(Dialect::Json);
    let one = |n: usize| format!("{{\"k\": \"{}\"}}", "x".repeat(n));
    let out = format(&one(69), &json, &FormatOptions::default()).unwrap();
    assert_eq!(out.len(), 81, "80 columns and the newline: {out}");
    assert!(out.starts_with("{ \"k\": "), "{out}");
    let out = format(&one(70), &json, &FormatOptions::default()).unwrap();
    assert!(out.starts_with("{\n  \"k\": "), "{out}");
    let bare = FormatOptions {
        bracket_spacing: false,
        ..FormatOptions::default()
    };
    let out = format(&one(70), &json, &bare).unwrap();
    assert!(out.starts_with("{\"k\": "), "{out}");
    assert_eq!(
        format("[{}, {\"a\": [{}]}]", &json, &FormatOptions::default()).unwrap(),
        "[{}, { \"a\": [{}] }]\n"
    );
}

fn filled(width: usize) -> FormatOptions {
    FormatOptions {
        width,
        fill_numbers: true,
        ..FormatOptions::default()
    }
}

/// Formats with `fill-numbers` on, and checks that a second run changes nothing.
fn fill(dialect: Dialect, width: usize, text: &str) -> String {
    let config = Config::new(dialect);
    let once = format(text, &config, &filled(width)).expect("formatted");
    assert_eq!(
        format(&once, &config, &filled(width)).expect("formatted twice"),
        once
    );
    once
}

#[test]
fn number_arrays_fill_the_width_when_asked() {
    let json = Dialect::Json;
    let seven = "[1,2,3,4,5,6,7]";
    // The width counts the indent and the comma: "  4, 5, 6, 7" is 12 columns.
    assert_eq!(fill(json, 12, seven), "[\n  1, 2, 3,\n  4, 5, 6, 7\n]\n");
    assert_eq!(fill(json, 11, seven), "[\n  1, 2, 3,\n  4, 5, 6,\n  7\n]\n");
    assert_eq!(fill(json, 10, seven), "[\n  1, 2, 3,\n  4, 5, 6,\n  7\n]\n");
    assert_eq!(fill(json, 9, seven), "[\n  1, 2,\n  3, 4,\n  5, 6, 7\n]\n");
    assert_eq!(fill(json, 80, seven), "[1, 2, 3, 4, 5, 6, 7]\n");
    let off = format(seven, &Config::new(json), &width(12)).unwrap();
    assert_eq!(off, "[\n  1,\n  2,\n  3,\n  4,\n  5,\n  6,\n  7\n]\n");
    // A number wider than the width still gets a line of its own.
    assert_eq!(
        fill(json, 6, "[123456789, 1, 2]"),
        "[\n  123456789,\n  1, 2\n]\n"
    );
    // Each inner array of a matrix fills; the outer one keeps a row per line.
    assert_eq!(
        fill(json, 14, "[[1,2,3,4,5,6,7],[8,9]]"),
        "[\n  [\n    1, 2, 3,\n    4, 5, 6, 7\n  ],\n  [8, 9]\n]\n"
    );
    assert_eq!(
        fill(json, 12, "{\"a\": [1,2,3,4,5,6,7]}"),
        "{\n  \"a\": [\n    1, 2, 3,\n    4, 5, 6,\n    7\n  ]\n}\n"
    );
    assert_eq!(
        fill(
            Dialect::Json5,
            25,
            "[+1, .5, 0x1F, Infinity, -NaN, 1e3, -2]"
        ),
        "[\n  +1, .5, 0x1F, Infinity,\n  -NaN, 1e3, -2\n]\n"
    );
    // JWCC keeps the trailing comma, and it counts toward the width.
    assert_eq!(
        fill(Dialect::Jwcc, 12, "[1,2,3,4,5,6,7,]"),
        "[\n  1, 2, 3,\n  4, 5, 6,\n  7,\n]\n"
    );
    assert_eq!(
        fill(json, 12, "[1,2,3,4,5,6,7]\r\n"),
        "[\r\n  1, 2, 3,\r\n  4, 5, 6, 7\r\n]\r\n"
    );
    // A tab counts as four columns, as for a broken container: "\t4, 5, 6, 7" is 14, so 7 fits at
    // 14 and wraps at 13. At 12 the first line "\t1, 2, 3," is exactly full.
    let tab = |width| FormatOptions {
        indent: Indent::Tab,
        ..filled(width)
    };
    let tabbed = |width| format(seven, &Config::new(json), &tab(width)).unwrap();
    assert_eq!(tabbed(14), "[\n\t1, 2, 3,\n\t4, 5, 6, 7\n]\n");
    assert_eq!(tabbed(13), "[\n\t1, 2, 3,\n\t4, 5, 6,\n\t7\n]\n");
    assert_eq!(tabbed(12), "[\n\t1, 2, 3,\n\t4, 5, 6,\n\t7\n]\n");
    assert_eq!(tabbed(11), "[\n\t1, 2,\n\t3, 4,\n\t5, 6, 7\n]\n");
    // A blank line before the first number or before `]` goes, so it does not stop the fill.
    assert_eq!(fill(json, 12, "[\n\n1,2,3,4,5,6,7]"), fill(json, 12, seven));
    assert_eq!(fill(json, 12, "[1,2,3,4,5,6,7\n\n]"), fill(json, 12, seven));
    assert_eq!(
        fill(Dialect::Jwcc, 12, "[1,2,3,4,5,6,7,\n\n]"),
        fill(Dialect::Jwcc, 12, "[1,2,3,4,5,6,7,]")
    );
}

/// The example in docs/format.md, typed out so the page cannot drift from the formatter.
#[test]
fn the_fill_numbers_example_in_the_docs_is_what_the_formatter_writes() {
    let input = r#"{"matrix": [[0.12, 0.98, 0.33, 0.47, 0.51, 0.06, 0.74, 0.29, 0.88, 0.15, 0.62, 0.41, 0.93, 0.37], [0, 1]]}"#;
    let doc = r#"{
  "matrix": [
    [
      0.12, 0.98, 0.33, 0.47, 0.51, 0.06, 0.74, 0.29, 0.88, 0.15, 0.62, 0.41,
      0.93, 0.37
    ],
    [0, 1]
  ]
}
"#;
    assert_eq!(fill(Dialect::Json, 80, input), doc);
}

#[test]
fn number_arrays_with_anything_else_in_them_keep_a_number_per_line() {
    for (dialect, text) in [
        (Dialect::Json, "[1,2,3,\n\n4,5,6,7]"),
        (Dialect::Json, "[1,2,3,\"4\",5,6,7]"),
        (Dialect::Json, "[1,2,3,true,5,6,7]"),
        (Dialect::Json, "[1,2,3,false,5,6,7]"),
        (Dialect::Json, "[1,2,3,null,5,6,7]"),
        (Dialect::Json, "[1,2,3,[4],5,6,7]"),
        (Dialect::Json5, "[1,2,3,'4',5,6,7]"),
        (Dialect::Jsonc, "[1,2,3, // c\n4,5,6,7]"),
        (Dialect::Jsonc, "[ // c\n1,2,3,4,5,6,7]"),
        (Dialect::Jsonc, "[1,2,3,4,5,6,7\n// c\n]"),
        (Dialect::Jsonc, "[1,2,3 // c\n]"),
        (Dialect::Json5, "[1,2,3,\n// c\n4,5,6,7]"),
        (Dialect::Json5, "[1,2,3, /* c */ 4,5,6,7]"),
        (Dialect::Jsonc, "[1,2,3,/* c */4,5,6,7]"),
        (Dialect::Json, "{\"a\":1,\"b\":2,\"c\":3,\"d\":4}"),
    ] {
        let off = format(text, &Config::new(dialect), &width(12)).unwrap();
        assert_eq!(fill(dialect, 12, text), off, "{text}");
    }
}

#[test]
fn an_output_over_sixteen_times_the_text_is_refused_with_its_size() {
    let depth = 9_000;
    let text = format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
    // 9,000 levels need more stack than a test thread has; the command runs on 64 MB.
    let bytes = std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || {
            let json = Config::new(Dialect::Json);
            match format(&text, &json, &FormatOptions::default()) {
                Err(jsonvet::FormatError::OutputTooLarge { bytes }) => bytes,
                other => panic!("too large: {other:?}"),
            }
        })
        .unwrap()
        .join()
        .unwrap();
    // Two lines for each level, each indented by twice its depth: about 2 x 9,000 x 9,000.
    assert!((150_000_000..170_000_000).contains(&bytes), "{bytes}");
    assert!(
        jsonvet::FormatError::OutputTooLarge { bytes }
            .to_string()
            .starts_with("not formatted: the output would be 16")
    );
}

// ---- the author's line break after an object's brace ----

fn out(dialect: Dialect, options: &FormatOptions, text: &str) -> String {
    format(text, &Config::new(dialect), options).expect("formats")
}

#[test]
fn an_object_with_a_line_break_before_its_first_key_stays_expanded() {
    let d = FormatOptions::default();
    let json = |text: &str| out(Dialect::Json, &d, text);
    assert_eq!(json("{\n\"a\": 1}"), "{\n  \"a\": 1\n}\n");
    assert_eq!(json("{\n\n\"a\": 1}"), "{\n  \"a\": 1\n}\n");
    assert_eq!(json("{\r\n\"a\": 1}"), "{\r\n  \"a\": 1\r\n}\r\n");
    // No break before the first key: the width rule decides, whatever else breaks.
    assert_eq!(json("{\"a\":\n1,\n\"b\": 2}"), "{ \"a\": 1, \"b\": 2 }\n");
    assert_eq!(json("{\"a\": 1}"), "{ \"a\": 1 }\n");
    // An empty object stays `{}`.
    assert_eq!(json("{\n}"), "{}\n");
    // The break sits on the object that has it; arrays keep the width rule.
    assert_eq!(json("[\n1, 2]"), "[1, 2]\n");
    assert_eq!(json("[\n{\"a\": 1}]"), "[{ \"a\": 1 }]\n");
    assert_eq!(
        json("{\"a\": 1,\n\"b\": {\"c\": 2}}"),
        "{ \"a\": 1, \"b\": { \"c\": 2 } }\n"
    );
    // An expanded object breaks the containers around it, and not the ones inside it.
    assert_eq!(json("[{\n\"a\": [1]}]"), "[\n  {\n    \"a\": [1]\n  }\n]\n");
    assert_eq!(
        json("{\"a\": {\n\"b\": 1}, \"c\": 2}"),
        "{\n  \"a\": {\n    \"b\": 1\n  },\n  \"c\": 2\n}\n"
    );
    assert_eq!(
        json("{\n\"a\": {\"b\": 1}}"),
        "{\n  \"a\": { \"b\": 1 }\n}\n"
    );
    let bare = FormatOptions {
        bracket_spacing: false,
        ..FormatOptions::default()
    };
    assert_eq!(
        out(Dialect::Json, &bare, "{\n\"a\": 1}"),
        "{\n  \"a\": 1\n}\n"
    );
}

#[test]
fn the_line_break_rule_holds_in_every_dialect_but_json_lines() {
    let d = FormatOptions::default();
    for dialect in [
        Dialect::IJson,
        Dialect::Jsonc,
        Dialect::Jwcc,
        Dialect::Json5,
    ] {
        assert_eq!(
            out(dialect, &d, "{\n\"a\": 1}"),
            "{\n  \"a\": 1\n}\n",
            "{dialect}"
        );
        assert_eq!(
            out(dialect, &d, "{\"a\":\n1}"),
            "{ \"a\": 1 }\n",
            "{dialect}"
        );
    }
    assert_eq!(
        out(Dialect::Jwcc, &d, "{\n\"a\": 1,}"),
        "{\n  \"a\": 1,\n}\n"
    );
    assert_eq!(out(Dialect::Json5, &d, "{\n a: 1}"), "{\n  a: 1\n}\n");
    assert_eq!(out(Dialect::Json5, &d, "{\u{2028}a: 1}"), "{\n  a: 1\n}\n");
    assert_eq!(
        out(Dialect::Json5, &normalized(), "{\n'a': 1}"),
        "{\n  a: 1,\n}\n"
    );
    assert_eq!(
        out(Dialect::Jsonc, &d, "{\r\"a\": 1}"),
        "{\r  \"a\": 1\r}\r"
    );
    // JSON Lines: every record is compact, and a break cannot be inside one.
    assert_eq!(out(Dialect::JsonLines, &d, "{ \"a\": 1}\n"), "{\"a\": 1}\n");
}

#[test]
fn an_expanded_object_formats_the_same_the_second_time() {
    for dialect in [Dialect::Json, Dialect::Jsonc, Dialect::Json5] {
        for text in [
            "{\n\"a\": [1, {\n\"b\": 2}]}",
            "[{\n}, {\"a\":\n1}]",
            "{\n\"a\": {\n\"b\": {\n}}}",
        ] {
            check_promises(text, dialect, &FormatOptions::default()).unwrap();
            check_promises(text, dialect, &width(1)).unwrap();
        }
    }
}

// ---- block comments move with their start ----

#[test]
fn a_block_comment_keeps_its_lines_in_step_with_its_start() {
    let d = FormatOptions::default();
    let c = |text: &str| out(Dialect::Jsonc, &d, text);
    // Moved right by two columns.
    assert_eq!(
        c("[1,\n/* a\n   b */\n2]"),
        "[\n  1,\n  /* a\n     b */\n  2\n]\n"
    );
    // A line with no indent gets the whole move.
    assert_eq!(
        c("[1,\n/* a\nb */\n2]"),
        "[\n  1,\n  /* a\n  b */\n  2\n]\n"
    );
    // Moved left by two: the lines give up two spaces.
    assert_eq!(
        c("{\n    /* a\n     * b\n     */\n\"x\": 1}"),
        "{\n  /* a\n   * b\n   */\n  \"x\": 1\n}\n"
    );
    // A line with exactly the whitespace to give is shifted; one with a column less stays.
    assert_eq!(
        c("{\n    /* a\n  b\n */\n\"x\": 1}"),
        "{\n  /* a\nb\n */\n  \"x\": 1\n}\n"
    );
    // A blank line or a line of only spaces is not touched.
    assert_eq!(
        c("[1,\n/* a\n\n  \n b */\n2]"),
        "[\n  1,\n  /* a\n\n  \n   b */\n  2\n]\n"
    );
    // Not at the start of its line: the column is where the comment began.
    assert_eq!(
        c("{\n\"a\":/* x\n   y */ 1}"),
        "{\n  \"a\": /* x\n      y */ 1\n}\n"
    );
    // Before the document.
    assert_eq!(c("  /* a\n   b */\n{}"), "/* a\n b */\n{}\n");
    assert_eq!(c("\u{feff}/* a\n b */ [1]"), "\u{feff}/* a\n b */ [1]\n");
    // Line comments and one-line block comments are left alone.
    assert_eq!(
        c("[1,\n// a\n   // b\n/* c */\n2]"),
        "[\n  1,\n  // a\n  // b\n  /* c */\n  2\n]\n"
    );
    // The file's line ending is kept inside the comment, and a shift keeps it too.
    assert_eq!(
        c("[1,\r\n/* a\r\n   b */\r\n2]"),
        "[\r\n  1,\r\n  /* a\r\n     b */\r\n  2\r\n]\r\n"
    );
    assert_eq!(
        c("[1,\r/* a\r   b */\r2]"),
        "[\r  1,\r  /* a\r     b */\r  2\r]\r"
    );
}

#[test]
fn a_block_comment_shifts_only_by_whole_indent_characters() {
    let tab = FormatOptions {
        indent: Indent::Tab,
        ..FormatOptions::default()
    };
    let c = |options: &FormatOptions, text: &str| out(Dialect::Jsonc, options, text);
    // Right by four columns: one tab goes before a line that starts with a tab.
    assert_eq!(
        c(&tab, "[1,\n/* a\n\t b */\n2]"),
        "[\n\t1,\n\t/* a\n\t\t b */\n\t2\n]\n"
    );
    // A line that starts with spaces stays, when the indent is a tab.
    assert_eq!(
        c(&tab, "[1,\n/* a\n   b */\n2]"),
        "[\n\t1,\n\t/* a\n   b */\n\t2\n]\n"
    );
    // Left by four columns: a tab goes.
    assert_eq!(
        c(&tab, "[\n\t\t/* a\n\t\t b */\n1]"),
        "[\n\t/* a\n\t b */\n\t1\n]\n"
    );
    // A shift that is not a whole number of tabs leaves the lines.
    assert_eq!(
        c(&tab, "{\n\"a\":  /* x\n\ty */ 1}"),
        "{\n\t\"a\": /* x\n\ty */ 1\n}\n"
    );
    // Spaces output: a line that starts with a tab stays.
    assert_eq!(
        c(&FormatOptions::default(), "[1,\n/* a\n\t b */\n2]"),
        "[\n  1,\n  /* a\n\t b */\n  2\n]\n"
    );
}

#[test]
fn shifted_block_comments_format_the_same_the_second_time() {
    let tab = FormatOptions {
        indent: Indent::Tab,
        ..FormatOptions::default()
    };
    for options in [FormatOptions::default(), tab, width(1)] {
        for dialect in [Dialect::Jsonc, Dialect::Jwcc, Dialect::Json5] {
            for text in [
                "[1,\n/* a\n   b */\n2]",
                "{\n      /* a\n       * b\n       */\n\"x\":/* c\n d */ [ /* e\n f */ 1]}",
                "{\"a\": /* x\n\ty */ 1, \"b\": {\n/* z\n  w */\n\"c\": 2}}",
                "  /* a\n   b */ {} /* t\n   u */",
                "\u{feff}[1,\n/* a\n   b */\n2]",
            ] {
                check_promises(text, dialect, &options).unwrap();
            }
        }
    }
}

#[test]
fn shifting_a_comment_counts_toward_the_output_size_limit() {
    // 3,000 levels of nesting hold a comment of 20,000 lines. Each line moves right by 3,000
    // columns, which is 60 MB the text does not show, on top of 18 MB for the nesting.
    let depth = 3_000;
    let comment = format!("/*{}*/", "\n x".repeat(20_000));
    let text = format!("{}{comment}1{}", "[".repeat(depth), "]".repeat(depth));
    let result = std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || {
            format(
                &text,
                &Config::new(Dialect::Jsonc),
                &FormatOptions::default(),
            )
            .map(|s| s.len())
            .map_err(|e| e.to_string())
        })
        .unwrap()
        .join()
        .unwrap();
    let err = result.expect_err("too large");
    assert!(
        err.starts_with("not formatted: the output would be 7"),
        "{err}"
    );
}
