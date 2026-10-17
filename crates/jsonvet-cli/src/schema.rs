//! `--schema`: each file's values checked against one JSON Schema, every error placed at the
//! value it is about.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::Path;

use jsonschema::error::ValidationErrorKind;
use jsonschema::{PatternOptions, ReferencingError, ValidationError, Validator};
use jsonvet::{Event, EventKind, PathStep, Severity};
use percent_encoding::percent_decode_str;
use serde_json::{Map, Number, Value};

use crate::settings::Settings;

/// One document's values, and the range of the events they were folded from.
struct Document {
    value: Value,
    events: Range<usize>,
    /// Why the values are not the text's, and where: the first such number or key.
    unchecked: Option<(usize, &'static str)>,
}

const NOT_A_NUMBER: &str =
    "NaN, Infinity and numbers beyond a double's range have no JSON Schema value";
// shortcut: two copies of one key holding U+FFFD or a lone surrogate also land here, though they
// were already the same; tell them apart by the key's source text if that is ever reported.
const SAME_KEYS: &str =
    "this key and an earlier one in the object are the same once lone surrogates become U+FFFD";

/// How many schema errors one document reports, in the order jsonschema finds them; one more
/// line counts the rest.
const MAX_ERRORS: usize = 100;

/// Past this many bytes, a message says "value" instead of quoting the value, and past it
/// still, it is cut short.
const MAX_MESSAGE: usize = 200;

/// How deep a schema may nest, counting each local `$ref` followed as one more level; real
/// schemas stay well under it.
const MAX_SCHEMA_DEPTH: usize = 128;

/// The keys whose value points at another schema.
const REF_KEYS: [&str; 3] = ["$ref", "$dynamicRef", "$recursiveRef"];

/// How many times one regex match may backtrack before it fails. About one step per character
/// for a lookaround over the whole string, so a string over a few thousand characters can hit it.
const BACKTRACK_LIMIT: usize = 5_000;

pub(crate) struct Schema(Validator);

impl Schema {
    /// Read and compile the schema at `path`, in its dialect under `settings`. Any problem
    /// with it is a usage error.
    pub(crate) fn load(path: &Path, settings: &Settings) -> Result<Schema, String> {
        let name = path.to_string_lossy();
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read the schema {name}: {e}"))?;
        // jsonschema compiles a schema recursively: a deep one can take gigabytes or the stack.
        let config = settings.config_for(&name).max_depth(MAX_SCHEMA_DEPTH);
        let (report, events) = jsonvet::parse(&text, &config);
        let at = |offset, problem: &dyn std::fmt::Display| {
            let at = report.location(offset);
            format!("the schema {name}:{}:{}: {problem}", at.line, at.column)
        };
        if let Some(problem) = report
            .problems()
            .iter()
            .find(|p| p.severity() == Severity::Error)
        {
            return Err(at(problem.offset(), problem));
        }
        let events = events.unwrap_or_default();
        let mut documents = documents(&events);
        if documents.len() != 1 {
            return Err(format!("the schema {name} must hold one value"));
        }
        let document = documents.swap_remove(0);
        if let Some((offset, reason)) = document.unchecked {
            return Err(at(offset, &reason));
        }
        // jsonschema also validates recursively through each `$ref`, so a long chain of them
        // takes the stack or hours, however shallow the schema is.
        if let Some((path, key)) = Reach::new(&document.value).too_deep() {
            let offset = place(
                &events[document.events.clone()],
                &[(path, vec![key.to_owned()])],
            )[0];
            return Err(at(
                offset,
                &format!(
                    "this `{key}` leads more than {MAX_SCHEMA_DEPTH} levels deep, counting each \
                     `$ref` followed as a level"
                ),
            ));
        }
        // Offline: a `$ref` to anything outside the schema file fails here.
        let options = jsonschema::options()
            .offline()
            // fancy-regex backtracks for back-references and lookaround, and its default limit
            // of a million lets one 30-byte string take 30 ms.
            .with_pattern_options(PatternOptions::fancy_regex().backtrack_limit(BACKTRACK_LIMIT));
        let error = match options.build(&document.value) {
            Ok(validator) => return Ok(Schema(validator)),
            Err(error) => error,
        };
        // A failed `$ref` has no instance path to place it by.
        if let ValidationErrorKind::Referencing(ReferencingError::Unretrievable { uri, .. }) =
            error.kind()
        {
            return Err(format!(
                "the schema {name} has a `$ref` to {uri}, outside the file; jsonvet fetches no \
                 schemas"
            ));
        }
        let offset = locate(&events[document.events], std::slice::from_ref(&error))[0];
        Err(at(offset, &error))
    }

