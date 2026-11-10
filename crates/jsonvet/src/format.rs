//! The formatter behind `jsonvet fmt`.
//!
//! It runs after a normal check has found no syntax error, so it reads its own token stream and
//! never has to report one. Comments are attached to the token they sit next to, then the layout
//! is computed from the tree alone: it does not depend on where the source broke its lines, which
//! is what makes a second run change nothing.
//!
//! The tree is a few flat vectors of small records that point into the text with 32-bit offsets,
//! so it takes a few bytes per byte of the file. A container's width on one line is worked out
//! as the container closes, from its members', and the output is counted as it is written, so a
//! file whose output would be huge is refused before that output is built.

use std::borrow::Cow;
use std::fmt;

use crate::parse::check_ending;
use crate::{Config, Dialect, Problem, Report, Rule};

/// How one level of nesting is indented.
///
/// Public for jsonvet-cli only and outside the semver contract: it can change in any release.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Indent {
    /// This many spaces.
    Spaces(usize),
    /// One tab, counted as four columns when a line is measured.
    Tab,
}

/// The layout settings of [`format`].
///
/// Public for jsonvet-cli only and outside the semver contract: it can change in any release.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatOptions {
    /// One level of indentation. Two spaces by default.
    pub indent: Indent,
    /// The most columns a container may take on one line. 80 by default.
    pub width: usize,
    /// For JSON5 only: double quotes, keys quoted only when an identifier will not do, and a
    /// trailing comma in every container that spans lines.
    pub normalize_json5: bool,
    /// A space inside the braces of an object on one line: `{ "a": 1 }`. On by default. JSON
    /// Lines records are always written without.
    pub bracket_spacing: bool,
}

impl Default for FormatOptions {
    fn default() -> Self {
        FormatOptions {
            indent: Indent::Spaces(2),
            width: 80,
            normalize_json5: false,
            bracket_spacing: true,
        }
    }
}

/// Why [`format`] gave no text.
///
/// Public for jsonvet-cli only and outside the semver contract: it can change in any release.
#[doc(hidden)]
#[derive(Debug)]
#[non_exhaustive]
pub enum FormatError<'a> {
    /// The text has a syntax error. The report has the problems up to it.
    Syntax(Report<'a>),
    /// The text is past what the formatter's 32-bit offsets reach, 512 MiB.
    InputTooLarge,
    /// Reflowing the text would change which problems an inline suppression hides.
    SuppressionTargets,
    /// The output would take this many bytes, more than 16 times the text or 64 MiB, whichever is
    /// more. Deep nesting does this: each level adds its indentation to every line below it.
    OutputTooLarge {
        /// The size the output would have.
        bytes: u64,
    },
}

impl fmt::Display for FormatError<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormatError::Syntax(report) => {
                let first = report.problems().first();
                write!(f, "{}", first.map_or_else(String::new, ToString::to_string))
            }
            FormatError::InputTooLarge => f.write_str("not formatted: the file is over 512 MiB"),
            FormatError::SuppressionTargets => f.write_str(
                "not formatted: formatting would change which problems an inline suppression hides",
            ),
            FormatError::OutputTooLarge { bytes } => {
                // Decimal units, rounded to a tenth.
                let (unit, size) = if *bytes >= 1_000_000_000 {
                    ("GB", (bytes + 50_000_000) / 100_000_000)
                } else {
                    ("MB", (bytes + 50_000) / 100_000)
                };
                write!(
                    f,
                    "not formatted: the output would be {}.{} {unit}",
                    size / 10,
                    size % 10
                )
            }
        }
    }
}

impl std::error::Error for FormatError<'_> {}

