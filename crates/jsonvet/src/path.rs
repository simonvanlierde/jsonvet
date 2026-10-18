//! JSONPath rendering, as check-json5 1.4 wrote it: `$.key` for identifier-like keys,
//! `$["other key"]` otherwise. Unlike 1.4's `json.dumps`, a quoted key keeps non-ASCII text as
//! written.

use std::borrow::Cow;
use std::fmt::Write;

use crate::unicode::is_hidden;

/// One step from a container to a child.
#[derive(Clone, Debug)]
pub(crate) enum Segment<'a> {
    Root,
    Key(Cow<'a, [u8]>),
    Index(usize),
}

/// Past this many bytes, a path keeps only the segments that fit in `MAX_PATH / 2` bytes at
/// each end, with `…` in place of the rest. A segment longer than that is cut, so a long key
/// still shows its start or its end. A full path never holds `…`: quoted keys escape it, and
/// it cannot be part of an unquoted key.
pub(crate) const MAX_PATH: usize = 256;

/// The JSONPath made of `count` segments, root first, each as [`rendered`] gives it, cut as
/// `MAX_PATH` describes. It reads at most `MAX_PATH + 1` segments from each end, so a
/// duplicate in a deep object costs no more than one near the root.
pub(crate) fn bounded<'s>(count: usize, segment: impl Fn(usize) -> &'s str) -> String {
    let mut path = String::new();
    let mut len = 0;
    let fits = (0..count).all(|i| {
        len += segment(i).len();
        len <= MAX_PATH
    });
    if fits {
        (0..count).for_each(|i| path.push_str(segment(i)));
        return path;
    }
    let half = MAX_PATH / 2;
    for i in 0..count {
        let s = segment(i);
        let room = half - path.len();
        if s.len() <= room {
            path.push_str(s);
            continue;
        }
        if s.len() > half {
            let mut end = room;
            while !s.is_char_boundary(end) {
                end -= 1;
            }
            path.push_str(&s[..end]);
        }
        break;
    }
    path.push('…');
    let (mut from, mut len) = (count, 0);
    while from > 0 {
        let s = segment(from - 1);
        if len + s.len() <= half {
            from -= 1;
            len += s.len();
            continue;
        }
        if s.len() > half {
            let mut start = s.len() - (half - len);
            while !s.is_char_boundary(start) {
                start += 1;
            }
            path.push_str(&s[start..]);
        }
        break;
    }
    (from..count).for_each(|i| path.push_str(segment(i)));
    path
}

/// One segment as a path shows it: `$`, `.key`, `["key"]` or `[0]`.
pub(crate) fn rendered(segment: &Segment<'_>) -> String {
    let mut path = String::new();
    push(&mut path, segment);
    path
}

/// One step of a [`json_path`]: an object key or an array index.
///
/// Public for jsonvet-cli only and outside the semver contract: it can change in any release.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PathStep<'a> {
    /// A member of an object, by its key as WTF-8, as in [`EventKind::Key`](crate::EventKind::Key).
    Key(&'a [u8]),
    /// An element of an array, by its index from 0.
    Index(usize),
}

/// The JSONPath of the value that `steps` lead to, written as duplicate-key messages write
/// paths: `$` for the root, `.key` for an identifier-like key, `["other key"]` for any other key,
/// and `[0]` for an index. A path longer than 256 bytes keeps its two ends with `…` between them,
/// as duplicate-key paths do.
///
/// ```
/// use jsonvet::{json_path, PathStep};
///
/// assert_eq!(json_path([]), "$");
/// let steps = [PathStep::Key(b"a"), PathStep::Index(0), PathStep::Key(b"\xED\xA0\x80")];
/// assert_eq!(json_path(steps), "$.a[0][\"\\ud800\"]");
/// ```
///
/// Public for jsonvet-cli only and outside the semver contract: it can change in any release.
#[doc(hidden)]
pub fn json_path<'a>(steps: impl IntoIterator<Item = PathStep<'a>>) -> String {
    let segments: Vec<String> = std::iter::once("$".to_owned())
        .chain(steps.into_iter().map(|step| match step {
            PathStep::Key(key) => rendered(&Segment::Key(Cow::Borrowed(key))),
            PathStep::Index(i) => rendered(&Segment::Index(i)),
        }))
        .collect();
    bounded(segments.len(), |i| &segments[i])
}

