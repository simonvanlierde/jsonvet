//! The parser: one pass over the bytes, a stack of open containers instead of recursion, and
//! each dialect rule applied where the syntax it governs appears.

use std::borrow::Cow;
use std::cell::OnceCell;
use std::hash::{BuildHasher, RandomState};

use hashbrown::HashTable;
use hashbrown::hash_table::Entry;

use crate::event::{Event, EventKind};
use crate::messages::{
    COMMENTS_NOT_ALLOWED, Message, NAN_AND_INFINITY_NOT_ALLOWED, NUMBER_FORM_NOT_ALLOWED,
    SINGLE_QUOTES_NOT_ALLOWED, UNQUOTED_KEYS_NOT_ALLOWED,
};
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
    let mut list = Vec::new();
    let mut push = |event| list.push(event);
    let problems = walk(text, 0..text.len(), config, events.then_some(&mut push));
    (problems, list)
}

/// Check the lines of `text` in `lines`, handing each value to `sink` as it is read, up to any
/// syntax error. For JSON Lines, `lines` can start at any line's start and end after a newline
/// or at the end, and the problems are those [`check`] finds there within the whole text:
/// offsets count from the start of `text`, and a byte order mark counts only at its start.
pub(crate) fn walk<'a>(
    text: &'a str,
    lines: std::ops::Range<usize>,
    config: &Config,
    sink: Option<&mut dyn FnMut(Event<'a>)>,
) -> Vec<Problem> {
    let dialect = config.dialect();
    let text = &text[..lines.end];
    let mut parser = Parser {
        text,
        bytes: text.as_bytes(),
        pos: lines.start,
        dialect,
        json5: dialect.json5(),
        json_lines: dialect == Dialect::JsonLines,
        duplicate_key: config.severity(Rule::DuplicateKey),
        invalid_number: config.severity(Rule::InvalidNumber),
        invalid_code_point: config.severity(Rule::InvalidCodePoint),
        max_depth: config.max_depth,
        problems: Vec::new(),
        stack: Vec::new(),
        depth: 0,
        events: sink,
    };
    if lines.start == 0 {
        parser.byte_order_mark(config.severity(Rule::ByteOrderMark));
    }
    let outcome = if parser.json_lines {
        parser.json_lines(config.severity(Rule::MissingFinalNewline))
    } else {
        parser.document()
    };
    if let Err(mut fail) = outcome {
        parser.unclosed(&mut fail);
        let end = parser.syntax_end(&fail);
        parser.problems.push(Problem {
            offset: fail.offset,
            end,
            severity: Severity::Error,
            detail: Detail::Syntax(SyntaxError {
                kind: fail.kind,
                started: fail.started,
            }),
        });
    }
    parser.problems
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

struct Parser<'a, 's> {
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
    /// The open containers are `stack[..depth]`. Frames past `depth` are closed ones kept for
    /// reuse in place, so a container's key list is not moved or allocated anew.
    stack: Vec<Frame<'a>>,
    depth: usize,
    /// Where each value goes as it is read, when the caller asked for them.
    events: Option<&'s mut dyn FnMut(Event<'a>)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Container {
    Object,
    Array,
}

/// An open object or array.
struct Frame<'a> {
    container: Container,
    /// The offset of its `{` or `[`.
    opened: usize,
    /// How this container is reached from its parent.
    segment: Segment<'a>,
    /// `segment` as a path shows it, made on the first duplicate inside the container.
    rendered: OnceCell<String>,
    /// The keys seen so far, for objects.
    keys: Keys<'a>,
    /// The current member's key, in an object.
    current_key: Key<'a>,
    /// The current element's position, in an array.
    element_index: usize,
}

impl Frame<'_> {
    fn rendered(&self) -> &str {
        self.rendered.get_or_init(|| path::rendered(&self.segment))
    }
}

/// An object's keys with the offset of each, in the order they came.
#[derive(Default)]
struct Keys<'a> {
    list: Vec<(Key<'a>, usize)>,
    /// `(hash, position in list)` per key once the list grows past `LINEAR_KEYS`. With the hash
    /// at hand a resize never rehashes a key, and a lookup compares keys only when hashes match.
    index: HashTable<(u32, u32)>,
    /// std's SipHash, with keys from the OS. A crafted file cannot make its keys collide without
    /// knowing those keys, so the duplicate check stays linear on untrusted input (SECURITY.md).
    state: RandomState,
}

