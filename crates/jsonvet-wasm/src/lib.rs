//! The jsonvet library as WebAssembly, for the browser playground in `playground/`.

use std::fmt::Write;

use jsonvet::{Config, Dialect, FormatError, FormatOptions, Indent, RelatedKind};
use wasm_bindgen::prelude::wasm_bindgen;

/// Check `text` in the dialect named `dialect` (a name or alias the CLI accepts, such as `json5`)
/// and return its problems as a JSON array, in document order. Each problem is an object with
/// `line` and `column` (1-based, columns in code points), `index` (where the problem starts, in
/// UTF-16 units, which is how a browser indexes the text), `severity`, `rule`, `message`, `path`
/// (the JSONPath of a duplicate key, otherwise `null`) and `related`: the other places the problem
/// names, as in the CLI's `--format json`, each with `message` (`first copy`, `opened here` or
/// `starts here`), `line`, `column`, `offset` (in bytes) and `index`. An unknown dialect is an
/// error.
///
/// # Errors
///
/// The message for an unknown dialect, which lists the known ones.
#[wasm_bindgen]
pub fn check(text: &str, dialect: &str) -> Result<String, String> {
    let dialect = dialect.parse::<Dialect>().map_err(|e| e.to_string())?;
    let report = jsonvet::check(text, &Config::new(dialect));
    let mut out = String::from("[");
    // Problems come in document order, and so, mostly, do their related places: each keeps its
    // own cursor, so counting stays linear.
    let (mut here, mut there) = (Utf16::default(), Utf16::default());
    for (i, problem) in report.problems().iter().enumerate() {
        let offset = problem.offset().min(text.len());
        let at = report.location(offset);
        if i > 0 {
            out.push(',');
        }
        // Writing to a String cannot fail.
        let _ = write!(
            out,
            r#"{{"line":{},"column":{},"index":{},"severity":"{}","rule":"{}","message":"#,
            at.line,
            at.column,
            here.at(text, offset),
            problem.severity(),
            problem.rule()
        );
        string(&mut out, &problem.to_string());
        out.push_str(r#","path":"#);
        match problem.path() {
            Some(path) => string(&mut out, path),
            None => out.push_str("null"),
        }
        out.push_str(r#","related":["#);
        if let Some(related) = problem.related() {
            let offset = related.offset.min(text.len());
            let at = report.location(offset);
            let label = match related.kind {
                RelatedKind::FirstOccurrence => "first copy",
                RelatedKind::Opened => "opened here",
                // NOTE: `RelatedKind` is non-exhaustive; `Start` is the only other kind so far.
                _ => "starts here",
            };
            let _ = write!(
                out,
                r#"{{"message":"{label}","line":{},"column":{},"offset":{offset},"index":{}}}"#,
                at.line,
                at.column,
                there.at(text, offset)
            );
        }
        out.push_str("]}");
    }
    out.push(']');
    Ok(out)
}

/// Fix the problems in `text` that have a mechanical fix, as `--fix` does, or with `unsafe_fixes`
/// also as `--unsafe-fixes` does (duplicate keys, keeping the last). Returns `null` when there is
/// nothing to fix, or the text has a syntax error the fixes would not remove, otherwise an object
/// with `text` (the fixed text) and `fixes`: each with `rule`, `index` (where the problem was in
/// the original text, in UTF-16 units) and `safe`. `only`, when not empty, limits the fixes to
/// those whose `index` it lists, which is how one correction's fix is applied.
///
/// # Errors
///
/// The message for an unknown dialect.
#[wasm_bindgen]
pub fn fix(text: &str, dialect: &str, unsafe_fixes: bool, only: &[u32]) -> Result<String, String> {
    let config = Config::new(dialect.parse::<Dialect>().map_err(|e| e.to_string())?);
    // The fixes name byte offsets: walk the text once to turn the listed UTF-16 indexes into them.
    let bytes: Vec<usize> = if only.is_empty() {
        Vec::new()
    } else {
        let (mut units, mut found) = (0_u32, Vec::new());
        for (byte, c) in text.char_indices() {
            if only.contains(&units) {
                found.push(byte);
            }
            units += u32::try_from(c.len_utf16()).unwrap_or(2);
        }
        // A missing final newline is reported at the end of the text, past every character.
        if only.contains(&units) {
            found.push(text.len());
        }
        found
    };
    // Indexes that name no character leave nothing to fix, rather than every fix.
    let only = (!only.is_empty()).then_some(bytes.as_slice());
    let Ok(fixed) = jsonvet::try_fix_at(text, &config, unsafe_fixes, only) else {
        return Ok("null".to_owned());
    };
    let mut out = String::from(r#"{"text":"#);
    string(&mut out, &fixed.text);
    out.push_str(r#","fixes":["#);
    let mut at = Utf16::default();
    for (i, fix) in fixed.fixes.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            r#"{{"rule":"{}","index":{},"safe":{}}}"#,
            fix.rule,
            at.at(text, fix.offset.min(text.len())),
            fix.safe
        );
    }
    out.push_str("]}");
    Ok(out)
}

/// Format `text` as `jsonvet fmt` does. `indent` is the spaces for one level, 0 for a tab; `width`
/// the most columns a container may take on one line; `normalize_json5` applies to JSON5 only.
///
/// # Errors
///
/// The reason nothing was formatted: an unknown dialect, a syntax error (as `line:column message`),
/// or a text whose output would be too large.
#[wasm_bindgen]
pub fn format(
    text: &str,
    dialect: &str,
    indent: u32,
    width: u32,
    bracket_spacing: bool,
    normalize_json5: bool,
) -> Result<String, String> {
    let config = Config::new(dialect.parse::<Dialect>().map_err(|e| e.to_string())?);
    let options = FormatOptions {
        indent: if indent == 0 {
            Indent::Tab
        } else {
            Indent::Spaces(indent as usize)
        },
        width: width as usize,
        normalize_json5,
        bracket_spacing,
    };
    jsonvet::format(text, &config, &options).map_err(|e| match &e {
        FormatError::Syntax(report) => report.problems().first().map_or_else(String::new, |p| {
            let at = report.location(p.offset().min(text.len()));
            format!("{}:{} {p}", at.line, at.column)
        }),
        e => e.to_string(),
    })
}

/// The jsonvet version this build is from, such as `3.2.0`.
#[wasm_bindgen]
#[must_use]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// A byte offset and the UTF-16 index it is at, moved forward or back from the last one asked for.
#[derive(Default)]
struct Utf16 {
    byte: usize,
    units: usize,
}

impl Utf16 {
    /// The UTF-16 index of byte `offset`, a character boundary in `text`.
    fn at(&mut self, text: &str, offset: usize) -> usize {
        let count = |from, to| {
            text.get(from..to)
                .map_or(0, |s: &str| s.encode_utf16().count())
        };
        if offset >= self.byte {
            self.units += count(self.byte, offset);
        } else {
            self.units -= count(offset, self.byte);
        }
        self.byte = offset;
        self.units
    }
}

/// Append `s` as a JSON string. Keys in a path can hold any character, quotes included.
fn string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c < ' ' => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::{check, fix, format, string};

    #[test]
    fn duplicate_key_in_json5() {
        let text = "{\n  // port\n  \"port\": 80,\n  \"port\": 8080,\n}\n";
        let expected = r#"[{"line":4,"column":3,"index":28,"severity":"error","rule":"duplicate-key","message":"duplicate key $.port","path":"$.port","related":[{"message":"first copy","line":3,"column":3,"offset":14,"index":14}]}]"#;
        assert_eq!(check(text, "json5").as_deref(), Ok(expected));
    }

    #[test]
    fn same_problems_as_the_library() {
        let text = "{\"a\": 1, \"a\": 2, \"b\": [1 2]}";
        let report = jsonvet::check(text, &jsonvet::Config::new(jsonvet::Dialect::Json));
        let out = check(text, "json").unwrap_or_default();
        assert_eq!(report.problems().len(), 2);
        assert_eq!(out.matches("\"rule\":").count(), 2);
        assert_eq!(out.matches("},{").count(), 1, "{out}");
        assert!(out.contains(r#""rule":"syntax""#), "{out}");
        assert!(out.contains(r#""path":null"#), "{out}");
    }

    #[test]
    fn escapes_keys_in_paths() {
        let out = check("{\"a\\\"\\n\": 1, \"a\\\"\\n\": 2}", "json").unwrap_or_default();
        assert!(out.contains(r#""path":"$[\"a\\\"\\n\"]""#), "{out}");
    }

    #[test]
    fn escapes_control_characters() {
        let mut out = String::new();
        string(&mut out, "a\u{1}\u{1f} b");
        assert_eq!(out, r#""a\u0001\u001f b""#);
    }

    #[test]
    fn columns_count_code_points() {
        // `é` and `★` are one code point each: the second key starts at column 11, and at byte
        // column 15.
        let text = "{\"é★\": 1, \"é★\": 2}";
        for dialect in ["jsonc", "json5"] {
            let out = check(text, dialect).unwrap_or_default();
            assert!(
                out.starts_with(r#"[{"line":1,"column":11,"#),
                "{dialect}: {out}"
            );
        }
    }

    #[test]
    fn index_counts_utf16_units() {
        // `😀` is two UTF-16 units and `é` one; the second key's quote is at unit 11, not at byte 15.
        let out = check("{\"😀é\": 1, \"😀é\": 2}", "json").unwrap_or_default();
        assert!(
            out.starts_with(r#"[{"line":1,"column":11,"index":11,"#),
            "{out}"
        );
        // Two problems: the running count carries on from the first.
        let out = check("{\"a\": 1,\n\"a\": 2,\n\"a\": 3}", "json").unwrap_or_default();
        assert!(out.contains(r#""line":2,"column":1,"index":9,"#), "{out}");
        assert!(out.contains(r#""line":3,"column":1,"index":17,"#), "{out}");
    }

    #[test]
    fn json5_counts_a_line_separator_as_a_line_end() {
        let out = check("{\"a\": 1,\u{2028}\"a\": 2}", "json5").unwrap_or_default();
        let expected = r#"[{"line":2,"column":1,"index":9,"severity":"error","rule":"duplicate-key","message":"duplicate key $.a","path":"$.a","related":[{"message":"first copy","line":1,"column":2,"offset":1,"index":1}]}]"#;
        assert_eq!(out, expected);
    }

    // The same texts and strings as playground/smoke.mjs, which checks the built wasm.
    #[test]
    fn the_playground_smoke_texts() {
        let syntax = r#"[{"line":1,"column":10,"index":9,"severity":"error","rule":"duplicate-key","message":"duplicate key $.a","path":"$.a","related":[{"message":"first copy","line":1,"column":2,"offset":1,"index":1}]},{"line":1,"column":25,"index":24,"severity":"error","rule":"syntax","message":"missing `,`","path":null,"related":[]}]"#;
        assert_eq!(
            check("{\"a\": 1, \"a\": 2, \"b\": [1 2]}", "json").as_deref(),
            Ok(syntax)
        );
    }

    #[test]
    fn related_places_count_utf16_units() {
        // `😀` is two units and four bytes, so each first copy's index is two less than its
        // offset. The second problem's first copy comes before the first one's: the count moves
        // back.
        let out =
            check("[\"😀\", {\"a\": 1, \"b\": 1, \"b\": 2, \"a\": 2}]", "json").unwrap_or_default();
        assert!(
            out.contains(r#""related":[{"message":"first copy","line":1,"column":16,"offset":18,"index":16}]"#),
            "{out}"
        );
        assert!(
            out.contains(
                r#""related":[{"message":"first copy","line":1,"column":8,"offset":10,"index":8}]"#
            ),
            "{out}"
        );
    }

    #[test]
    fn an_unclosed_bracket_names_where_it_opened() {
        let out = check("{\"a\": [1,\n", "json").unwrap_or_default();
        assert!(out.contains(r#""rule":"syntax""#), "{out}");
        assert!(
            out.contains(
                r#""related":[{"message":"opened here","line":1,"column":7,"offset":6,"index":6}]"#
            ),
            "{out}"
        );
    }

    #[test]
    fn version_is_the_crates() {
        assert_eq!(super::version(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn jsonl_reports_the_line_of_the_record() {
        let out = check("{}\n{\"a\":1,\"a\":2}\n", "jsonl").unwrap_or_default();
        assert!(out.starts_with(r#"[{"line":2,"column":8,"#), "{out}");
        assert_eq!(out.matches("\"rule\":").count(), 1, "{out}");
    }

    #[test]
    fn fix_removes_a_trailing_comma_and_lists_it() {
        let out = fix("[1, 2,]", "json", false, &[]).unwrap_or_default();
        assert_eq!(
            out,
            r#"{"text":"[1, 2]","fixes":[{"rule":"syntax","index":5,"safe":true}]}"#
        );
    }

    #[test]
    fn fix_lists_every_fix_it_made() {
        let out = fix("{\"a\": [1,], \"b\": 2,}", "json", false, &[]).unwrap_or_default();
        assert_eq!(
            out,
            r#"{"text":"{\"a\": [1], \"b\": 2}","fixes":[{"rule":"syntax","index":8,"safe":true},{"rule":"syntax","index":18,"safe":true}]}"#
        );
    }

    #[test]
    fn mixed_line_endings_are_checked_and_fixed_from_their_note() {
        // `😀` is two units, so the CRLF at byte offset 9 is at index 7.
        let text = "[\n\"😀\",\r\n1]";
        let out = check(text, "json").unwrap_or_default();
        assert!(
            out.contains(r#""rule":"mixed-line-endings""#) && out.contains(r#""index":7,"#),
            "{out}"
        );
        assert_eq!(
            fix(text, "json", false, &[7]).as_deref(),
            Ok(
                r#"{"text":"[\u000a\"😀\",\u000a1]","fixes":[{"rule":"mixed-line-endings","index":7,"safe":true}]}"#
            )
        );
        assert_eq!(fix(text, "json", false, &[0]).as_deref(), Ok("null"));
    }

    #[test]
    fn fix_keeps_duplicate_keys_unless_asked() {
        let text = "{\"a\": 1, \"a\": 2}";
        assert_eq!(fix(text, "json", false, &[]).as_deref(), Ok("null"));
        let out = fix(text, "json", true, &[]).unwrap_or_default();
        assert!(out.starts_with(r#"{"text":"{\"a\": 2}","#), "{out}");
        assert!(out.contains(r#""safe":false"#), "{out}");
    }

    #[test]
    fn fix_with_only_applies_just_those_fixes() {
        let text = "{\"😀\": [1,], \"b\": [2,]}";
        // Indexes are UTF-16: the first comma is at 9, the second at 20.
        let all = fix(text, "json", false, &[]).unwrap_or_default();
        assert!(
            all.contains(r#""index":9,"#) && all.contains(r#""index":20,"#),
            "{all}"
        );
        let one = fix(text, "json", false, &[20]).unwrap_or_default();
        assert!(
            one.starts_with(r#"{"text":"{\"😀\": [1,], \"b\": [2]}""#),
            "{one}"
        );
        assert_eq!(fix(text, "json", false, &[3]).as_deref(), Ok("null"));
    }

    #[test]
    fn a_fix_at_the_end_of_the_text_applies_from_its_note() {
        // A missing final newline is reported just past the last character.
        let text = "{\"a\":1}\n{\"b\":2}";
        let out = fix(text, "jsonl", false, &[15]).unwrap_or_default();
        assert!(
            out.contains(r#""text":"{\"a\":1}\u000a{\"b\":2}\u000a""#),
            "{out}"
        );
    }

    #[test]
    fn fix_leaves_a_syntax_error_and_an_unknown_dialect() {
        assert_eq!(fix("[1 2]", "json", true, &[]).as_deref(), Ok("null"));
        assert!(fix("{}", "yaml", false, &[]).is_err());
    }

    #[test]
    fn format_uses_the_options() {
        let text = "{\"a\":[1,2],\"b\":{}}";
        assert_eq!(
            format(text, "json", 2, 80, true, false).as_deref(),
            Ok("{ \"a\": [1, 2], \"b\": {} }\n")
        );
        assert_eq!(
            format(text, "json", 4, 10, false, false).as_deref(),
            Ok("{\n    \"a\": [\n        1,\n        2\n    ],\n    \"b\": {}\n}\n")
        );
        let tab = format(text, "json", 0, 10, true, false).unwrap_or_default();
        assert!(tab.contains("\n\t\"a\""), "{tab:?}");
    }

    #[test]
    fn format_normalizes_json5_only_when_asked() {
        let text = "{'a': 1}";
        assert_eq!(
            format(text, "json5", 2, 80, true, false).as_deref(),
            Ok("{ 'a': 1 }\n")
        );
        assert_eq!(
            format(text, "json5", 2, 80, true, true).as_deref(),
            Ok("{ a: 1 }\n")
        );
    }

    #[test]
    fn format_says_why_it_refused() {
        let err = format("{\n\"a\": [1 2]}", "json", 2, 80, true, false).unwrap_err();
        assert!(err.starts_with("2:"), "{err}");
        assert!(err.contains("missing `,`"), "{err}");
        let deep = format(&"[".repeat(400), "json", 2, 80, true, false);
        assert!(deep.is_err());
        assert!(format("{}", "yaml", 2, 80, true, false).is_err());
        let text = "{\"a\":1,\n// jsonvet-ignore duplicate-key\n\"a\":2,\"b\":1,\"b\":2}\n";
        assert_eq!(
            format(text, "jsonc", 2, 80, true, false).unwrap_err(),
            "not formatted: formatting would change which problems an inline suppression hides"
        );
    }

    #[test]
    fn a_problem_and_its_fix_share_an_index() {
        // The playground matches a correction to its fix by index. A trailing comma is the exception:
        // the problem is at the closing bracket, the fix at the comma before it.
        for (text, dialect) in [("\u{feff}[1]", "json"), ("{}", "jsonl")] {
            let problems = check(text, dialect).unwrap_or_default();
            let fixed = fix(text, dialect, false, &[]).unwrap_or_default();
            let index = problems
                .split(r#""index":"#)
                .nth(1)
                .and_then(|r| r.split(',').next());
            let at = format!(r#""index":{},"#, index.unwrap_or("?"));
            assert!(
                fixed.contains(&at),
                "{dialect} {text:?}: {problems} vs {fixed}"
            );
        }
    }

    #[test]
    fn clean_text_and_unknown_dialect() {
        assert_eq!(check("{}", "jsonc").as_deref(), Ok("[]"));
        let err = check("{}", "yaml").unwrap_err();
        assert!(err.starts_with("unknown dialect `yaml`"), "{err}");
    }
}
