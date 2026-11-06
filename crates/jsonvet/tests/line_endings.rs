//! The `mixed-line-endings` rule: where it fires, what counts as a line ending, and its fix.

use jsonvet::{Config, Dialect, FormatOptions, Rule, Severity, check, fix, format, try_fix_at};
use proptest::prelude::*;

const ALL: [Dialect; 6] = [
    Dialect::Json,
    Dialect::IJson,
    Dialect::Jsonc,
    Dialect::Jwcc,
    Dialect::Json5,
    Dialect::JsonLines,
];

/// The `(offset, end)` of each mixed-line-endings problem.
fn mixed(text: &str, dialect: Dialect) -> Vec<(usize, usize)> {
    check(text, &Config::new(dialect))
        .problems()
        .iter()
        .filter(|p| p.rule() == Rule::MixedLineEndings)
        .map(|p| (p.offset(), p.end()))
        .collect()
}

fn fixed(text: &str, dialect: Dialect) -> Option<String> {
    fix(text, &Config::new(dialect), false).map(|f| f.text)
}

#[test]
fn lf_then_crlf_is_reported_at_the_crlf_in_every_dialect() {
    for dialect in ALL {
        let text = if dialect == Dialect::JsonLines {
            "[1]\n[2]\r\n[3]\n"
        } else {
            "[1,\n2,\r\n3]"
        };
        let at = text.find('\r').unwrap();
        assert_eq!(mixed(text, dialect), [(at, at + 2)], "{dialect:?}");
    }
}

#[test]
fn crlf_then_lf_is_reported_at_the_lf() {
    for dialect in ALL {
        let text = if dialect == Dialect::JsonLines {
            "[1]\r\n[2]\n[3]\r\n"
        } else {
            "[1,\r\n2,\n3]"
        };
        let at = text.find("\n[3").or_else(|| text.find("\n3")).unwrap();
        assert_eq!(mixed(text, dialect), [(at, at + 1)], "{dialect:?}");
    }
}

#[test]
fn only_the_first_difference_is_reported() {
    assert_eq!(mixed("[1,\n2,\r\n3,\n4,\r\n5]", Dialect::Json), [(6, 8)]);
}

#[test]
fn a_lone_cr_is_an_ending_outside_json_lines() {
    for dialect in ALL.iter().filter(|d| **d != Dialect::JsonLines) {
        assert_eq!(mixed("[1,\n2,\r3]", *dialect), [(6, 7)], "{dialect:?}");
        assert_eq!(mixed("[1,\r2,\n3]", *dialect), [(6, 7)], "{dialect:?}");
        assert_eq!(mixed("[1,\r\n2,\r3]", *dialect), [(7, 8)], "{dialect:?}");
        assert_eq!(mixed("[1,\r2,\r\n3]", *dialect), [(6, 8)], "{dialect:?}");
    }
}

#[test]
fn a_file_with_one_kind_of_ending_has_no_problem() {
    for dialect in ALL {
        for eol in ["\n", "\r\n", "\r"] {
            let text = if dialect == Dialect::JsonLines {
                if eol == "\r" {
                    continue;
                }
                format!("[1]{eol}[2]{eol}")
            } else {
                format!("[1,{eol}2,{eol}  3,{eol}{eol}4]{eol}")
            };
            assert_eq!(mixed(&text, dialect), [], "{dialect:?} {eol:?}");
        }
    }
}

#[test]
fn white_space_around_an_ending_does_not_hide_it() {
    // A space or tab in front of the CR, and a CR after an LF: runs the parser skips whole.
    assert_eq!(mixed("[1,\n 2, \r\n3]", Dialect::Json), [(8, 10)]);
    assert_eq!(mixed("[1,\n\t2,\t\r3]", Dialect::Json), [(8, 9)]);
    assert_eq!(mixed("[1,\n\r2]", Dialect::Json), [(4, 5)]);
    assert_eq!(mixed("[1,\r\n \n2]", Dialect::Json), [(6, 7)]);
    // The run ends the file.
    assert_eq!(mixed("[1]\n \r\n", Dialect::Json), [(5, 7)]);
    assert_eq!(mixed("[1]\r\n \n", Dialect::Json), [(6, 7)]);
}

