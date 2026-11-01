//! A schema's events folded straight into the `serde_json` value it is compiled from, which costs
//! less than the tree and a copy of it: the tree is built again from the events only to place an
//! error in the schema.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;

use jsonvet::{Event, EventKind, key_text};
use serde_json::{Map, Value};

use super::fold::number;
use crate::messages;

/// One document's values, and the range of the events they were folded from.
pub(super) struct Document {
    pub(super) value: Value,
    pub(super) events: Range<usize>,
    /// Why the values are not the text's, and where: the first such number or key.
    pub(super) unchecked: Option<(usize, &'static str)>,
}

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
                self.unchecked
                    .get_or_insert((event.offset, messages::text("schema.not-a-number")));
                Value::Null
            }),
            _ => return None,
        };
        match self.stack.last_mut() {
            None => {
                return Some(Document {
                    value,
                    events: self.first.0..i + 1,
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
                            self.unchecked
                                .get_or_insert((*offset, messages::text("schema.same-keys")));
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

#[cfg(test)]
mod tests {
    use super::super::fold::{self, tests::any_json};
    use super::documents;
    use jsonvet::{Config, Dialect};
    use proptest::prelude::*;

    #[test]
    fn keys_that_only_read_the_same_after_lone_surrogates_are_not_checked() {
        let unchecked = |text: &str| {
            let (_, events) = jsonvet::parse(text, &Config::new(Dialect::Json5));
            let events = events.unwrap_or_default();
            documents(&events).next().map(|d| d.unchecked.is_some())
        };
        assert_eq!(unchecked(r#"{"\ud800": 1, "\ud801": 2}"#), Some(true));
        assert_eq!(unchecked(r#"{"\ud800": 1, "\ud800": 2}"#), Some(false));
    }

    fn same(text: &str) {
        let (_, events) = jsonvet::parse(text, &Config::new(Dialect::Json5));
        let events = events.unwrap_or_default();
        let direct: Vec<_> = documents(&events).map(|d| d.value).collect();
        let tree: Vec<_> = fold::documents(&events).map(|d| d.tree.value()).collect();
        assert_eq!(direct, tree, "{text}");
    }

    #[test]
    fn the_schema_fold_reads_what_the_tree_does() {
        for text in [
            r#"{"a": [1, {"b": 2}], "bé/~": "x", "a": "last"}"#,
            r#"{"\ud800": 1, "a": [0x10, +1, .5, 5.], 'q': null}"#,
            "[1, [2, {}], true]",
        ] {
            same(text);
        }
    }

    proptest! {
        #[test]
        fn the_schema_fold_reads_any_json_as_the_tree_does(value in any_json()) {
            same(&serde_json::to_string(&value).unwrap_or_default());
        }
    }
}