/// Format one text, keeping comments and values with line endings normalized.
/// A text with a syntax error comes back as [`FormatError::Syntax`] with its report,
/// and nothing is formatted. Formatting that changes which problems an inline suppression hides
/// comes back as [`FormatError::SuppressionTargets`].
///
/// ```
/// use jsonvet::{format, Config, Dialect, FormatOptions};
///
/// let text = "{\"a\":[1,2], // c\n\"b\":{}}";
/// let out = format(text, &Config::new(Dialect::Jsonc), &FormatOptions::default()).unwrap();
/// assert_eq!(out, "{\n  \"a\": [1, 2], // c\n  \"b\": {}\n}\n");
/// ```
///
/// It recurses once per nesting level, which [`Config::max_depth`] bounds.
///
/// Public for jsonvet-cli only and outside the semver contract: it can change in any release.
#[doc(hidden)]
#[allow(
    clippy::result_large_err,
    reason = "the error is the report the caller prints, and it is returned once per file"
)]
pub fn format<'a>(
    text: &'a str,
    config: &Config,
    options: &FormatOptions,
) -> Result<String, FormatError<'a>> {
    if text.len() > MAX_TEXT {
        return Err(FormatError::InputTooLarge);
    }
    let (problems, eol, hidden) = check_ending(text, config);
    if problems.iter().any(|p| p.rule() == Rule::Syntax) {
        return Err(FormatError::Syntax(Report::new(
            text,
            problems,
            config.dialect() == Dialect::Json5,
        )));
    }
    let dialect = config.dialect();
    let (bom, body) = match text.strip_prefix('\u{feff}') {
        Some(body) => ("\u{feff}", body),
        None => ("", text),
    };
    let norm = options.normalize_json5 && dialect == Dialect::Json5;
    let trailing = if norm {
        Trailing::Always
    } else if dialect.trailing_commas() {
        Trailing::Keep
    } else {
        Trailing::Never
    };
    let limit = (text.len() as u64 * 16).max(64 << 20);
    // A writer that only counts, or one that keeps `capacity` bytes of output before it spills.
    let writer = |counting: bool, capacity: usize, pad: bool| {
        let mut w = Writer {
            out: String::with_capacity(capacity),
            total: 0,
            limit,
            over: counting,
            col: 0,
            eol,
            options,
            trailing,
            pad,
        };
        w.put(bom);
        // A byte order mark takes no column.
        w.col = 0;
        w
    };
    if dialect == Dialect::JsonLines {
        // Each record is worked out and written on its own, so the output is built as it goes.
        let capacity = (text.len() + text.len() / 4).min(usize::try_from(limit).unwrap_or(0));
        let mut w = writer(false, capacity, false);
        let mut parser = Parser::new(body, Dialect::Json, false, eol);
        for line in body.split('\n') {
            let line = line.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
            if line.is_empty() {
                continue;
            }
            parser.reset(line);
            if let Some(value) = parser.document().value {
                w.flat_value(&parser.t, value);
                w.newline();
            }
        }
        return w.finish();
    }
    let mut parser = Parser::new(body, dialect, norm, eol);
    parser.pad = options.bracket_spacing;
    parser.reserve();
    let doc = parser.document();
    // The first pass only measures, so the output is built once, at its size, and never when it
    // would be huge.
    let mut counted = writer(true, 0, options.bracket_spacing);
    counted.document(&parser.t, &doc);
    if counted.total > limit {
        return counted.finish();
    }
    let mut w = writer(
        false,
        usize::try_from(counted.total).unwrap_or(0),
        options.bracket_spacing,
    );
    w.document(&parser.t, &doc);
    let out = w.finish()?;
    if parser
        .t
        .comments
        .iter()
        .chain(&doc.leading)
        .chain(&doc.trailing)
        .any(|&c| crate::suppress::has_target(parser.t.comment(c)))
        && suppressed(text, dialect, &problems, &hidden) != {
            let (problems, _, hidden) = check_ending(&out, config);
            suppressed(&out, dialect, &problems, &hidden)
        }
    {
        return Err(FormatError::SuppressionTargets);
    }
    Ok(out)
}

/// Each problem in `text` that a suppression could hide, as its token's ordinal, its rule and,
/// when one hides it, the ordinal of that suppression. Token ordinals survive white space changes
/// and JSON5 normalization. Commas can be added or removed by normalization, so they have no
/// identity here.
fn suppressed<'a>(
    text: &str,
    dialect: Dialect,
    problems: &'a [Problem],
    hidden: &'a [(usize, Problem)],
) -> Vec<(usize, &'static str, Option<usize>)> {
    let mut lexer = Lexer {
        text,
        at: 0,
        dialect,
    };
    let mut tokens = Vec::new();
    let mut suppressions = Vec::new();
    while let Some(token) = lexer.next() {
        let (start, end) = (token.start as usize, token.end as usize);
        match token.kind {
            Kind::Line | Kind::Block => {
                if crate::suppress::has_target(&text[start..end]) {
                    suppressions.push(start);
                }
            }
            Kind::Comma => {}
            _ => tokens.push(start),
        }
    }
    let token = |p: &Problem| tokens.partition_point(|&start| start <= p.offset());
    let mut list: Vec<_> = problems
        .iter()
        .filter(|p| crate::suppress::hideable(p.rule()))
        .map(|p| (token(p), p.rule().id(), None))
        .chain(hidden.iter().map(|(comment, p)| {
            // Each comment that hides a problem names a rule it can hide, so it is listed.
            let by = suppressions.binary_search(comment).ok();
            (token(p), p.rule().id(), by)
        }))
        .collect();
    list.sort_unstable();
    list
}

#[cfg(test)]
mod tests {
    use super::suppressed;
    use crate::parse::check_ending;
    use crate::{Config, Dialect, FormatOptions, Rule, Severity, format};

    fn numbers() -> Config {
        Config::new(Dialect::Json5).rule(Rule::InvalidNumber, Severity::Error)
    }

    fn formats(text: &str) -> bool {
        format(text, &numbers(), &FormatOptions::default()).is_ok()
    }

    #[test]
    fn a_reflow_that_keeps_what_each_suppression_hides_is_formatted() {
        assert!(formats(
            "{\"a\":1,\n// jsonvet-ignore invalid-number\n\"b\":1e999}"
        ));
        // A suppression that hides nothing may cover other tokens after formatting.
        assert!(formats("[1,\n// jsonvet-ignore duplicate-key\n2, 3]"));
    }

    #[test]
    fn added_commas_do_not_shift_the_hidden_tokens() {
        // Normalizing puts a trailing comma in the array, before the hidden duplicate.
        let text = "{x: [1111, 2222, 3333, 4444, 5555],\n// jsonvet-ignore duplicate-key\nx: 2}";
        let options = FormatOptions {
            width: 20,
            normalize_json5: true,
            ..FormatOptions::default()
        };
        let out = format(text, &Config::new(Dialect::Json5), &options).expect("formatted");
        assert!(out.contains("5555,\n  ],"), "{out}");
    }

