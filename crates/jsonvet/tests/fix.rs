//! `jsonvet::fix`: what each fix does in each dialect, that fixing twice equals fixing once, and
//! that strict JSON keeps its value.

use jsonvet::{Config, Dialect, Fix, Rule, Severity, check, fix, report, try_fix, try_fix_at};
use proptest::prelude::*;

#[test]
fn each_fix_exposes_the_same_edits_as_selecting_it_alone() {
    for (dialect, text) in [
        (Dialect::Json, "\u{feff}[1,]\r\n"),
        (Dialect::JsonLines, "\u{feff}{\"a\":1}\r\n{\"a\":2}"),
        (Dialect::Json5, "{\"a\":[1,\r\n2],\n\"a\":3,\r\"b\":0\n}"),
        (
            Dialect::Json5,
            "{a:1,\n// jsonvet-ignore duplicate-key\na:2,\r\nb:0}\n",
        ),
    ] {
        let config = Config::new(dialect);
        for unsafe_fixes in [false, true] {
            let fixed = try_fix(text, &config, unsafe_fixes).expect("fixes");
            for fix in fixed.fixes {
                let expected = try_fix_at(text, &config, unsafe_fixes, Some(&[fix.offset]))
                    .expect("selected fix");
                let mut actual = text.to_owned();
                for (range, insert) in fix.edits.into_iter().rev() {
                    assert_ne!(&text[range.clone()], insert);
                    actual.replace_range(range, insert);
                }
                assert_eq!(actual, expected.text, "{dialect} {:?}", fix.rule);
            }
        }
    }
}

#[test]
fn selected_fix_offsets_allow_any_order_and_repeated_values() {
    let text = "[[1,],[2,],[3,]]";
    let config = Config::new(Dialect::Json);
    let all = try_fix(text, &config, false).expect("fixes");
    let mut offsets: Vec<_> = all.fixes.iter().rev().map(|fix| fix.offset).collect();
    offsets.extend(offsets.clone());
    offsets.push(usize::MAX);
    assert_eq!(
        try_fix_at(text, &config, false, Some(&offsets))
            .expect("selected fixes")
            .text,
        all.text
    );
    let first = all.fixes[0].offset;
    assert_eq!(
        try_fix_at(text, &config, false, Some(&[first, first, usize::MAX]))
            .expect("one fix")
            .fixes
            .len(),
        1
    );
}

fn safe(text: &str, dialect: Dialect) -> Option<String> {
    fix(text, &Config::new(dialect), false).map(|f| f.text)
}

fn all(text: &str, dialect: Dialect) -> Option<String> {
    fix(text, &Config::new(dialect), true).map(|f| f.text)
}

#[test]
fn removes_a_byte_order_mark_where_the_rule_is_on() {
    for dialect in [Dialect::Json, Dialect::IJson, Dialect::JsonLines] {
        assert_eq!(safe("\u{FEFF}[1]\n", dialect).as_deref(), Some("[1]\n"));
    }
    // Off by default in the comment dialects, so there is no problem to fix.
    for dialect in [Dialect::Jsonc, Dialect::Jwcc, Dialect::Json5] {
        assert_eq!(safe("\u{FEFF}[1]", dialect), None);
        let config = Config::new(dialect).rule(Rule::ByteOrderMark, Severity::Warning);
        assert_eq!(
            fix("\u{FEFF}[1]", &config, false)
                .map(|f| f.text)
                .as_deref(),
            Some("[1]")
        );
    }
}

#[test]
fn removes_trailing_commas_where_the_dialect_forbids_them() {
    for dialect in [Dialect::Json, Dialect::IJson, Dialect::Jsonc] {
        assert_eq!(
            safe("{\"a\": [1, 2,],}", dialect).as_deref(),
            Some("{\"a\": [1, 2]}")
        );
    }
    // JWCC and JSON5 allow them.
    for dialect in [Dialect::Jwcc, Dialect::Json5] {
        assert_eq!(safe("[1,]", dialect), None);
    }
    assert_eq!(
        safe("[1,]\n[2,]\n", Dialect::JsonLines).as_deref(),
        Some("[1]\n[2]\n")
    );
}

#[test]
fn a_trailing_comma_keeps_the_comments_and_layout_around_it() {
    let text = "{\n  \"a\": 1, // one\n  \"b\": [\n    2,\n    3, /* end */\n  ],\n}\n";
    assert_eq!(
        safe(text, Dialect::Jsonc).as_deref(),
        Some("{\n  \"a\": 1, // one\n  \"b\": [\n    2,\n    3 /* end */\n  ]\n}\n")
    );
}

#[test]
fn adds_the_final_newline_to_json_lines_only() {
    assert_eq!(
        safe("[1]\n[2]", Dialect::JsonLines).as_deref(),
        Some("[1]\n[2]\n")
    );
    assert_eq!(safe("[1]", Dialect::Json), None);
    // Nothing missing: nothing to do.
    assert_eq!(safe("[1]\n", Dialect::JsonLines), None);
}

