use std::fmt;

use crate::messages::{
    self, BLANK_LINE, BYTE_ORDER_MARK, COMMENTS_NOT_ALLOWED, CONTROL_CHARACTER, DIALECT_ALLOWS,
    DID_YOU_MEAN, DUPLICATE_KEY, INVALID_ESCAPE, INVALID_HEX_ESCAPE, INVALID_UNICODE_ESCAPE,
    JWCC_AND_JSON5_ALLOW, LEADING_ZERO, LINE_BREAK_IN_STRING, LONE_SURROGATE, MISSING_COMMA,
    MISSING_FINAL_NEWLINE, Message, NONCHARACTER, NUMBER_INEXACT, NUMBER_OVERFLOWS, TOO_DEEP,
    TRAILING_COMMA, Translate, UNCLOSED, UNEXPECTED, UNEXPECTED_END, UNEXPECTED_LINE_END,
    UNEXPECTED_WORD, UNTERMINATED_COMMENT, UNTERMINATED_STRING, fill,
};
use crate::unicode::{is_bidi_control, is_surrogate};
use crate::{Dialect, Rule, Severity};

/// One problem in a file: what is wrong, where, and how much it matters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub(crate) offset: usize,
    pub(crate) end: usize,
    pub(crate) severity: Severity,
    pub(crate) detail: Detail,
}

impl Problem {
    /// The byte offset in the text where the problem is. Always on a character boundary.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// The byte offset just past the text the problem is about, such as the end of a duplicate
    /// key or of a number. It equals [`offset`](Self::offset) when the problem is about a place
    /// between characters, such as a missing `,` or the end of the file. Always on a character
    /// boundary. It is on the line of `offset`, except after a JSON5 key that holds a line end:
    /// a line continuation, or a raw U+2028 or U+2029, which JSON5 counts as line ends.
    ///
    /// ```
    /// use jsonvet::{check, Config, Dialect};
    ///
    /// let report = check(r#"{"port": 1, "port": 2}"#, &Config::new(Dialect::Json));
    /// let problem = &report.problems()[0];
    /// assert_eq!((problem.offset(), problem.end()), (12, 18));
    /// ```
    pub fn end(&self) -> usize {
        self.end
    }

    /// The problem's severity under the config it was checked with. Never `Off`.
    pub fn severity(&self) -> Severity {
        self.severity
    }

    /// The rule the problem belongs to.
    pub fn rule(&self) -> Rule {
        match self.detail {
            Detail::Syntax(_) => Rule::Syntax,
            Detail::DuplicateKey { .. } => Rule::DuplicateKey,
            Detail::InvalidNumber { .. } => Rule::InvalidNumber,
            Detail::InvalidCodePoint { .. } => Rule::InvalidCodePoint,
            Detail::ByteOrderMark => Rule::ByteOrderMark,
            Detail::MissingFinalNewline => Rule::MissingFinalNewline,
        }
    }

    /// A second place the problem refers to: the first occurrence of a duplicate key, or where
    /// an unterminated string or comment starts.
    pub fn related(&self) -> Option<Related> {
        let (offset, kind) = match &self.detail {
            Detail::DuplicateKey { first, .. } => (*first, RelatedKind::FirstOccurrence),
            Detail::Syntax(s) => {
                let kind = match s.kind {
                    SyntaxKind::Unclosed { .. } => RelatedKind::Opened,
                    _ => RelatedKind::Start,
                };
                (s.started?, kind)
            }
            _ => return None,
        };
        Some(Related { offset, kind })
    }

    /// The JSONPath of a duplicate key, such as `$.server.port`, or `None` for other problems.
    /// A path longer than 256 bytes keeps its ends, with `…` in place of the middle, so a path is
    /// at most 259 bytes: 256 and the three of `…`.
    ///
    /// ```
    /// use jsonvet::{check, Config, Dialect};
    ///
    /// let report = check(r#"{"server": {"port": 80, "port": 8080}}"#, &Config::new(Dialect::Json));
    /// assert_eq!(report.problems()[0].path(), Some("$.server.port"));
    /// assert_eq!(check("[1 2]", &Config::new(Dialect::Json)).problems()[0].path(), None);
    /// ```
    pub fn path(&self) -> Option<&str> {
        match &self.detail {
            Detail::DuplicateKey { path, .. } => Some(path),
            _ => None,
        }
    }

