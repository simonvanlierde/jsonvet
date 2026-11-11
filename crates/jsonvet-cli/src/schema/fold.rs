//! A file's events folded into the trees jsonschema checks.

use std::borrow::Cow;
use std::collections::HashMap;

use jsonvet::{Event, EventKind, key_text};
use serde_json::{Number, Value};

use super::nodes::{Builder, Tree};
use crate::messages;

/// One document's values.
pub(super) struct Document<'e> {
    pub(super) tree: Tree<'e>,
    /// The offset of its first event.
    pub(super) offset: usize,
    /// Why the values are not the text's, and where: the first such number or key.
    pub(super) unchecked: Option<(usize, &'static str)>,
}

/// An open container: where its children begin in the builder, and for an object its current
/// key (as text, its offset, and as written if it holds U+FFFD) and each earlier key holding
/// U+FFFD, by text, as written.
enum Open<'e> {
    /// Where its children begin, and its first byte's offset.
    Array(usize, usize),
    Object(
        usize,
        usize,
        (Cow<'e, str>, usize, Cow<'e, [u8]>),
        HashMap<String, Cow<'e, [u8]>>,
    ),
}

/// Events folded into documents one at a time, so no more than the open document's values are
/// held: one document, or one per line in JSON Lines.
#[derive(Default)]
pub(super) struct Fold<'e> {
    stack: Vec<Open<'e>>,
    build: Builder<'e>,
    /// How many events came before this one.
    seen: usize,
    /// The index and offset of the open document's first event.
    first: (usize, usize),
    unchecked: Option<(usize, &'static str)>,
}

impl<'e> Fold<'e> {
    /// A fold of the documents in `source`, a text in `dialect`.
    pub(super) fn new(source: &'e str, dialect: jsonvet::Dialect) -> Self {
        Fold {
            build: Builder::new(source, dialect),
            ..Fold::default()
        }
    }

    /// Build the next document in the vectors of `tree`, which is done with.
    pub(super) fn recycle(&mut self, tree: Tree<'e>) {
        self.build.recycle(tree);
    }

    /// Take the next event, and give back the document it completes, if any.
    pub(super) fn event(&mut self, event: &Event<'e>) -> Option<Document<'e>> {
        let i = self.seen;
        self.seen += 1;
        if self.stack.is_empty() {
            self.first = (i, event.offset);
            self.build.begin(event.offset);
        }
        // The tree keeps offsets from the document's start in 32 bits.
        if event.offset - self.first.1 > super::nodes::MAX_OFFSET {
            self.unchecked
                .get_or_insert((event.offset, messages::text("schema.too-far")));
        }
        let node = match &event.kind {
            EventKind::Key(key) => {
                if let Some(Open::Object(_, _, current, _)) = self.stack.last_mut() {
                    let text = match key {
                        Cow::Borrowed(bytes) => key_text(bytes),
                        Cow::Owned(bytes) => Cow::Owned(key_text(bytes).into_owned()),
                    };
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
                    self.build.members.len(),
                    event.offset,
                    (Cow::Borrowed(""), 0, Cow::Borrowed(&[])),
                    HashMap::new(),
                ));
                return None;
            }
            EventKind::ArrayStart => {
                self.stack
                    .push(Open::Array(self.build.elems.len(), event.offset));
                return None;
            }
            EventKind::End => match self.stack.pop() {
                Some(Open::Array(start, at)) => self.build.end_array(start, at, event.offset + 1),
                Some(Open::Object(start, at, ..)) => {
                    self.build.end_object(start, at, event.offset + 1)
                }
                None => return None,
            },
            EventKind::Null => self.build.scalar(Value::Null, event.offset),
            EventKind::Bool(b) => self.build.scalar(Value::Bool(*b), event.offset),
            EventKind::String(s) => self.build.string(s.clone(), event.offset),
            EventKind::Number(text) => {
                let value = number(text).unwrap_or_else(|| {
                    self.unchecked
                        .get_or_insert((event.offset, messages::text("schema.not-a-number")));
                    Value::Null
                });
                self.build.scalar(value, event.offset)
            }
            _ => return None,
        };
        match self.stack.last_mut() {
            None => {
                return Some(Document {
                    tree: self.build.finish(node),
                    offset: self.first.1,
                    unchecked: self.unchecked.take(),
                });
            }
            Some(Open::Array(..)) => self.build.elems.push(node),
            Some(Open::Object(_, _, (key, offset, raw), seen)) => {
                let key = std::mem::take(key);
                // Real duplicates are the duplicate-key rule's; keys that only become the same
                // once lone surrogates are U+FFFD are not.
                if key.contains('\u{FFFD}') {
                    self.build.raw(node, raw.clone());
                    match seen.get(key.as_ref()) {
                        Some(earlier) if earlier != raw => {
                            self.unchecked
                                .get_or_insert((*offset, messages::text("schema.same-keys")));
                        }
                        Some(_) => {}
                        None => {
                            seen.insert(key.to_string(), std::mem::take(raw));
                        }
                    }
                }
                self.build
                    .members
                    .push((key, node, self.build.relative(*offset)));
            }
        }
        None
    }
}

