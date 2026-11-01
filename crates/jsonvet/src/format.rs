//! The formatter behind `jsonvet fmt`.
//!
//! It runs after a normal check has found no syntax error, so it reads its own token stream and
//! never has to report one. Comments are attached to the token they sit next to, then the layout
//! is computed from the tree alone: it does not depend on where the source broke its lines, which
//! is what makes a second run change nothing.

use std::borrow::Cow;

use crate::{Config, Dialect, Report, Rule, check};

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
}

impl Default for FormatOptions {
    fn default() -> Self {
        FormatOptions {
            indent: Indent::Spaces(2),
            width: 80,
            normalize_json5: false,
        }
    }
}

/// Format one text. Comments, strings and numbers stay as written. A text with a syntax error
/// comes back as `Err` with its report, and nothing is formatted.
///
/// ```
/// use jsonvet::{format, Config, Dialect, FormatOptions};
///
/// let text = "{\"a\":[1,2], // c\n\"b\":{}}";
/// let out = format(text, &Config::new(Dialect::Jsonc), &FormatOptions::default()).unwrap();
/// assert_eq!(out, "{\n  \"a\": [1, 2], // c\n  \"b\": {}\n}\n");
/// ```
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
) -> Result<String, Report<'a>> {
    let report = check(text, config);
    if report.problems().iter().any(|p| p.rule() == Rule::Syntax) {
        return Err(report);
    }
    let dialect = config.dialect();
    let crlf = text.find('\n').is_some_and(|i| text[..i].ends_with('\r'));
    let eol = if crlf { "\r\n" } else { "\n" };
    let (bom, body) = match text.strip_prefix('\u{feff}') {
        Some(body) => ("\u{feff}", body),
        None => ("", text),
    };
    let mut w = Writer {
        out: String::with_capacity(text.len() + text.len() / 8),
        eol,
        options,
        norm: options.normalize_json5 && dialect == Dialect::Json5,
        trailing_commas: dialect.trailing_commas(),
    };
    if dialect == Dialect::JsonLines {
        for line in body.split('\n') {
            let line = line.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
            if line.is_empty() {
                continue;
            }
            let doc = Parser::new(lex(line, Dialect::Json), false).document();
            if let Some(value) = &doc.value {
                w.flat_value(value);
                w.out.push_str(eol);
            }
        }
    } else {
        let doc = Parser::new(lex(body, dialect), w.norm).document();
        w.document(&doc);
    }
    Ok(format!("{bom}{}", w.out))
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

struct Tok<'a> {
    kind: Kind,
    text: &'a str,
    /// Line breaks in the white space before the token.
    nl: usize,
}

/// Split valid text into tokens. Strings, numbers, literals and bare keys are all atoms. On text
/// that is not valid it still ends, with tokens the parser ignores.
fn lex(text: &str, dialect: Dialect) -> Vec<Tok<'_>> {
    let bytes = text.as_bytes();
    let mut toks = Vec::new();
    let (mut i, mut nl) = (0, 0);
    while i < bytes.len() {
        let Some(c) = text[i..].chars().next() else {
            break;
        };
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
        let tok_text = &text[start..i];
        let tok_text = if kind == Kind::Line {
            tok_text.trim_end()
        } else {
            tok_text
        };
        toks.push(Tok {
            kind,
            text: tok_text,
            nl,
        });
        nl = 0;
    }
    toks
}

// ---- tree ----

struct Comment<'a> {
    text: &'a str,
    /// A blank line sits before it.
    blank_before: bool,
    /// A line break sits before it, so it began its own line.
    nl_before: bool,
    /// A line break follows it, or the text ends.
    nl_after: bool,
}

enum Value<'a> {
    Atom(Cow<'a, str>),
    Container(Box<Container<'a>>),
}

impl Comment<'_> {
    fn is_line(&self) -> bool {
        self.text.starts_with("//")
    }
}