    /// The message in another language: `translate` gives the text for a message id in
    /// [`MESSAGES`](crate::MESSAGES), or `None` to keep the English text. Each `{name}` in a
    /// text is filled in as in the English one.
    ///
    /// ```
    /// use jsonvet::{check, Config, Dialect};
    ///
    /// let report = check(r#"{"a": 1, "a": 2}"#, &Config::new(Dialect::Json));
    /// let german = |id: &str| (id == "duplicate-key").then_some("doppelter Schlüssel {path}");
    /// assert_eq!(report.problems()[0].message_with(german), "doppelter Schlüssel $.a");
    /// assert_eq!(report.problems()[0].message_with(|_| None), "duplicate key $.a");
    /// ```
    ///
    /// Public for jsonvet-cli only and outside the semver contract: it can change in any release.
    #[doc(hidden)]
    pub fn message_with<'t>(&self, translate: impl Fn(&str) -> Option<&'t str>) -> String {
        let mut message = String::new();
        // NOTE: writing to a String cannot fail.
        let _ = self.detail.write(&mut message, &translate);
        message
    }

    /// The offset of [`related`](Self::related).
    #[deprecated(since = "3.0.0", note = "use related().map(|r| r.offset)")]
    pub fn related_offset(&self) -> Option<usize> {
        self.related().map(|r| r.offset)
    }
}

/// A second place a [`Problem`] refers to, and what it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Related {
    /// The byte offset in the text. Always on a character boundary.
    pub offset: usize,
    /// What sits at the offset.
    pub kind: RelatedKind,
}

/// What a [`Related`] offset points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RelatedKind {
    /// The first occurrence of a duplicate key.
    FirstOccurrence,
    /// The start of the unterminated string or comment a syntax error is about.
    Start,
    /// The `{` or `[` of the object or array that is still open at the end of the file.
    Opened,
}

/// The message, without its position or rule id, in English.
impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.detail.fmt(f)
    }
}

/// What a problem is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Detail {
    /// A syntax error.
    Syntax(SyntaxError),
    /// A key repeated in the same object.
    DuplicateKey {
        /// The offset of the key's first occurrence.
        first: usize,
        /// The JSONPath of the key, such as `$.server.port`.
        path: String,
    },
    /// A number I-JSON does not allow.
    InvalidNumber {
        /// True when the number overflows a double; false for an integer past ±(2⁵³ − 1).
        overflows: bool,
    },
    /// A lone surrogate or a noncharacter in a string.
    InvalidCodePoint {
        /// The code point.
        code_point: u32,
    },
    /// A byte order mark at the start of the file.
    ByteOrderMark,
    /// A JSON Lines file that does not end with a newline.
    MissingFinalNewline,
}

impl Detail {
    /// Write the message in `translate`'s words, or in English where it gives none.
    fn write(&self, out: &mut dyn fmt::Write, translate: Translate<'_, '_>) -> fmt::Result {
        let t = |message| messages::text(message, translate);
        match self {
            Detail::Syntax(s) => s.write(out, translate),
            Detail::DuplicateKey { path, .. } => fill(out, t(DUPLICATE_KEY), &[("path", path)]),
            Detail::InvalidNumber { overflows: true } => out.write_str(t(NUMBER_OVERFLOWS)),
            Detail::InvalidNumber { overflows: false } => out.write_str(t(NUMBER_INEXACT)),
            Detail::InvalidCodePoint { code_point } => {
                let message = if is_surrogate(*code_point) {
                    LONE_SURROGATE
                } else {
                    NONCHARACTER
                };
                let code_point = format!("U+{code_point:04X}");
                fill(out, t(message), &[("code_point", &code_point)])
            }
            Detail::ByteOrderMark => out.write_str(t(BYTE_ORDER_MARK)),
            Detail::MissingFinalNewline => out.write_str(t(MISSING_FINAL_NEWLINE)),
        }
    }
}