#[test]
fn json_lines_does_not_count_a_lone_cr() {
    assert_eq!(mixed("[1,\r2]\n[3]\n", Dialect::JsonLines), []);
    assert_eq!(mixed("[1,\r2]\n[3]\r\n", Dialect::JsonLines), [(10, 12)]);
    assert_eq!(mixed("[1]", Dialect::JsonLines), []);
    // CRLF first: a lone CR after it is still white space.
    assert_eq!(mixed("[1]\r\n[2,\r3]\r\n", Dialect::JsonLines), []);
    assert_eq!(fixed("[1]\r\n[2,\r3]\r\n", Dialect::JsonLines), None);
    // A file with a lone CR in it and no LF has no ending to compare.
    assert_eq!(mixed("[1,\r2]", Dialect::JsonLines), []);
}

#[test]
fn json_lines_chunks_know_the_first_ending_of_the_file() {
    let text = "[1]\r\n[2]\r\n[3]\n[4]\r\n";
    let config = Config::new(Dialect::JsonLines);
    let whole = check(text, &config);
    let rule_offsets = |r: &jsonvet::Report| -> Vec<usize> {
        r.problems()
            .iter()
            .filter(|p| p.rule() == Rule::MixedLineEndings)
            .map(jsonvet::Problem::offset)
            .collect()
    };
    assert_eq!(rule_offsets(&whole), [13]);
    // The second chunk starts after the CRLF lines and sees the LF as a difference.
    let parts = [0..10, 10..text.len()];
    let found: Vec<_> = parts
        .into_iter()
        .map(|p| jsonvet::parse_lines(text, p, &config, None))
        .collect();
    let report = jsonvet::join_lines(text, found).0;
    assert_eq!(rule_offsets(&report), [13]);
}

#[test]
fn an_ending_in_a_string_is_a_syntax_error_not_a_line_ending() {
    for dialect in [Dialect::Json, Dialect::Jsonc, Dialect::Jwcc] {
        let problems = check("[\n\"a\r\nb\"]", &Config::new(dialect));
        assert!(
            problems
                .problems()
                .iter()
                .all(|p| p.rule() != Rule::MixedLineEndings),
            "{dialect:?}"
        );
        assert_eq!(problems.problems().len(), 1, "{dialect:?}");
    }
}

#[test]
fn a_json5_line_continuation_is_a_line_ending() {
    let d = Dialect::Json5;
    // LF first, then a continuation with CRLF, with CR alone, and with LF.
    assert_eq!(mixed("[\n\"a\\\r\nb\"]", d), [(5, 7)]);
    assert_eq!(mixed("[\n\"a\\\rb\"]", d), [(5, 6)]);
    assert_eq!(mixed("[\n\"a\\\nb\"]", d), []);
    // CRLF first, then a continuation with LF.
    assert_eq!(mixed("[\r\n\"a\\\nb\"]", d), [(6, 7)]);
    // The continuation is the first ending, and a later LF differs from it.
    assert_eq!(mixed("[\"a\\\r\nb\",\n1]", d), [(9, 10)]);
    assert_eq!(mixed("[\"a\\\rb\",\r\n1]", d), [(8, 10)]);
    assert_eq!(mixed("[\"a\\\nb\",\r\n1]", d), [(8, 10)]);
    // Same ending in both places.
    assert_eq!(mixed("[\"a\\\r\nb\",\r\n1]", d), []);
    // Other dialects have no continuations.
    assert!(
        check("[\n\"a\\\r\nb\"]", &Config::new(Dialect::Json))
            .problems()
            .iter()
            .all(|p| p.rule() == Rule::Syntax)
    );
}

#[test]
fn json5_line_and_paragraph_separators_are_not_counted() {
    assert_eq!(mixed("[\n1,\u{2028}2,\u{2029}3]", Dialect::Json5), []);
    assert_eq!(
        mixed("[1,\r\n2, // a\u{2028}\n3]", Dialect::Json5),
        [(15, 16)]
    );
}