    #[test]
    fn equal_counts_with_other_hidden_tokens_differ() {
        let list = |text| {
            let (problems, _, hidden) = check_ending(text, &numbers());
            suppressed(text, Dialect::Json5, &problems, &hidden)
        };
        // Each token's number counts the tokens up to it, from 1 at the `[`.
        let number = "invalid-number";
        assert_eq!(
            list("[\n// jsonvet-ignore invalid-number\n1e999,\n2e999\n]"),
            [(2, number, Some(0)), (3, number, None)]
        );
        assert_eq!(
            list("[\n1e999,\n// jsonvet-ignore invalid-number\n2e999\n]"),
            [(2, number, None), (3, number, Some(0))]
        );
    }
}

/// The most text the 32-bit offsets, with one bit kept for rewritten strings, can point into.
const MAX_TEXT: usize = 512 << 20;

/// The column at which `at` sits in its line of `text`, a tab counting as four.
fn column(text: &str, at: usize) -> usize {
    let start = text[..at].rfind(['\n', '\r']).map_or(0, |i| i + 1);
    text[start..at]
        .chars()
        .map(|c| if c == '\t' { 4 } else { 1 })
        .sum()
}

fn narrow(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

// ---- tokens ----

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Open,
    Close,
    Comma,
    Colon,
    Atom,
    Line,
    Block,
}

#[derive(Clone, Copy)]
struct Tok {
    kind: Kind,
    start: u32,
    end: u32,
    /// Line breaks in the white space before the token.
    nl: u32,
}

/// Splits valid text into tokens, one at a time. Strings, numbers, literals and bare keys are
/// all atoms. On text that is not valid it still ends, with tokens the parser ignores.
struct Lexer<'a> {
    text: &'a str,
    at: usize,
    dialect: Dialect,
}

impl Lexer<'_> {
    fn next(&mut self) -> Option<Tok> {
        let (text, dialect) = (self.text, self.dialect);
        let bytes = text.as_bytes();
        let mut i = self.at;
        let mut nl = 0;
        loop {
            let c = text.get(i..)?.chars().next()?;
            let start = i;
            let kind = match c {
                '\r' if bytes.get(i + 1) == Some(&b'\n') => {
                    i += 2;
                    nl += 1;
                    continue;
                }
                '\n' | '\r' => {
                    i += 1;
                    nl += 1;
                    continue;
                }
                '\u{2028}' | '\u{2029}' if dialect == Dialect::Json5 => {
                    i += c.len_utf8();
                    nl += 1;
                    continue;
                }
                c if c.is_whitespace() || c == '\u{feff}' => {
                    i += c.len_utf8();
                    continue;
                }
                '{' | '[' => {
                    i += 1;
                    Kind::Open
                }
                '}' | ']' => {
                    i += 1;
                    Kind::Close
                }
                ',' => {
                    i += 1;
                    Kind::Comma
                }
                ':' => {
                    i += 1;
                    Kind::Colon
                }
                '/' if bytes.get(i + 1) == Some(&b'/') => {
                    let line_end = |c: char| match c {
                        '\n' => true,
                        '\r' => dialect != Dialect::Jwcc,
                        '\u{2028}' | '\u{2029}' => dialect == Dialect::Json5,
                        _ => false,
                    };
                    i = text[i..].find(line_end).map_or(text.len(), |at| i + at);
                    Kind::Line
                }
                '/' if bytes.get(i + 1) == Some(&b'*') => {
                    i = text[i + 2..]
                        .find("*/")
                        .map_or(text.len(), |at| i + 2 + at + 2);
                    Kind::Block
                }
                '"' | '\'' => {
                    let mut chars = text[i + 1..].char_indices();
                    i = text.len();
                    while let Some((at, ch)) = chars.next() {
                        if ch == '\\' {
                            chars.next();
                        } else if ch == c {
                            i = start + 1 + at + 1;
                            break;
                        }
                    }
                    Kind::Atom
                }
                _ => {
                    i += c.len_utf8();
                    let stop =
                        |c: char| c.is_whitespace() || c == '\u{feff}' || ",:[]{}\"'/".contains(c);
                    i = text[i..].find(stop).map_or(text.len(), |at| i + at);
                    Kind::Atom
                }
            };
            self.at = i;
            let end = if kind == Kind::Line {
                start + text[start..i].trim_end().len()
            } else {
                i
            };
            return Some(Tok {
                kind,
                start: narrow(start),
                end: narrow(end),
                nl,
            });
        }
    }
}

// ---- tree ----

/// A piece of text: of the file, or with the high bit set of `Tree::owned`, where JSON5
/// normalization put what it rewrote.
#[derive(Clone, Copy)]
struct Span {
    start: u32,
    len: u32,
}

const OWNED: u32 = 1 << 31;
const NONE: u32 = u32::MAX;

#[derive(Clone, Copy)]
struct Comment {
    start: u32,
    len: u32,
    flags: u8,
}

/// A line comment, not a block comment.
const LINE: u8 = 1;
/// A blank line sits before it.
const BLANK: u8 = 2;
/// A line break sits before it, so it began its own line.
const NL_BEFORE: u8 = 4;
/// A line break follows it, or the text ends.
const NL_AFTER: u8 = 8;

