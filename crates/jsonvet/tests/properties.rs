//! Properties that hold for every input in every dialect: no panic, every report keeping the
//! promises `common::report_violation` lists, a text that passes in a narrower dialect
//! passing in the wider ones, and `parse` agreeing with `check`.

mod common;

use std::fmt::Write as _;

use common::{nesting_violations, report_violation, strict};
use jsonvet::{Config, Dialect, EventKind, Rule, check, parse};
use proptest::prelude::*;

/// JSON-ish token soup, so most inputs get past the first byte, mixed with arbitrary text for
/// the rest.
fn soup() -> impl Strategy<Value = String> {
    let tokens = prop::sample::select(vec![
        // Whole values, so some inputs are valid in the narrower dialects.
        "[]",
        "[1]",
        "[1,]",
        "{\"a\":1}",
        "{\"a\":1,}",
        "{",
        "}",
        "[",
        "]",
        ":",
        ",",
        "\"a\"",
        "'b'",
        "c",
        "1",
        "-2.5e3",
        "0x1F",
        ".5",
        "NaN",
        "true",
        "null",
        "// x\n",
        "/* y */",
        "/*",
        "\"",
        "\\",
        "\\u0061",
        "\n",
        "\r\n",
        "\r",
        " ",
        "\t",
        "\u{2028}",
        "\u{A0}",
        "\u{FEFF}",
        "é",
        "😀",
        "\u{1}",
        "\"\\ud800\"",
    ]);
    // Runs deep enough to reach the depth limit and long enough to overflow a number's range.
    let runs = prop::sample::select(vec![
        "[".repeat(300),
        "{\"a\":".repeat(150),
        "[".repeat(12_000),
        "9".repeat(400),
        "1".repeat(20),
        format!("0.{}", "0".repeat(400)),
        format!("1e{}", "9".repeat(30)),
    ]);
    let token = prop_oneof![
        20 => tokens.prop_map(String::from),
        1 => runs,
    ];
    let soup = prop::collection::vec(token, 0..40).prop_map(|t| t.concat());
    prop_oneof![4 => soup, 1 => "\\PC{0,64}"]
}

/// A value as text, with the events it should parse into, written as short tokens.
type Piece = (String, Vec<String>);

/// What a dialect adds to JSON, for [`well_formed`].
#[derive(Clone, Copy)]
struct Syntax(Dialect);

impl Syntax {
    fn comments(self) -> bool {
        matches!(self.0, Dialect::Jsonc | Dialect::Jwcc | Dialect::Json5)
    }

    fn trailing_commas(self) -> bool {
        matches!(self.0, Dialect::Jwcc | Dialect::Json5)
    }

    fn json5(self) -> bool {
        self.0 == Dialect::Json5
    }

    /// JSON Lines: no line breaks inside a document.
    fn one_line(self) -> bool {
        self.0 == Dialect::JsonLines
    }
}

/// Space between tokens: whitespace, and comments where the dialect allows them.
fn gap(syntax: Syntax) -> BoxedStrategy<&'static str> {
    let mut gaps = vec!["", " "];
    if !syntax.one_line() {
        gaps.push("\n\t");
    }
    if syntax.comments() {
        gaps.extend(["/* c */", "// c\n"]);
    }
    prop::sample::select(gaps).boxed()
}

/// One value the dialect accepts: nested arrays and objects, escaped and repeated keys,
/// and JSON5's own numbers, strings and keys.
fn value(syntax: Syntax) -> BoxedStrategy<Piece> {
    let mut numbers = vec!["0", "-1", "2.5e3", "-0.5E-2", "18446744073709551616"];
    let mut strings = vec![
        ("\"a\"", "a"),
        ("\"\"", ""),
        ("\"\\n\\u00e9\\/\"", "\n\u{e9}/"),
        ("\"é\\\"\"", "é\""),
        ("\"\\ud83d\\ude00\"", "\u{1F600}"),
    ];
    if syntax.json5() {
        numbers.extend(["+1", "0x1F", "-0XaB", ".5", "5.", "Infinity", "-NaN"]);
        strings.extend([("'q\"'", "q\""), ("'\\x41'", "A")]);
    }
    let mut keys = strings.clone();
    // Repeated keys, one of them spelled with an escape.
    keys.extend([("\"a\"", "a"), ("\"\\u0061\"", "a")]);
    if syntax.json5() {
        keys.extend([("id", "id"), ("$a\\u0062", "$ab")]);
    }
    let leaf = prop_oneof![
        prop::sample::select(vec!["null", "true", "false"])
            .prop_map(|word| (word.to_owned(), vec![word.to_owned()])),
        prop::sample::select(numbers).prop_map(|n| (n.to_owned(), vec![format!("n:{n}")])),
        prop::sample::select(strings)
            .prop_map(|(text, s)| (text.to_owned(), vec![format!("s:{s}")])),
    ];
    leaf.prop_recursive(4, 32, 4, move |inner| {
        let array = (
            prop::collection::vec((gap(syntax), inner.clone()), 0..4),
            any::<bool>(),
        )
            .prop_map(move |(items, trailing)| {
                let mut piece = ("[".to_owned(), vec!["[".to_owned()]);
                for (i, (gap, (text, tokens))) in items.iter().enumerate() {
                    if i > 0 {
                        piece.0.push(',');
                    }
                    piece.0.push_str(gap);
                    piece.0.push_str(text);
                    piece.1.extend(tokens.iter().cloned());
                }
                close(
                    &mut piece,
                    "]",
                    syntax.trailing_commas() && trailing && !items.is_empty(),
                );
                piece
            });
        let object = (
            prop::collection::vec(
                (
                    gap(syntax),
                    prop::sample::select(keys.clone()),
                    gap(syntax),
                    inner,
                ),
                0..4,
            ),
            any::<bool>(),
        )
            .prop_map(move |(members, trailing)| {
                let mut piece = ("{".to_owned(), vec!["{".to_owned()]);
                for (i, (before, (key, decoded), after, (text, tokens))) in
                    members.iter().enumerate()
                {
                    if i > 0 {
                        piece.0.push(',');
                    }
                    let _ = write!(piece.0, "{before}{key}{after}:{after}{text}");
                    piece.1.push(format!("k:{decoded}"));
                    piece.1.extend(tokens.iter().cloned());
                }
                close(
                    &mut piece,
                    "}",
                    syntax.trailing_commas() && trailing && !members.is_empty(),
                );
                piece
            });
        prop_oneof![array, object]
    })
    .boxed()
}