#[test]
fn comments_hold_line_endings_too() {
    // A block comment spanning lines.
    assert_eq!(mixed("[\n/* a\r\nb */1]", Dialect::Jsonc), [(6, 8)]);
    assert_eq!(mixed("[\n/* a\rb */1]", Dialect::Jsonc), [(6, 7)]);
    assert_eq!(mixed("[\r\n/* a\nb */1]", Dialect::Jsonc), [(7, 8)]);
    assert_eq!(mixed("[\n/* a\nb */1]", Dialect::Jsonc), []);
    // The same near the start of the file, and at a later offset.
    assert_eq!(mixed("[\n\n/*\r\n*/1]", Dialect::Jsonc), [(5, 7)]);
    assert_eq!(mixed("[\n\n\n\n\n/*\r*/1]", Dialect::Jsonc), [(8, 9)]);
    // A line comment ends at CR in JSONC and JSON5.
    assert_eq!(mixed("[1, // a\r2,\n3]", Dialect::Jsonc), [(11, 12)]);
    assert_eq!(mixed("[1, // a\r\n2,\n3]", Dialect::Json5), [(12, 13)]);
}

#[test]
fn jwcc_line_comments_end_only_at_lf() {
    let d = Dialect::Jwcc;
    // The CR before the LF is part of the ending.
    assert_eq!(mixed("[1, // a\r\n2,\n3]", d), [(12, 13)]);
    assert_eq!(mixed("[\n1, // a\r\n2]", d), [(9, 11)]);
    assert_eq!(mixed("[1, // a\r\n2, // b\r\n3]", d), []);
    // A lone CR in a comment is comment text.
    assert_eq!(mixed("[1, // a\rb\n2,\n3]", d), []);
    assert_eq!(mixed("[1, // a\rb\r\n2,\r\n3]", d), []);
    assert_eq!(mixed("[1, // a\rb\n2,\r\n3]", d), [(13, 15)]);
    // A comment that runs to the end of the file with a CR last.
    assert_eq!(mixed("[1]\n// a\r", d), []);
    // A file of lone CRs alone is uniform.
    assert_eq!(mixed("[1,\r2,\r3]\r", d), []);
    assert_eq!(fixed("[1,\r2,\r3]\r", d), None);
    // Elsewhere a lone CR is an ending.
    assert_eq!(mixed("[1,\r2,\n3]", d), [(6, 7)]);
}

#[test]
fn the_rule_can_be_turned_off_or_raised() {
    let text = "[1,\n2,\r\n3]";
    let off = Config::new(Dialect::Json).rule(Rule::MixedLineEndings, Severity::Off);
    assert_eq!(check(text, &off).problems(), []);
    assert!(fix(text, &off, false).is_none());
    let error = Config::new(Dialect::Json).rule(Rule::MixedLineEndings, Severity::Error);
    assert_eq!(
        check(text, &error).problems()[0].severity(),
        Severity::Error
    );
    // A warning by default.
    assert_eq!(
        check(text, &Config::new(Dialect::Json)).problems()[0].severity(),
        Severity::Warning
    );
}

#[test]
fn a_syntax_error_after_the_mix_keeps_the_warning() {
    let problems = check("[1,\n2,\r\n3,,]", &Config::new(Dialect::Json));
    let rules: Vec<_> = problems
        .problems()
        .iter()
        .map(jsonvet::Problem::rule)
        .collect();
    assert_eq!(rules, [Rule::MixedLineEndings, Rule::Syntax]);
}

#[test]
fn the_fix_rewrites_every_ending_to_the_first() {
    for dialect in [Dialect::Json, Dialect::Jsonc, Dialect::Json5] {
        assert_eq!(
            fixed("[1,\n2,\r\n3,\r4]\r", dialect).as_deref(),
            Some("[1,\n2,\n3,\n4]\n"),
            "{dialect:?}"
        );
        assert_eq!(
            fixed("[1,\r\n2,\n3,\r4]\n", dialect).as_deref(),
            Some("[1,\r\n2,\r\n3,\r\n4]\r\n"),
            "{dialect:?}"
        );
        assert_eq!(
            fixed("[1,\r2,\n3,\r\n4]\n", dialect).as_deref(),
            Some("[1,\r2,\r3,\r4]\r"),
            "{dialect:?}"
        );
    }
    // Nothing to do for a uniform file.
    assert_eq!(fixed("[1,\r\n2]\r\n", Dialect::Json), None);
}