#[test]
fn the_final_newline_follows_the_file_s_own_line_ending() {
    assert_eq!(
        safe("[1]\r\n[2]", Dialect::JsonLines).as_deref(),
        Some("[1]\r\n[2]\r\n")
    );
    // The first line ending decides; a lone CR is not a line ending in JSON Lines.
    assert_eq!(
        safe("[1]\n[2]\r\n[3]", Dialect::JsonLines).as_deref(),
        Some("[1]\n[2]\n[3]\n")
    );
}

#[test]
fn duplicate_keys_are_an_unsafe_fix() {
    let text = "{\"a\": 1, \"a\": 2}";
    assert_eq!(safe(text, Dialect::Json), None);
    assert_eq!(all(text, Dialect::Json).as_deref(), Some("{\"a\": 2}"));
    let fixed = fix(text, &Config::new(Dialect::Json), true).expect("fixed");
    assert_eq!(fixed.fixes.len(), 1);
    assert_eq!(fixed.fixes[0].rule, Rule::DuplicateKey);
    assert!(!fixed.fixes[0].safe);
}

#[test]
fn keeps_the_last_of_three_and_leaves_comments() {
    let text = "{\n  \"a\": 1, // first\n  \"b\": 0,\n  \"a\": 2 /* mid */,\n  \"a\": 3\n}\n";
    assert_eq!(
        all(text, Dialect::Jsonc).as_deref(),
        Some("{\n  // first\n  \"b\": 0,\n   /* mid */\"a\": 3\n}\n")
    );
}

#[test]
fn drops_a_duplicate_inside_a_dropped_member_without_clashing() {
    let text = "{\"a\": {\"x\": 1, \"x\": 2}, \"a\": {\"y\": 1}}";
    assert_eq!(
        all(text, Dialect::Json).as_deref(),
        Some("{\"a\": {\"y\": 1}}")
    );
}

#[test]
fn drops_duplicates_in_json5_and_in_each_json_lines_line() {
    assert_eq!(
        all("{a: 1, 'a': 2}", Dialect::Json5).as_deref(),
        Some("{'a': 2}")
    );
    assert_eq!(
        all("{\"a\":1,\"a\":2}\n{\"b\":1,\"b\":2}\n", Dialect::JsonLines).as_deref(),
        Some("{\"a\":2}\n{\"b\":2}\n")
    );
}

#[test]
fn applies_every_kind_in_one_pass() {
    let text = "\u{FEFF}{\"a\": [1,], \"a\": 2,}";
    let fixed = fix(text, &Config::new(Dialect::Json), true).expect("fixed");
    assert_eq!(fixed.text, "{\"a\": 2}");
    // The first `a` holds a trailing comma too, but it goes whole, so its fix is skipped.
    assert_eq!(fixed.fixes.len(), 3);
    assert!(check(&fixed.text, &Config::new(Dialect::Json)).passed());
}

#[test]
fn leaves_a_text_with_another_syntax_error_alone() {
    assert!(all("[1,]", Dialect::Json).is_some());
    assert_eq!(all("[1,] x", Dialect::Json), None);
    assert_eq!(all("\u{FEFF}[1 2]", Dialect::Json), None);
    assert_eq!(all("[1]\n\n[2]", Dialect::JsonLines), None);
}

#[test]
fn a_rule_that_is_off_is_not_fixed() {
    let config = Config::new(Dialect::Json).rule(Rule::DuplicateKey, Severity::Off);
    assert_eq!(fix("{\"a\": 1, \"a\": 2}", &config, true), None);
    let fixed = fix("{\"a\": 1, \"a\": 2,}", &config, true).expect("the comma is fixed");
    assert_eq!(fixed.text, "{\"a\": 1, \"a\": 2}");
}

#[test]
fn fixing_twice_equals_fixing_once() {
    for (text, dialect) in [
        ("\u{FEFF}{\"a\": [1,], \"a\": 2,}", Dialect::Json),
        ("{\"a\": 1, \"a\": 2, \"a\": 3,}", Dialect::Jsonc),
        ("[1,]\n[2,]", Dialect::JsonLines),
    ] {
        let once = all(text, dialect).expect("fixed");
        assert_eq!(all(&once, dialect), None, "{text:?}");
    }
}

#[test]
fn the_fix_list_names_the_problems() {
    let fixed = fix("\u{FEFF}[1,]", &Config::new(Dialect::Json), false).expect("fixed");
    let rules: Vec<(Rule, usize)> = fixed
        .fixes
        .iter()
        .map(|f: &Fix| (f.rule, f.offset))
        .collect();
    assert_eq!(rules, [(Rule::ByteOrderMark, 0), (Rule::Syntax, 5)]);
    assert!(fixed.fixes.iter().all(|f| f.safe));
}

