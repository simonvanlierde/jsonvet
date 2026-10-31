//! The English text of every problem message, by id, so a program can show the messages in
//! another language with [`Problem::message_with`](crate::Problem::message_with).

use std::fmt;

/// A message: its id, then its English text. `{name}` in the text is a value the message fills
/// in, such as `{path}`.
pub(crate) type Message = (&'static str, &'static str);

pub(crate) const DUPLICATE_KEY: Message = ("duplicate-key", "duplicate key {path}");
pub(crate) const NUMBER_OVERFLOWS: Message =
    ("invalid-number.overflows", "number overflows a double");
pub(crate) const NUMBER_INEXACT: Message = (
    "invalid-number.inexact",
    "integer beyond ±(2^53 − 1) cannot be represented exactly",
);
pub(crate) const LONE_SURROGATE: Message = (
    "invalid-code-point.surrogate",
    "lone surrogate {code_point}",
);
pub(crate) const NONCHARACTER: Message = (
    "invalid-code-point.noncharacter",
    "noncharacter {code_point}",
);
pub(crate) const BYTE_ORDER_MARK: Message = (
    "byte-order-mark",
    "byte order mark at the start of the file",
);
pub(crate) const MISSING_FINAL_NEWLINE: Message =
    ("missing-final-newline", "no newline at the end of the file");
pub(crate) const UNEXPECTED_END: Message = (
    "syntax.unexpected-end",
    "expected {expected}, found the end of the file",
);
pub(crate) const UNEXPECTED_LINE_END: Message = (
    "syntax.unexpected-line-end",
    "expected {expected}, found the end of the line",
);
pub(crate) const UNCLOSED: Message = (
    "syntax.unclosed",
    "unclosed `{open}`: expected {expected}, found the end of the file",
);
pub(crate) const MISSING_COMMA: Message = ("syntax.missing-comma", "missing `,`");
pub(crate) const UNEXPECTED_WORD: Message = (
    "syntax.unexpected-word",
    "expected {expected}, found `{word}`",
);
pub(crate) const DID_YOU_MEAN: Message = ("syntax.did-you-mean", "; did you mean `{literal}`?");
pub(crate) const LEADING_ZERO: Message = ("syntax.leading-zero", "leading zeros are not allowed");
pub(crate) const UNEXPECTED: Message = ("syntax.unexpected", "expected {expected}, found {found}");
pub(crate) const COMMENTS_NOT_ALLOWED: Message = (
    "not-allowed.comments",
    "comments are not allowed in {dialect}",
);
pub(crate) const SINGLE_QUOTES_NOT_ALLOWED: Message = (
    "not-allowed.single-quoted-strings",
    "single-quoted strings are not allowed in {dialect}",
);
pub(crate) const UNQUOTED_KEYS_NOT_ALLOWED: Message = (
    "not-allowed.unquoted-keys",
    "unquoted keys are not allowed in {dialect}",
);
pub(crate) const NUMBER_FORM_NOT_ALLOWED: Message = (
    "not-allowed.number-form",
    "this number form is not allowed in {dialect}",
);
pub(crate) const NAN_AND_INFINITY_NOT_ALLOWED: Message = (
    "not-allowed.nan-and-infinity",
    "NaN and Infinity are not allowed in {dialect}",
);
pub(crate) const DIALECT_ALLOWS: Message = (
    "syntax.dialect-allows",
    "; the {dialect} dialect allows this",
);
pub(crate) const TRAILING_COMMA: Message = (
    "syntax.trailing-comma",
    "trailing comma before `{close}` is not allowed in {dialect}",
);
pub(crate) const JWCC_AND_JSON5_ALLOW: Message = (
    "syntax.jwcc-and-json5-allow",
    "; the jwcc and json5 dialects allow this",
);
pub(crate) const UNTERMINATED_STRING: Message =
    ("syntax.unterminated-string", "unterminated string");
pub(crate) const LINE_BREAK_IN_STRING: Message =
    ("syntax.line-break-in-string", "line break inside a string");
pub(crate) const CONTROL_CHARACTER: Message = (
    "syntax.control-character",
    "control character {found} inside a string must be escaped",
);
pub(crate) const INVALID_ESCAPE: Message = ("syntax.invalid-escape", "invalid escape `\\{found}`");
pub(crate) const INVALID_UNICODE_ESCAPE: Message = (
    "syntax.invalid-unicode-escape",
    "`\\u` needs four hex digits",
);
pub(crate) const INVALID_HEX_ESCAPE: Message =
    ("syntax.invalid-hex-escape", "`\\x` needs two hex digits");
pub(crate) const UNTERMINATED_COMMENT: Message =
    ("syntax.unterminated-comment", "unterminated block comment");
pub(crate) const BLANK_LINE: Message = (
    "syntax.blank-line",
    "blank line; JSON Lines needs a value on every line",
);
pub(crate) const TOO_DEEP: Message = ("syntax.too-deep", "nested deeper than {limit} levels");