    /// The schema errors in one file's values, as (offset, message, summary): each document's
    /// in order, then a summary counting any past [`MAX_ERRORS`].
    pub(crate) fn errors(&self, events: &[Event<'_>]) -> Vec<(usize, String, bool)> {
        let mut errors = Vec::new();
        for document in documents(events) {
            if let Some((offset, reason)) = document.unchecked {
                errors.push((
                    offset,
                    format!("{reason}, so this document is not checked against the schema"),
                    false,
                ));
                continue;
            }
            let mut all = self.0.iter_errors(&document.value);
            let found: Vec<ValidationError<'_>> = all.by_ref().take(MAX_ERRORS).collect();
            if found.is_empty() {
                continue;
            }
            let events = &events[document.events];
            let mut placed: Vec<(usize, String, bool)> = locate(events, &found)
                .into_iter()
                .zip(&found)
                .map(|(offset, error)| {
                    let at = json_path(&document.value, error.instance_path().as_str());
                    (offset, cut(format!("{} (at {at})", message(error))), false)
                })
                .collect();
            // Stable, so errors at one value keep the schema's order.
            placed.sort_by_key(|(offset, ..)| *offset);
            errors.append(&mut placed);
            let more = all.count();
            if more > 0 {
                errors.push((
                    events[0].offset,
                    format!("... and {more} more schema errors in this document"),
                    true,
                ));
            }
        }
        errors
    }
}

/// The JSONPath of the value at the JSON Pointer `pointer` in `value`, written as duplicate-key
/// paths are. The pointer alone cannot tell the index `0` from the key `"0"`, so each segment
/// is read as an index only where the value it steps into is an array.
// shortcut: a key holding a lone surrogate is written with U+FFFD here and with the surrogate's
// escape in a duplicate-key path; carry the key's source text if that is ever reported.
fn json_path(value: &Value, pointer: &str) -> String {
    let segments: Vec<String> = segments(pointer).collect();
    let mut value = Some(value);
    jsonvet::json_path(segments.iter().map(|segment| {
        if let (Some(Value::Array(items)), Ok(i)) = (value, segment.parse::<usize>()) {
            value = items.get(i);
            return PathStep::Index(i);
        }
        value = value.and_then(|v| v.get(segment));
        PathStep::Key(segment)
    }))
}

/// The unescaped segments of the JSON Pointer `pointer`: `~1` is `/` and `~0` is `~`, in that
/// order, so `~01` is `~1`.
fn segments(pointer: &str) -> impl Iterator<Item = String> + '_ {
    pointer
        .split('/')
        .skip(1)
        .map(|segment| segment.replace("~1", "/").replace("~0", "~"))
}

/// An error's message, quoting the value it is about only when that is short.
fn message(error: &ValidationError<'_>) -> String {
    if fits(error.instance()) {
        let message = error.to_string();
        if message.len() <= MAX_MESSAGE {
            return message;
        }
    }
    error.masked().to_string()
}

/// Whether `value` as compact JSON takes [`MAX_MESSAGE`] bytes or fewer. Writing stops at the
/// first byte past that.
fn fits(value: &Value) -> bool {
    struct Budget(usize);
    impl std::io::Write for Budget {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_sub(bytes.len())
                .ok_or(std::io::ErrorKind::WriteZero)?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Budget(MAX_MESSAGE), value).is_ok()
}

/// `message` cut to [`MAX_MESSAGE`] bytes at a character boundary, ending in `...` if cut.
fn cut(mut message: String) -> String {
    if message.len() > MAX_MESSAGE {
        let mut end = MAX_MESSAGE;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
        message.push_str("...");
    }
    message
}

/// The values some errors are about, as a tree of JSON Pointer segments, with the offset of
/// each value and, for an object member, of its key.
struct Wanted(Vec<Node>);

#[derive(Default)]
struct Node {
    children: HashMap<String, usize>,
    /// The value's offset, and for an object member, its key's offset and its object's.
    at: (usize, Option<(usize, usize)>),
}

impl Wanted {
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

/// In an open container: an array's next index, or an object's current key and its offset.
type Slot<'e> = Result<usize, (&'e str, usize)>;

/// An open container: its node if any error is inside it, its offset, and its next index or key.
type Container<'e> = (Option<usize>, usize, Slot<'e>);