impl<'a> Keys<'a> {
    /// The offset of `key`'s first appearance, or `None` after recording it at `at`.
    fn first_or_insert(&mut self, key: &Key<'a>, at: usize) -> Option<usize> {
        let list = &mut self.list;
        // NOTE: past u32::MAX keys (a 128 GiB list) the check falls back to the linear scan.
        let position = u32::try_from(list.len())
            .ok()
            .filter(|_| list.len() > LINEAR_KEYS);
        let Some(position) = position else {
            let target = key.as_ref();
            if let Some((_, first)) = list.iter().find(|(k, _)| {
                let existing = k.as_ref();
                existing.len() == target.len()
                    && (existing.as_ptr() == target.as_ptr()
                        || (existing.is_empty()
                            || (existing[0] == target[0] && existing == target)))
            }) {
                return Some(*first);
            }
            list.push((key.clone(), at));
            if list.len() == LINEAR_KEYS + 1 {
                for (i, (k, _)) in (0..).zip(list.iter()) {
                    let hash = short_hash(&self.state, k);
                    self.index
                        .insert_unique(spread(hash), (hash, i), |&(h, _)| spread(h));
                }
            }
            return None;
        };
        let hash = short_hash(&self.state, key);
        let entry = self.index.entry(
            spread(hash),
            |&(h, i)| h == hash && list[i as usize].0 == *key,
            |&(h, _)| spread(h),
        );
        match entry {
            Entry::Occupied(found) => Some(list[found.get().1 as usize].1),
            Entry::Vacant(vacant) => {
                vacant.insert((hash, position));
                list.push((key.clone(), at));
                None
            }
        }
    }

    /// Empty, but keeping the allocations for the next object.
    fn clear(&mut self) {
        self.list.clear();
        // Clearing takes time in the table's capacity, so a big object's table must not pass to
        // the small ones after it: each would pay to clear it.
        if self.index.capacity() > 4 * self.index.len() {
            self.index = HashTable::new();
        } else {
            self.index.clear();
        }
    }
}

/// The low 32 bits of `key`'s hash.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the low 32 bits are the point"
)]
fn short_hash(state: &RandomState, key: &[u8]) -> u32 {
    state.hash_one(key) as u32
}

