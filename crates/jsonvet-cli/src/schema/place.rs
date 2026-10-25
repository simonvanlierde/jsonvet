//! Each schema error placed at its value's offset, with that value's path.

use std::borrow::Cow;
use std::collections::HashMap;

use jsonschema::ValidationError;
use jsonvet::{Event, EventKind, PathStep, key_text};

use super::Found;

/// The segments of `error`'s instance path, under element `index` if it is on one.
pub(super) fn pointer_path(index: Option<usize>, error: &ValidationError<'_>) -> Vec<String> {
    index
        .map(|i| i.to_string())
        .into_iter()
        .chain(segments(error.instance_path().as_str()))
        .collect()
}

/// The unescaped segments of the JSON Pointer `pointer`: `~1` is `/` and `~0` is `~`, in that
/// order, so `~01` is `~1`.
pub(super) fn segments(pointer: &str) -> impl Iterator<Item = String> + '_ {
    pointer
        .split('/')
        .skip(1)
        .map(|segment| segment.replace("~1", "/").replace("~0", "~"))
}

/// The values some errors are about, as a tree of JSON Pointer segments, with the offset of
/// each value and, for an object member, of its key.
struct Wanted<'e>(Vec<Node<'e>>);

#[derive(Default)]
struct Node<'e> {
    children: HashMap<String, usize>,
    /// The value's offset, and for an object member, its key's offset and its object's.
    at: (usize, Option<(usize, usize)>),
    /// The step to the value as written: an index, or the key's source text, which keeps lone
    /// surrogates that the pointer holds as U+FFFD.
    step: Option<PathStep<'e>>,
}

impl Wanted<'_> {
    /// The node for `key` under `node`, added if new.
    fn child(&mut self, node: usize, key: &str) -> usize {
        if let Some(&child) = self.0[node].children.get(key) {
            return child;
        }
        self.0.push(Node::default());
        let child = self.0.len() - 1;
        self.0[node].children.insert(key.to_owned(), child);
        child
    }
}

/// In an open container: an array's next index, or an object's current key as text and as
/// written, and its offset.
type Slot<'e> = Result<usize, (Cow<'e, str>, &'e [u8], usize)>;

/// An open container: its node if any error is inside it, its offset, and its next index or key.
type Container<'e> = (Option<usize>, usize, Slot<'e>);

