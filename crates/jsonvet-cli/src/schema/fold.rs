//! A file's events folded into the `serde_json` values jsonschema checks.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;

use jsonvet::{Event, EventKind, key_text};
use serde_json::{Map, Number, Value};

/// One document's values, and the range of the events they were folded from.
pub(super) struct Document {
    pub(super) value: Value,
    pub(super) events: Range<usize>,
    /// The offset of its first event.
    pub(super) offset: usize,
    /// Why the values are not the text's, and where: the first such number or key.
    pub(super) unchecked: Option<(usize, &'static str)>,
}

const NOT_A_NUMBER: &str =
    "NaN, Infinity and numbers beyond a double's range have no JSON Schema value";
const SAME_KEYS: &str =
    "this key and an earlier one in the object are the same once lone surrogates become U+FFFD";

/// An open container while the events are folded into values, with an object's current key
/// (as text, its offset, and as written if it holds U+FFFD) and each earlier key holding U+FFFD,
/// by text, as written.
enum Open<'e> {
    Array(Vec<Value>),
    Object(
        Map<String, Value>,
        (String, usize, Cow<'e, [u8]>),
        HashMap<String, Cow<'e, [u8]>>,
    ),
}

/// Events folded into documents one at a time, so no more than the open document's values are
/// held: one document, or one per line in JSON Lines.
#[derive(Default)]
pub(super) struct Fold<'e> {
    stack: Vec<Open<'e>>,
    /// How many events came before this one.
    seen: usize,
    /// The index and offset of the open document's first event.
    first: (usize, usize),
    unchecked: Option<(usize, &'static str)>,
}

impl<'e> Fold<'e> {
    /// Take the next event, and give back the document it completes, if any.
    pub(super) fn event(&mut self, event: &Event<'e>) -> Option<Document> {
        let i = self.seen;
        self.seen += 1;
        if self.stack.is_empty() {
            self.first = (i, event.offset);
        }
        let value = match &event.kind {
            EventKind::Key(key) => {
                if let Some(Open::Object(_, current, _)) = self.stack.last_mut() {
                    let text = key_text(key).into_owned();
                    // Only a key holding U+FFFD is ever compared as written.
                    let raw = if text.contains('\u{FFFD}') {
                        key.clone()
                    } else {
                        Cow::Borrowed(&[][..])
                    };
                    *current = (text, event.offset, raw);
                }
                return None;
            }
            EventKind::ObjectStart => {
                self.stack.push(Open::Object(
                    Map::new(),
                    (String::new(), 0, Cow::Borrowed(&[])),
                    HashMap::new(),
                ));
                return None;
            }
            EventKind::ArrayStart => {
                self.stack.push(Open::Array(Vec::new()));
                return None;
            }
            EventKind::End => match self.stack.pop() {
                Some(Open::Array(items)) => Value::Array(items),
                Some(Open::Object(members, ..)) => Value::Object(members),
                None => return None,
            },
            EventKind::Null => Value::Null,
            EventKind::Bool(b) => Value::Bool(*b),
            EventKind::String(s) => Value::String(s.to_string()),
            EventKind::Number(text) => number(text).unwrap_or_else(|| {
                self.unchecked.get_or_insert((event.offset, NOT_A_NUMBER));
                Value::Null
            }),
            _ => return None,
        };
        match self.stack.last_mut() {
            None => {
                return Some(Document {
                    value,
                    events: self.first.0..i + 1,
                    offset: self.first.1,
                    unchecked: self.unchecked.take(),
                });
            }
            Some(Open::Array(items)) => items.push(value),
            Some(Open::Object(members, (key, offset, raw), seen)) => {
                let key = std::mem::take(key);
                // Real duplicates are the duplicate-key rule's; keys that only become the same
                // once lone surrogates are U+FFFD are not.
                if key.contains('\u{FFFD}') {
                    match seen.get(&key) {
                        Some(earlier) if earlier != raw => {
                            self.unchecked.get_or_insert((*offset, SAME_KEYS));
                        }
                        Some(_) => {}
                        None => {
                            seen.insert(key.clone(), std::mem::take(raw));
                        }
                    }
                }
                members.insert(key, value);
            }
        }
        None
    }
}

/// Fold events into documents, one at a time: one, or one per line in JSON Lines. One at a
/// time, a document's values can be dropped before the next is folded.
pub(super) fn documents(events: &[Event<'_>]) -> impl Iterator<Item = Document> {
    let mut fold = Fold::default();
    events.iter().filter_map(move |event| fold.event(event))
}

/// A number as written in any dialect, or `None` for `NaN`, `Infinity` and numbers past a
/// double's range. An integer too large for 64 bits becomes a double, as `serde_json` reads one.
fn number(text: &str) -> Option<Value> {
    let (negative, unsigned) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let hex = unsigned
        .strip_prefix("0x")
        .or_else(|| unsigned.strip_prefix("0X"));
    let integer = hex.is_some() || !unsigned.contains(['.', 'e', 'E']);
    let exact = match hex {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None if integer => unsigned.parse::<u64>().ok(),
        None => None,
    };
    let exact = exact.and_then(|n| {
        if negative {
            0i64.checked_sub_unsigned(n).map(Number::from)
        } else {
            Some(Number::from(n))
        }
    });
    if let Some(n) = exact {
        return Some(Value::Number(n));
    }
    let magnitude = match hex {
        Some(hex) => hex.chars().fold(0.0, |n, d| {
            n * 16.0 + f64::from(d.to_digit(16).unwrap_or_default())
        }),
        // `f64::from_str` takes `.5` and `5.` too, and reads the words `NaN` and `Infinity`.
        None => unsigned.parse().ok()?,
    };
    Number::from_f64(if negative { -magnitude } else { magnitude }).map(Value::Number)
}

#[cfg(test)]
pub(super) mod tests {
    use super::{documents, number};
    use jsonvet::{Config, Dialect};
    use proptest::prelude::*;
    use serde_json::{Value, json};

    #[test]
    fn events_fold_into_the_values_serde_json_reads() {
        let text = r#"{"a": [1, -2.5e3, true, null, {"b": [false, "c"]}], "b\u00e9/~": "x\n\ud83d\ude00"}"#;
        let (_, events) = jsonvet::parse(text, &Config::new(Dialect::Json));
        let documents: Vec<_> = documents(&events.unwrap_or_default()).collect();
        let expected: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].value, expected);
    }