/// Where each error is, found in one walk over its document's events: its value; for unexpected
/// properties, the first such key; for a property name, that key.
fn locate(events: &[Event<'_>], errors: &[ValidationError<'_>]) -> Vec<usize> {
    let targets: Vec<(Vec<String>, Vec<String>)> = errors
        .iter()
        .map(|error| {
            let keys = match error.kind() {
                ValidationErrorKind::AdditionalProperties { unexpected }
                | ValidationErrorKind::UnevaluatedProperties { unexpected } => unexpected.clone(),
                ValidationErrorKind::PropertyNames { error } => error
                    .instance()
                    .as_str()
                    .map(|key| vec![key.to_owned()])
                    .unwrap_or_default(),
                _ => Vec::new(),
            };
            (segments(error.instance_path().as_str()).collect(), keys)
        })
        .collect();
    place(events, &targets)
}

/// Where each target is, found in one walk over its document's events: given as a value's path
/// and keys in it, the first of those keys found, or else the value.
fn place(events: &[Event<'_>], targets: &[(Vec<String>, Vec<String>)]) -> Vec<usize> {
    let mut wanted = Wanted(vec![Node::default()]);
    // For each target, its value's node and the nodes of its keys.
    let nodes: Vec<(usize, Vec<usize>)> = targets
        .iter()
        .map(|(path, keys)| {
            let mut node = 0;
            for segment in path {
                node = wanted.child(node, segment);
            }
            let keys = keys.iter().map(|key| wanted.child(node, key)).collect();
            (node, keys)
        })
        .collect();

    let mut stack: Vec<Container<'_>> = Vec::new();
    for event in events {
        match (&event.kind, stack.last_mut()) {
            (EventKind::Key(key), Some((_, _, slot))) => {
                *slot = Err((key, event.offset));
                continue;
            }
            (EventKind::End, _) => {
                stack.pop();
                continue;
            }
            _ => {}
        }
        let node = match stack.last_mut() {
            None => Some((0, None)),
            Some((parent, _, Ok(index))) => {
                *index += 1;
                let key = *index - 1;
                parent
                    .and_then(|p| wanted.0[p].children.get(&key.to_string()))
                    .map(|&n| (n, None))
            }
            Some((parent, object, Err((key, offset)))) => parent
                .and_then(|p| wanted.0[p].children.get(*key))
                .map(|&n| (n, Some((*offset, *object)))),
        };
        if let Some((node, key)) = node {
            let at = &mut wanted.0[node].at;
            // A later duplicate key's value overwrites, as it does in the folded value, but the
            // key stays at its first copy in that object.
            at.0 = event.offset;
            if at.1.map(|(_, object)| object) != key.map(|(_, object)| object) {
                at.1 = key;
            }
        }
        let node = node
            .map(|(n, _)| n)
            .filter(|&n| !wanted.0[n].children.is_empty());
        match event.kind {
            EventKind::ObjectStart => stack.push((node, event.offset, Err(("", 0)))),
            EventKind::ArrayStart => stack.push((node, event.offset, Ok(0))),
            _ => {}
        }
    }

    nodes
        .into_iter()
        .map(|(node, keys)| {
            keys.iter()
                .filter_map(|&key| wanted.0[key].at.1.map(|(offset, _)| offset))
                .min()
                .unwrap_or(wanted.0[node].at.0)
        })
        .collect()
}

/// How deep a schema reaches when its local `$ref`s are followed: `#` with a JSON Pointer or an
/// anchor (`$anchor`, `$dynamicAnchor`, or an `$id` of `#name`), or a URI naming an `$id` in the
/// file. A `$ref` back to a schema on the way to it is recursion, which jsonschema handles: it
/// adds nothing.
// shortcut: a URI matches an `$id` by its text or last path segments, not by full RFC 3986
// resolution against each base, and a relative `#` reference resolves against the root; a
// missed match only undercounts. Resolve as jsonschema's `referencing` crate does if one slips by.
struct Reach<'v> {
    root: &'v Value,
    anchors: HashMap<&'v str, (&'v Value, Vec<String>)>,
    ids: Vec<(&'v str, &'v Value, Vec<String>)>,
    /// The levels at and under each schema already measured, by address.
    height: HashMap<*const Value, usize>,
    /// The schemas on the way to the current one.
    open: HashSet<*const Value>,
    /// Where the current schema is in the file.
    path: Vec<String>,
    /// Where each `$ref` on the way is: its object's path and the key.
    refs: Vec<(Vec<String>, &'static str)>,
}