impl fmt::Display for Detail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write(f, &|_| None)
    }
}

/// A syntax error: the first place where the text stops being valid in its dialect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SyntaxError {
    /// What went wrong.
    pub(crate) kind: SyntaxKind,
    /// Where the unterminated string or comment the error is about starts, if it is about one.
    pub(crate) started: Option<usize>,
}

/// The kinds of syntax error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SyntaxKind {
    /// The text ended where something else was needed.
    UnexpectedEnd {
        /// What was needed, as the message names it: "a value", "`:`".
        expected: &'static str,
    },
    /// A JSON Lines line ended where something else was needed.
    UnexpectedLineEnd {
        /// What was needed, as for `UnexpectedEnd`.
        expected: &'static str,
    },
    /// The text ended inside an object or array, where something else was needed.
    Unclosed {
        /// The bracket that opened it: `{` or `[`.
        open: char,
        /// What was needed, as for `UnexpectedEnd`.
        expected: &'static str,
    },
    /// No `,` between two members or elements. The error sits just after the first one.
    MissingComma,
    /// A word that cannot continue the document here, such as `True`.
    UnexpectedWord {
        /// The word: an ASCII letter, then ASCII letters, digits and `_`.
        word: String,
        /// What was needed instead, as for `UnexpectedEnd`.
        expected: &'static str,
    },
    /// A number with a `0` before its other integer digits, such as `01`.
    LeadingZero,
    /// A character that cannot continue the document here.
    Unexpected {
        /// The character found.
        found: char,
        /// What was needed instead, as for `UnexpectedEnd`.
        expected: &'static str,
    },
    /// Syntax that a larger dialect allows but this one does not.
    NotAllowed {
        /// The message, which names the syntax.
        what: Message,
        /// The dialect being checked.
        dialect: Dialect,
        /// Whether the larger dialect reads this syntax in full, so the message may name it:
        /// not for `NaNx`, `0x` or an unclosed `/*`, which it rejects too, nor in JSON Lines,
        /// which no larger dialect extends.
        hint: bool,
    },
    /// A trailing comma before `]` or `}` in a dialect without them.
    TrailingComma {
        /// The closing bracket.
        close: char,
        /// The dialect being checked.
        dialect: Dialect,
    },
    /// A string that runs to the end of the text.
    UnterminatedString,
    /// A raw line break inside a string.
    LineBreakInString,
    /// A raw control character inside a string, which this dialect requires to be escaped.
    ControlCharacter {
        /// The character.
        found: char,
    },
    /// A backslash escape this dialect does not define.
    InvalidEscape {
        /// The character after the backslash.
        found: char,
    },
    /// `\u` without four hex digits after it.
    InvalidUnicodeEscape,
    /// `\x` without two hex digits after it.
    InvalidHexEscape,
    /// A block comment without its closing `*/`.
    UnterminatedComment,
    /// An empty line in a JSON Lines file.
    BlankLine,
    /// An object or array opened one level past the config's maximum depth.
    TooDeep {
        /// The limit that was passed.
        limit: usize,
    },
}