struct Container<'a> {
    open: char,
    /// Comments on the line of the opening bracket, after it.
    open_trailing: Vec<Comment<'a>>,
    members: Vec<Member<'a>>,
    /// Comments after the last member, on their own lines.
    tail: Vec<Comment<'a>>,
    /// Whether the last member had a comma after it.
    trailing_comma: bool,
    /// The width on one line, or `None` if a comment inside forces lines.
    flat: Option<usize>,
}

struct Member<'a> {
    /// Comments before the member, each on its own line unless it ends on the member's line.
    leading: Vec<Comment<'a>>,
    /// A blank line sits before the key or element itself.
    blank_before: bool,
    key: Option<Cow<'a, str>>,
    before_colon: Vec<Comment<'a>>,
    after_colon: Vec<Comment<'a>>,
    value: Value<'a>,
    /// Block comments between the value and its comma, on the value's line.
    before_comma: Vec<Comment<'a>>,
    /// Comments after the comma, on the value's line.
    trailing: Vec<Comment<'a>>,
}

struct Document<'a> {
    leading: Vec<Comment<'a>>,
    value_blank: bool,
    value: Option<Value<'a>>,
    trailing: Vec<Comment<'a>>,
}

struct Parser<'a> {
    toks: Vec<Tok<'a>>,
    pos: usize,
    norm: bool,
}

impl<'a> Parser<'a> {
    fn new(toks: Vec<Tok<'a>>, norm: bool) -> Self {
        Parser { toks, pos: 0, norm }
    }

    fn kind(&self) -> Option<Kind> {
        self.toks.get(self.pos).map(|t| t.kind)
    }

    fn comments(&mut self) -> Vec<Comment<'a>> {
        let mut found = Vec::new();
        while let Some(tok) = self.toks.get(self.pos)
            && matches!(tok.kind, Kind::Line | Kind::Block)
        {
            let next_nl = self.toks.get(self.pos + 1).is_none_or(|next| next.nl > 0);
            found.push(Comment {
                text: tok.text,
                blank_before: tok.nl >= 2,
                nl_before: tok.nl > 0,
                nl_after: tok.kind == Kind::Line || next_nl,
            });
            self.pos += 1;
        }
        found
    }

    /// Comments after a token, split into those that stay on its line, ending it, and the rest.
    /// A comment followed by more code on the same line belongs to that code instead.
    fn split_trailing(&self, mut all: Vec<Comment<'a>>) -> (Vec<Comment<'a>>, Vec<Comment<'a>>) {
        let same = all.iter().take_while(|c| !c.nl_before).count();
        let at_close = matches!(self.kind(), Some(Kind::Close) | None);
        let ends_line = same > 0 && (all[same - 1].nl_after || at_close);
        let rest = all.split_off(if ends_line { same } else { 0 });
        (all, rest)
    }

    fn document(&mut self) -> Document<'a> {
        let leading = self.comments();
        let value_blank = self.toks.get(self.pos).is_some_and(|t| t.nl >= 2);
        let value = matches!(self.kind(), Some(Kind::Open | Kind::Atom)).then(|| self.value());
        let trailing = self.comments();
        Document {
            leading,
            value_blank,
            value,
            trailing,
        }
    }

    fn value(&mut self) -> Value<'a> {
        match self.toks.get(self.pos) {
            Some(tok) if tok.kind == Kind::Open => Value::Container(Box::new(self.container())),
            Some(tok) if tok.kind == Kind::Atom => {
                self.pos += 1;
                Value::Atom(if self.norm {
                    double_quoted(tok.text)
                } else {
                    Cow::Borrowed(tok.text)
                })
            }
            _ => Value::Atom(Cow::Borrowed("")),
        }
    }

    fn container(&mut self) -> Container<'a> {
        let open = if self.toks[self.pos].text == "{" {
            '{'
        } else {
            '['
        };
        self.pos += 1;
        let first = self.comments();
        let (open_trailing, mut pending) = self.split_trailing(first);
        let mut members = Vec::new();
        let mut trailing_comma = false;
        loop {
            match self.kind() {
                None => break,
                Some(Kind::Close) => {
                    self.pos += 1;
                    break;
                }
                Some(Kind::Open | Kind::Atom) => {}
                Some(_) => {
                    self.pos += 1;
                    continue;
                }
            }
            let blank_before = self.toks[self.pos].nl >= 2;
            let (mut key, mut before_colon, mut after_colon) = (None, Vec::new(), Vec::new());
            if open == '{' {
                if self.kind() == Some(Kind::Atom) {
                    let text = self.toks[self.pos].text;
                    key = Some(if self.norm {
                        key_text(text)
                    } else {
                        Cow::Borrowed(text)
                    });
                    self.pos += 1;
                }
                before_colon = self.comments();
                if self.kind() == Some(Kind::Colon) {
                    self.pos += 1;
                    after_colon = self.comments();
                }
                // A line comment cannot sit before the colon: it moves to after it.
                if before_colon.iter().any(Comment::is_line) {
                    before_colon.append(&mut after_colon);
                    after_colon = before_colon;
                    before_colon = Vec::new();
                }
            }
            let value = self.value();
            let mut gap = self.comments();
            let inline = gap
                .iter()
                .take_while(|c| !c.is_line() && !c.nl_before)
                .count();
            let mut all = gap.split_off(inline);
            let before_comma = gap;
            let comma = self.kind() == Some(Kind::Comma);
            if comma {
                self.pos += 1;
                let mut after = self.comments();
                // A line comment ends its line, so what follows the comma starts a new one.
                if all.last().is_some_and(Comment::is_line)
                    && let Some(first) = after.first_mut()
                {
                    first.nl_before = true;
                }
                all.append(&mut after);
            }
            let (trailing, rest) = self.split_trailing(all);
            members.push(Member {
                leading: std::mem::replace(&mut pending, rest),
                blank_before,
                key,
                before_colon,
                after_colon,
                value,
                before_comma,
                trailing,
            });
            trailing_comma = comma;
        }
        let flat = flat_width(&open_trailing, &members, &pending);
        Container {
            open,
            open_trailing,
            members,
            tail: pending,
            trailing_comma,
            flat,
        }
    }
}