impl Comment {
    fn has(self, flag: u8) -> bool {
        self.flags & flag != 0
    }
}

/// A value: an atom's span, or a container as `start == NONE` and `len` its index.
#[derive(Clone, Copy)]
struct Value {
    start: u32,
    len: u32,
}

impl Value {
    fn container(self) -> Option<usize> {
        (self.start == NONE).then_some(self.len as usize)
    }
}

/// A range of `Tree::comments`.
#[derive(Clone, Copy, Default)]
struct List {
    start: u32,
    len: u32,
}

struct Container {
    object: bool,
    trailing_comma: bool,
    /// The width on one line, or `NONE` if a comment inside forces lines.
    flat: u32,
    /// Comments on the line of the opening bracket, after it.
    open_trailing: List,
    /// Comments after the last member, on their own lines.
    tail: List,
    /// A range of `Tree::members`.
    members: List,
}

/// The comment lists of the few members that have any.
#[derive(Clone, Copy)]
struct MemberComments {
    /// Before the member, each on its own line unless it ends on the member's line.
    leading: List,
    before_colon: List,
    after_colon: List,
    /// Block comments between the value and its comma, on the value's line.
    before_comma: List,
    /// After the comma, on the value's line.
    trailing: List,
}

/// A key (none for an array element) and its value, in 16 bytes.
#[derive(Clone, Copy)]
struct Member {
    value: Value,
    /// `start` is `NONE` for an array element. The high bit of `len` is set if a blank line sits
    /// before the key or element itself: a key's length is under 2^29.
    key: Span,
}

const BLANK_MEMBER: u32 = 1 << 31;

impl Member {
    fn new(value: Value, key: Span, blank_before: bool) -> Member {
        let len = key.len | if blank_before { BLANK_MEMBER } else { 0 };
        Member {
            value,
            key: Span { len, ..key },
        }
    }

    fn blank_before(&self) -> bool {
        self.key.len & BLANK_MEMBER != 0
    }

    fn key(&self) -> Option<Span> {
        (self.key.start != NONE).then_some(Span {
            len: self.key.len & !BLANK_MEMBER,
            ..self.key
        })
    }
}

#[derive(Default)]
struct Tree<'a> {
    text: &'a str,
    owned: String,
    comments: Vec<Comment>,
    members: Vec<Member>,
    /// The comments of the members that have any, by index into `members`, in that order.
    extras: Vec<(u32, MemberComments)>,
    containers: Vec<Container>,
}

impl<'a> Tree<'a> {
    fn atom(&self, span: Span) -> &str {
        let (start, len) = (span.start as usize, span.len as usize);
        if span.start & OWNED == 0 {
            self.text.get(start..start + len).unwrap_or_default()
        } else {
            let start = (span.start & !OWNED) as usize;
            self.owned.get(start..start + len).unwrap_or_default()
        }
    }

    fn comment(&self, c: Comment) -> &'a str {
        let start = c.start as usize;
        self.text
            .get(start..start + c.len as usize)
            .unwrap_or_default()
    }

    fn list(&self, list: List) -> &[Comment] {
        let start = list.start as usize;
        self.comments
            .get(start..start + list.len as usize)
            .unwrap_or_default()
    }

    fn members_of(&self, c: &Container) -> &[Member] {
        let start = c.members.start as usize;
        self.members
            .get(start..start + c.members.len as usize)
            .unwrap_or_default()
    }

    fn clear(&mut self, text: &'a str) {
        self.text = text;
        self.owned.clear();
        self.comments.clear();
        self.members.clear();
        self.extras.clear();
        self.containers.clear();
    }
}

/// A member and what its container needs to know of it.
struct Parsed {
    member: Member,
    /// Its comments, if it has any.
    comments: Option<MemberComments>,
    /// Its width on one line: key, colon and value.
    width: usize,
    /// A comment in it forces its container onto lines.
    forced: bool,
    /// A comma follows it.
    comma: bool,
}

struct Document {
    leading: Vec<Comment>,
    value_blank: bool,
    value: Option<Value>,
    trailing: Vec<Comment>,
}

