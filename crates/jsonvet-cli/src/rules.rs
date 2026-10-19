//! What `--list-rules` and `--explain` print, and the rule help in SARIF output.

use std::fmt::Write as _;

use jsonvet::{Dialect, Rule, Severity};

use crate::sarif::{SCHEMA, UNREADABLE};

/// Where each rule's section is: `docs/usage.md#<id>` on GitHub.
pub(crate) fn help_uri(id: &str) -> String {
    format!("https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md#{id}")
}

/// Every rule a problem can come under, in the order the docs list them: the library's rules,
/// then the command line's own `unreadable` and `schema`.
pub(crate) fn ids() -> impl Iterator<Item = &'static str> {
    Rule::ALL
        .iter()
        .map(|rule| rule.id())
        .chain([UNREADABLE, SCHEMA])
}

/// A rule's one-line description.
pub(crate) fn description(id: &str) -> &'static str {
    match id {
        SCHEMA => "a value the JSON Schema given with --schema rejects",
        UNREADABLE => "a file that is not a regular file, could not be read, or is not UTF-8",
        _ => id.parse().map_or("", Rule::description),
    }
}

/// Why a rule matters, with a bad and a good example. docs/usage.md has the same text.
pub(crate) fn explanation(id: &str) -> &'static str {
    match id {
        "syntax" => {
            "The file is not valid in its dialect: a missing or extra comma, a comment in strict \
             JSON, an unclosed string. jsonvet reports the first one and stops there, since what \
             follows cannot be read reliably.\n\n  Bad (json):  {\"a\": 1,}\n  Good (json): {\"a\": 1}"
        }
        "duplicate-key" => {
            "Most parsers keep the last copy of a repeated key and drop the others without a \
             word, so a value is lost; RFC 8259 leaves the result undefined. The message gives \
             the key's JSONPath and where its first copy is.\n\n  Bad:  {\"port\": 80, \"port\": \
             8080}\n  Good: {\"port\": 8080}"
        }
        "invalid-number" => {
            "I-JSON (RFC 7493) asks for numbers a double can hold, and for integers within \
             2^53 - 1 either side of zero, so that every parser reads them the same. jsonvet \
             reports an overflow, and an integer written as digits alone past that limit.\n\n  \
             Bad (i-json):  {\"id\": 9007199254740993}\n  Good (i-json): {\"id\": \
             \"9007199254740993\"}"
        }
        "invalid-code-point" => {
            "I-JSON (RFC 7493) forbids lone surrogates and noncharacters in strings: many \
             decoders reject them or replace them with U+FFFD.\n\n  Bad (i-json):  {\"a\": \
             \"\\ud800\"}\n  Good (i-json): {\"a\": \"\\ud83d\\ude00\"}"
        }
        "byte-order-mark" => {
            "RFC 8259 says JSON text must not start with a byte order mark (U+FEFF), and many \
             parsers fail on one. Save the file as UTF-8 without a BOM.\n\n  Bad:  \
             <U+FEFF>{\"a\": 1}\n  Good: {\"a\": 1}"
        }
        "missing-final-newline" => {
            "JSON Lines ends every record with a newline. Without one, the next record appended \
             joins the last line and breaks it.\n\n  Bad (jsonl):  {\"a\": 1}<end of file>\n  \
             Good (jsonl): {\"a\": 1}<newline>"
        }
        SCHEMA => {
            "A value the JSON Schema given with --schema rejects, such as a missing required \
             key or a string where the schema wants a number. --error, --warn, --ignore and \
             settings files do not take this rule."
        }
        UNREADABLE => {
            "jsonvet could not check the file at all: it does not exist, is a directory, \
             device, FIFO or socket, could not be read, or is not UTF-8. It always fails the \
             run, and no option changes that."
        }
        _ => "",
    }
}

/// A rule's default severity in each dialect, such as `error in jsonl, warning in json and
/// i-json, off otherwise`, or just `error` when it is the same in all.
pub(crate) fn defaults(id: &str) -> String {
    let Ok(rule) = id.parse::<Rule>() else {
        return "error".to_owned();
    };
    let in_each = |severity: Severity| -> Vec<String> {
        Dialect::ALL
            .iter()
            .filter(|&&dialect| rule.default_severity(dialect) == severity)
            .map(ToString::to_string)
            .collect()
    };
    let mut parts = Vec::new();
    for severity in [Severity::Error, Severity::Warning, Severity::Off] {
        let dialects = in_each(severity);
        if dialects.len() == Dialect::ALL.len() {
            return severity.to_string();
        }
        if !dialects.is_empty() {
            parts.push(if severity == Severity::Off {
                "off otherwise".to_owned()
            } else {
                format!("{severity} in {}", dialects.join(" and "))
            });
        }
    }
    parts.join(", ")
}

/// What `--list-rules` prints: each rule's id and default severity, then its description.
pub(crate) fn list() -> String {
    let mut text = String::new();
    for id in ids() {
        let _ = writeln!(text, "{id} ({})\n    {}", defaults(id), description(id));
    }
    text.trim_end().to_owned()
}

/// What `--explain RULE` prints, or the usage error for a rule jsonvet does not know.
pub(crate) fn explain(id: &str) -> Result<String, String> {
    if !ids().any(|known| known == id) {
        return Err(unknown(id));
    }
    Ok(format!(
        "{id}: {}\n\nDefault: {}.\n\n{}\n\nMore: {}",
        description(id),
        defaults(id),
        explanation(id),
        help_uri(id)
    ))
}

/// The error for an unknown rule id, naming the closest one when it is a likely typo.
pub(crate) fn unknown(id: &str) -> String {
    let closest = ids()
        .map(|known| (distance(id, known), known))
        .min()
        .filter(|&(d, _)| d <= 3);
    match closest {
        Some((_, known)) => format!("unknown rule `{id}`; did you mean `{known}`?"),
        None => format!(
            "unknown rule `{id}`; expected one of {}",
            ids().collect::<Vec<_>>().join(", ")
        ),
    }
}

/// How many one-character insertions, deletions and substitutions turn `a` into `b`.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let next = (diagonal + usize::from(ca != cb))
                .min(row[j] + 1)
                .min(row[j + 1] + 1);
            diagonal = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::{defaults, distance, explain, explanation, ids, unknown};

    #[test]
    fn every_rule_has_help_and_a_docs_section() {
        let usage = include_str!("../../../docs/usage.md");
        for id in ids() {
            assert!(!explanation(id).is_empty(), "{id}");
            assert!(usage.contains(&format!("\n### {id}\n")), "{id} in usage.md");
            assert!(explain(id).is_ok(), "{id}");
        }
    }

    #[test]
    fn defaults_name_the_dialects_where_they_differ() {
        assert_eq!(defaults("duplicate-key"), "error");
        assert_eq!(defaults("schema"), "error");
        assert_eq!(defaults("invalid-number"), "error in i-json, off otherwise");
        assert_eq!(
            defaults("byte-order-mark"),
            "error in jsonl, warning in json and i-json, off otherwise"
        );
    }

    #[test]
    fn an_unknown_rule_suggests_the_closest() {
        assert_eq!(distance("kitten", "sitting"), 3);
        assert_eq!(distance("", "ab"), 2);
        assert_eq!(
            unknown("duplicate-keys"),
            "unknown rule `duplicate-keys`; did you mean `duplicate-key`?"
        );
        assert!(unknown("nope-nope-nope").contains("expected one of syntax,"));
        assert!(explain("bom").is_err());
    }
}