    #[test]
    fn numbers_in_every_dialect_become_json_numbers() {
        let cases = [
            ("0", json!(0)),
            ("-7", json!(-7)),
            ("+7", json!(7)),
            (
                "18446744073709551615",
                json!(18_446_744_073_709_551_615_u64),
            ),
            ("1.5e3", json!(1500.0)),
            (".5", json!(0.5)),
            ("5.", json!(5.0)),
            ("-0x1F", json!(-31)),
            ("-9223372036854775808", json!(i64::MIN)),
            ("-0x8000000000000000", json!(i64::MIN)),
            ("0XfF", json!(255)),
            ("0x10000000000000000", json!(18_446_744_073_709_551_616.0)),
        ];
        for (text, expected) in cases {
            assert_eq!(number(text), Some(expected), "{text}");
        }
        assert_eq!(number("NaN"), None);
        assert_eq!(number("-Infinity"), None);
        assert_eq!(number("1e999"), None);
    }

    /// Any JSON value, with numbers `serde_json` reads exactly without its `float_roundtrip`
    /// feature, and strings that need escapes.
    pub(in crate::schema) fn any_json() -> impl Strategy<Value = Value> {
        let leaf = prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::Bool),
            any::<i64>().prop_map(Value::from),
            any::<u64>().prop_map(Value::from),
            any::<i32>().prop_map(|n| Value::from(f64::from(n) / 64.0)),
            "\\PC{0,6}|[\\x00-\\x1f\"\\\\/]{0,3}".prop_map(Value::String),
        ];
        leaf.prop_recursive(4, 32, 4, |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
                prop::collection::vec(("\\PC{0,3}", inner), 0..4)
                    .prop_map(|members| Value::Object(members.into_iter().collect())),
            ]
        })
    }

    proptest! {

        #[test]
        fn strict_json_folds_into_what_serde_json_reads(value in any_json(), pretty in any::<bool>()) {
            let text = if pretty {
                serde_json::to_string_pretty(&value)
            } else {
                serde_json::to_string(&value)
            }
            .unwrap_or_default();
            let (_, events) = jsonvet::parse(&text, &Config::new(Dialect::Json));
            prop_assert!(events.is_some(), "{}", text);
            let documents: Vec<_> = documents(&events.unwrap_or_default()).collect();
            let expected: Value = serde_json::from_str(&text).unwrap_or_default();
            prop_assert_eq!(documents.len(), 1);
            prop_assert_eq!(&documents[0].value, &expected);
        }
    }
}