/// Fold events into documents, one at a time: one, or one per line in JSON Lines. One at a
/// time, a document's values can be dropped before the next is folded.
pub(super) fn documents<'e>(events: &[Event<'e>]) -> impl Iterator<Item = Document<'e>> {
    let mut fold = Fold::default();
    events.iter().filter_map(move |event| fold.event(event))
}

/// A number as written in any dialect, or `None` for `NaN`, `Infinity` and numbers past a
/// double's range. An integer too large for 64 bits becomes a double, as `serde_json` reads one.
pub(super) fn number(text: &str) -> Option<Value> {
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
    use crate::messages;
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
        assert_eq!(documents[0].tree.value(), expected);
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

    /// The events of `text`, each moved on by the matching entry of `moves`.
    fn moved(text: &'static str, moves: &[usize]) -> Vec<jsonvet::Event<'static>> {
        let (_, events) = jsonvet::parse(text, &Config::new(Dialect::Json));
        let mut events = events.unwrap_or_default();
        for (event, by) in events.iter_mut().zip(moves) {
            event.offset += by;
        }
        events
    }

    fn first_document(events: &[jsonvet::Event<'static>]) -> super::Document<'static> {
        let mut fold = super::Fold::new("", Dialect::JsonLines);
        events
            .iter()
            .find_map(|event| fold.event(event))
            .expect("a document")
    }

    #[test]
    fn a_record_far_into_a_json_lines_file_is_checked() {
        let far = u32::MAX as usize + 10;
        let events = moved("[true]", &[far, far, far]);
        assert!(first_document(&events).unchecked.is_none());
    }

    #[test]
    fn a_record_far_into_a_json_lines_file_is_placed_where_it_is() {
        let far = u32::MAX as usize + 10;
        let events = moved("[true]", &[far, far, far]);
        let document = first_document(&events);
        assert_eq!(document.tree.place(&[(vec![], vec![])])[0].0, far);
    }

    #[test]
    fn a_value_too_far_into_one_document_is_not_checked_and_says_why() {
        let events = moved("[true]", &[0, 0, super::super::nodes::MAX_OFFSET - 4]);
        let reason = first_document(&events).unchecked.map(|(_, reason)| reason);
        assert_eq!(reason, Some(messages::text("schema.too-far")));
    }

    #[test]
    fn the_end_of_a_container_at_the_most_is_checked() {
        // The `]` of "[true]" is at 5.
        let events = moved("[true]", &[0, 0, super::super::nodes::MAX_OFFSET - 5]);
        assert!(first_document(&events).unchecked.is_none());
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
            prop_assert_eq!(&documents[0].tree.value(), &expected);
        }
    }
}
