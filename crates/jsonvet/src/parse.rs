//! The parser: one pass over the bytes, a stack of open containers instead of recursion, and
//! each dialect rule applied where the syntax it governs appears.

use std::borrow::Cow;
use std::cell::OnceCell;
use std::collections::HashMap;

use crate::event::{Event, EventKind};
use crate::path::{self, Segment};
use crate::problem::{Detail, SyntaxError, SyntaxKind};
use crate::unicode::{
    is_high_surrogate, is_identifier_part, is_identifier_start, is_json5_space, is_low_surrogate,
    is_noncharacter, is_surrogate, line_separator_at,
};
use crate::{Config, Dialect, Problem, Rule, Severity};

/// Past this many keys, an object's duplicate check switches from a linear scan to a hash map.
const LINEAR_KEYS: usize = 16;

/// The largest integer a double holds exactly: 2⁵³ − 1.
const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// `MAX_SAFE_INTEGER` has 16 decimal digits, so an integer with more significant digits is past
/// it without parsing.
const SAFE_DECIMAL_DIGITS: usize = 16;

/// `MAX_SAFE_INTEGER` has 14 hex digits (`1FFFFFFFFFFFFF`), so a hex integer with more
/// significant digits is past it without parsing.
const SAFE_HEX_DIGITS: usize = 14;

/// A double rounds to infinity from 2¹⁰²⁴ − 2⁹⁷⁰: more than 256 hex digits, or 256 that start
/// with thirteen `F`s and a digit from `C` up.
const DOUBLE_HEX_DIGITS: usize = 256;

/// Check `text`; with `events`, also list its values (see [`Event`]) up to any syntax error.
pub(crate) fn check<'a>(
    text: &'a str,
    config: &Config,
    events: bool,
) -> (Vec<Problem>, Vec<Event<'a>>) {
    let dialect = config.dialect();
    let mut parser = Parser {
        text,
        bytes: text.as_bytes(),
        pos: 0,
        dialect,
        json5: dialect.json5(),
        json_lines: dialect == Dialect::JsonLines,
        duplicate_key: config.severity(Rule::DuplicateKey),
        invalid_number: config.severity(Rule::InvalidNumber),
        invalid_code_point: config.severity(Rule::InvalidCodePoint),
        max_depth: config.max_depth,
        problems: Vec::new(),
        stack: Vec::new(),
        events: events.then(Vec::new),
    };
    parser.byte_order_mark(config.severity(Rule::ByteOrderMark));
    let outcome = if parser.json_lines {
        parser.json_lines(config.severity(Rule::MissingFinalNewline))
    } else {
        parser.document()
    };
    if let Err(fail) = outcome {
        parser.problems.push(Problem {
            offset: fail.offset,
            severity: Severity::Error,
            detail: Detail::Syntax(SyntaxError {
                kind: fail.kind,
                started: fail.started,
            }),
        });
    }
    (parser.problems, parser.events.unwrap_or_default())
}

/// A decoded key as WTF-8: UTF-8, except that a lone surrogate escape such as `\\uD800` is kept
/// as its own three bytes (`ED A0 80`). Valid UTF-8 never holds those bytes, so two keys that
/// differ only in their lone surrogates stay different, as they are in Python.
pub(crate) type Key<'a> = Cow<'a, [u8]>;

/// A syntax error on its way out of the parser.
struct Fail {
    offset: usize,
    kind: SyntaxKind,
    started: Option<usize>,
}

impl Fail {
    fn at(offset: usize, kind: SyntaxKind) -> Fail {
        Fail {
            offset,
            kind,
            started: None,
        }
    }

    /// An error inside a string or comment, which the report also points back to the start of.
    fn inside(offset: usize, kind: SyntaxKind, started: usize) -> Fail {
        Fail {
            offset,
            kind,
            started: Some(started),
        }
    }
}