fn close(piece: &mut Piece, bracket: &str, trailing_comma: bool) {
    if trailing_comma {
        piece.0.push(',');
    }
    piece.0.push_str(bracket);
    piece.1.push("end".to_owned());
}

/// A whole text the dialect accepts, with its events as tokens: one value, or one per line in
/// JSON Lines.
fn well_formed() -> impl Strategy<Value = (Dialect, String, Vec<String>)> {
    prop::sample::select(Dialect::ALL.to_vec()).prop_flat_map(|dialect| {
        let syntax = Syntax(dialect);
        let count = if syntax.one_line() { 1..4 } else { 1..2 };
        (
            prop::collection::vec((gap(syntax), value(syntax), gap(syntax)), count),
            Just(dialect),
        )
            .prop_map(|(documents, dialect)| {
                let mut text = Vec::new();
                let mut tokens = Vec::new();
                for (before, (value, events), after) in documents {
                    text.push(format!("{before}{value}{after}"));
                    tokens.extend(events);
                }
                (dialect, text.join("\n") + "\n", tokens)
            })
    })
}

/// An event as a [`Piece`] token.
fn token(kind: &EventKind<'_>) -> String {
    match kind {
        EventKind::ObjectStart => "{".to_owned(),
        EventKind::ArrayStart => "[".to_owned(),
        EventKind::End => "end".to_owned(),
        EventKind::Key(key) => format!("k:{}", jsonvet::key_text(key)),
        EventKind::Null => "null".to_owned(),
        EventKind::Bool(b) => b.to_string(),
        EventKind::Number(n) => format!("n:{n}"),
        EventKind::String(s) => format!("s:{s}"),
        _ => "?".to_owned(),
    }
}

proptest! {
    // 4000 cases by default; PROPTEST_CASES sets more for a long run.
    #![proptest_config(ProptestConfig::with_cases(
        std::env::var("PROPTEST_CASES").ok().and_then(|n| n.parse().ok()).unwrap_or(4000)
    ))]

    #[test]
    fn reports_keep_their_promises(text in soup(), dialect in prop::sample::select(Dialect::ALL.to_vec())) {
        for config in [Config::new(dialect), strict()] {
            prop_assert_eq!(report_violation(&text, &config), Ok(()));
        }
    }

    #[test]
    fn parse_reports_as_check_does_and_its_events_balance(text in soup(), dialect in prop::sample::select(Dialect::ALL.to_vec())) {
        let config = Config::new(dialect);
        let (report, events) = parse(&text, &config);
        let checked = check(&text, &config);
        prop_assert_eq!(report.problems(), checked.problems());
        let syntax_error = report.problems().iter().any(|p| p.rule() == Rule::Syntax);
        prop_assert_eq!(events.is_none(), syntax_error);
        let mut depth = 0usize;
        for event in events.unwrap_or_default() {
            prop_assert!(text.is_char_boundary(event.offset) && event.offset < text.len());
            match event.kind {
                EventKind::ObjectStart | EventKind::ArrayStart => depth += 1,
                EventKind::End => depth -= 1,
                _ => {}
            }
        }
        prop_assert_eq!(depth, 0);
    }

    #[test]
    fn well_formed_text_parses_into_the_events_it_was_built_from((dialect, text, expected) in well_formed()) {
        let config = Config::new(dialect);
        let (report, events) = parse(&text, &config);
        let checked = check(&text, &config);
        prop_assert_eq!(report.problems(), checked.problems());
        prop_assert!(events.is_some(), "a syntax error: {:?}", report.problems());
        let events = events.unwrap_or_default();
        let tokens: Vec<String> = events.iter().map(|event| token(&event.kind)).collect();
        prop_assert_eq!(tokens, expected);
        for event in &events {
            prop_assert!(text.is_char_boundary(event.offset) && event.offset < text.len());
        }
    }

    #[test]
    fn dialects_nest(text in soup()) {
        prop_assert_eq!(nesting_violations(&text), Vec::<String>::new());
    }
}