impl<'v> Reach<'v> {
    fn new(root: &'v Value) -> Self {
        let mut reach = Reach {
            root,
            anchors: HashMap::new(),
            ids: Vec::new(),
            height: HashMap::new(),
            open: HashSet::new(),
            path: Vec::new(),
            refs: Vec::new(),
        };
        reach.collect(root, &mut Vec::new());
        reach
    }

    /// Note the anchors and `$id`s in `value`, which is at `path`.
    fn collect(&mut self, value: &'v Value, path: &mut Vec<String>) {
        match value {
            Value::Object(members) => {
                for (key, member) in members {
                    if let Value::String(name) = member {
                        match key.as_str() {
                            "$anchor" | "$dynamicAnchor" => {
                                self.anchors.insert(name, (value, path.clone()));
                            }
                            "$id" | "id" => match name.strip_prefix('#') {
                                Some(anchor) => {
                                    self.anchors.insert(anchor, (value, path.clone()));
                                }
                                None => self.ids.push((name, value, path.clone())),
                            },
                            _ => {}
                        }
                    }
                    path.push(key.clone());
                    self.collect(member, path);
                    path.pop();
                }
            }
            Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    path.push(i.to_string());
                    self.collect(item, path);
                    path.pop();
                }
            }
            _ => {}
        }
    }

    /// The schema a reference points at, and its path, if it is in the file.
    fn resolve(&self, reference: &str) -> Option<(&'v Value, Vec<String>)> {
        let (uri, fragment) = reference.split_once('#').unwrap_or((reference, ""));
        let (resource, mut path) = if uri.is_empty() {
            (self.root, Vec::new())
        } else {
            let (_, value, path) = self.ids.iter().find(|(id, ..)| {
                let id = id.split_once('#').map_or(*id, |(id, _)| id);
                id == uri || id.ends_with(&format!("/{uri}")) || uri.ends_with(&format!("/{id}"))
            })?;
            (*value, path.clone())
        };
        if fragment.is_empty() {
            return Some((resource, path));
        }
        if !fragment.starts_with('/') {
            return self.anchors.get(fragment).cloned();
        }
        let mut value = resource;
        for segment in segments(&percent_decode_str(fragment).decode_utf8_lossy()) {
            value = match value {
                Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
                _ => value.get(&segment)?,
            };
            path.push(segment);
        }
        Some((value, path))
    }

    /// Where a `$ref` leads past [`MAX_SCHEMA_DEPTH`] levels, as its object's path and key.
    fn too_deep(mut self) -> Option<(Vec<String>, &'static str)> {
        self.visit(self.root, 1).err()
    }

    /// The levels at and under `value`, which is `level` levels deep, or where that is too many.
    fn visit(
        &mut self,
        value: &'v Value,
        level: usize,
    ) -> Result<usize, (Vec<String>, &'static str)> {
        if !(value.is_object() || value.is_array()) {
            return Ok(0);
        }
        let address: *const Value = value;
        let known = self.height.get(&address).copied();
        if level + known.unwrap_or(1) - 1 > MAX_SCHEMA_DEPTH {
            // Containers alone stay within the parser's depth limit, so a `$ref` is on the way.
            return Err(self.refs.last().cloned().unwrap_or((Vec::new(), "$ref")));
        }
        if let Some(height) = known {
            return Ok(height);
        }
        self.open.insert(address);
        let mut below = 0;
        match value {
            Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    self.path.push(i.to_string());
                    below = below.max(self.visit(item, level + 1)?);
                    self.path.pop();
                }
            }
            Value::Object(members) => {
                for (key, member) in members {
                    self.path.push(key.clone());
                    below = below.max(self.visit(member, level + 1)?);
                    self.path.pop();
                }
                for key in REF_KEYS {
                    let Some((target, path)) = members
                        .get(key)
                        .and_then(Value::as_str)
                        .and_then(|reference| self.resolve(reference))
                    else {
                        continue;
                    };
                    if self.open.contains(&std::ptr::from_ref(target)) {
                        continue;
                    }
                    self.refs.push((self.path.clone(), key));
                    let here = std::mem::replace(&mut self.path, path);
                    below = below.max(self.visit(target, level + 1)?);
                    self.path = here;
                    self.refs.pop();
                }
            }
            _ => {}
        }
        self.open.remove(&address);
        self.height.insert(address, below + 1);
        Ok(below + 1)
    }
}