struct Parser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    pos: usize,
    dialect: Dialect,
    json5: bool,
    /// JSON Lines: a newline ends the document instead of being whitespace.
    json_lines: bool,
    duplicate_key: Severity,
    invalid_number: Severity,
    invalid_code_point: Severity,
    max_depth: usize,
    problems: Vec<Problem>,
    stack: Vec<Frame<'a>>,
    /// The values seen so far, when the caller asked for them.
    events: Option<Vec<Event<'a>>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Container {
    Object,
    Array,
}

/// An open object or array.
struct Frame<'a> {
    container: Container,
    /// How this container is reached from its parent.
    segment: Segment<'a>,
    /// `segment` as a path shows it, made on the first duplicate inside the container.
    rendered: OnceCell<String>,
    /// The keys seen so far with the offset of each, for objects.
    seen_keys: Vec<(Key<'a>, usize)>,
    /// The same keys, indexed once the object grows past `LINEAR_KEYS`.
    seen_index: Option<HashMap<Key<'a>, usize>>,
    /// The current member's key, in an object.
    current_key: Key<'a>,
    /// The current element's position, in an array.
    element_index: usize,
}

impl<'a> Frame<'a> {
    fn rendered(&self) -> &str {
        self.rendered.get_or_init(|| path::rendered(&self.segment))
    }

    fn first_offset(&self, key: &[u8]) -> Option<usize> {
        match &self.seen_index {
            Some(index) => index.get(key).copied(),
            None => self
                .seen_keys
                .iter()
                .find(|(k, _)| **k == *key)
                .map(|(_, at)| *at),
        }
    }