#[test]
fn the_fix_in_json_lines_knows_lf_and_crlf_only() {
    assert_eq!(
        fixed("[1]\n[2]\r\n[3]\n", Dialect::JsonLines).as_deref(),
        Some("[1]\n[2]\n[3]\n")
    );
    assert_eq!(
        fixed("[1]\r\n[2]\n[3]", Dialect::JsonLines).as_deref(),
        Some("[1]\r\n[2]\r\n[3]\r\n")
    );
    // A lone CR is white space and stays.
    assert_eq!(
        fixed("[1,\r2]\n[3]\r\n", Dialect::JsonLines).as_deref(),
        Some("[1,\r2]\n[3]\n")
    );
}

#[test]
fn the_fix_in_jwcc_turns_a_first_cr_into_lf_and_leaves_comment_text() {
    let d = Dialect::Jwcc;
    assert_eq!(
        fixed("[1,\r2,\r\n3]\n", d).as_deref(),
        Some("[1,\n2,\n3]\n")
    );
    // A CR in a comment is not an ending, so it stays; the CR of a CRLF goes.
    assert_eq!(
        fixed("[1, // a\rb\n2,\r\n3]", d).as_deref(),
        Some("[1, // a\rb\n2,\n3]")
    );
    assert_eq!(
        fixed("[\r\n1, // a\rb\r\n2,\n3]", d).as_deref(),
        Some("[\r\n1, // a\rb\r\n2,\r\n3]")
    );
}

#[test]
fn the_fix_reaches_endings_in_comments_and_continuations() {
    assert_eq!(
        fixed("[\n/* a\r\nb */1]", Dialect::Jsonc).as_deref(),
        Some("[\n/* a\nb */1]")
    );
    assert_eq!(
        fixed("[\n\"a\\\r\nb\",\r\n1]", Dialect::Json5).as_deref(),
        Some("[\n\"a\\\nb\",\n1]")
    );
}

#[test]
fn fixing_twice_equals_fixing_once() {
    for dialect in ALL {
        for text in [
            "[1,\n2,\r\n3,\r4]\r",
            "[1]\n[2]\r\n[3]",
            "\u{feff}[1,\r\n2,\n3,]\n",
            "[1, // a\r\n2,\n3] // b\r",
        ] {
            let Some(once) = fixed(text, dialect) else {
                continue;
            };
            assert_eq!(fixed(&once, dialect), None, "{dialect:?} {text:?}");
            assert!(mixed(&once, dialect).is_empty(), "{dialect:?} {text:?}");
        }
    }
}

#[test]
fn the_fix_works_with_the_other_fixes() {
    // A byte order mark, a trailing comma and a missing final newline, in a CRLF file.
    assert_eq!(
        fixed("\u{feff}[1]\r\n[2]\n[3]", Dialect::JsonLines).as_deref(),
        Some("[1]\r\n[2]\r\n[3]\r\n")
    );
    assert_eq!(
        fixed("[1,\r\n2,\n3,\n]", Dialect::Jwcc),
        Some("[1,\r\n2,\r\n3,\r\n]".to_owned())
    );
    assert_eq!(
        fixed("\u{feff}[1,\r\n2,\n3,\n]", Dialect::Json).as_deref(),
        Some("[1,\r\n2,\r\n3\r\n]")
    );
}

#[test]
fn endings_inside_a_dropped_duplicate_member_are_not_edited_twice() {
    let text = "{\"a\": [\n1,\r\n2],\n\"a\": 3}\n";
    let fixed = fix(text, &Config::new(Dialect::Json), true).unwrap();
    assert_eq!(fixed.text, "{\"a\": 3}\n");
    let safe = fix(text, &Config::new(Dialect::Json), false).unwrap();
    assert_eq!(safe.text, "{\"a\": [\n1,\n2],\n\"a\": 3}\n");
}