struct Parser<'a> {
    lexer: Lexer<'a>,
    cur: Option<Tok>,
    nxt: Option<Tok>,
    norm: bool,
    eol: &'static str,
    /// Whether an object on one line has a space inside its braces.
    pad: bool,
    t: Tree<'a>,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str, dialect: Dialect, norm: bool, eol: &'static str) -> Self {
        let mut lexer = Lexer {
            text,
            at: 0,
            dialect,
        };
        let (cur, nxt) = (lexer.next(), lexer.next());
        Parser {
            lexer,
            cur,
            nxt,
            norm,
            eol,
            pad: false,
            t: Tree {
                text,
                ..Tree::default()
            },
        }
    }

    /// Start over on another piece of the same text, keeping the vectors' room.
    fn reset(&mut self, text: &'a str) {
        self.lexer.text = text;
        self.lexer.at = 0;
        self.cur = self.lexer.next();
        self.nxt = self.lexer.next();
        self.t.clear(text);
    }

    /// Room for the most members and containers the text can hold, so the vectors never grow by
    /// doubling: a comma or an opening bracket stands for one member, and an opening bracket for
    /// one container. Strings with brackets or commas in them count too, so it is capped.
    fn reserve(&mut self) {
        let bytes = self.t.text.as_bytes();
        let opens = memchr::memchr2_iter(b'{', b'[', bytes).count();
        let commas = memchr::memchr_iter(b',', bytes).count();
        self.t
            .members
            .reserve((commas + opens).min(bytes.len() / 4 + 16));
        self.t.containers.reserve(opens.min(bytes.len() / 8 + 16));
    }

    fn advance(&mut self) {
        self.cur = self.nxt;
        self.nxt = self.lexer.next();
    }

    fn kind(&self) -> Option<Kind> {
        self.cur.map(|t| t.kind)
    }

    fn comments(&mut self) -> Vec<Comment> {
        let mut found = Vec::new();
        while let Some(tok) = self.cur
            && matches!(tok.kind, Kind::Line | Kind::Block)
        {
            let line = tok.kind == Kind::Line;
            let mut flags = 0;
            if line {
                flags |= LINE;
            }
            if tok.nl >= 2 {
                flags |= BLANK;
            }
            if tok.nl > 0 {
                flags |= NL_BEFORE;
            }
            if line || self.nxt.is_none_or(|next| next.nl > 0) {
                flags |= NL_AFTER;
            }
            found.push(Comment {
                start: tok.start,
                len: tok.end - tok.start,
                flags,
            });
            self.advance();
        }
        found
    }

    /// Comments after a token, split into those that stay on its line, ending it, and the rest.
    /// A comment followed by more code on the same line belongs to that code instead.
    fn split_trailing(&self, mut all: Vec<Comment>) -> (Vec<Comment>, Vec<Comment>) {
        let same = all.iter().take_while(|c| !c.has(NL_BEFORE)).count();
        let at_close = matches!(self.kind(), Some(Kind::Close) | None);
        let ends_line = same > 0 && (all[same - 1].has(NL_AFTER) || at_close);
        let rest = all.split_off(if ends_line { same } else { 0 });
        (all, rest)
    }

    fn store(&mut self, comments: Vec<Comment>) -> List {
        let start = narrow(self.t.comments.len());
        let len = narrow(comments.len());
        self.t.comments.extend(comments);
        List { start, len }
    }

    fn document(&mut self) -> Document {
        let leading = self.comments();
        let value_blank = self.cur.is_some_and(|t| t.nl >= 2);
        let value = matches!(self.kind(), Some(Kind::Open | Kind::Atom)).then(|| self.value().0);
        let trailing = self.comments();
        Document {
            leading,
            value_blank,
            value,
            trailing,
        }
    }

    /// A value, and its width on one line, or `None` if a comment inside forces lines.
    fn value(&mut self) -> (Value, Option<usize>) {
        match self.cur {
            Some(tok) if tok.kind == Kind::Open => {
                let index = self.container();
                let flat = self.t.containers[index].flat;
                let value = Value {
                    start: NONE,
                    len: narrow(index),
                };
                (value, (flat != NONE).then_some(flat as usize))
            }
            Some(tok) if tok.kind == Kind::Atom => {
                self.advance();
                let span = self.atom(tok, false);
                let width = self.t.atom(span).chars().count();
                (
                    Value {
                        start: span.start,
                        len: span.len,
                    },
                    Some(width),
                )
            }
            _ => (Value { start: 0, len: 0 }, Some(0)),
        }
    }

    /// The span of an atom token, rewritten first under JSON5 normalization.
    fn atom(&mut self, tok: Tok, key: bool) -> Span {
        let text = self.t.text;
        let source = text
            .get(tok.start as usize..tok.end as usize)
            .unwrap_or_default();
        let rewritten = if !self.norm {
            Cow::Borrowed(source)
        } else if key {
            key_text(source)
        } else {
            double_quoted(source)
        };
        // Rewrite continuations before measuring, so a second format uses the same widths.
        let rewritten = if rewritten.contains(['\n', '\r']) {
            let mut out = String::with_capacity(rewritten.len());
            let mut rest = rewritten.as_ref();
            while let Some(i) = rest.find(['\n', '\r']) {
                out.push_str(&rest[..i]);
                out.push_str(self.eol);
                let len = if rest[i..].starts_with("\r\n") { 2 } else { 1 };
                rest = &rest[i + len..];
            }
            out.push_str(rest);
            Cow::Owned(out)
        } else {
            rewritten
        };
        match rewritten {
            Cow::Borrowed(_) => Span {
                start: tok.start,
                len: tok.end - tok.start,
            },
            Cow::Owned(own) => {
                let start = narrow(self.t.owned.len()) | OWNED;
                self.t.owned.push_str(&own);
                Span {
                    start,
                    len: narrow(own.len()),
                }
            }
        }
    }

    /// One member of a container: its comments, key, value and comma. `pending` holds the
    /// comments found since the last member, and takes the ones after this one.
    fn member(&mut self, object: bool, pending: &mut Vec<Comment>) -> Parsed {
        let mut width = 0;
        let blank_before = self.cur.is_some_and(|t| t.nl >= 2);
        let (mut key, mut before_colon, mut after_colon) = (
            Span {
                start: NONE,
                len: 0,
            },
            Vec::new(),
            Vec::new(),
        );
        if object {
            if let Some(tok) = self.cur
                && tok.kind == Kind::Atom
            {
                key = self.atom(tok, true);
                self.advance();
            }
            before_colon = self.comments();
            if self.kind() == Some(Kind::Colon) {
                self.advance();
                after_colon = self.comments();
            }
            // A line comment cannot sit before the colon: it moves to after it.
            if before_colon.iter().any(|c| c.has(LINE)) {
                before_colon.append(&mut after_colon);
                after_colon = before_colon;
                before_colon = Vec::new();
            }
        }
        let (value, value_width) = self.value();
        let mut gap = self.comments();
        let inline = gap
            .iter()
            .take_while(|c| !c.has(LINE) && !c.has(NL_BEFORE))
            .count();
        let mut all = gap.split_off(inline);
        let before_comma = gap;
        let comma = self.kind() == Some(Kind::Comma);
        if comma {
            self.advance();
            let mut after = self.comments();
            // A line comment ends its line, so what follows the comma starts a new one.
            if all.last().is_some_and(|c| c.has(LINE))
                && let Some(first) = after.first_mut()
            {
                first.flags |= NL_BEFORE;
            }
            all.append(&mut after);
        }
        let (trailing, rest) = self.split_trailing(all);
        let leading = std::mem::replace(pending, rest);
        let commented = [
            &leading,
            &before_colon,
            &after_colon,
            &before_comma,
            &trailing,
        ]
        .iter()
        .any(|list| !list.is_empty());
        let mut forced = commented;
        let comments = commented.then(|| MemberComments {
            leading: self.store(leading),
            before_colon: self.store(before_colon),
            after_colon: self.store(after_colon),
            before_comma: self.store(before_comma),
            trailing: self.store(trailing),
        });
        if key.start != NONE {
            width += self.t.atom(key).chars().count() + 2;
        }
        match value_width {
            Some(w) => width += w,
            None => forced = true,
        }
        Parsed {
            member: Member::new(value, key, blank_before),
            comments,
            width,
            forced,
            comma,
        }
    }

    fn container(&mut self) -> usize {
        let text = self.t.text;
        let object = self
            .cur
            .is_some_and(|t| text.as_bytes().get(t.start as usize) == Some(&b'{'));
        self.advance();
        let first = self.comments();
        let (open_trailing, mut pending) = self.split_trailing(first);
        // An object the author began on a new line stays expanded, as in Prettier. With a
        // comment before the first key the object is expanded anyway.
        let mut forced = !open_trailing.is_empty()
            || (object
                && pending.is_empty()
                && matches!(self.kind(), Some(Kind::Open | Kind::Atom))
                && self.cur.is_some_and(|t| t.nl > 0));
        let open_trailing = self.store(open_trailing);
        let mut members = Vec::new();
        let mut commented = Vec::new();
        let mut trailing_comma = false;
        let mut width = 2 + usize::from(object && self.pad) * 2;
        loop {
            match self.kind() {
                None => break,
                Some(Kind::Close) => {
                    self.advance();
                    break;
                }
                Some(Kind::Open | Kind::Atom) => {}
                Some(_) => {
                    self.advance();
                    continue;
                }
            }
            let parsed = self.member(object, &mut pending);
            forced |= parsed.forced;
            width += parsed.width + 2;
            if let Some(lists) = parsed.comments {
                commented.push((narrow(members.len()), lists));
            }
            members.push(parsed.member);
            trailing_comma = parsed.comma;
        }
        // Each member counted its separator; the last has none.
        width = if members.is_empty() {
            2
        } else {
            width.saturating_sub(2)
        };
        forced |= !pending.is_empty();
        let tail = self.store(pending);
        let start = narrow(self.t.members.len());
        let count = narrow(members.len());
        self.t.members.extend(members);
        self.t
            .extras
            .extend(commented.into_iter().map(|(i, lists)| (start + i, lists)));
        let flat = if forced {
            NONE
        } else {
            narrow(width).min(NONE - 1)
        };
        self.t.containers.push(Container {
            object,
            trailing_comma,
            flat,
            open_trailing,
            tail,
            members: List { start, len: count },
        });
        self.t.containers.len() - 1
    }
}