/// Where each error's value is, found in one walk over its document's events, and that value's
/// path; for an error about a key, the path of the key's member.
pub(super) fn place_found(events: &[Event<'_>], found: &[Found]) -> Vec<(usize, String)> {
    let mut targets = Vec::with_capacity(found.len() * 2);
    for found in found {
        targets.push((found.path.clone(), found.keys.clone()));
        if found.key {
            let mut member = found.path.clone();
            member.extend(found.keys.iter().cloned());
            targets.push((member, Vec::new()));
        }
    }
    let mut spots = place(events, &targets).into_iter();
    found
        .iter()
        .filter_map(|found| {
            let (offset, at) = spots.next()?;
            if found.key {
                spots.next().map(|(_, at)| (offset, at))
            } else {
                Some((offset, at))
            }
        })
        .collect()
}

/// Where each target is, found in one walk over its document's events: given as a value's path
/// and keys in it, the first of those keys found, or else the value. With it, the value's
/// JSONPath, written as duplicate-key paths are.
pub(super) fn place(
    events: &[Event<'_>],
    targets: &[(Vec<String>, Vec<String>)],
) -> Vec<(usize, String)> {
    let mut wanted = Wanted(vec![Node::default()]);
    // For each target, the nodes on the way to its value and the nodes of its keys.
    let nodes: Vec<(Vec<usize>, Vec<usize>)> = targets
        .iter()
        .map(|(path, keys)| {
            let mut way = Vec::with_capacity(path.len());
            let mut node = 0;
            for segment in path {
                node = wanted.child(node, segment);
                way.push(node);
            }
            let keys = keys.iter().map(|key| wanted.child(node, key)).collect();
            (way, keys)
        })
        .collect();

    let mut stack: Vec<Container<'_>> = Vec::new();
    for event in events {
        match (&event.kind, stack.last_mut()) {
            (EventKind::Key(key), Some((_, _, slot))) => {
                *slot = Err((key_text(key), &**key, event.offset));
                continue;
            }
            (EventKind::End, _) => {
                stack.pop();
                continue;
            }
            _ => {}
        }
        let node = match stack.last_mut() {
            None => Some((0, None, None)),
            Some((parent, _, Ok(index))) => {
                *index += 1;
                let i = *index - 1;
                parent
                    .and_then(|p| wanted.0[p].children.get(&i.to_string()))
                    .map(|&n| (n, Some(PathStep::Index(i)), None))
            }
            Some((parent, object, Err((key, raw, offset)))) => parent
                .and_then(|p| wanted.0[p].children.get(key.as_ref()))
                .map(|&n| (n, Some(PathStep::Key(raw)), Some((*offset, *object)))),
        };
        if let Some((node, step, key)) = node {
            wanted.0[node].step = step;
            let at = &mut wanted.0[node].at;
            // A later duplicate key's value overwrites, as it does in the folded value, but the
            // key stays at its first copy in that object.
            at.0 = event.offset;
            if at.1.map(|(_, object)| object) != key.map(|(_, object)| object) {
                at.1 = key;
            }
        }
        let node = node
            .map(|(n, ..)| n)
            .filter(|&n| !wanted.0[n].children.is_empty());
        match event.kind {
            EventKind::ObjectStart => {
                stack.push((node, event.offset, Err((Cow::Borrowed(""), &[], 0))));
            }
            EventKind::ArrayStart => stack.push((node, event.offset, Ok(0))),
            _ => {}
        }
    }

    nodes
        .into_iter()
        .zip(targets)
        .map(|((way, keys), (path, _))| {
            let node = way.last().copied().unwrap_or(0);
            let offset = keys
                .iter()
                .filter_map(|&key| wanted.0[key].at.1.map(|(offset, _)| offset))
                .min()
                .unwrap_or(wanted.0[node].at.0);
            // Every value an error is about is in the events, so each step is found.
            let steps = way.iter().zip(path).map(|(&n, segment)| {
                wanted.0[n]
                    .step
                    .unwrap_or(PathStep::Key(segment.as_bytes()))
            });
            (offset, jsonvet::json_path(steps))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::fold::documents;
    use super::super::found;
    use super::{place, place_found, segments};
    use jsonvet::{Config, Dialect};
    use serde_json::json;

    #[test]
    fn a_later_duplicate_key_wins_in_the_value_and_its_position() {
        let text = r#"{"a": [1, {"b": 2}], "b\u00e9/~": "x", "a": "last"}"#;
        let (_, events) = jsonvet::parse(text, &Config::new(Dialect::Json));
        let events = events.unwrap_or_default();
        let documents = documents(&events);
        assert_eq!(documents[0].value, json!({"a": "last", "b\u{e9}/~": "x"}));
        let schema =
            json!({"properties": {"a": {"type": "integer"}, "b\u{e9}/~": {"type": "integer"}}});
        let validator = jsonschema::validator_for(&schema).expect("a schema");
        let errors: Vec<_> = validator
            .iter_errors(&documents[0].value)
            .flat_map(|e| found(None, &documents[0].value, &e, String::new))
            .collect();
        let paths: Vec<_> = errors.iter().map(|e| e.path.join("/")).collect();
        assert_eq!(paths, ["a", "b\u{e9}/~"]);
        let located: Vec<_> = place_found(&events, &errors)
            .into_iter()
            .map(|(o, _)| o)
            .collect();
        assert_eq!(located, [44, 34]);
    }

    #[test]
    fn pointers_become_the_paths_duplicate_key_writes() {
        let text = r#"{"": 1, "0": [true, {"a/b~c.d\"\u00e9": 2}], "x": {"": 3}, "\ud800": 4}"#;
        let (_, events) = jsonvet::parse(text, &Config::new(Dialect::Json));
        let events = events.unwrap_or_default();
        let cases = [
            ("", "$"),
            ("/", "$[\"\"]"),
            ("/x/", "$.x[\"\"]"),
            ("/0", "$[\"0\"]"),
            ("/0/1", "$[\"0\"][1]"),
            ("/0/1/a~1b~0c.d\"\u{e9}", "$[\"0\"][1][\"a/b~c.d\\\"é\"]"),
            ("/\u{FFFD}", "$[\"\\ud800\"]"),
        ];
        for (pointer, path) in cases {
            let target = (segments(pointer).collect(), Vec::new());
            assert_eq!(place(&events, &[target])[0].1, path, "{pointer}");
        }
    }
}