#[test]
fn the_fix_of_one_note_applies_alone() {
    let text = "[1,\n2,\r\n3,]";
    let config = Config::new(Dialect::Jwcc);
    let comma = text.rfind(',').unwrap();
    let ending = text.find('\r').unwrap();
    let only = try_fix_at(text, &config, false, Some(&[ending])).unwrap();
    assert_eq!(only.text, "[1,\n2,\n3,]");
    assert_eq!(only.fixes.len(), 1);
    assert_eq!(only.fixes[0].rule, Rule::MixedLineEndings);
    assert_eq!(only.fixes[0].offset, ending);
    assert!(only.fixes[0].safe);
    let json = Config::new(Dialect::Json);
    let other = try_fix_at(text, &json, false, Some(&[comma])).unwrap();
    assert_eq!(other.text, "[1,\n2,\r\n3]");
    // An offset that names no fix changes nothing.
    assert!(try_fix_at(text, &config, false, Some(&[0])).is_err());
}

/// The first ending in `text`.
fn first_ending(text: &str) -> &'static str {
    let at = text.find(['\n', '\r']).unwrap_or(text.len());
    match &text[at..] {
        t if t.starts_with("\r\n") => "\r\n",
        t if t.starts_with('\r') => "\r",
        _ => "\n",
    }
}

#[test]
fn fmt_and_fix_agree_on_the_ending() {
    for dialect in [
        Dialect::Json,
        Dialect::Jsonc,
        Dialect::Jwcc,
        Dialect::Json5,
        Dialect::JsonLines,
    ] {
        for text in [
            "[1,\n2,\r\n3]\n",
            "[1,\r\n2,\n3]\n",
            "[1,\r2,\n3]\r",
            "[1]\n[2]\r\n",
            "[1]\r\n[2]\n",
        ] {
            let config = Config::new(dialect);
            let Ok(formatted) = format(text, &config, &FormatOptions::default()) else {
                continue;
            };
            let Some(fixed) = fixed(text, dialect) else {
                continue;
            };
            assert_eq!(
                first_ending(&formatted),
                first_ending(&fixed),
                "{dialect:?} {text:?}"
            );
            // After the fix, the formatter keeps the ending it chose.
            let again = format(&fixed, &config, &FormatOptions::default()).unwrap();
            assert_eq!(first_ending(&again), first_ending(&fixed), "{dialect:?}");
        }
    }
}

/// The endings of `text` as `(offset, length)`, where a CR before an LF is one ending.
fn endings(text: &str) -> Vec<(usize, usize)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\r' if b.get(i + 1) == Some(&b'\n') => {
                out.push((i, 2));
                i += 2;
            }
            b'\r' | b'\n' => {
                out.push((i, 1));
                i += 1;
            }
            _ => i += 1,
        }
    }
    out
}

proptest! {
    /// In a document of numbers and white space, the problem sits at the first ending that
    /// differs from the first one, and the fix makes them all alike.
    #[test]
    fn white_space_runs_are_read_as_the_endings_they_hold(
        gaps in proptest::collection::vec("[ \t\r\n]{0,5}", 1..8),
        dialect in prop_oneof![
            Just(Dialect::Json), Just(Dialect::Jsonc), Just(Dialect::Json5), Just(Dialect::Jwcc)
        ],
    ) {
        let mut text = String::from("[");
        for (i, gap) in gaps.iter().enumerate() {
            if i > 0 {
                text.push(',');
            }
            text.push_str(gap);
            text.push('1');
        }
        text.push_str("\r\n"[..(gaps.len() % 3).min(2)].as_ref());
        text.push(']');
        let list = endings(&text);
        let kind = |(at, len): (usize, usize)| &text[at..at + len];
        let expected = list.first().and_then(|&f| {
            list.iter().copied().find(|&e| kind(e) != kind(f)).map(|(at, len)| (at, at + len))
        });
        prop_assert_eq!(mixed(&text, dialect), expected.into_iter().collect::<Vec<_>>());
        if expected.is_some() {
            let out = fixed(&text, dialect).unwrap();
            prop_assert!(mixed(&out, dialect).is_empty());
            let target = if dialect == Dialect::Jwcc && kind(list[0]) == "\r" { "\n" } else { kind(list[0]) };
            let want: String = {
                let mut s = String::new();
                let mut from = 0;
                for &(at, len) in &list {
                    s.push_str(&text[from..at]);
                    s.push_str(target);
                    from = at + len;
                }
                s.push_str(&text[from..]);
                s
            };
            prop_assert_eq!(out, want);
        }
    }
}