/// `hash` in both halves of a `u64`: hashbrown picks the bucket from the low bits and its 7-bit
/// tag from the top ones.
fn spread(hash: u32) -> u64 {
    u64::from(hash) * 0x1_0000_0001
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

impl<'a> Parser<'a, '_> {
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
        if let Some(word) = self.word_at(at) {
            let word = word.to_owned();
            Fail::at(at, SyntaxKind::UnexpectedWord { word, expected })
        } else if at >= self.bytes.len() {
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

    /// The word starting at `at`, for a message to show whole: an ASCII letter, then ASCII
    /// letters, digits and `_`, at most 32 bytes. `None` when no letter is there, or when the
    /// run does not start and end at a token boundary, as in the `ish` of `trueish` or the
    /// `Tru` of `Trué`.
    fn word_at(&self, at: usize) -> Option<&'a str> {
        let part = |b: &u8| b.is_ascii_alphanumeric() || *b == b'_';
        let rest = self.bytes.get(at..)?;
        if !rest.first()?.is_ascii_alphabetic()
            || at.checked_sub(1).is_some_and(|i| part(&self.bytes[i]))
        {
            return None;
        }
        let len = rest.iter().take(33).take_while(|b| part(b)).count();
        // NOTE: a word over 32 bytes, or followed by a non-ASCII byte, shows as its first
        // letter only; show the whole word if users report long JSON5 identifiers shown that way.
        (len <= 32 && rest.get(len).is_none_or(u8::is_ascii)).then(|| &self.text[at..at + len])
    }

    /// Turn an end of file inside an open object or array, where the document's structure
    /// needed more, into an error that points back at the bracket still open.
    fn unclosed(&self, fail: &mut Fail) {
        let SyntaxKind::UnexpectedEnd { expected } = fail.kind else {
            return;
        };
        let structural = ["a value", "a key", "`:`", "`,` or `}`", "`,` or `]`"];
        let Some(frame) = self.top() else {
            return;
        };
        if fail.started.is_none() && structural.contains(&expected) {
            let open = match frame.container {
                Container::Object => '{',
                Container::Array => '[',
            };
            fail.kind = SyntaxKind::Unclosed { open, expected };
            fail.started = Some(frame.opened);
        }
    }

    /// `hint` says whether the larger dialect really reads the syntax at `at`, so that the
    /// message may point to it. JSON Lines gets none: no larger dialect extends it, and a
    /// file checked in parts must report what one pass does.
    ///
    /// This and the hint helpers below run only on a syntax error. `#[cold]` keeps them out of
    /// the scanning loops they are called from, which they otherwise slowed by 1.4 to 3.2% in
    /// instructions on clean files.
    #[cold]
    fn not_allowed(&self, at: usize, what: Message, hint: bool) -> Fail {
        Fail::at(
            at,
            SyntaxKind::NotAllowed {
                what,
                dialect: self.dialect,
                hint: hint && !self.json_lines,
            },
        )
    }

    /// A single-quoted string at `at`, which only JSON5 allows; the hint holds only when JSON5
    /// reads the string to its closing quote.
    #[cold]
    fn single_quoted(&mut self, at: usize) -> Fail {
        let hint = self.json5_reads(at, |p| p.string(b'\'', false).map(|_| true));
        self.not_allowed(at, SINGLE_QUOTES_NOT_ALLOWED, hint)
    }

    /// Whether JSON5 reads the token at `at`: `scan` runs from there with JSON5's rules and
    /// says whether the token it read ends where it should. The parser's state is restored.
    /// Never in JSON Lines, which gets no hint.
    #[cold]
    fn json5_reads(
        &mut self,
        at: usize,
        scan: impl FnOnce(&mut Self) -> Result<bool, Fail>,
    ) -> bool {
        if self.json_lines {
            return false;
        }
        let (pos, json5, problems) = (self.pos, self.json5, self.problems.len());
        self.pos = at;
        self.json5 = true;
        let reads = scan(self).unwrap_or(false);
        self.problems.truncate(problems);
        (self.pos, self.json5) = (pos, json5);
        reads
    }

    /// A number form only JSON5 allows, starting at `start`. The hint holds only when JSON5
    /// reads the number whole: not `0x`, `+x` or `5.e`, and not `NaNx` or `0x1g`, where the
    /// token runs on into a word.
    #[cold]
    fn json5_number(&mut self, start: usize, what: Message) -> Fail {
        let hint = self.json5_reads(start, |p| {
            p.number()?;
            let c = p.char_at(p.pos);
            Ok(p.pos == p.bytes.len() || !(c == '\\' || is_identifier_part(c)))
        });
        self.not_allowed(start, what, hint)
    }

    /// A comment at the parser's position in a dialect without them. Only a block comment can
    /// fail to end, and then JSONC rejects it too. JSON Lines gets no hint, so it skips the
    /// search to the end of the text.
    #[cold]
    fn comment_not_allowed(&self) -> Fail {
        let hint = !self.json_lines
            && (self.bytes[self.pos + 1] == b'/'
                || memchr::memmem::find(&self.bytes[self.pos + 2..], b"*/").is_some());
        self.not_allowed(self.pos, COMMENTS_NOT_ALLOWED, hint)
    }

    fn emit(&mut self, offset: usize, kind: EventKind<'a>) {
        if let Some(sink) = &mut self.events {
            sink(Event { offset, kind });
        }
    }

    /// Where the text a syntax error is about ends: after the word for an unexpected word, after
    /// the character at the error for most others, and nowhere past the offset when the error
    /// is about a place between characters, such as a missing `,` or the end of a line.
    #[cold]
    fn syntax_end(&self, fail: &Fail) -> usize {
        let at = fail.offset;
        match &fail.kind {
            SyntaxKind::UnexpectedWord { word, .. } => at + word.len(),
            SyntaxKind::UnexpectedEnd { .. }
            | SyntaxKind::UnexpectedLineEnd { .. }
            | SyntaxKind::Unclosed { .. }
            | SyntaxKind::MissingComma
            | SyntaxKind::UnterminatedString
            | SyntaxKind::UnterminatedComment => at,
            _ => match self.text.get(at..).and_then(|rest| rest.chars().next()) {
                // A line break would put the end on the next line.
                Some('\n' | '\r' | '\u{2028}' | '\u{2029}') | None => at,
                Some(c) => at + c.len_utf8(),
            },
        }
    }

    fn report(&mut self, offset: usize, end: usize, severity: Severity, detail: Detail) {
        if severity.is_on() {
            self.problems.push(Problem {
                offset,
                end,
                severity,
                detail,
            });
        }
    }

    fn byte_order_mark(&mut self, severity: Severity) {
        if self.text.starts_with('\u{FEFF}') {
            self.report(0, '\u{FEFF}'.len_utf8(), severity, Detail::ByteOrderMark);
            self.pos = '\u{FEFF}'.len_utf8();
        }
    }

    // --- documents -------------------------------------------------------------------------

    /// One document. A document only ends with every container closed, so in JSON Lines each
    /// line starts with an empty stack.
    fn document(&mut self) -> Result<(), Fail> {
        let mut state = State::Value;
        loop {
            // Where the last value ended, before any white space or comment after it.
            let value_end = self.pos;
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
                State::AfterValue => self.after_value(value_end)?,
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

    /// The innermost open container.
    fn top(&self) -> Option<&Frame<'a>> {
        self.stack[..self.depth].last()
    }

    fn top_mut(&mut self) -> Option<&mut Frame<'a>> {
        self.stack[..self.depth].last_mut()
    }

    /// The state after a complete value.
    fn state_after_value(&self) -> State {
        if self.depth == 0 {
            State::Done
        } else {
            State::AfterValue
        }
    }

    /// `,` or the closing bracket after a value that ended at `value_end`.
    fn after_value(&mut self, value_end: usize) -> Result<State, Fail> {
        let object = self.top().is_some_and(|f| f.container == Container::Object);
        match self.peek() {
            Some(b',') => {
                self.pos += 1;
                if let Some(frame) = self.top_mut() {
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
            Some(b) if self.starts_item(b, object) => {
                Err(Fail::at(value_end, SyntaxKind::MissingComma))
            }
            _ => Err(self.unexpected(if object { "`,` or `}`" } else { "`,` or `]`" })),
        }
    }

    /// Whether `b`, at the current position, can start the next key in an object or the next
    /// value in an array, in any dialect: a sign that a `,` is missing before it.
    fn starts_item(&self, b: u8, object: bool) -> bool {
        if object {
            matches!(b, b'"' | b'\'') || self.identifier_start(self.pos)
        } else {
            matches!(
                b,
                b'{' | b'[' | b'"' | b'\'' | b'-' | b'+' | b'.' | b'0'
                    ..=b'9' | b't' | b'f' | b'n' | b'I' | b'N'
            )
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
        if let Some(depth) = self.depth.checked_sub(1) {
            self.depth = depth;
            self.stack[depth].keys.clear();
        }
        Ok(self.state_after_value())
    }

    fn open(&mut self, container: Container) -> Result<State, Fail> {
        if self.depth >= self.max_depth {
            let limit = self.max_depth;
            return Err(Fail::at(self.pos, SyntaxKind::TooDeep { limit }));
        }
        let segment = match self.top() {
            None => Segment::Root,
            Some(parent) => match parent.container {
                Container::Object => Segment::Key(parent.current_key.clone()),
                Container::Array => Segment::Index(parent.element_index),
            },
        };
        if let Some(frame) = self.stack.get_mut(self.depth) {
            frame.container = container;
            frame.opened = self.pos;
            frame.segment = segment;
            frame.rendered = OnceCell::new();
            frame.current_key = Key::Borrowed(&[]);
            frame.element_index = 0;
        } else {
            self.stack.push(Frame {
                container,
                opened: self.pos,
                segment,
                rendered: OnceCell::new(),
                keys: Keys::default(),
                current_key: Key::Borrowed(&[]),
                element_index: 0,
            });
        }
        self.depth += 1;
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
            b'\'' => return Err(self.single_quoted(self.pos)),
            b'-' | b'+' | b'.' | b'0'..=b'9' | b'I' | b'N' => {
                self.number()?;
                EventKind::Number(&self.text[start..self.pos])
            }
            b't' => {
                self.literal("`true`", "a value")?;
                EventKind::Bool(true)
            }
            b'f' => {
                self.literal("`false`", "a value")?;
                EventKind::Bool(false)
            }
            b'n' => {
                self.literal("`null`", "a value")?;
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
                self.report(len, len, final_newline, Detail::MissingFinalNewline);
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
        if let Some(frame) = self.top_mut() {
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
            Some(b'\'') => Err(self.single_quoted(start)),
            Some(_) if self.identifier_start(start) => {
                if !self.json5 {
                    let hint = self.json5_reads(start, |p| p.identifier().map(|_| true));
                    return Err(self.not_allowed(start, UNQUOTED_KEYS_NOT_ALLOWED, hint));
                }
                self.identifier().map(Some)
            }
            _ => Err(self.unexpected("a key")),
        }
    }

    /// Report `key`, found at `start`, if the current object already has it; record it if not.
    fn check_duplicate(&mut self, start: usize, key: &Key<'a>) {
        let Some(frame) = self.top_mut() else {
            return;
        };
        match frame.keys.first_or_insert(key, start) {
            None => {}
            Some(first) => {
                let mut last = String::new();
                path::push_key(&mut last, key);
                let stack = &self.stack[..self.depth];
                let path = path::bounded(stack.len() + 1, |i| {
                    stack.get(i).map_or(last.as_str(), Frame::rendered)
                });
                self.report(
                    start,
                    self.pos,
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
                if is_high_surrogate(unit)
                    && self.bytes.get(self.pos + 6..self.pos + 8) == Some(b"\\u")
                    && let Some(low) = self.hex(self.pos + 8, 4)
                    && is_low_surrogate(low)
                {
                    let code_point = 0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00);
                    (char::from_u32(code_point).unwrap_or('\u{FFFD}'), 12)
                } else {
                    (char::from_u32(unit).unwrap_or('\u{FFFD}'), 6)
                }
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
        let ascii_only = self.invalid_code_point.is_on();
        let stops = string_stops(quote, self.json5, ascii_only);
        loop {
            self.pos += plain_bytes(&self.bytes[self.pos..], stops, quote, ascii_only);
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
                // Outside JSON5 only: string_stops lets JSON5 pass control characters.
                0..0x20 => {
                    debug_assert!(!self.json5);
                    return Err(self.fail(SyntaxKind::ControlCharacter {
                        found: char::from(b),
                    }));
                }
                // Past ASCII: string_stops stops there only with the invalid-code-point check on.
                _ => {
                    debug_assert!(b >= 0x80 && self.invalid_code_point.is_on());
                    let c = self.char_at(self.pos);
                    if is_noncharacter(u32::from(c)) {
                        self.report(
                            self.pos,
                            self.pos + c.len_utf8(),
                            self.invalid_code_point,
                            Detail::InvalidCodePoint {
                                code_point: u32::from(c),
                            },
                        );
                    }
                    self.pos += c.len_utf8();
                }
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
            // Any other character stands for itself: `\'` among them.
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
                self.pos,
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
            Some(b'+') if !self.json5 => {
                return Err(self.json5_number(start, NUMBER_FORM_NOT_ALLOWED));
            }
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
            return Err(self.json5_number(start, NUMBER_FORM_NOT_ALLOWED));
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
            if self.peek().is_some_and(|b| b.is_ascii_digit()) {
                return Err(Fail::at(int_start, SyntaxKind::LeadingZero));
            }
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
                return Err(self.json5_number(start, NUMBER_FORM_NOT_ALLOWED));
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
        let rest = &self.bytes[self.pos..];
        let mut len = 0;
        while len < rest.len() && rest[len].is_ascii_digit() {
            len += 1;
        }
        self.pos += len;
        len > 0
    }

    /// `NaN` or `Infinity`, which only JSON5 allows. `start` is at the sign, if there is one.
    fn nan_or_infinity(&mut self, start: usize, word: &'static str) -> Result<(), Fail> {
        if !self.json5 {
            return Err(
                if self.text[self.pos..].starts_with(word.trim_matches('`')) {
                    self.json5_number(start, NAN_AND_INFINITY_NOT_ALLOWED)
                } else {
                    self.unexpected(self.after_sign(start))
                },
            );
        }
        self.literal(word, self.after_sign(start))
    }

    /// What a word at the current position stands in for: a value, or after a sign at `start`
    /// a digit, where no literal hint applies.
    fn after_sign(&self, start: usize) -> &'static str {
        if self.pos > start {
            "a digit"
        } else {
            "a value"
        }
    }

    /// A literal such as `null`. `word` comes in backticks, as an error message shows it; a
    /// different word whole in its place shows as found where `instead` was needed.
    fn literal(&mut self, word: &'static str, instead: &'static str) -> Result<(), Fail> {
        let start = self.pos;
        // Every caller passes the word in backticks.
        let bare = &word.as_bytes()[1..word.len() - 1];
        if self.bytes[start..].starts_with(bare) {
            self.pos += bare.len();
            return Ok(());
        }
        for &expected in bare {
            if self.peek() != Some(expected) {
                // A different word, such as `None` for `NaN`, shows whole; a cut-off `nul` or
                // `nul]` still says which literal it started.
                let other = self
                    .word_at(start)
                    .is_some_and(|w| start + w.len() > self.pos);
                return Err(if other {
                    self.unexpected_at(start, instead)
                } else {
                    self.unexpected(word)
                });
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
                self.pos,
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
                self.pos,
                self.invalid_number,
                Detail::InvalidNumber { overflows },
            );
        }
    }

    // --- whitespace and comments -----------------------------------------------------------

    /// Skip white space and comments. Most calls find a token right away, so this inlined check
    /// returns at once unless the next byte is one [`Self::skip_trivia`] can act on in some
    /// dialect (see [`ends_trivia`]). It cut instructions by 21% on minified JSON.
    #[inline]
    fn trivia(&mut self) -> Result<(), Fail> {
        if self.peek().is_none_or(ends_trivia) {
            return Ok(());
        }
        // The two common runs before a token are skipped here, as the call costs more than they
        // do: one space, as after `:`, and a line break plus an indent of spaces.
        if self.peek() == Some(b' ')
            && self
                .bytes
                .get(self.pos + 1)
                .is_some_and(|&b| ends_trivia(b))
        {
            self.pos += 1;
            return Ok(());
        }
        if self.peek() == Some(b'\n') && !self.json_lines {
            let rest = &self.bytes[self.pos + 1..];
            if let Some(run) = rest.iter().position(|&b| b != b' ')
                && ends_trivia(rest[run])
            {
                self.pos += 1 + run;
                return Ok(());
            }
        }
        self.skip_trivia()
    }

    #[inline(never)]
    fn skip_trivia(&mut self) -> Result<(), Fail> {
        loop {
            let Some(b) = self.peek() else { return Ok(()) };
            match b {
                b'\n' | b'\r' if self.line_end().is_some() => return Ok(()),
                b' ' | b'\t' | b'\r' | b'\n' => {
                    // The whole run at once, with one bounds check. In JSON Lines it stops
                    // before a CR or LF, which the arm above decides on.
                    let rest = &self.bytes[self.pos + 1..];
                    let run = if self.json_lines {
                        rest.iter().position(|&b| !matches!(b, b' ' | b'\t'))
                    } else {
                        rest.iter()
                            .position(|&b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
                    };
                    let Some(run) = run else {
                        self.pos = self.bytes.len();
                        return Ok(());
                    };
                    self.pos += 1 + run;
                    if ends_trivia(rest[run]) {
                        return Ok(());
                    }
                }
                b'/' => match self.bytes.get(self.pos + 1) {
                    Some(b'/' | b'*') if !self.dialect.comments() => {
                        return Err(self.comment_not_allowed());
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

/// Whether no white space, comment or JSON Lines line end can start at `b` in any dialect: it
/// is not ASCII white space or a control, `/`, or a byte past ASCII.
fn ends_trivia(b: u8) -> bool {
    b > b' ' && b != b'/' && b < 0x80
}

/// Byte classes for [`string_stops`]: the bits a byte carries in [`STRING_CLASS`].
const DOUBLE_QUOTE: u8 = 1;
const SINGLE_QUOTE: u8 = 2;
const BACKSLASH: u8 = 4;
const LINE_BREAK: u8 = 8;
const CONTROL: u8 = 16;
const NON_ASCII: u8 = 32;

/// The class of every byte, as `serde_json`'s `ESCAPE` table: one load per byte in place of a
/// `match` with a dialect check in each arm.
#[expect(clippy::cast_possible_truncation, reason = "b stays below 256")]
static STRING_CLASS: [u8; 256] = {
    let mut table = [0; 256];
    let mut b = 0;
    while b < 256 {
        table[b] = match b as u8 {
            b'"' => DOUBLE_QUOTE,
            b'\'' => SINGLE_QUOTE,
            b'\\' => BACKSLASH,
            b'\n' | b'\r' => LINE_BREAK,
            0..0x20 => CONTROL,
            0x80.. => NON_ASCII,
            _ => 0,
        };
        b += 1;
    }
    table
};

/// The [`STRING_CLASS`] bits that stop the plain bytes of a string: its closing `quote`, a
/// backslash, a line break, a control character unless `json5` allows it, and with `ascii_only`
/// a byte past ASCII.
fn string_stops(quote: u8, json5: bool, ascii_only: bool) -> u8 {
    let quote = if quote == b'"' {
        DOUBLE_QUOTE
    } else {
        SINGLE_QUOTE
    };
    let control = if json5 { 0 } else { CONTROL };
    let high = if ascii_only { NON_ASCII } else { 0 };
    quote | BACKSLASH | LINE_BREAK | control | high
}

/// How many bytes at the start of `bytes` a string can pass without a decision: none of them is
/// in `stops`. Byte by byte through the first 16, since most strings end sooner, then in blocks.
fn plain_bytes(bytes: &[u8], stops: u8, quote: u8, ascii_only: bool) -> usize {
    let stop = |&b: &u8| STRING_CLASS[usize::from(b)] & stops != 0;
    let head = bytes.len().min(16);
    if let Some(i) = bytes[..head].iter().position(stop) {
        return i;
    }
    let run = head + plain_run(&bytes[head..], quote, ascii_only);
    run + bytes[run..]
        .iter()
        .position(stop)
        .unwrap_or(bytes.len() - run)
}

/// How many bytes at the start of `bytes`, in whole 16-byte blocks, a string can skip: no
/// `quote`, backslash or control character, and with `ascii_only` no byte past ASCII. The byte
/// loop decides the rest. The fixed-size blocks let the compiler compare 16 bytes at once
/// (SSE2, NEON) without `unsafe`.
#[expect(
    clippy::needless_bitwise_bool,
    reason = "no branch per byte, so the block compiles to vector compares"
)]
fn plain_run(bytes: &[u8], quote: u8, ascii_only: bool) -> usize {
    let high = if ascii_only { 0x80 } else { 0 };
    bytes
        .as_chunks::<16>()
        .0
        .iter()
        .take_while(|block| {
            !block.iter().fold(false, |stop, &b| {
                stop | (b == quote) | (b == b'\\') | (b < 0x20) | (b & high != 0)
            })
        })
        .count()
        * 16
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

#[cfg(test)]
mod tests {
    use std::hash::RandomState;

    use super::{Key, Keys, LINEAR_KEYS, plain_bytes, plain_run, short_hash, spread, string_stops};

    // A hash that lost its spread still finds every duplicate, only in quadratic time, which a
    // test cannot time reliably. These pin the spread itself.

    #[test]
    fn spread_puts_the_hash_in_both_halves() {
        assert_eq!(spread(0x1234_5678), 0x1234_5678_1234_5678);
    }

    #[test]
    fn short_hash_tells_keys_apart() {
        let state = RandomState::new();
        let hashes: std::collections::HashSet<u32> = (0..1000)
            .map(|i| short_hash(&state, format!("k{i}").as_bytes()))
            .collect();
        // This allows two collisions; even one comes about once in 10,000 runs.
        assert!(hashes.len() >= 998, "{}", hashes.len());
    }

    #[test]
    fn keys_index_only_an_object_past_the_linear_limit() {
        let mut keys = Keys::default();
        let key = |i: usize| Key::from(format!("k{i}").into_bytes());
        for i in 0..LINEAR_KEYS {
            assert_eq!(keys.first_or_insert(&key(i), i), None);
        }
        assert_eq!(keys.index.len(), 0);
        for i in LINEAR_KEYS..2 * LINEAR_KEYS {
            assert_eq!(keys.first_or_insert(&key(i), i), None);
        }
        assert_eq!(keys.index.len(), 2 * LINEAR_KEYS);
        assert_eq!(keys.first_or_insert(&key(3), 99), Some(3));
        assert_eq!(
            keys.first_or_insert(&key(LINEAR_KEYS + 3), 99),
            Some(LINEAR_KEYS + 3)
        );
    }

    #[test]
    fn a_plain_run_stops_at_the_block_holding_any_byte_that_needs_a_decision() {
        let plain = "a".repeat(40);
        assert_eq!(plain_run(plain.as_bytes(), b'"', true), 32);
        assert_eq!(plain_run(&plain.as_bytes()[..15], b'"', true), 0);
        for stop in ["\"", "\\", "\u{1f}", "\u{0}", "é"] {
            // One stop byte in the second block, and two, which must not cancel out.
            for stops in [stop.to_owned(), stop.repeat(2)] {
                let text = format!("{}{stops}{}", "a".repeat(20), "a".repeat(20));
                assert_eq!(plain_run(text.as_bytes(), b'"', true), 16, "{stops:?}");
            }
        }
        // `'` ends only a single-quoted string; past ASCII is plain unless checked.
        let text = format!("{}'é{}", "a".repeat(20), "a".repeat(20));
        assert_eq!(plain_run(text.as_bytes(), b'"', false), 32);
        assert_eq!(plain_run(text.as_bytes(), b'\'', false), 16);
        assert_eq!(plain_run(" ".repeat(16).as_bytes(), b'"', true), 16);
    }

    #[test]
    fn plain_bytes_stop_at_the_first_byte_that_needs_a_decision_in_the_dialect() {
        for &quote in b"\"'" {
            for json5 in [false, true] {
                for ascii_only in [false, true] {
                    let stops = string_stops(quote, json5, ascii_only);
                    let other = if quote == b'"' { "'" } else { "\"" };
                    let cases = [
                        (char::from(quote).to_string(), true),
                        (other.to_owned(), false),
                        ("\\".to_owned(), true),
                        ("\n".to_owned(), true),
                        ("\r".to_owned(), true),
                        ("\x01".to_owned(), !json5),
                        ("\t".to_owned(), !json5),
                        ("é".to_owned(), ascii_only),
                    ];
                    // Before, inside and after the 16-byte blocks.
                    for at in [0, 5, 15, 16, 20, 40] {
                        for (probe, stop) in &cases {
                            let text = format!("{}{probe}{}", "a".repeat(at), "a".repeat(40));
                            let want = if *stop { at } else { text.len() };
                            assert_eq!(
                                plain_bytes(text.as_bytes(), stops, quote, ascii_only),
                                want,
                                "{probe:?} at {at}, quote {quote}, json5 {json5}, ascii_only {ascii_only}"
                            );
                        }
                    }
                    let plain = "a".repeat(40);
                    assert_eq!(plain_bytes(plain.as_bytes(), stops, quote, ascii_only), 40);
                }
            }
        }
        // In JSON5 a raw tab stops a block but not the byte scan, so the scan goes on to the
        // stop whether the tab is in the head or in the block the tail then reads.
        let json5 = string_stops(b'\'', true, false);
        for text in [
            format!("a\t{}'", "a".repeat(40)),
            format!("{}\t{}\r", "a".repeat(20), "a".repeat(20)),
        ] {
            assert_eq!(
                plain_bytes(text.as_bytes(), json5, b'\'', false),
                text.len() - 1,
                "{text:?}"
            );
        }
    }
}
