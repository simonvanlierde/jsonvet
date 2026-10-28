//! What `--list-rules` and `--explain` print, and the rule help in SARIF output.

use std::fmt::Write as _;

use jsonvet::{Dialect, Rule, Severity};

use crate::messages;
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

/// A rule's one-line description, in English.
pub(crate) fn description(id: &str) -> &'static str {
    messages::english(&format!("rule.{id}.description"))
}

/// Why a rule matters, with a bad and a good example, in English. docs/usage.md has the same
/// text.
pub(crate) fn explanation(id: &str) -> &'static str {
    messages::english(&format!("rule.{id}.explanation"))
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
                messages::text("defaults.off-otherwise").to_owned()
            } else {
                messages::fill(
                    "defaults.in",
                    &[
                        ("severity", &severity),
                        ("dialects", &dialects.join(messages::text("defaults.and"))),
                    ],
                )
            });
        }
    }
    parts.join(messages::text("list.separator"))
}

/// What `--list-rules` prints: each rule's id and default severity, then its description.
pub(crate) fn list() -> String {
    let mut text = String::new();
    for id in ids() {
        let description = messages::text(&format!("rule.{id}.description"));
        let _ = writeln!(text, "{id} ({})\n    {description}", defaults(id));
    }
    text.trim_end().to_owned()
}

/// What `--explain RULE` prints, or the usage error for a rule jsonvet does not know.
pub(crate) fn explain(id: &str) -> Result<String, String> {
    if !ids().any(|known| known == id) {
        return Err(unknown(id));
    }
    Ok(messages::fill(
        "explain",
        &[
            ("id", &id),
            (
                "description",
                &messages::text(&format!("rule.{id}.description")),
            ),
            ("default", &defaults(id)),
            (
                "explanation",
                &messages::text(&format!("rule.{id}.explanation")),
            ),
            ("uri", &help_uri(id)),
        ],
    ))
}

/// The error for an unknown rule id, naming the closest one when it is a likely typo.
pub(crate) fn unknown(id: &str) -> String {
    match messages::did_you_mean(id, ids()) {
        hint if hint.is_empty() => messages::fill(
            "error.unknown-rule",
            &[
                ("id", &id),
                ("hint", &hint),
                (
                    "ids",
                    &ids()
                        .collect::<Vec<_>>()
                        .join(messages::text("list.separator")),
                ),
            ],
        ),
        hint => messages::fill("error.unknown-rule-close", &[("id", &id), ("hint", &hint)]),
    }
}

#[cfg(test)]
mod tests {
    use super::{defaults, explain, explanation, ids, unknown};

    /// Each paragraph of a rule's explanation, words only, is in its section of usage.md,
    /// which may add a sentence on the default.
    #[test]
    fn every_rule_has_help_that_its_docs_section_holds() {
        let usage = include_str!("../../../docs/usage.md");
        let words = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
        for id in ids() {
            // lines(), not "\n", because a Windows checkout may have CRLF line endings.
            let section: Vec<&str> = usage
                .lines()
                .skip_while(|l| *l != format!("### {id}"))
                .skip(1)
                .take_while(|l| !l.starts_with('#'))
                .filter(|l| !l.starts_with("```"))
                .collect();
            assert!(!section.is_empty(), "{id} in usage.md");
            let section = words(&section.join("\n")).replace('`', "");
            assert!(!explanation(id).is_empty(), "{id}");
            for paragraph in explanation(id).split("\n\n") {
                assert!(section.contains(&words(paragraph)), "{id}: {paragraph}");
            }
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
        assert_eq!(
            unknown("duplicate-keys"),
            "unknown rule `duplicate-keys` (did you mean `duplicate-key`?)"
        );
        assert!(unknown("nope-nope-nope").contains("expected one of syntax,"));
        assert!(explain("bom").is_err());
    }
}