#[test]
fn the_final_newline_follows_the_first_ending_with_the_rule_off() {
    let config = Config::new(Dialect::JsonLines).rule(Rule::MixedLineEndings, Severity::Off);
    let text = |text| fix(text, &config, false).map(|f| f.text);
    assert_eq!(text("[1]\r\n[2]").as_deref(), Some("[1]\r\n[2]\r\n"));
    assert_eq!(text("[1]\n[2]").as_deref(), Some("[1]\n[2]\n"));
    // The endings are left alone.
    assert_eq!(
        text("[1]\n[2]\r\n[3]").as_deref(),
        Some("[1]\n[2]\r\n[3]\n")
    );
}

fn formatted(text: &str, dialect: Dialect) -> Option<String> {
    format(text, &Config::new(dialect), &FormatOptions::default()).ok()
}

#[test]
fn fmt_writes_the_endings_inside_a_block_comment_as_its_own() {
    let out = formatted("[\n1, /*\r\n*/ 2]\n", Dialect::Jsonc).unwrap_or_default();
    assert_eq!(out, "[\n  1,\n  /*\n*/ 2\n]\n");
    assert_eq!(mixed(&out, Dialect::Jsonc), []);
    let out = formatted("[\r\n1, /*\n a\r b\r\n*/ 2]\r\n", Dialect::Jsonc).unwrap_or_default();
    // The comment moves one column left, and its lines with it.
    assert_eq!(out, "[\r\n  1,\r\n  /*\r\na\r\nb\r\n*/ 2\r\n]\r\n");
    assert_eq!(mixed(&out, Dialect::Jsonc), []);
}

#[test]
fn fmt_writes_json5_line_continuations_as_its_own_ending() {
    let out = formatted("[\n\"a\\\r\nb\"]\n", Dialect::Json5).unwrap_or_default();
    assert_eq!(out, "[\"a\\\nb\"]\n");
    assert_eq!(mixed(&out, Dialect::Json5), []);
}

#[test]
fn fmt_keeps_the_text_of_a_jwcc_line_comment() {
    // The CR is comment text, not an ending, so it stays and the file's ending is LF.
    let out = formatted("// a\rb\n[1,\n2]\n", Dialect::Jwcc).unwrap_or_default();
    assert_eq!(out, "// a\rb\n[1, 2]\n");
    assert_eq!(mixed(&out, Dialect::Jwcc), []);
}

#[test]
fn fmt_picks_the_ending_the_rule_sees_first() {
    // The CR in the comment is not an ending, so the first ending is the CRLF.
    let text = "// a\rb\r\n[1,\n2]\n";
    let out = formatted(text, Dialect::Jwcc).unwrap_or_default();
    assert_eq!(out, "// a\rb\r\n[1, 2]\r\n");
    assert_eq!(
        fixed(text, Dialect::Jwcc).as_deref(),
        Some("// a\rb\r\n[1,\r\n2]\r\n")
    );
    // With the rule off, the choice is the same.
    let off = Config::new(Dialect::Jwcc).rule(Rule::MixedLineEndings, Severity::Off);
    assert_eq!(
        format(text, &off, &FormatOptions::default()).unwrap_or_default(),
        out
    );
}