    fn insert(&mut self, key: Key<'a>, at: usize) {
        if let Some(index) = &mut self.seen_index {
            index.insert(key, at);
            return;
        }
        self.seen_keys.push((key, at));
        if self.seen_keys.len() > LINEAR_KEYS {
            self.seen_index = Some(self.seen_keys.drain(..).collect());
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    /// A value must come next.
    Value,
    /// Just after `[`: a value or `]`.
    ArrayFirst,
    /// Just after `,` in an array: a value, or `]` where trailing commas are allowed.
    ArrayNext,
    /// Just after `{`: a key or `}`.
    ObjectFirst,
    /// Just after `,` in an object: a key, or `}` where trailing commas are allowed.
    ObjectNext,
    /// After a key: `:`.
    Colon,
    /// After a value inside a container: `,` or the closing bracket.
    AfterValue,
    /// The document's value is complete.
    Done,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    /// In JSON Lines, the length of the line ending at the current position: 1 for `\n`, 2 for
    /// `\r\n`. A CR before the LF belongs to the line ending, so positions do not depend on
    /// which one a file uses.
    fn line_end(&self) -> Option<usize> {
        self.line_end_at(self.pos)
    }

    fn line_end_at(&self, at: usize) -> Option<usize> {
        if !self.json_lines {
            return None;
        }
        match self.bytes.get(at..)? {
            [b'\n', ..] => Some(1),
            [b'\r', b'\n', ..] => Some(2),
            _ => None,
        }
    }

    fn char_at(&self, at: usize) -> char {
        self.text
            .get(at..)
            .and_then(|s| s.chars().next())
            .unwrap_or('\u{FFFD}')
    }

    fn fail(&self, kind: SyntaxKind) -> Fail {
        Fail::at(self.pos, kind)
    }

    /// The error for whatever sits at the current position when `expected` was needed.
    fn unexpected(&self, expected: &'static str) -> Fail {
        self.unexpected_at(self.pos, expected)
    }

    fn unexpected_at(&self, at: usize, expected: &'static str) -> Fail {
        if at >= self.bytes.len() {
            Fail::at(at, SyntaxKind::UnexpectedEnd { expected })
        } else if self.line_end_at(at).is_some() {
            Fail::at(at, SyntaxKind::UnexpectedLineEnd { expected })
        } else {
            Fail::at(
                at,
                SyntaxKind::Unexpected {
                    found: self.char_at(at),
                    expected,
                },
            )
        }
    }

    fn not_allowed(&self, at: usize, what: &'static str) -> Fail {
        Fail::at(
            at,
            SyntaxKind::NotAllowed {
                what,
                dialect: self.dialect,
            },
        )
    }

    fn emit(&mut self, offset: usize, kind: EventKind<'a>) {
        if let Some(events) = &mut self.events {
            events.push(Event { offset, kind });
        }
    }

    fn report(&mut self, offset: usize, severity: Severity, detail: Detail) {
        if severity.is_on() {
            self.problems.push(Problem {
                offset,
                severity,
                detail,
            });
        }
    }

    fn byte_order_mark(&mut self, severity: Severity) {
        if self.text.starts_with('\u{FEFF}') {
            self.report(0, severity, Detail::ByteOrderMark);
            self.pos = '\u{FEFF}'.len_utf8();
        }
    }

    // --- documents -------------------------------------------------------------------------

    /// One document. A document only ends with every container closed, so in JSON Lines each
    /// line starts with an empty stack.
    fn document(&mut self) -> Result<(), Fail> {
        let mut state = State::Value;
        loop {
            self.trivia()?;
            state = match state {
                State::ArrayFirst | State::ArrayNext if self.peek() == Some(b']') => {
                    self.close(']', state == State::ArrayNext)?
                }
                State::Value | State::ArrayFirst | State::ArrayNext => self.value()?,
                State::ObjectFirst | State::ObjectNext if self.peek() == Some(b'}') => {
                    self.close('}', state == State::ObjectNext)?
                }
                State::ObjectFirst | State::ObjectNext => {
                    self.key()?;
                    State::Colon
                }
                State::Colon if self.peek() == Some(b':') => {
                    self.pos += 1;
                    State::Value
                }
                State::Colon => return Err(self.unexpected("`:`")),
                State::AfterValue => self.after_value()?,
                State::Done => {
                    return match self.peek() {
                        None => Ok(()),
                        Some(_) if self.line_end().is_some() => Ok(()),
                        Some(_) if self.json_lines => Err(self.unexpected("the end of the line")),
                        Some(_) => Err(self.unexpected("the end of the file")),
                    };
                }
            };
        }
    }

    /// The state after a complete value.
    fn state_after_value(&self) -> State {
        if self.stack.is_empty() {
            State::Done
        } else {
            State::AfterValue
        }
    }

    fn after_value(&mut self) -> Result<State, Fail> {
        let object = self
            .stack
            .last()
            .is_some_and(|f| f.container == Container::Object);
        match self.peek() {
            Some(b',') => {
                self.pos += 1;
                if let Some(frame) = self.stack.last_mut() {
                    frame.element_index += 1;
                }
                Ok(if object {
                    State::ObjectNext
                } else {
                    State::ArrayNext
                })
            }
            Some(b'}') if object => self.close('}', false),
            Some(b']') if !object => self.close(']', false),
            _ => Err(self.unexpected(if object { "`,` or `}`" } else { "`,` or `]`" })),
        }
    }

    fn close(&mut self, close: char, after_comma: bool) -> Result<State, Fail> {
        if after_comma && !self.dialect.trailing_commas() {
            return Err(self.fail(SyntaxKind::TrailingComma {
                close,
                dialect: self.dialect,
            }));
        }
        self.emit(self.pos, EventKind::End);
        self.pos += 1;
        self.stack.pop();
        Ok(self.state_after_value())
    }

    fn open(&mut self, container: Container) -> Result<State, Fail> {
        if self.stack.len() >= self.max_depth {
            let limit = self.max_depth;
            return Err(Fail::at(self.pos, SyntaxKind::TooDeep { limit }));
        }
        let segment = match self.stack.last() {
            None => Segment::Root,
            Some(parent) => match parent.container {
                Container::Object => Segment::Key(parent.current_key.clone()),
                Container::Array => Segment::Index(parent.element_index),
            },
        };
        self.stack.push(Frame {
            container,
            segment,
            rendered: OnceCell::new(),
            seen_keys: Vec::new(),
            seen_index: None,
            current_key: Key::Borrowed(&[]),
            element_index: 0,
        });
        let kind = match container {
            Container::Object => EventKind::ObjectStart,
            Container::Array => EventKind::ArrayStart,
        };
        self.emit(self.pos, kind);
        self.pos += 1;
        Ok(match container {
            Container::Object => State::ObjectFirst,
            Container::Array => State::ArrayFirst,
        })
    }

    fn value(&mut self) -> Result<State, Fail> {
        let start = self.pos;
        let Some(b) = self.peek() else {
            return Err(self.unexpected("a value"));
        };
        let kind = match b {
            b'{' => return self.open(Container::Object),
            b'[' => return self.open(Container::Array),
            b'"' | b'\'' if b == b'"' || self.json5 => {
                // Decoded only when events are wanted; scanning is enough otherwise.
                let value = self.string(b, self.events.is_some())?;
                match value {
                    Some(value) => EventKind::String(path::text(value)),
                    None => return Ok(self.state_after_value()),
                }
            }
            b'\'' => return Err(self.not_allowed(self.pos, "single-quoted strings are")),
            b'-' | b'+' | b'.' | b'0'..=b'9' | b'I' | b'N' => {
                self.number()?;
                EventKind::Number(&self.text[start..self.pos])
            }
            b't' => {
                self.literal("`true`")?;
                EventKind::Bool(true)
            }
            b'f' => {
                self.literal("`false`")?;
                EventKind::Bool(false)
            }
            b'n' => {
                self.literal("`null`")?;
                EventKind::Null
            }
            _ => return Err(self.unexpected("a value")),
        };
        self.emit(start, kind);
        Ok(self.state_after_value())
    }

    fn json_lines(&mut self, final_newline: Severity) -> Result<(), Fail> {
        let len = self.bytes.len();
        while self.pos < len {
            let line_start = self.pos;
            self.trivia()?;
            match self.peek() {
                None => return Err(Fail::at(line_start, SyntaxKind::BlankLine)),
                Some(_) if self.line_end().is_some() => {
                    return Err(Fail::at(line_start, SyntaxKind::BlankLine));
                }
                Some(_) => self.document()?,
            }
            if let Some(len) = self.line_end() {
                self.pos += len;
            } else {
                self.report(len, final_newline, Detail::MissingFinalNewline);
            }
        }
        Ok(())
    }

    // --- keys and duplicates ---------------------------------------------------------------

    fn key(&mut self) -> Result<(), Fail> {
        let start = self.pos;
        // Without the duplicate check, quoted keys are only scanned: nothing reads their value.
        let track_keys = self.duplicate_key.is_on();
        let key = self
            .read_key(track_keys || self.events.is_some())?
            .unwrap_or(Key::Borrowed(&[]));
        // Only duplicate-key paths read a key after this, so without the check it moves into
        // its event uncopied.
        if !track_keys {
            if self.events.is_some() {
                self.emit(start, EventKind::Key(key));
            }
            return Ok(());
        }
        if self.events.is_some() {
            self.emit(start, EventKind::Key(key.clone()));
        }
        self.check_duplicate(start, &key);
        if let Some(frame) = self.stack.last_mut() {
            frame.current_key = key;
        }
        Ok(())
    }

    /// Read the key at the current position. A quoted key comes back as `None` unless `decode`
    /// is set; an unquoted one is always decoded.
    fn read_key(&mut self, decode: bool) -> Result<Option<Key<'a>>, Fail> {
        let start = self.pos;
        match self.peek() {
            Some(b'"') => self.string(b'"', decode),
            Some(b'\'') if self.json5 => self.string(b'\'', decode),
            Some(b'\'') => Err(self.not_allowed(start, "single-quoted strings are")),
            Some(_) if self.identifier_start(start) => {
                if !self.json5 {
                    return Err(self.not_allowed(start, "unquoted keys are"));
                }
                self.identifier().map(Some)
            }
            _ => Err(self.unexpected("a key")),
        }
    }

    /// Report `key`, found at `start`, if the current object already has it; record it if not.
    fn check_duplicate(&mut self, start: usize, key: &Key<'a>) {
        let Some(frame) = self.stack.last_mut() else {
            return;
        };
        match frame.first_offset(key) {
            None => frame.insert(key.clone(), start),
            Some(first) => {
                let mut last = String::new();
                path::push_key(&mut last, key);
                let stack = &self.stack;
                let path = path::bounded(stack.len() + 1, |i| {
                    stack.get(i).map_or(last.as_str(), Frame::rendered)
                });
                self.report(
                    start,
                    self.duplicate_key,
                    Detail::DuplicateKey { first, path },
                );
            }
        }
    }

    fn identifier_start(&self, at: usize) -> bool {
        let c = self.char_at(at);
        c == '\\' || is_identifier_start(c)
    }

    /// A JSON5 identifier key, decoded.
    fn identifier(&mut self) -> Result<Key<'a>, Fail> {
        let start = self.pos;
        // Only a key with an escape needs its own copy; the rest borrow from the text.
        let mut decoded: Option<String> = None;
        while self.pos < self.bytes.len() {
            let escaped = self.bytes[self.pos] == b'\\';
            let (c, len) = if escaped {
                if self.bytes.get(self.pos + 1) != Some(&b'u') {
                    return Err(self.fail(SyntaxKind::InvalidEscape {
                        found: self.char_at(self.pos + 1),
                    }));
                }
                let Some(unit) = self.hex(self.pos + 2, 4) else {
                    return Err(self.fail(SyntaxKind::InvalidUnicodeEscape));
                };
                (char::from_u32(unit).unwrap_or('\u{FFFD}'), 6)
            } else {
                let c = self.char_at(self.pos);
                (c, c.len_utf8())
            };
            let fits = if self.pos == start {
                is_identifier_start(c)
            } else {
                is_identifier_part(c)
            };
            if !fits {
                if self.pos == start {
                    return Err(self.unexpected("a key"));
                }
                break;
            }
            if let Some(buffer) = &mut decoded {
                buffer.push(c);
            } else if escaped {
                let mut buffer = self.text[start..self.pos].to_owned();
                buffer.push(c);
                decoded = Some(buffer);
            }
            self.pos += len;
        }
        Ok(match decoded {
            Some(buffer) => Cow::Owned(buffer.into_bytes()),
            None => Cow::Borrowed(&self.bytes[start..self.pos]),
        })
    }