// ---- JSON5 normalization ----

/// A JSON5 string in double quotes. A double-quoted one is returned as written.
fn double_quoted(text: &str) -> Cow<'_, str> {
    let Some(inner) = text
        .strip_prefix('\'')
        .and_then(|rest| rest.strip_suffix('\''))
    else {
        return Cow::Borrowed(text);
    };
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('\'') => out.push('\''),
                Some(next) => {
                    out.push('\\');
                    out.push(next);
                }
                None => out.push('\\'),
            },
            '"' => out.push_str("\\\""),
            c => out.push(c),
        }
    }
    out.push('"');
    Cow::Owned(out)
}

/// An object key under `normalize-json5`: bare when it is a plain identifier, else in double
/// quotes.
fn key_text(text: &str) -> Cow<'_, str> {
    let quoted = text.strip_prefix(['"', '\'']);
    let Some(inner) = quoted.and_then(|rest| rest.strip_suffix(['"', '\''])) else {
        return Cow::Borrowed(text);
    };
    let mut chars = inner.chars();
    let identifier = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    if identifier {
        Cow::Owned(inner.to_owned())
    } else {
        double_quoted(text)
    }
}

// ---- layout ----

/// Writes the output, and counts it. Past `limit` bytes it drops what it holds and only counts,
/// so the size of an output too big to build is still known.
struct Writer<'o> {
    out: String,
    total: u64,
    limit: u64,
    over: bool,
    /// The column of the end of the line being written.
    col: usize,
    eol: &'o str,
    options: &'o FormatOptions,
    trailing: Trailing,
    /// Whether an object on one line has a space inside its braces.
    pad: bool,
}