/// Which kinds of line ending `text` holds: a lone LF, a CRLF, a lone CR.
fn kinds(text: &str) -> [bool; 3] {
    let list = endings(text);
    let has = |s: &str| list.iter().any(|&(at, len)| &text[at..at + len] == s);
    [has("\n"), has("\r\n"), has("\r")]
}

proptest! {
    /// Whatever the white space and comments between values, `--fix` and `fmt` write the same
    /// kind of ending, and the formatted text has a single kind.
    #[test]
    fn fix_and_fmt_choose_the_same_ending(
        gaps in proptest::collection::vec(
            prop_oneof![
                Just(""), Just(" "), Just("\n"), Just("\r\n"), Just("\r"), Just("/*\r\n*/"),
                Just("/*\n*/"), Just(" // c\r\n"), Just(" // c\rd\n"), Just(" // c\n"),
            ],
            1..8,
        ),
        dialect in prop_oneof![
            Just(Dialect::Json), Just(Dialect::Jsonc), Just(Dialect::Json5), Just(Dialect::Jwcc)
        ],
    ) {
        let mut text = String::from("[");
        for (i, gap) in gaps.iter().enumerate() {
            if i > 0 {
                text.push(',');
            }
            text.push_str(gap);
            text.push('1');
        }
        text.push_str("]\n");
        let Some(by_fmt) = formatted(&text, dialect) else { return Ok(()); };
        prop_assert_eq!(mixed(&by_fmt, dialect), vec![], "{:?}", by_fmt);
        let by_fix = fixed(&text, dialect).unwrap_or_else(|| text.clone());
        let after = formatted(&by_fix, dialect).unwrap_or_default();
        // The comments are the same text in both, so only the endings the tools wrote differ.
        prop_assert_eq!(kinds(&by_fmt), kinds(&after), "{:?} {:?}", by_fmt, after);
        prop_assert_eq!(&formatted(&by_fmt, dialect).unwrap_or_default(), &by_fmt);
    }
}

#[test]
fn a_json5_syntax_error_from_fmt_counts_its_line_separators() {
    // U+2028 starts a line in JSON5, so the stray `x` is on line 2.
    let Err(jsonvet::FormatError::Syntax(report)) = format(
        "[1,\u{2028}x]",
        &Config::new(Dialect::Json5),
        &FormatOptions::default(),
    ) else {
        panic!("a syntax error");
    };
    assert_eq!(report.location(report.problems()[0].offset()).line, 2);
}

#[test]
fn a_cr_before_a_crlf_that_is_no_ending_goes_with_it() {
    // A white-space CR in JSON Lines, and comment text in a JWCC line comment: left in place,
    // it would pair with the LF the fix writes and make a CRLF again.
    for (text, dialect, want) in [
        ("1\n2\r\r\n3\n", Dialect::JsonLines, "1\n2\n3\n"),
        ("1\n2\r\r\r\n3", Dialect::JsonLines, "1\n2\n3\n"),
        ("[1,\n// a\r\r\n2]\n", Dialect::Jwcc, "[1,\n// a\n2]\n"),
        // A lone CR outside a comment is an ending of its own in JWCC.
        ("[1,\n\r\r\n2]\n", Dialect::Jwcc, "[1,\n\n\n2]\n"),
    ] {
        let out = fixed(text, dialect);
        assert_eq!(out.as_deref(), Some(want), "{dialect} {text:?}");
        assert_eq!(fixed(want, dialect), None, "{dialect} {want:?}");
    }
}

#[test]
fn adding_the_final_newline_does_not_create_a_mixed_ending() {
    let config = Config::new(Dialect::JsonLines);
    for (text, want) in [
        ("1\n2\r", "1\n2\n"),
        ("1\n2\r\n3\r", "1\n2\n3\n"),
        ("1\n2\r\r", "1\n2\n"),
        ("1\n2\r ", "1\n2\r \n"),
        ("1\r\n2\r", "1\r\n2\r\r\n"),
        ("1\r", "1\n"),
    ] {
        let out = fix(text, &config, false).unwrap();
        assert_eq!(out.text, want, "{text:?}");
        assert!(check(&out.text, &config).problems().is_empty(), "{text:?}");
        assert!(fix(&out.text, &config, false).is_none(), "{text:?}");
    }
    let text = "1\n2\r\n3\r";
    let only = try_fix_at(text, &config, false, Some(&[text.len()])).unwrap();
    assert_eq!(only.text, "1\n2\r\n3\n");
    assert_eq!(only.fixes.len(), 1);
}