/// Every problem message and every part of one, by id, with its English text. A translation
/// gives its own text for any of these ids.
///
/// Public for jsonvet-cli only and outside the semver contract: ids, `{name}`s and text can
/// change in any release.
#[doc(hidden)]
pub const MESSAGES: &[(&str, &str)] = &[
    DUPLICATE_KEY,
    NUMBER_OVERFLOWS,
    NUMBER_INEXACT,
    LONE_SURROGATE,
    NONCHARACTER,
    BYTE_ORDER_MARK,
    MISSING_FINAL_NEWLINE,
    UNEXPECTED_END,
    UNEXPECTED_LINE_END,
    UNCLOSED,
    MISSING_COMMA,
    UNEXPECTED_WORD,
    DID_YOU_MEAN,
    LEADING_ZERO,
    UNEXPECTED,
    COMMENTS_NOT_ALLOWED,
    SINGLE_QUOTES_NOT_ALLOWED,
    UNQUOTED_KEYS_NOT_ALLOWED,
    NUMBER_FORM_NOT_ALLOWED,
    NAN_AND_INFINITY_NOT_ALLOWED,
    DIALECT_ALLOWS,
    TRAILING_COMMA,
    JWCC_AND_JSON5_ALLOW,
    UNTERMINATED_STRING,
    LINE_BREAK_IN_STRING,
    CONTROL_CHARACTER,
    INVALID_ESCAPE,
    INVALID_UNICODE_ESCAPE,
    INVALID_HEX_ESCAPE,
    UNTERMINATED_COMMENT,
    BLANK_LINE,
    TOO_DEEP,
    // The parts the parser fills into `{expected}`. The parser carries their English text, so
    // a translation finds them by it.
    ("expected.a-value", "a value"),
    ("expected.a-key", "a key"),
    ("expected.colon", "`:`"),
    ("expected.comma-or-brace", "`,` or `}`"),
    ("expected.comma-or-bracket", "`,` or `]`"),
    ("expected.the-end-of-the-line", "the end of the line"),
    ("expected.the-end-of-the-file", "the end of the file"),
    ("expected.a-digit", "a digit"),
    ("expected.a-hex-digit", "a hex digit"),
    ("expected.true", "`true`"),
    ("expected.false", "`false`"),
    ("expected.null", "`null`"),
    ("expected.infinity", "`Infinity`"),
    ("expected.nan", "`NaN`"),
];

/// Looks a message's text up: a translation's, or the English one.
pub(crate) type Translate<'a, 't> = &'a dyn Fn(&str) -> Option<&'t str>;

/// The text of `message` in `translate`'s words, or in English.
pub(crate) fn text<'t>((id, english): Message, translate: Translate<'_, 't>) -> &'t str {
    translate(id).unwrap_or(english)
}

/// The text of a part the parser filled in, such as `a value`, in `translate`'s words.
pub(crate) fn part<'t>(english: &'t str, translate: Translate<'_, 't>) -> &'t str {
    part_id(english).and_then(translate).unwrap_or(english)
}

/// The id of a part the parser fills in, found by its English text.
fn part_id(english: &str) -> Option<&'static str> {
    MESSAGES
        .iter()
        .find(|(id, text)| *text == english && id.starts_with("expected."))
        .map(|(id, _)| *id)
}

/// Write `template` with each `{name}` that `values` names replaced by its value. Any other `{`
/// stays as it is, and a value is never searched for names itself.
///
/// Public for jsonvet-cli only and outside the semver contract: it can change in any release.
#[doc(hidden)]
pub fn fill(
    out: &mut dyn fmt::Write,
    template: &str,
    values: &[(&str, &dyn fmt::Display)],
) -> fmt::Result {
    let mut rest = template;
    while let Some((before, after)) = rest.split_once('{') {
        out.write_str(before)?;
        let named = after.split_once('}').and_then(|(name, tail)| {
            values
                .iter()
                .find(|(known, _)| *known == name)
                .map(|(_, value)| (value, tail))
        });
        if let Some((value, tail)) = named {
            write!(out, "{value}")?;
            rest = tail;
        } else {
            out.write_char('{')?;
            rest = after;
        }
    }
    out.write_str(rest)
}

#[cfg(test)]
mod tests {
    use super::{MESSAGES, fill, part_id};

    #[test]
    fn every_message_has_one_id_and_english_text() {
        let mut ids: Vec<&str> = MESSAGES.iter().map(|(id, _)| *id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), MESSAGES.len(), "an id is listed twice");
        assert!(MESSAGES.iter().all(|(_, english)| !english.is_empty()));
    }

    /// Each string the parser passes as an `{expected}` part has an id, or a translation could
    /// not reach it.
    #[test]
    fn every_part_the_parser_names_has_an_id() {
        let source = include_str!("parse.rs");
        let code = source.split("#[cfg(test)]").next().unwrap_or(source);
        let callers = ["unexpected", "nan_or_infinity", "literal(", "structural"];
        let mut checked = 0;
        for line in code
            .lines()
            .filter(|l| callers.iter().any(|c| l.contains(c)))
        {
            // Every other piece between quotes is a string literal; parts hold no `"`.
            for part in line.split('"').skip(1).step_by(2) {
                assert!(part_id(part).is_some(), "{part:?} has no id in MESSAGES");
                checked += 1;
            }
        }
        assert!(checked > 20, "only {checked} parts found");
    }

    #[test]
    fn fill_replaces_only_named_values() {
        let mut out = String::new();
        fill(&mut out, "a {x} `{` {y} {", &[("x", &"{y}"), ("y", &2)]).expect("a write to memory");
        assert_eq!(out, "a {y} `{` 2 {");
    }

    #[test]
    fn part_id_resolves_known_parts_and_rejects_unknown() {
        assert_eq!(part_id("a value"), Some("expected.a-value"));
        assert_eq!(part_id("a key"), Some("expected.a-key"));
        assert_eq!(part_id("not an expected part"), None);
        assert_eq!(part_id("duplicate key {path}"), None);
    }
}