/// What a container with several lines has after its last member.
#[derive(Clone, Copy)]
enum Trailing {
    /// No comma.
    Never,
    /// A comma if the text had one.
    Keep,
    /// A comma.
    Always,
}

impl Writer<'_> {
    /// The output, or the error if it was too big to keep.
    #[allow(
        clippy::result_large_err,
        reason = "the error is returned once per file"
    )]
    fn finish<'a>(self) -> Result<String, FormatError<'a>> {
        if self.over {
            Err(FormatError::OutputTooLarge { bytes: self.total })
        } else {
            Ok(self.out)
        }
    }

    fn spill(&mut self) {
        if !self.over && self.total > self.limit {
            self.over = true;
            self.out = String::new();
        }
    }

    fn put(&mut self, s: &str) {
        self.total += s.len() as u64;
        if !self.over {
            self.out.push_str(s);
            self.spill();
        }
        let (last, base) = match s.rfind(['\n', '\r']) {
            Some(i) => (&s[i + 1..], 0),
            None => (s, self.col),
        };
        self.col = base
            + last
                .chars()
                .map(|c| if c == '\t' { 4 } else { 1 })
                .sum::<usize>();
    }

    /// A comment. A block comment over several lines moves its later lines by as many columns
    /// as the comment itself moved from its place in the text, when each line can follow.
    fn comment(&mut self, t: &Tree<'_>, c: Comment) {
        let text = t.comment(c);
        if c.has(LINE) || !text.contains(['\n', '\r']) {
            self.put(text);
            return;
        }
        let old = column(t.text, c.start as usize);
        let (unit, cols) = match self.options.indent {
            Indent::Spaces(_) => (' ', 1),
            Indent::Tab => ('\t', 4),
        };
        let (moved, right) = (self.col.abs_diff(old), self.col > old);
        // Only a whole number of indent characters can move a line.
        let n = if moved % cols == 0 { moved / cols } else { 0 };
        let mut rest = text;
        let mut first = true;
        while !rest.is_empty() {
            let end = rest.find(['\n', '\r']).map_or(rest.len(), |i| {
                i + if rest[i..].starts_with("\r\n") { 2 } else { 1 }
            });
            let (line, tail) = rest.split_at(end);
            rest = tail;
            let body = line.trim_end_matches(['\n', '\r']);
            let lead = body.len() - body.trim_start_matches([' ', '\t']).len();
            let same = body[..lead].chars().take_while(|&ch| ch == unit).count();
            // The first line starts the comment, and a blank line stays as it is.
            if first || n == 0 || lead == body.len() {
                self.put(body);
            } else if right && (lead == 0 || same > 0) {
                self.pad(unit, n);
                self.put(body);
            } else if !right && same >= n {
                self.put(&body[n * unit.len_utf8()..]);
            } else {
                self.put(body);
            }
            // Each ending is written as the file's own, so the output has one kind.
            if body.len() < line.len() {
                self.put(self.eol);
            }
            first = false;
        }
    }

    /// `n` of the indent character, counted but built only while the output is kept.
    fn pad(&mut self, c: char, n: usize) {
        self.total += n as u64;
        if !self.over {
            self.out.extend(std::iter::repeat_n(c, n));
            self.spill();
        }
        self.col += n * if c == '\t' { 4 } else { 1 };
    }

    fn put_char(&mut self, c: char) {
        let mut buf = [0; 4];
        self.put(c.encode_utf8(&mut buf));
    }

    fn newline(&mut self) {
        let eol = self.eol;
        self.put(eol);
        self.col = 0;
    }

    fn indent(&mut self, depth: usize) {
        match self.options.indent {
            Indent::Spaces(n) => self.pad(' ', n * depth),
            Indent::Tab => self.pad('\t', depth),
        }
    }

    /// Start a new line at `depth`, after a blank line if `blank`.
    fn line(&mut self, depth: usize, blank: bool) {
        self.newline();
        if blank {
            self.newline();
        }
        self.indent(depth);
    }

    fn document(&mut self, t: &Tree<'_>, doc: &Document) {
        let mut started = false;
        let mut new_line = false;
        for &c in &doc.leading {
            if new_line {
                self.line(0, c.has(BLANK));
            } else if started {
                self.put(" ");
            }
            self.comment(t, c);
            (started, new_line) = (true, c.has(NL_AFTER));
        }
        if let Some(value) = doc.value {
            if new_line {
                self.line(0, doc.value_blank);
            } else if started {
                self.put(" ");
            }
            self.value(t, value, 0, 0);
            started = true;
        }
        for &c in &doc.trailing {
            if !started {
                started = true;
            } else if c.has(NL_BEFORE) {
                self.line(0, c.has(BLANK));
            } else {
                self.put(" ");
            }
            self.comment(t, c);
        }
        if started {
            self.newline();
        }
    }

    /// `suffix` is the width of what must follow the value on its last line, such as a comma.
    fn value(&mut self, t: &Tree<'_>, value: Value, depth: usize, suffix: usize) {
        let Some(index) = value.container() else {
            self.put(t.atom(Span {
                start: value.start,
                len: value.len,
            }));
            return;
        };
        let c = &t.containers[index];
        if c.flat != NONE
            && (c.members.len == 0 || self.col + c.flat as usize + suffix <= self.options.width)
        {
            self.flat(t, c);
        } else {
            self.broken(t, c, depth);
        }
    }

    /// A value with no comment in it, all on this line.
    fn flat_value(&mut self, t: &Tree<'_>, value: Value) {
        match value.container() {
            Some(index) => self.flat(t, &t.containers[index]),
            None => self.put(t.atom(Span {
                start: value.start,
                len: value.len,
            })),
        }
    }

    fn flat(&mut self, t: &Tree<'_>, c: &Container) {
        let padded = c.object && self.pad && c.members.len > 0;
        self.put_char(if c.object { '{' } else { '[' });
        if padded {
            self.put(" ");
        }
        for (i, m) in t.members_of(c).iter().enumerate() {
            if i > 0 {
                self.put(", ");
            }
            if let Some(key) = m.key() {
                self.put(t.atom(key));
                self.put(": ");
            }
            self.flat_value(t, m.value);
        }
        if padded {
            self.put(" ");
        }
        self.put_char(if c.object { '}' } else { ']' });
    }

    fn broken(&mut self, t: &Tree<'_>, c: &Container, depth: usize) {
        let inner = depth + 1;
        let members = t.members_of(c);
        let comma_last = match self.trailing {
            Trailing::Never => false,
            Trailing::Keep => c.trailing_comma,
            Trailing::Always => true,
        };
        self.put_char(if c.object { '{' } else { '[' });
        for &o in t.list(c.open_trailing) {
            self.put(" ");
            self.comment(t, o);
        }
        let mut extras = t.extras[t.extras.partition_point(|(j, _)| *j < c.members.start)..]
            .iter()
            .peekable();
        let mut first = true;
        for (i, m) in members.iter().enumerate() {
            let at = c.members.start + narrow(i);
            let extra = extras.next_if(|(j, _)| *j == at).map(|(_, e)| *e);
            let mut new_line = true;
            if let Some(extra) = extra {
                for &l in t.list(extra.leading) {
                    if new_line {
                        self.line(inner, l.has(BLANK) && !first);
                    } else {
                        self.put(" ");
                    }
                    self.comment(t, l);
                    (first, new_line) = (false, l.has(NL_AFTER));
                }
            }
            if new_line {
                self.line(inner, m.blank_before() && !first);
            } else {
                self.put(" ");
            }
            first = false;
            let comma = i + 1 < members.len() || comma_last;
            let before_comma = extra.map_or(&[][..], |e| t.list(e.before_comma));
            let mut suffix = usize::from(comma);
            for &b in before_comma.iter().rev() {
                let text = t.comment(b);
                // Only the first line shares the value's line; later suffixes go after it.
                let first = if let Some((first, _)) = text.split_once(['\n', '\r']) {
                    suffix = 0;
                    first
                } else {
                    text
                };
                suffix += 1 + first
                    .chars()
                    .map(|c| if c == '\t' { 4 } else { 1 })
                    .sum::<usize>();
            }
            self.member(t, m, extra, inner, suffix);
            for &b in before_comma {
                self.put(" ");
                self.comment(t, b);
            }
            if comma {
                self.put(",");
            }
            if let Some(extra) = extra {
                for &tr in t.list(extra.trailing) {
                    self.put(" ");
                    self.comment(t, tr);
                }
            }
        }
        for &tl in t.list(c.tail) {
            self.line(inner, tl.has(BLANK) && !first);
            self.comment(t, tl);
            first = false;
        }
        self.line(depth, false);
        self.put_char(if c.object { '}' } else { ']' });
    }

    /// A key, its colon and its value, starting at the current position on a line at `depth`.
    fn member(
        &mut self,
        t: &Tree<'_>,
        m: &Member,
        extra: Option<MemberComments>,
        depth: usize,
        suffix: usize,
    ) {
        let mut value_depth = depth;
        if let Some(key) = m.key() {
            self.put(t.atom(key));
            if let Some(extra) = extra {
                for &b in t.list(extra.before_colon) {
                    self.put(" ");
                    self.comment(t, b);
                }
            }
            self.put(":");
            let after = extra.map_or(&[][..], |e| t.list(e.after_colon));
            let mut new_line = false;
            for (i, &a) in after.iter().enumerate() {
                if new_line {
                    self.line(depth + 1, false);
                    value_depth = depth + 1;
                } else {
                    self.put(" ");
                }
                self.comment(t, a);
                new_line = a.has(LINE) || (i + 1 == after.len() && a.has(NL_AFTER));
            }
            if new_line {
                self.line(depth + 1, false);
                value_depth = depth + 1;
            } else {
                self.put(" ");
            }
        }
        self.value(t, m.value, value_depth, suffix);
    }
}
