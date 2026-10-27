//! The jsonvet library as WebAssembly, for the browser playground in `playground/`.

use std::fmt::Write;

use jsonvet::{Config, Dialect};
use wasm_bindgen::prelude::wasm_bindgen;

/// Check `text` in the dialect named `dialect` (a name or alias the CLI accepts, such as `json5`)
/// and return its problems as a JSON array, in document order. Each problem is an object with
/// `line` and `column` (1-based, columns in code points), `index` (where the problem starts, in
/// UTF-16 units, which is how a browser indexes the text), `severity`, `rule`, `message`, and
/// `path` (the JSONPath of a duplicate key, otherwise `null`). An unknown dialect is an error.
///
/// # Errors
///
/// The message for an unknown dialect, which lists the known ones.
#[wasm_bindgen]
pub fn check(text: &str, dialect: &str) -> Result<String, String> {
    let dialect = dialect.parse::<Dialect>().map_err(|e| e.to_string())?;
    let report = jsonvet::check(text, &Config::new(dialect));
    let mut out = String::from("[");
    // Problems come in document order, so count UTF-16 units forward from the last one.
    let (mut byte, mut units) = (0, 0);
    for (i, problem) in report.problems().iter().enumerate() {
        let offset = problem.offset().min(text.len());
        if offset < byte {
            (byte, units) = (0, 0);
        }
        units += text
            .get(byte..offset)
            .map_or(0, |s| s.encode_utf16().count());
        byte = offset;
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
            units,
            problem.severity(),
            problem.rule()
        );
        string(&mut out, &problem.to_string());
        out.push_str(r#","path":"#);
        match problem.path() {
            Some(path) => string(&mut out, path),
            None => out.push_str("null"),
        }
        out.push('}');
    }
    out.push(']');
    Ok(out)
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
    use super::{check, string};

    #[test]
    fn duplicate_key_in_json5() {
        let text = "{\n  // port\n  \"port\": 80,\n  \"port\": 8080,\n}\n";
        let expected = r#"[{"line":4,"column":3,"index":28,"severity":"error","rule":"duplicate-key","message":"duplicate key $.port","path":"$.port"}]"#;
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
        let expected = r#"[{"line":2,"column":1,"index":9,"severity":"error","rule":"duplicate-key","message":"duplicate key $.a","path":"$.a"}]"#;
        assert_eq!(out, expected);
    }

    // The same texts and strings as playground/smoke.mjs, which checks the built wasm.
    #[test]
    fn the_playground_smoke_texts() {
        let syntax = r#"[{"line":1,"column":10,"index":9,"severity":"error","rule":"duplicate-key","message":"duplicate key $.a","path":"$.a"},{"line":1,"column":25,"index":24,"severity":"error","rule":"syntax","message":"missing `,`","path":null}]"#;
        assert_eq!(
            check("{\"a\": 1, \"a\": 2, \"b\": [1 2]}", "json").as_deref(),
            Ok(syntax)
        );
    }

    #[test]
    fn jsonl_reports_the_line_of_the_record() {
        let out = check("{}\n{\"a\":1,\"a\":2}\n", "jsonl").unwrap_or_default();
        assert!(out.starts_with(r#"[{"line":2,"column":8,"#), "{out}");
        assert_eq!(out.matches("\"rule\":").count(), 1, "{out}");
    }

    #[test]
    fn clean_text_and_unknown_dialect() {
        assert_eq!(check("{}", "jsonc").as_deref(), Ok("[]"));
        let err = check("{}", "yaml").unwrap_err();
        assert!(err.starts_with("unknown dialect `yaml`"), "{err}");
    }
}