/// Append one segment to a path.
fn push(path: &mut String, segment: &Segment<'_>) {
    match segment {
        Segment::Root => path.push('$'),
        Segment::Key(key) => push_key(path, key),
        Segment::Index(i) => {
            let _ = write!(path, "[{i}]");
        }
    }
}

/// Append a key, given as the parser's WTF-8 (see `parse::Key`).
pub(crate) fn push_key(path: &mut String, key: &[u8]) {
    let chars = decode(key);
    if is_identifier(&chars) {
        path.push('.');
        path.extend(chars.iter().flatten());
    } else {
        path.push('[');
        quote(path, &chars);
        path.push(']');
    }
}

/// A key from [`EventKind::Key`](crate::EventKind::Key) as text, with U+FFFD for each lone
/// surrogate, as `serde_json` reads it.
///
/// ```
/// assert_eq!(jsonvet::key_text(b"a\xED\xA0\x80"), "a\u{FFFD}");
/// ```
///
/// Public for jsonvet-cli only and outside the semver contract: it can change in any release.
#[doc(hidden)]
pub fn key_text(key: &[u8]) -> Cow<'_, str> {
    text(Cow::Borrowed(key))
}

/// A decoded string or key as text, with U+FFFD for each lone surrogate.
pub(crate) fn text(key: Cow<'_, [u8]>) -> Cow<'_, str> {
    let lossy = |bytes: &[u8]| -> String {
        decode(bytes)
            .into_iter()
            .map(|c| c.unwrap_or('\u{FFFD}'))
            .collect()
    };
    match key {
        Cow::Borrowed(bytes) => match std::str::from_utf8(bytes) {
            Ok(text) => Cow::Borrowed(text),
            Err(_) => Cow::Owned(lossy(bytes)),
        },
        Cow::Owned(bytes) => {
            Cow::Owned(String::from_utf8(bytes).unwrap_or_else(|e| lossy(e.as_bytes())))
        }
    }
}

/// A key's characters, with each lone surrogate as `Err(unit)`.
fn decode(mut key: &[u8]) -> Vec<Result<char, u32>> {
    let mut chars = Vec::new();
    loop {
        let valid = match std::str::from_utf8(key) {
            Ok(text) => text,
            Err(e) => std::str::from_utf8(&key[..e.valid_up_to()]).unwrap_or_default(),
        };
        chars.extend(valid.chars().map(Ok));
        // Anything after the valid part is a lone surrogate, which the parser wrote as
        // `ED`, then 6 bits, then 6 bits.
        let Some(&[_, high, low]) = key.get(valid.len()..valid.len() + 3) else {
            return chars;
        };
        chars.push(Err(0xD000
            | (u32::from(high & 0x3F) << 6)
            | u32::from(low & 0x3F)));
        key = &key[valid.len() + 3..];
    }
}

/// Python's `[A-Za-z_$][\w$]*`, where `\w` takes a character whose general category is a letter
/// or a number, and no surrogate. `char::is_alphanumeric` is wider: it adds marks and symbols
/// such as U+093E and U+24B6.
fn is_identifier(chars: &[Result<char, u32>]) -> bool {
    use unicode_properties::GeneralCategoryGroup::{Letter, Number};
    use unicode_properties::UnicodeGeneralCategory;
    let mut chars = chars.iter();
    chars
        .next()
        .is_some_and(|c| matches!(c, Ok(c) if c.is_ascii_alphabetic() || *c == '_' || *c == '$'))
        && chars.all(|c| {
            matches!(c, Ok(c) if *c == '_' || *c == '$'
                || matches!(c.general_category_group(), Letter | Number))
        })
}

/// Quote a key as JSON: `\"`, `\\`, the short escapes for control characters, and lowercase
/// `\uXXXX` for other control characters, hidden ones (see [`is_hidden`]), `…` and lone
/// surrogates. Any other character is kept as written.
fn quote(out: &mut String, chars: &[Result<char, u32>]) {
    out.push('"');
    for c in chars {
        let c = match c {
            Ok(c) => *c,
            Err(unit) => {
                let _ = write!(out, "\\u{unit:04x}");
                continue;
            }
        };
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if c.is_control() || is_hidden(c) || c == '…' => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::{MAX_PATH, bounded, push_key};

    fn rendered(key: &str) -> String {
        let mut path = "$".to_owned();
        push_key(&mut path, key.as_bytes());
        path
    }

    #[test]
    fn keys_render_as_check_json5_rendered_them_but_keep_non_ascii() {
        assert_eq!(rendered("server"), "$.server");
        assert_eq!(rendered("$ref"), "$.$ref");
        assert_eq!(rendered("café"), "$.café");
        assert_eq!(rendered("1st"), "$[\"1st\"]");
        assert_eq!(rendered("b c"), "$[\"b c\"]");
        assert_eq!(rendered("ünicode"), "$[\"ünicode\"]");
        assert_eq!(rendered("日本 語"), "$[\"日本 語\"]");
        assert_eq!(rendered("q\"\\\n"), "$[\"q\\\"\\\\\\n\"]");
        assert_eq!(rendered("😀"), "$[\"😀\"]");
        // Control, hidden and the cut mark `…` stay escaped, past U+FFFF as a surrogate pair.
        assert_eq!(
            rendered("a\u{7f}\u{85}\u{202e}\u{ad}…\u{e0041}"),
            "$[\"a\\u007f\\u0085\\u202e\\u00ad\\u2026\\udb40\\udc41\"]"
        );
        assert_eq!(rendered(""), "$[\"\"]");
        // Marks and symbols are not `\w` in Python, so the key is quoted; numbers like U+00B2 are.
        assert_eq!(rendered("a\u{093E}"), "$[\"a\u{093E}\"]");
        assert_eq!(rendered("a\u{24B6}"), "$[\"a\u{24B6}\"]");
        assert_eq!(rendered("a\u{0345}"), "$[\"a\u{0345}\"]");
        assert_eq!(rendered("a\u{00B2}"), "$.a\u{00B2}");
        let mut path = "$".to_owned();
        push_key(&mut path, b"a\xED\xA0\x80b");
        assert_eq!(path, "$[\"a\\ud800b\"]");
    }

    #[test]
    fn a_lone_surrogate_becomes_one_replacement_character() {
        use std::borrow::Cow;
        // WTF-8 for the escape `\ud800`, as the parser writes it.
        let lone = [0xED, 0xA0, 0x80];
        assert!(matches!(
            super::text(Cow::Borrowed(b"ok")),
            Cow::Borrowed("ok")
        ));
        assert_eq!(super::text(Cow::Borrowed(&lone)), "\u{FFFD}");
        let owned = [b"a".as_slice(), &lone, b"b"].concat();
        assert_eq!(super::text(Cow::Owned(owned)), "a\u{FFFD}b");
    }

    #[test]
    fn long_paths_keep_their_ends() {
        let path = |segments: &[&str]| bounded(segments.len(), |i| segments[i]);
        let key = format!(".{}", "k".repeat(MAX_PATH - 2));
        assert_eq!(path(&["$", &key]), format!("${key}"));
        // A key too long to fit keeps its start and its end.
        let key = format!("{key}k");
        let long = path(&["$", &key]);
        assert_eq!(long.len(), MAX_PATH + '…'.len_utf8());
        assert!(long.starts_with("$.kkk") && long.ends_with("kkk") && long.contains('…'));
        // A deep path keeps whole segments at each end.
        let deep: Vec<&str> = std::iter::once("$")
            .chain(std::iter::repeat_n("[0]", 1000))
            .chain([".é", ".last"])
            .collect();
        let head = format!("${}", "[0]".repeat(42));
        let tail = format!("{}.é.last", "[0]".repeat(40));
        assert_eq!(path(&deep), format!("{head}…{tail}"));
        // A cut never splits a character.
        let wide = format!(".{}", "é".repeat(MAX_PATH));
        let wide = path(&["$", &wide]);
        assert!(wide.len() <= MAX_PATH + '…'.len_utf8());
        // A segment of exactly half the room is dropped whole at either end, not cut.
        let half = MAX_PATH / 2;
        let x = format!(".{}", "x".repeat(half - 1));
        let y = format!(".{}", "y".repeat(half - 1));
        assert_eq!(path(&["$", &x, &y, ".z"]), "$….z");
        // A cut never lands inside a character: the head's moves back, the tail's forward.
        let s = format!(".a{}", "é".repeat(200));
        let cut = "é".repeat(62);
        assert_eq!(path(&["$", &s, &s, ".zz"]), format!("$.a{cut}…{cut}.zz"));
    }
}