/// A JSON value, rendered with a byte order mark, trailing commas and earlier copies of keys
/// (junk the fix must drop), and the value it means.
fn tree() -> impl Strategy<Value = (String, serde_json::Value)> {
    let leaf = prop_oneof![
        Just(("null".to_owned(), serde_json::Value::Null)),
        any::<bool>().prop_map(|b| (b.to_string(), serde_json::Value::Bool(b))),
        (-1000i32..1000).prop_map(|n| (n.to_string(), serde_json::json!(n))),
        "[a-z ]{0,6}".prop_map(|s| (format!("{s:?}"), serde_json::Value::String(s))),
    ];
    leaf.prop_recursive(4, 32, 4, |inner| {
        let array = prop::collection::vec((inner.clone(), any::<bool>()), 0..4).prop_map(|items| {
            let body: Vec<String> = items.iter().map(|((t, _), _)| t.clone()).collect();
            let comma = items.last().is_some_and(|(_, c)| *c);
            let text = format!("[{}{}]", body.join(", "), if comma { "," } else { "" });
            let value = items.into_iter().map(|((_, v), _)| v).collect();
            (text, serde_json::Value::Array(value))
        });
        let object = (
            prop::collection::vec(
                (
                    prop::sample::select(vec!["a", "b", "c"]),
                    inner.clone(),
                    prop::collection::vec(inner, 0..3),
                ),
                0..4,
            ),
            any::<bool>(),
        )
            .prop_map(|(members, comma)| {
                let mut parts = Vec::new();
                let mut map = serde_json::Map::new();
                for (key, (text, value), earlier) in members {
                    // Earlier copies of the key come first, so the last one wins.
                    for (junk, _) in earlier {
                        parts.push(format!("\"{key}\": {junk}"));
                    }
                    parts.push(format!("\"{key}\": {text}"));
                    map.insert(key.to_owned(), value);
                }
                let comma = if comma && !parts.is_empty() { "," } else { "" };
                (
                    format!("{{{}{comma}}}", parts.join(", ")),
                    serde_json::Value::Object(map),
                )
            });
        prop_oneof![array, object]
    })
}

proptest! {
    // When nothing is fixed, the problems `try_fix` returns are the ones `check` finds, positions
    // included, in every dialect. A trailing comma is the case where they could differ.
    #[test]
    fn unfixed_problems_are_the_checks(
        tokens in prop::collection::vec(
            prop::sample::select(vec![
                "{", "}", "[", "]", ",", ":", "\"a\"", "\"b\"", "1", "01", "0x1", "true", "// c\n",
                "/* c */", "\n", " ", "\u{FEFF}", "'s'", "x", "\"\\ud800\"",
            ]),
            0..14,
        ),
        dialect in prop::sample::select(vec![
            Dialect::Json, Dialect::IJson, Dialect::Jsonc, Dialect::Json5, Dialect::JsonLines,
        ]),
        unsafe_fixes in any::<bool>(),
    ) {
        let text = tokens.concat();
        let config = Config::new(dialect);
        if let Err(Some(problems)) = try_fix(&text, &config, unsafe_fixes) {
            let again = report(&text, problems, &config);
            let plain = check(&text, &config);
            prop_assert_eq!(again.problems(), plain.problems());
        }
    }

    // Dropping the earlier copies leaves what serde_json (and JSON.parse) read from the original.
    #[test]
    fn fixed_strict_json_keeps_its_value(
        (text, value) in tree(),
        bom in any::<bool>(),
        dialect in prop::sample::select(vec![Dialect::Json, Dialect::IJson, Dialect::Jsonc]),
    ) {
        // The byte order mark rule is off in JSONC.
        let bom = bom && dialect != Dialect::Jsonc;
        let text = format!("{}{text}\n", if bom { "\u{FEFF}" } else { "" });
        let config = Config::new(dialect);
        let fixed = fix(&text, &config, true).map_or(text.clone(), |f| f.text);
        prop_assert_eq!(serde_json::from_str::<serde_json::Value>(&fixed).ok(), Some(value));
        prop_assert!(check(&fixed, &config).problems().is_empty());
        prop_assert_eq!(fix(&fixed, &config, true), None);
    }

    // Without the unsafe opt-in no member is dropped.
    #[test]
    fn safe_fixes_keep_every_member((text, _) in tree()) {
        let config = Config::new(Dialect::Json);
        let commas = text.contains(",]") || text.contains(",}");
        prop_assert_eq!(fix(&text, &config, false).is_some(), commas);
        if let Some(fixed) = fix(&text, &config, false) {
            prop_assert!(fixed.fixes.iter().all(|f| f.safe));
            // Each fix drops one comma and nothing else.
            prop_assert_eq!(fixed.text.len() + fixed.fixes.len(), text.len());
            prop_assert!(check(&fixed.text, &config).problems().iter().all(|p| p.rule() == Rule::DuplicateKey));
        }
    }
}
