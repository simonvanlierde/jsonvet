use std::fmt;

use crate::unicode::{is_bidi_control, is_surrogate};
use crate::{Dialect, Rule, Severity};

/// One problem in a file: what is wrong, where, and how much it matters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub(crate) offset: usize,
    pub(crate) severity: Severity,
    pub(crate) detail: Detail,
}

impl Problem {
    /// The byte offset in the text where the problem is. Always on a character boundary.
    pub fn offset(&self) -> usize {
        self.offset
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

/// The message, without its position or rule id.
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

impl fmt::Display for Detail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Detail::Syntax(s) => s.fmt(f),
            Detail::DuplicateKey { path, .. } => write!(f, "duplicate key {path}"),
            Detail::InvalidNumber { overflows: true } => f.write_str("number overflows a double"),
            Detail::InvalidNumber { overflows: false } => {
                f.write_str("integer beyond ±(2^53 − 1) cannot be represented exactly")
            }
            Detail::InvalidCodePoint { code_point } if is_surrogate(*code_point) => {
                write!(f, "lone surrogate U+{code_point:04X}")
            }
            Detail::InvalidCodePoint { code_point } => {
                write!(f, "noncharacter U+{code_point:04X}")
            }
            Detail::ByteOrderMark => f.write_str("byte order mark at the start of the file"),
            Detail::MissingFinalNewline => f.write_str("no newline at the end of the file"),
        }
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
        /// The syntax as the message names it: "comments are".
        what: &'static str,
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

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            SyntaxKind::UnexpectedEnd { expected } => {
                write!(f, "expected {expected}, found the end of the file")
            }
            SyntaxKind::UnexpectedLineEnd { expected } => {
                write!(f, "expected {expected}, found the end of the line")
            }
            SyntaxKind::Unclosed { open, expected } => {
                write!(
                    f,
                    "unclosed `{open}`: expected {expected}, found the end of the file"
                )
            }
            SyntaxKind::MissingComma => f.write_str("missing `,`"),
            SyntaxKind::UnexpectedWord { word, expected } => {
                write!(f, "expected {expected}, found `{word}`")?;
                // Only where a value goes: after `{"a"`, `True` is no typo for `true`.
                match literal_for(word).filter(|_| *expected == "a value") {
                    Some(literal) => write!(f, "; did you mean `{literal}`?"),
                    None => Ok(()),
                }
            }
            SyntaxKind::LeadingZero => f.write_str("leading zeros are not allowed"),
            SyntaxKind::Unexpected { found, expected } => {
                write!(f, "expected {expected}, found {}", Shown(*found))
            }
            SyntaxKind::NotAllowed {
                what,
                dialect,
                hint,
            } => {
                write!(f, "{what} not allowed in {dialect}")?;
                // Comments need only JSONC; everything else NotAllowed covers is JSON5's.
                let allows = if *what == "comments are" {
                    "jsonc"
                } else {
                    "json5"
                };
                if *hint {
                    write!(f, "; the {allows} dialect allows this")?;
                }
                Ok(())
            }
            SyntaxKind::TrailingComma { close, dialect } => {
                write!(
                    f,
                    "trailing comma before `{close}` is not allowed in {dialect}"
                )?;
                match dialect {
                    Dialect::JsonLines => Ok(()),
                    _ => f.write_str("; the jwcc and json5 dialects allow this"),
                }
            }
            SyntaxKind::UnterminatedString => f.write_str("unterminated string"),
            SyntaxKind::LineBreakInString => f.write_str("line break inside a string"),
            SyntaxKind::ControlCharacter { found } => {
                write!(
                    f,
                    "control character {} inside a string must be escaped",
                    Shown(*found)
                )
            }
            SyntaxKind::InvalidEscape { found } => {
                write!(f, "invalid escape `\\{}`", found.escape_debug())
            }
            SyntaxKind::InvalidUnicodeEscape => f.write_str("`\\u` needs four hex digits"),
            SyntaxKind::InvalidHexEscape => f.write_str("`\\x` needs two hex digits"),
            SyntaxKind::UnterminatedComment => f.write_str("unterminated block comment"),
            SyntaxKind::BlankLine => {
                f.write_str("blank line; JSON Lines needs a value on every line")
            }
            SyntaxKind::TooDeep { limit } => write!(f, "nested deeper than {limit} levels"),
        }
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