/// The width of a container on one line, or `None` if any comment sits in it.
fn flat_width(
    open_trailing: &[Comment<'_>],
    members: &[Member<'_>],
    tail: &[Comment<'_>],
) -> Option<usize> {
    if !open_trailing.is_empty() || !tail.is_empty() {
        return None;
    }
    let mut width = 2 + 2 * members.len().saturating_sub(1);
    for m in members {
        if [
            &m.leading,
            &m.before_colon,
            &m.after_colon,
            &m.before_comma,
            &m.trailing,
        ]
        .iter()
        .any(|list| !list.is_empty())
        {
            return None;
        }
        width += m.key.as_ref().map_or(0, |k| k.chars().count() + 2);
        width += match &m.value {
            Value::Atom(text) => text.chars().count(),
            Value::Container(c) => c.flat?,
        };
    }
    Some(width)
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

struct Writer<'o> {
    out: String,
    eol: &'o str,
    options: &'o FormatOptions,
    norm: bool,
    trailing_commas: bool,
}

impl Writer<'_> {
    /// Start a new line at `depth`, after a blank line if `blank`.
    fn line(&mut self, depth: usize, blank: bool) {
        self.out.push_str(self.eol);
        if blank {
            self.out.push_str(self.eol);
        }
        match self.options.indent {
            Indent::Spaces(n) => self.out.extend(std::iter::repeat_n(' ', n * depth)),
            Indent::Tab => self.out.extend(std::iter::repeat_n('\t', depth)),
        }
    }

    fn column(&self) -> usize {
        let line = self.out.rsplit('\n').next().unwrap_or_default();
        line.chars().map(|c| if c == '\t' { 4 } else { 1 }).sum()
    }

    fn document(&mut self, doc: &Document<'_>) {
        let mut started = false;
        let mut new_line = false;
        for c in &doc.leading {
            if new_line {
                self.line(0, c.blank_before);
            } else if started {
                self.out.push(' ');
            }
            self.out.push_str(c.text);
            (started, new_line) = (true, c.nl_after);
        }
        if let Some(value) = &doc.value {
            if new_line {
                self.line(0, doc.value_blank);
            } else if started {
                self.out.push(' ');
            }
            self.value(value, 0, 0);
            started = true;
        }
        for c in &doc.trailing {
            if !started {
                started = true;
            } else if c.nl_before {
                self.line(0, c.blank_before);
            } else {
                self.out.push(' ');
            }
            self.out.push_str(c.text);
        }
        if started {
            self.out.push_str(self.eol);
        }
    }

    /// `suffix` is the width of what must follow the value on its last line, such as a comma.
    fn value(&mut self, value: &Value<'_>, depth: usize, suffix: usize) {
        match value {
            Value::Atom(text) => self.out.push_str(text),
            Value::Container(c) => match c.flat {
                Some(width)
                    if c.members.is_empty()
                        || self.column() + width + suffix <= self.options.width =>
                {
                    self.flat(c);
                }
                _ => self.broken(c, depth),
            },
        }
    }

    /// A value with no comment in it, all on this line.
    fn flat_value(&mut self, value: &Value<'_>) {
        match value {
            Value::Atom(text) => self.out.push_str(text),
            Value::Container(c) => self.flat(c),
        }
    }

    fn flat(&mut self, c: &Container<'_>) {
        self.out.push(c.open);
        for (i, m) in c.members.iter().enumerate() {
            if i > 0 {
                self.out.push_str(", ");
            }
            if let Some(key) = &m.key {
                self.out.push_str(key);
                self.out.push_str(": ");
            }
            self.flat_value(&m.value);
        }
        self.out.push(close(c.open));
    }

    fn broken(&mut self, c: &Container<'_>, depth: usize) {
        let inner = depth + 1;
        let comma_last = self.norm || (c.trailing_comma && self.trailing_commas);
        self.out.push(c.open);
        for t in &c.open_trailing {
            self.out.push(' ');
            self.out.push_str(t.text);
        }
        let mut first = true;
        for (i, m) in c.members.iter().enumerate() {
            let mut new_line = true;
            for l in &m.leading {
                if new_line {
                    self.line(inner, l.blank_before && !first);
                } else {
                    self.out.push(' ');
                }
                self.out.push_str(l.text);
                (first, new_line) = (false, l.nl_after);
            }
            if new_line {
                self.line(inner, m.blank_before && !first);
            } else {
                self.out.push(' ');
            }
            first = false;
            let comma = i + 1 < c.members.len() || comma_last;
            let suffix = usize::from(comma)
                + m.before_comma
                    .iter()
                    .map(|b| 1 + b.text.chars().count())
                    .sum::<usize>();
            self.member(m, inner, suffix);
            for b in &m.before_comma {
                self.out.push(' ');
                self.out.push_str(b.text);
            }
            if comma {
                self.out.push(',');
            }
            for t in &m.trailing {
                self.out.push(' ');
                self.out.push_str(t.text);
            }
        }
        for t in &c.tail {
            self.line(inner, t.blank_before && !first);
            self.out.push_str(t.text);
            first = false;
        }
        self.line(depth, false);
        self.out.push(close(c.open));
    }

    /// A key, its colon and its value, starting at the current position on a line at `depth`.
    fn member(&mut self, m: &Member<'_>, depth: usize, suffix: usize) {
        let mut value_depth = depth;
        if let Some(key) = &m.key {
            self.out.push_str(key);
            for b in &m.before_colon {
                self.out.push(' ');
                self.out.push_str(b.text);
            }
            self.out.push(':');
            let mut new_line = false;
            for (i, a) in m.after_colon.iter().enumerate() {
                if new_line {
                    self.line(depth + 1, false);
                    value_depth = depth + 1;
                } else {
                    self.out.push(' ');
                }
                self.out.push_str(a.text);
                new_line = a.is_line() || (i + 1 == m.after_colon.len() && a.nl_after);
            }
            if new_line {
                self.line(depth + 1, false);
                value_depth = depth + 1;
            } else {
                self.out.push(' ');
            }
        }
        self.value(&m.value, value_depth, suffix);
    }
}

fn close(open: char) -> char {
    if open == '{' { '}' } else { ']' }
}