impl SyntaxError {
    /// Write the message in `translate`'s words, or in English where it gives none.
    fn write(&self, out: &mut dyn fmt::Write, translate: Translate<'_, '_>) -> fmt::Result {
        let t = |message| messages::text(message, translate);
        let part = |english| messages::part(english, translate);
        match &self.kind {
            SyntaxKind::UnexpectedEnd { expected } => {
                fill(out, t(UNEXPECTED_END), &[("expected", &part(expected))])
            }
            SyntaxKind::UnexpectedLineEnd { expected } => fill(
                out,
                t(UNEXPECTED_LINE_END),
                &[("expected", &part(expected))],
            ),
            SyntaxKind::Unclosed { open, expected } => fill(
                out,
                t(UNCLOSED),
                &[("open", open), ("expected", &part(expected))],
            ),
            SyntaxKind::MissingComma => out.write_str(t(MISSING_COMMA)),
            SyntaxKind::UnexpectedWord { word, expected } => {
                fill(
                    out,
                    t(UNEXPECTED_WORD),
                    &[("expected", &part(expected)), ("word", word)],
                )?;
                // Only where a value goes: after `{"a"`, `True` is no typo for `true`.
                match literal_for(word).filter(|_| *expected == "a value") {
                    Some(literal) => fill(out, t(DID_YOU_MEAN), &[("literal", &literal)]),
                    None => Ok(()),
                }
            }
            SyntaxKind::LeadingZero => out.write_str(t(LEADING_ZERO)),
            SyntaxKind::Unexpected { found, expected } => fill(
                out,
                t(UNEXPECTED),
                &[("expected", &part(expected)), ("found", &Shown(*found))],
            ),
            SyntaxKind::NotAllowed {
                what,
                dialect,
                hint,
            } => {
                fill(out, t(*what), &[("dialect", dialect)])?;
                // Comments need only JSONC; everything else NotAllowed covers is JSON5's.
                let allows = if *what == COMMENTS_NOT_ALLOWED {
                    "jsonc"
                } else {
                    "json5"
                };
                if *hint {
                    fill(out, t(DIALECT_ALLOWS), &[("dialect", &allows)])?;
                }
                Ok(())
            }
            SyntaxKind::TrailingComma { close, dialect } => {
                fill(
                    out,
                    t(TRAILING_COMMA),
                    &[("close", close), ("dialect", dialect)],
                )?;
                match dialect {
                    Dialect::JsonLines => Ok(()),
                    _ => out.write_str(t(JWCC_AND_JSON5_ALLOW)),
                }
            }
            SyntaxKind::UnterminatedString => out.write_str(t(UNTERMINATED_STRING)),
            SyntaxKind::LineBreakInString => out.write_str(t(LINE_BREAK_IN_STRING)),
            SyntaxKind::ControlCharacter { found } => {
                fill(out, t(CONTROL_CHARACTER), &[("found", &Shown(*found))])
            }
            SyntaxKind::InvalidEscape { found } => {
                fill(out, t(INVALID_ESCAPE), &[("found", &found.escape_debug())])
            }
            SyntaxKind::InvalidUnicodeEscape => out.write_str(t(INVALID_UNICODE_ESCAPE)),
            SyntaxKind::InvalidHexEscape => out.write_str(t(INVALID_HEX_ESCAPE)),
            SyntaxKind::UnterminatedComment => out.write_str(t(UNTERMINATED_COMMENT)),
            SyntaxKind::BlankLine => out.write_str(t(BLANK_LINE)),
            SyntaxKind::TooDeep { limit } => fill(out, t(TOO_DEEP), &[("limit", limit)]),
        }
    }
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write(f, &|_| None)
    }
}

/// The JSON literal a word from another language most likely means, such as `true` for Python's
/// `True` or `null` for `undefined`, or `None` when the word is that literal or none of them.
fn literal_for(word: &str) -> Option<&'static str> {
    let literal = match word.to_ascii_lowercase().as_str() {
        "true" => "true",
        "false" => "false",
        "null" | "none" | "nil" | "undefined" => "null",
        _ => return None,
    };
    (literal != word).then_some(literal)
}

/// A character as a message shows it: in backticks when printable, as U+XXXX otherwise. Bidi
/// controls show as U+XXXX too, since printed they would reorder the rest of the line.
struct Shown(char);

impl fmt::Display for Shown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let c = self.0;
        if c.is_control() || c.is_whitespace() || c == '\u{FEFF}' || is_bidi_control(c) {
            write!(f, "U+{:04X}", u32::from(c))
        } else {
            write!(f, "`{c}`")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SyntaxError, SyntaxKind};

    #[test]
    fn syntax_error_display() {
        let err = SyntaxError {
            kind: SyntaxKind::BlankLine,
            started: None,
        };
        assert_eq!(
            format!("{err}"),
            "blank line; JSON Lines needs a value on every line"
        );
    }
}