/// An open container while the events are folded into values, with an object's current key.
enum Open {
    Array(Vec<Value>),
    Object(Map<String, Value>, (String, usize)),
}

/// Fold events into documents: one, or one per line in JSON Lines.
fn documents(events: &[Event<'_>]) -> Vec<Document> {
    let mut documents = Vec::new();
    let mut first_event = 0;
    let mut stack: Vec<Open> = Vec::new();
    let mut unchecked = None;
    for (i, event) in events.iter().enumerate() {
        let value = match &event.kind {
            EventKind::Key(key) => {
                if let Some(Open::Object(_, current)) = stack.last_mut() {
                    *current = (key.to_string(), event.offset);
                }
                continue;
            }
            EventKind::ObjectStart => {
                stack.push(Open::Object(Map::new(), (String::new(), 0)));
                continue;
            }
            EventKind::ArrayStart => {
                stack.push(Open::Array(Vec::new()));
                continue;
            }
            EventKind::End => match stack.pop() {
                Some(Open::Array(items)) => Value::Array(items),
                Some(Open::Object(members, _)) => Value::Object(members),
                None => continue,
            },
            EventKind::Null => Value::Null,
            EventKind::Bool(b) => Value::Bool(*b),
            EventKind::String(s) => Value::String(s.to_string()),
            EventKind::Number(text) => number(text).unwrap_or_else(|| {
                unchecked.get_or_insert((event.offset, NOT_A_NUMBER));
                Value::Null
            }),
            _ => continue,
        };
        match stack.last_mut() {
            None => documents.push(Document {
                value,
                events: std::mem::replace(&mut first_event, i + 1)..i + 1,
                unchecked: unchecked.take(),
            }),
            Some(Open::Array(items)) => items.push(value),
            Some(Open::Object(members, (key, offset))) => {
                let key = std::mem::take(key);
                // Real duplicates are the duplicate-key rule's; lone surrogates are not.
                if key.contains('\u{FFFD}') && members.contains_key(&key) {
                    unchecked.get_or_insert((*offset, SAME_KEYS));
                }
                members.insert(key, value);
            }
        }
    }
    documents
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
mod tests {
    use super::{MAX_MESSAGE, Reach, cut, documents, fits, json_path, locate, message, number};
    use jsonvet::{Config, Dialect};
    use proptest::prelude::*;
    use serde_json::{Value, json};

    #[test]
    fn local_refs_resolve_by_pointer_anchor_and_id() {
        let schema = json!({
            "$id": "https://example.com/root.json",
            "$defs": {
                "a/b": [{"$anchor": "x"}, {"type": "string"}],
                "%": {"$id": "#old"},
                "sub": {"$id": "sub.json", "$defs": {"c": true}},
            },
        });
        let reach = Reach::new(&schema);
        let path = |reference: &str| reach.resolve(reference).map(|(_, path)| path.join("/"));
        assert_eq!(path("#").as_deref(), Some(""));
        assert_eq!(path("#/$defs/a~1b/1").as_deref(), Some("$defs/a/b/1"));
        assert_eq!(path("#/$defs/%25").as_deref(), Some("$defs/%"));
        assert_eq!(path("#x").as_deref(), Some("$defs/a/b/0"));
        assert_eq!(path("#old").as_deref(), Some("$defs/%"));
        assert_eq!(path("root.json#/$defs").as_deref(), Some("$defs"));
        assert_eq!(
            path("https://example.com/sub.json#/$defs/c").as_deref(),
            Some("$defs/sub/$defs/c")
        );
        assert_eq!(path("#/$defs/none"), None);
        assert_eq!(path("other.json"), None);
    }

    #[test]
    fn events_fold_into_the_values_serde_json_reads() {
        let text = r#"{"a": [1, -2.5e3, true, null, {"b": [false, "c"]}], "b\u00e9/~": "x\n\ud83d\ude00"}"#;
        let (_, events) = jsonvet::parse(text, &Config::new(Dialect::Json));
        let documents = documents(&events.unwrap_or_default());
        let expected: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].value, expected);
    }

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
        let errors: Vec<_> = validator.iter_errors(&documents[0].value).collect();
        let pointers: Vec<_> = errors.iter().map(|e| e.instance_path().as_str()).collect();
        assert_eq!(pointers, ["/a", "/b\u{e9}~1~0"]);
        assert_eq!(locate(&events, &errors), [44, 34]);
    }

    #[test]
    fn pointers_become_the_paths_duplicate_key_writes() {
        let value = json!({"": 1, "0": [true, {"a/b~c.d\"\u{e9}": 2}], "x": {"": 3}});
        let cases = [
            ("", "$"),
            ("/", "$[\"\"]"),
            ("/x/", "$.x[\"\"]"),
            ("/0", "$[\"0\"]"),
            ("/0/1", "$[\"0\"][1]"),
            (
                "/0/1/a~1b~0c.d\"\u{e9}",
                "$[\"0\"][1][\"a/b~c.d\\\"\\u00e9\"]",
            ),
        ];
        for (pointer, path) in cases {
            assert_eq!(json_path(&value, pointer), path, "{pointer}");
        }
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

    #[test]
    fn a_large_nested_value_is_named_not_quoted() {
        let validator = jsonschema::validator_for(&json!({"type": "string"})).expect("a schema");
        let long = "x".repeat(300);
        for instance in [
            json!([{"a": [long.clone()]}]),
            json!({"a": {"b": long.clone()}}),
            json!({ long.clone(): 1 }),
        ] {
            let error = validator.iter_errors(&instance).next().expect("an error");
            assert_eq!(message(&error), "value is not of type \"string\"");
        }
        let small = json!([{"a": [1]}]);
        let error = validator.iter_errors(&small).next().expect("an error");
        assert_eq!(message(&error), "[{\"a\":[1]}] is not of type \"string\"");
    }

    #[test]
    fn a_value_fits_while_its_json_takes_at_most_the_budget() {
        let x = |n| json!("x".repeat(n));
        assert!(fits(&x(MAX_MESSAGE - 2)));
        assert!(!fits(&x(MAX_MESSAGE - 1)));
    }

    #[test]
    fn a_long_message_is_cut_at_a_character_boundary() {
        // `é` takes two bytes, and the one-byte `x` puts the cut inside one.
        let long = format!("x{}", "\u{e9}".repeat(200));
        let message = cut(long.clone());
        assert_eq!(message.len(), MAX_MESSAGE - 1 + "...".len());
        assert!(message.ends_with("\u{e9}..."), "{message}");
        assert_eq!(
            cut(long[..MAX_MESSAGE - 1].to_owned()),
            long[..MAX_MESSAGE - 1]
        );
        let full = "x".repeat(MAX_MESSAGE);
        assert_eq!(cut(full.clone()), full);
    }

    /// A schema `depth` levels deep: `allOf` arrays `depth / 2` times, then `last`.
    fn nested(depth: usize, last: Value) -> Value {
        (0..depth / 2).fold(last, |inner, _| json!({"allOf": [inner]}))
    }

    #[test]
    fn ref_chains_count_through_arrays_and_from_each_place_they_are_reached() {
        let too_deep = |schema: &Value| {
            Reach::new(schema)
                .too_deep()
                .map(|(path, key)| (path.join("/"), key))
        };
        // `$defs/long` is first reached near the root, then again under 100 levels of `allOf`.
        let long = nested(60, json!({"type": "integer"}));
        let schema = json!({
            "$defs": {"long": long},
            "allOf": [nested(100, json!({"$ref": "#/$defs/long"}))],
        });
        let path = format!("allOf/0{}", "/allOf/0".repeat(50));
        assert_eq!(too_deep(&schema), Some((path, "$ref")));
        // The same `$ref` reached only near the root stays under the cap.
        let schema = json!({"$defs": {"long": nested(60, json!(true))}, "allOf": [{"$ref": "#/$defs/long"}]});
        assert_eq!(too_deep(&schema), None);
        // A chain of `$ref`s in arrays, and a pointer whose escape sits mid-segment.
        let mut defs = serde_json::Map::new();
        for i in 0..100 {
            defs.insert(
                format!("d%{i}"),
                json!({"allOf": [{"$ref": format!("#/$defs/d%25{}", i + 1)}]}),
            );
        }
        let schema = json!({"$defs": defs, "$ref": "#/$defs/d%250"});
        assert!(too_deep(&schema).is_some());
    }

    /// Any JSON value, with numbers `serde_json` reads exactly without its `float_roundtrip`
    /// feature, and strings that need escapes.
    fn any_json() -> impl Strategy<Value = Value> {
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
            let documents = documents(&events.unwrap_or_default());
            let expected: Value = serde_json::from_str(&text).unwrap_or_default();
            prop_assert_eq!(documents.len(), 1);
            prop_assert_eq!(&documents[0].value, &expected);
        }
    }
}