    // --- strings ---------------------------------------------------------------------------

    /// Scan a string starting at its quote. With `decode`, return its value; without, `None`.
    fn string(&mut self, quote: u8, decode: bool) -> Result<Option<Key<'a>>, Fail> {
        let start = self.pos;
        self.pos += 1;
        let mut decoded: Option<Vec<u8>> = None;
        let mut run_start = self.pos;
        loop {
            let Some(b) = self.peek() else {
                return Err(Fail::inside(
                    self.pos,
                    SyntaxKind::UnterminatedString,
                    start,
                ));
            };
            match b {
                _ if b == quote => {
                    let value = match decoded {
                        Some(mut buffer) => {
                            buffer.extend_from_slice(&self.bytes[run_start..self.pos]);
                            Some(Cow::Owned(buffer))
                        }
                        None if decode => Some(Cow::Borrowed(&self.bytes[start + 1..self.pos])),
                        None => None,
                    };
                    self.pos += 1;
                    return Ok(value);
                }
                b'\\' => {
                    let buffer = decode.then(|| {
                        let buffer = decoded.get_or_insert_with(Vec::new);
                        buffer.extend_from_slice(&self.bytes[run_start..self.pos]);
                        buffer
                    });
                    let code_point = self.escape(start)?;
                    if let (Some(buffer), Some(code_point)) = (buffer, code_point) {
                        push_wtf8(buffer, code_point);
                    }
                    run_start = self.pos;
                }
                b'\n' | b'\r' => {
                    return Err(Fail::inside(self.pos, SyntaxKind::LineBreakInString, start));
                }
                0..0x20 if !self.json5 => {
                    return Err(self.fail(SyntaxKind::ControlCharacter {
                        found: char::from(b),
                    }));
                }
                0x80.. if self.invalid_code_point.is_on() => {
                    let c = self.char_at(self.pos);
                    if is_noncharacter(u32::from(c)) {
                        self.report(
                            self.pos,
                            self.invalid_code_point,
                            Detail::InvalidCodePoint {
                                code_point: u32::from(c),
                            },
                        );
                    }
                    self.pos += c.len_utf8();
                }
                _ => self.pos += 1,
            }
        }
    }

    /// Read the escape at the current backslash and return the code point it stands for (a lone
    /// surrogate included), or `None` for a line continuation.
    fn escape(&mut self, string_start: usize) -> Result<Option<u32>, Fail> {
        let at = self.pos;
        let Some(b) = self.bytes.get(at + 1).copied() else {
            return Err(Fail::inside(
                self.bytes.len(),
                SyntaxKind::UnterminatedString,
                string_start,
            ));
        };
        self.pos += 2;
        let simple = match b {
            b'"' => Some('"'),
            b'\\' => Some('\\'),
            b'/' => Some('/'),
            b'b' => Some('\u{8}'),
            b'f' => Some('\u{c}'),
            b'n' => Some('\n'),
            b'r' => Some('\r'),
            b't' => Some('\t'),
            _ => None,
        };
        if let Some(c) = simple {
            return Ok(Some(u32::from(c)));
        }
        if b == b'u' {
            return self.unicode_escape(at).map(Some);
        }
        if !self.json5 {
            return Err(Fail::at(
                at,
                SyntaxKind::InvalidEscape {
                    found: self.char_at(at + 1),
                },
            ));
        }
        match b {
            b'\'' => Ok(Some(u32::from('\''))),
            b'v' => Ok(Some(0xB)),
            // `\0` may not be followed by a digit: JSON5 has no octal escapes.
            b'0' if self.peek().is_some_and(|d| d.is_ascii_digit()) => {
                Err(Fail::at(at, SyntaxKind::InvalidEscape { found: '0' }))
            }
            b'0' => Ok(Some(0)),
            b'1'..=b'9' => Err(Fail::at(
                at,
                SyntaxKind::InvalidEscape {
                    found: char::from(b),
                },
            )),
            b'x' => {
                let Some(value) = self.hex(self.pos, 2) else {
                    return Err(Fail::at(at, SyntaxKind::InvalidHexEscape));
                };
                self.pos += 2;
                Ok(Some(value))
            }
            // Line continuations: the backslash and the line break vanish.
            b'\n' => Ok(None),
            b'\r' => {
                if self.peek() == Some(b'\n') {
                    self.pos += 1;
                }
                Ok(None)
            }
            _ => {
                let c = self.char_at(at + 1);
                self.pos = at + 1 + c.len_utf8();
                Ok(if c == '\u{2028}' || c == '\u{2029}' {
                    None
                } else {
                    Some(u32::from(c))
                })
            }
        }
    }

    /// The `\uXXXX` escape whose backslash is at `at`, with the current position just after
    /// the `u`. Joins a surrogate pair written as two escapes; a lone surrogate comes back as it is.
    fn unicode_escape(&mut self, at: usize) -> Result<u32, Fail> {
        let Some(unit) = self.hex(self.pos, 4) else {
            return Err(Fail::at(at, SyntaxKind::InvalidUnicodeEscape));
        };
        self.pos += 4;
        let code_point = if is_high_surrogate(unit)
            && self.bytes.get(self.pos..self.pos + 2) == Some(b"\\u")
            && let Some(low) = self.hex(self.pos + 2, 4)
            && is_low_surrogate(low)
        {
            self.pos += 6;
            0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00)
        } else {
            unit
        };
        if is_surrogate(code_point) || is_noncharacter(code_point) {
            self.report(
                at,
                self.invalid_code_point,
                Detail::InvalidCodePoint { code_point },
            );
        }
        Ok(code_point)
    }

    /// The value of `digits` hex digits at `at`, if they are all there.
    fn hex(&self, at: usize, digits: usize) -> Option<u32> {
        let text = self.text.get(at..at + digits)?;
        if !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        u32::from_str_radix(text, 16).ok()
    }

    // --- numbers and literals --------------------------------------------------------------

    fn number(&mut self) -> Result<(), Fail> {
        let start = self.pos;
        match self.peek() {
            Some(b'+') if !self.json5 => return Err(self.not_allowed(start, "this number form is")),
            Some(b'+' | b'-') => self.pos += 1,
            _ => {}
        }
        match self.peek() {
            Some(b'I') => self.nan_or_infinity(start, "`Infinity`"),
            Some(b'N') => self.nan_or_infinity(start, "`NaN`"),
            Some(b'0') if matches!(self.bytes.get(self.pos + 1), Some(b'x' | b'X')) => {
                self.hex_number(start)
            }
            _ => self.decimal_number(start),
        }
    }

    /// The rest of a JSON5 hex number such as `-0x1F`, from its `0x`. `start` is at the sign.
    fn hex_number(&mut self, start: usize) -> Result<(), Fail> {
        if !self.json5 {
            return Err(self.not_allowed(start, "this number form is"));
        }
        self.pos += 2;
        let hex_start = self.pos;
        while self.peek().is_some_and(|b| b.is_ascii_hexdigit()) {
            self.pos += 1;
        }
        if self.pos == hex_start {
            return Err(self.unexpected("a hex digit"));
        }
        self.check_hex(start, hex_start);
        Ok(())
    }

    /// The rest of a decimal number such as `-1.5e3`, after its sign. `start` is at the sign.
    fn decimal_number(&mut self, start: usize) -> Result<(), Fail> {
        let int_start = self.pos;
        let int_digits = if self.peek() == Some(b'0') {
            self.pos += 1;
            true
        } else {
            self.skip_digits()
        };
        if self.peek() == Some(b'.') {
            let fraction_digits = self.bytes.get(self.pos + 1).is_some_and(u8::is_ascii_digit);
            if !int_digits && !fraction_digits {
                // A point with no digit on either side, such as `-.`, is no value at all.
                return Err(self.unexpected_at(int_start, "a value"));
            }
            if !self.json5 && int_digits != fraction_digits {
                // Only JSON5 allows a point with digits on one side alone, as in `.5` and `5.`.
                return Err(self.not_allowed(start, "this number form is"));
            }
            self.pos += 1;
            self.skip_digits();
        } else if !int_digits {
            return Err(self.unexpected("a digit"));
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !self.skip_digits() {
                return Err(self.unexpected("a digit"));
            }
        }
        self.check_decimal(start);
        Ok(())
    }

    /// Skip a run of decimal digits; true if there was at least one.
    fn skip_digits(&mut self) -> bool {
        let start = self.pos;
        while self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.pos += 1;
        }
        self.pos > start
    }

    /// `NaN` or `Infinity`, which only JSON5 allows. `start` is at the sign, if there is one.
    fn nan_or_infinity(&mut self, start: usize, word: &'static str) -> Result<(), Fail> {
        if !self.json5 {
            return Err(
                if self.text[self.pos..].starts_with(word.trim_matches('`')) {
                    self.not_allowed(start, "NaN and Infinity are")
                } else {
                    self.unexpected("a value")
                },
            );
        }
        self.literal(word)
    }

    /// A literal such as `null`. `word` comes in backticks, as an error message shows it.
    fn literal(&mut self, word: &'static str) -> Result<(), Fail> {
        for &expected in word.trim_matches('`').as_bytes() {
            if self.peek() != Some(expected) {
                return Err(self.unexpected(word));
            }
            self.pos += 1;
        }
        Ok(())
    }

    fn check_decimal(&mut self, start: usize) {
        if !self.invalid_number.is_on() {
            return;
        }
        let text = &self.text[start..self.pos];
        let overflows = text.parse::<f64>().is_ok_and(f64::is_infinite);
        let integer = !text.contains(['.', 'e', 'E']);
        let digits = text.trim_start_matches(['+', '-']).trim_start_matches('0');
        let unsafe_integer = integer
            && (digits.len() > SAFE_DECIMAL_DIGITS
                || digits.parse::<u64>().is_ok_and(|n| n > MAX_SAFE_INTEGER));
        if overflows || unsafe_integer {
            self.report(
                start,
                self.invalid_number,
                Detail::InvalidNumber { overflows },
            );
        }
    }

    fn check_hex(&mut self, start: usize, hex_start: usize) {
        if !self.invalid_number.is_on() {
            return;
        }
        let significant = self.text[hex_start..self.pos].trim_start_matches('0');
        let top = significant
            .as_bytes()
            .get(..14)
            .map(<[u8]>::to_ascii_lowercase);
        let overflows = significant.len() > DOUBLE_HEX_DIGITS
            || (significant.len() == DOUBLE_HEX_DIGITS
                && top.is_some_and(|top| top[..13].iter().all(|&b| b == b'f') && top[13] >= b'c'));
        let unsafe_integer = significant.len() > SAFE_HEX_DIGITS
            || u64::from_str_radix(significant, 16).is_ok_and(|n| n > MAX_SAFE_INTEGER);
        if overflows || unsafe_integer {
            self.report(
                start,
                self.invalid_number,
                Detail::InvalidNumber { overflows },
            );
        }
    }

    // --- whitespace and comments -----------------------------------------------------------

    fn trivia(&mut self) -> Result<(), Fail> {
        loop {
            let Some(b) = self.peek() else { return Ok(()) };
            match b {
                b'\n' | b'\r' if self.line_end().is_some() => return Ok(()),
                b' ' | b'\t' | b'\r' | b'\n' => self.pos += 1,
                b'/' => match self.bytes.get(self.pos + 1) {
                    Some(b'/' | b'*') if !self.dialect.comments() => {
                        return Err(self.not_allowed(self.pos, "comments are"));
                    }
                    Some(b'/') => self.line_comment(),
                    Some(b'*') => self.block_comment()?,
                    _ => return Ok(()),
                },
                0x0B | 0x0C if self.json5 => self.pos += 1,
                0x80.. if self.json5 => {
                    let c = self.char_at(self.pos);
                    if !is_json5_space(c) {
                        return Ok(());
                    }
                    self.pos += c.len_utf8();
                }
                _ => return Ok(()),
            }
        }
    }

    /// Skip to the line terminator that ends a `//` comment, leaving it in place.
    fn line_comment(&mut self) {
        let mut from = self.pos + 2;
        if self.dialect.line_comment_ends_only_at_lf() {
            self.pos =
                memchr::memchr(b'\n', &self.bytes[from..]).map_or(self.bytes.len(), |n| from + n);
            return;
        }
        while let Some(found) = memchr::memchr3(b'\n', b'\r', 0xE2, &self.bytes[from..]) {
            let at = from + found;
            let separator =
                self.bytes[at] != 0xE2 || (self.json5 && line_separator_at(self.bytes, at));
            if separator {
                self.pos = at;
                return;
            }
            from = at + 1;
        }
        self.pos = self.bytes.len();
    }

    fn block_comment(&mut self) -> Result<(), Fail> {
        let start = self.pos;
        let mut from = self.pos + 2;
        while let Some(found) = memchr::memchr(b'*', &self.bytes[from..]) {
            let at = from + found;
            if self.bytes.get(at + 1) == Some(&b'/') {
                self.pos = at + 2;
                return Ok(());
            }
            from = at + 1;
        }
        Err(Fail::inside(
            self.bytes.len(),
            SyntaxKind::UnterminatedComment,
            start,
        ))
    }
}

/// Append a code point as WTF-8 (see [`Key`]): UTF-8, or three bytes for a lone surrogate.
fn push_wtf8(buffer: &mut Vec<u8>, code_point: u32) {
    match char::from_u32(code_point) {
        Some(c) => buffer.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()),
        None => buffer.extend_from_slice(&[
            0xED,
            0x80 | ((code_point >> 6) & 0x3F) as u8,
            0x80 | (code_point & 0x3F) as u8,
        ]),
    }
}