#[test]
fn fmt_measures_json5_continuations_after_rewriting_their_endings() {
    let config = Config::new(Dialect::Json5);
    for width in 1..35 {
        for normalize_json5 in [false, true] {
            let options = FormatOptions {
                width,
                normalize_json5,
                ..FormatOptions::default()
            };
            for text in [
                "/*\r\n*/['a\\\nb', 123456789]",
                "/*\n*/['a\\\r\nb', 123456789]",
                "/*\r\n*/{'a\\\nb': 123456789}",
                "/*\n*/{'a\\\r\nb': 123456789}",
            ] {
                let once = format(text, &config, &options).unwrap();
                let twice = format(&once, &config, &options).unwrap();
                assert_eq!(
                    once, twice,
                    "width={width}, normalize={normalize_json5}, {text:?}"
                );
                assert!(check(&once, &config).problems().is_empty(), "{once:?}");
            }
        }
    }
}

#[test]
fn fmt_syntax_reports_keep_the_configured_line_ending_severity() {
    let text = "[\n1,\r\nx]";
    for severity in [Severity::Off, Severity::Warning, Severity::Error] {
        let config = Config::new(Dialect::Json).rule(Rule::MixedLineEndings, severity);
        let Err(jsonvet::FormatError::Syntax(report)) =
            format(text, &config, &FormatOptions::default())
        else {
            panic!("a syntax error");
        };
        assert_eq!(report.problems(), check(text, &config).problems());
    }
}

/// `text` fixed one note at a time, in document order, as the playground applies them.
fn fixed_step_by_step(text: &str, config: &Config) -> String {
    let mut text = text.to_owned();
    while let Ok(all) = try_fix_at(&text, config, true, None) {
        // A listed fix that does not apply alone stops here, and the comparison fails.
        let Ok(one) = try_fix_at(&text, config, true, Some(&[all.fixes[0].offset])) else {
            break;
        };
        text = one.text;
    }
    text
}

#[test]
fn a_dropped_member_does_not_pick_the_line_ending() {
    // The first ending goes with the first "a", so the CRs that stay set the ending.
    let text = "{\"a\": 1,\r\n\"a\": 2,\"v\":\"x\"\r}\r";
    let config = Config::new(Dialect::Jsonc);
    let full = fix(text, &config, true).unwrap().text;
    assert_eq!(full, "{\"a\": 2,\"v\":\"x\"\r}\r");
    assert_eq!(fixed_step_by_step(text, &config), full);
}

proptest! {
    /// Fixing every note at once gives the text that fixing them one at a time does.
    #[test]
    fn one_fix_equals_fixes_one_at_a_time(
        members in proptest::collection::vec(
            (
                prop_oneof![Just("a"), Just("b")],
                prop_oneof![Just(""), Just("\n"), Just("\r\n"), Just("\r")],
            ),
            1..6,
        ),
        last in prop_oneof![Just(""), Just("\n"), Just("\r\n"), Just("\r")],
        dialect in prop_oneof![
            Just(Dialect::Json), Just(Dialect::Jsonc), Just(Dialect::Json5), Just(Dialect::Jwcc)
        ],
    ) {
        let mut text = String::from("{");
        for (i, (key, gap)) in members.iter().enumerate() {
            if i > 0 {
                text.push(',');
            }
            text.push_str(gap);
            text.push_str(&["\"", key, "\": ", &i.to_string()].concat());
        }
        text.push_str(last);
        text.push('}');
        let config = Config::new(dialect);
        let full = fix(&text, &config, true).map_or_else(|| text.clone(), |f| f.text);
        prop_assert_eq!(fixed_step_by_step(&text, &config), full, "{:?}", text);
    }
}
