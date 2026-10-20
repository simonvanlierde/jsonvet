//! `--schema`: each file's values checked against one JSON Schema, every error placed at the
//! value it is about.

use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap};
use std::ops::Range;
use std::path::Path;

use jsonschema::error::ValidationErrorKind;
use jsonschema::{
    Draft, PatternOptions, ReferencingError, Registry, ValidationError, ValidationOptions,
    Validator,
};
use jsonvet::{Dialect, Event, EventKind, PathStep, Severity, key_text};
use referencing::{Resolver, ResourceRef};
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
const SAME_KEYS: &str =
    "this key and an earlier one in the object are the same once lone surrogates become U+FFFD";

/// How many schema errors one document reports, in the order jsonschema finds them; one more
/// line says there are more.
const MAX_ERRORS: usize = 100;

/// Past this many bytes, a message says "value" instead of quoting the value, and past it
/// still, it is cut short.
const MAX_MESSAGE: usize = 200;

/// How deep a schema may nest, counting each local `$ref` followed as one more level; real
/// schemas stay well under it.
const MAX_SCHEMA_DEPTH: usize = 128;

/// How many objects and arrays a schema may hold once each `$ref` is replaced by what it points
/// at, with the values in each `enum` and `const`. jsonschema may check every one of them for
/// one value, so `$ref`s that each point twice at the next schema, 24 levels deep, took 12.9 s
/// and 11.4 GB.
const MAX_SCHEMA_SIZE: usize = 5_000_000;

/// The keys whose value points at another schema.
const REF_KEYS: [&str; 3] = ["$ref", "$dynamicRef", "$recursiveRef"];

/// The keywords that make jsonschema check the schemas beside them that apply to the same
/// value once more, to find what they evaluated: those under [`IN_PLACE`] and the `$ref`s.
const UNEVALUATED: [&str; 2] = ["unevaluatedProperties", "unevaluatedItems"];

/// The keywords whose schemas apply to the value itself, not to its elements or members.
const IN_PLACE: [&str; 7] = [
    "allOf",
    "anyOf",
    "oneOf",
    "if",
    "then",
    "else",
    "dependentSchemas",
];

/// The base URI jsonschema gives a schema without an `$id`.
const DEFAULT_BASE: &str = "json-schema:///";

/// How many times one regex match may backtrack before it fails. About one step per character
/// for a lookaround over the whole string, so a string over a few thousand characters can hit it.
const BACKTRACK_LIMIT: usize = 5_000;

/// The keys of a schema that check nothing themselves, kept beside the part checked on its own
/// when a schema is split, so that its `$ref`s resolve as before.
const NOT_CHECKS: [&str; 9] = [
    "$schema",
    "$id",
    "id",
    "$defs",
    "definitions",
    "$anchor",
    "$dynamicAnchor",
    "$vocabulary",
    "$comment",
];

/// The keywords that can apply a schema to an array's elements or an object's members besides
/// `items`, `properties`, `patternProperties` and `additionalProperties`, or change what those
/// check.
const ON_CHILDREN: [&str; 13] = [
    "allOf",
    "anyOf",
    "oneOf",
    "not",
    "if",
    "then",
    "else",
    "$ref",
    "$dynamicRef",
    "$recursiveRef",
    "dependentSchemas",
    "dependencies",
    "extends",
];

/// The keywords that check each member of an object on its own.
const ON_MEMBERS: [&str; 3] = ["properties", "patternProperties", "additionalProperties"];

/// Where the part of a split schema checked on each element or member is moved to.
const EACH: &str = "jsonvet-each";

pub(crate) struct Schema {
    validator: Validator,
    /// For a schema whose elements only `items` checks: the schema without `items`, checked
    /// once on the whole array, and `items` alone, checked on each element. jsonschema collects
    /// every error before handing out the first, so a 10 MB array failing everywhere took 2.0 GB
    /// in one pass; one element at a time, it holds at most one element's errors.
    items: Option<Split>,
    /// The same for an object whose members only `properties`, `patternProperties` and
    /// `additionalProperties` check, each member checked as an object of its own.
    members: Option<Split>,
    /// `{"pattern": p}` for each regex in the schema tried on keys: `patternProperties` and a
    /// `pattern` under `propertyNames`.
    key_patterns: Vec<(String, Validator)>,
    /// `{"pattern": p}` for each `pattern` in the schema, tried on strings.
    string_patterns: Vec<(String, Validator)>,
}

/// A schema split in two: the part checked on the whole value, and the part checked on each
/// element or member.
struct Split {
    whole: Validator,
    each: Validator,
}

/// The members of an object by key, or the elements of an array by index; nothing for any other
/// value.
fn children(value: &Value) -> impl Iterator<Item = (Cow<'_, str>, &Value)> {
    let members = value.as_object().into_iter().flatten();
    let items = value.as_array().into_iter().flatten().enumerate();
    members
        .map(|(key, member)| (Cow::Borrowed(key.as_str()), member))
        .chain(items.map(|(i, item)| (Cow::Owned(i.to_string()), item)))
}

/// Whether `error` says a regex match ran past [`BACKTRACK_LIMIT`].
fn backtracked(error: &ValidationError<'_>) -> bool {
    match error.kind() {
        ValidationErrorKind::BacktrackLimitExceeded { .. } => true,
        ValidationErrorKind::PropertyNames { error } => backtracked(error),
        _ => false,
    }
}

/// The regexes in `schema`: those tried on keys, and every `pattern`, tried on strings.
fn patterns<'v>(
    schema: &'v Value,
    names: bool,
    keys: &mut BTreeSet<&'v str>,
    strings: &mut BTreeSet<&'v str>,
) {
    for (key, member) in children(schema) {
        match (key.as_ref(), member) {
            ("patternProperties", Value::Object(regexes)) => {
                keys.extend(regexes.keys().map(String::as_str));
            }
            ("pattern", Value::String(regex)) => {
                strings.insert(regex);
                if names {
                    keys.insert(regex);
                }
            }
            _ => {}
        }
        patterns(member, names || key == "propertyNames", keys, strings);
    }
}

/// A schema error ready to be placed: the path of the value it is about, the keys in that value
/// it is placed at (the first one found), and its message. With `key`, it is about the one key
/// in `keys` itself, and names that member's path.
struct Found {
    path: Vec<String>,
    keys: Vec<String>,
    key: bool,
    message: String,
}

/// The first [`MAX_ERRORS`] errors in a document, and whether there are more.
#[derive(Default)]
struct Errors {
    kept: Vec<Found>,
    more: bool,
}

impl Errors {
    /// Keep each of `found` while there is room, and say whether to look for more.
    fn add(&mut self, found: impl IntoIterator<Item = Found>) -> bool {
        for found in found {
            if self.kept.len() == MAX_ERRORS {
                self.more = true;
                break;
            }
            self.kept.push(found);
        }
        !self.more
    }
}

/// `error`, on the element at `index` if any, as the errors to report: none for a regex that ran
/// past [`BACKTRACK_LIMIT`], which is reported on its own; one per key for unexpected
/// properties, up to one more than [`MAX_ERRORS`]; otherwise one.
fn found(index: Option<usize>, value: &Value, error: &ValidationError<'_>) -> Vec<Found> {
    if backtracked(error) {
        return Vec::new();
    }
    let path = pointer_path(index, error);
    if let Some(keys) = unexpected(value, error) {
        return keys
            .into_iter()
            .take(MAX_ERRORS + 1)
            .map(|key| Found {
                path: path.clone(),
                message: format!("Additional properties are not allowed ('{key}' was unexpected)"),
                keys: vec![key],
                key: true,
            })
            .collect();
    }
    let keys = match error.kind() {
        ValidationErrorKind::UnevaluatedProperties { unexpected } => unexpected.clone(),
        ValidationErrorKind::PropertyNames { error } => error
            .instance()
            .as_str()
            .map(|key| vec![key.to_owned()])
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    vec![Found {
        path,
        keys,
        key: false,
        message: message(error),
    }]
}

/// The keys an `additionalProperties` error is about, to report one error each: those it lists,
/// or every key for `"additionalProperties": false` with no `properties` or `patternProperties`
/// beside it, which jsonschema reports once as a false schema at the object's first value.
fn unexpected(value: &Value, error: &ValidationError<'_>) -> Option<Vec<String>> {
    match error.kind() {
        ValidationErrorKind::AdditionalProperties { unexpected } => Some(unexpected.clone()),
        ValidationErrorKind::FalseSchema
            if error
                .schema_path()
                .as_str()
                .ends_with("/additionalProperties") =>
        {
            // A `false` under `properties` with that name is about the value itself.
            let object = value.pointer(error.instance_path().as_str())?;
            let members = object.as_object()?;
            (object != error.instance().as_ref()).then(|| members.keys().cloned().collect())
        }
        _ => None,
    }
}

impl Schema {
    /// Read and compile the schema at `path`, in its dialect under `settings`. Any problem
    /// with it is a usage error.
    pub(crate) fn load(path: &Path, settings: &Settings) -> Result<Schema, String> {
        let name = path.to_string_lossy();
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read the schema {name}: {e}"))?;
        // jsonschema compiles a schema recursively: a deep one can take gigabytes or the stack.
        let mut config = settings.config_for(&name);
        // A schema is one value, so a JSON Lines dialect set for the documents does not apply.
        if config.dialect() == Dialect::JsonLines
            && Dialect::for_path(path) != Some(Dialect::JsonLines)
        {
            config = settings.config(Dialect::Json5);
        }
        let config = config.max_depth(MAX_SCHEMA_DEPTH);
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
        let events = &events[document.events.clone()];
        if let Some((offset, reason)) = document.unchecked {
            return Err(at(offset, &reason));
        }
        let schema = &document.value;
        let outside = |error: &ReferencingError| match error {
            ReferencingError::Unretrievable { uri, .. } => Some(format!(
                "the schema {name} has a `$ref` to {uri}, outside the file; jsonvet fetches no \
                 schemas"
            )),
            _ => None,
        };
        // The same index of `$id`s and anchors jsonschema builds, offline.
        let registry = registry(schema)
            .map_err(|error| outside(&error).unwrap_or_else(|| at(events[0].offset, &error)))?;
        // jsonschema also validates recursively through each `$ref`, so a long chain of them
        // takes the stack or hours, however shallow the schema is.
        let kept = Reach::new(schema).measure(&registry).map_err(|over| {
            let (offset, _) = place(events, &[(over.path.clone(), vec![over.key.clone()])])[0];
            at(offset, &over.message())
        })?;
        // Offline: a `$ref` to anything outside the schema file fails here.
        let options = jsonschema::options()
            .offline()
            // fancy-regex backtracks for back-references and lookaround, and its default limit
            // of a million lets one 30-byte string take 30 ms.
            .with_pattern_options(PatternOptions::fancy_regex().backtrack_limit(BACKTRACK_LIMIT));
        let error = match options.build(schema) {
            Ok(validator) => {
                let (mut keys, mut strings) = (BTreeSet::new(), BTreeSet::new());
                patterns(schema, false, &mut keys, &mut strings);
                // A regex jsonschema could not compile already failed the build above.
                let compile = |regexes: BTreeSet<&str>| {
                    regexes
                        .into_iter()
                        .filter_map(|regex| {
                            let schema = serde_json::json!({ "pattern": regex });
                            Some((regex.to_owned(), options.build(&schema).ok()?))
                        })
                        .collect()
                };
                return Ok(Schema {
                    validator,
                    items: split(schema, kept, false, &options),
                    members: split(schema, kept, true, &options),
                    key_patterns: compile(keys),
                    string_patterns: compile(strings),
                });
            }
            Err(error) => error,
        };
        // A failed `$ref` has no instance path to place it by.
        if let ValidationErrorKind::Referencing(error) = error.kind()
            && let Some(message) = outside(error)
        {
            return Err(message);
        }
        let (offset, _) = place_found(events, &found(None, schema, &error))[0];
        Err(at(offset, &error))
    }

    /// The schema errors in one file's values, as (offset, message, summary): each document's
    /// in order, then a summary if it has more than [`MAX_ERRORS`].
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
            let found = self.collect(&document.value);
            if found.kept.is_empty() {
                continue;
            }
            let events = &events[document.events];
            let spots = place_found(events, &found.kept);
            let mut placed: Vec<(usize, String, bool)> = found
                .kept
                .into_iter()
                .zip(spots)
                .map(|(found, (offset, at))| {
                    (offset, cut(format!("{} (at {at})", found.message)), false)
                })
                .collect();
            // Stable, so errors at one value keep the schema's order.
            placed.sort_by_key(|(offset, ..)| *offset);
            errors.append(&mut placed);
            if found.more {
                errors.push((
                    events[0].offset,
                    "... and more schema errors in this document".to_owned(),
                    true,
                ));
            }
        }
        errors
    }

    /// The first [`MAX_ERRORS`] errors in `value`: keys and strings no regex could be matched
    /// on, then jsonschema's own, which stop being asked for once there is one too many.
    fn collect(&self, value: &Value) -> Errors {
        let mut errors = Errors::default();
        // jsonschema counts a regex that runs past the backtrack limit as not matching, so
        // `patternProperties` and any `pattern` under `not` or `if` would pass silently.
        // Every key and string is tried here instead, and jsonschema's own errors for it,
        // which this repeats, are dropped.
        // shortcut: each regex is tried on every key or string, not only where the schema
        // applies it; follow the schema's applicators if that fails a file wrongly.
        if !(self.key_patterns.is_empty() && self.string_patterns.is_empty()) {
            self.unmatched(value, &mut Vec::new(), &mut errors);
        }
        if errors.more || self.validator.is_valid(value) {
            return errors;
        }
        // Each error is turned into what is reported as it comes, as jsonschema's own holds
        // much of the schema: an `enum`'s values, or every branch's errors for an `anyOf`.
        match (value, &self.items, &self.members) {
            (Value::Array(items), Some(Split { whole, each }), _) => {
                // jsonschema checks `contains` after `items`, and the rest before.
                let mut after = Vec::new();
                for error in whole.iter_errors(value) {
                    let found = found(None, value, &error);
                    let keyword = segments(error.schema_path().as_str()).next();
                    if keyword.is_some_and(|k| {
                        ["contains", "minContains", "maxContains"].contains(&k.as_str())
                    }) {
                        after.extend(found);
                    } else if !errors.add(found) {
                        return errors;
                    }
                }
                for (i, item) in items.iter().enumerate() {
                    for error in each.iter_errors(item) {
                        if !errors.add(found(Some(i), item, &error)) {
                            return errors;
                        }
                    }
                }
                errors.add(after);
            }
            (Value::Object(members), _, Some(Split { whole, each })) => {
                for error in whole.iter_errors(value) {
                    if !errors.add(found(None, value, &error)) {
                        return errors;
                    }
                }
                for (key, member) in members {
                    let one = Value::Object(Map::from_iter([(key.clone(), member.clone())]));
                    for error in each.iter_errors(&one) {
                        if !errors.add(found(None, &one, &error)) {
                            return errors;
                        }
                    }
                }
            }
            _ => {
                for error in self.validator.iter_errors(value) {
                    if !errors.add(found(None, value, &error)) {
                        break;
                    }
                }
            }
        }
        errors
    }

    /// The keys and strings in `value`, which is at `path`, that a regex in the schema could
    /// not be matched on within [`BACKTRACK_LIMIT`], until there is one error too many.
    fn unmatched(&self, value: &Value, path: &mut Vec<String>, errors: &mut Errors) {
        let fails = |regex: &Validator, value: &Value| {
            regex
                .validate(value)
                .is_err_and(|error| backtracked(&error))
        };
        let message = |what: &str, regex: &str| {
            format!(
                "this {what} could not be checked against the pattern \"{regex}\": matching it \
                 takes more than {BACKTRACK_LIMIT} backtracking steps"
            )
        };
        if value.is_string() {
            for (regex, validator) in &self.string_patterns {
                if fails(validator, value)
                    && !errors.add([Found {
                        path: path.clone(),
                        keys: Vec::new(),
                        key: false,
                        message: message("string", regex),
                    }])
                {
                    return;
                }
            }
        }
        for (key, member) in children(value) {
            if value.is_object() {
                let name = Value::String(key.clone().into_owned());
                for (regex, validator) in &self.key_patterns {
                    if fails(validator, &name)
                        && !errors.add([Found {
                            path: path.clone(),
                            keys: vec![key.clone().into_owned()],
                            key: true,
                            message: message("key", regex),
                        }])
                    {
                        return;
                    }
                }
            }
            path.push(key.into_owned());
            self.unmatched(member, path, errors);
            path.pop();
            if errors.more {
                return;
            }
        }
    }
}

/// `schema` split for its arrays, or with `members` for its objects, if nothing but `items`, or
/// `properties`, `patternProperties` and `additionalProperties`, checks their elements or
/// members, and every `$ref` in it points into one of [`NOT_CHECKS`] at the root (`kept`), as
/// those are kept in both parts. `None` also when either part fails to build: the whole schema
/// is then checked in one pass.
fn split(schema: &Value, kept: bool, members: bool, options: &ValidationOptions) -> Option<Split> {
    let all = schema.as_object()?;
    let (moved, also): (&[&str], &[&str]) = if members {
        (&ON_MEMBERS, &["unevaluatedProperties"])
    } else {
        (&["items"], &["prefixItems", "unevaluatedItems"])
    };
    let checks = |key: &&str| all.contains_key(*key);
    if !kept || ON_CHILDREN.iter().chain(also).any(checks) || !moved.iter().any(checks) {
        return None;
    }
    let mut whole = all.clone();
    let mut part = Map::new();
    for key in moved {
        if let Some(value) = whole.remove(*key) {
            part.insert((*key).to_owned(), value);
        }
    }
    let part = match (members, part.remove("items")) {
        // `items` with an array of schemas checks elements by position.
        (false, Some(items)) if items.is_object() || items.is_boolean() => items,
        (false, _) => return None,
        (true, _) => Value::Object(part),
    };
    let mut each: Map<String, Value> = all
        .iter()
        .filter(|(key, _)| NOT_CHECKS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    each.insert(EACH.to_owned(), part);
    // Under `allOf`, so a draft 7 `$id` beside it still sets the base for the part's `$ref`s.
    each.insert(
        "allOf".to_owned(),
        serde_json::json!([{ "$ref": format!("#/{EACH}") }]),
    );
    Some(Split {
        whole: options.build(&Value::Object(whole)).ok()?,
        each: options.build(&Value::Object(each)).ok()?,
    })
}

/// The segments of `error`'s instance path, under element `index` if it is on one.
fn pointer_path(index: Option<usize>, error: &ValidationError<'_>) -> Vec<String> {
    index
        .map(|i| i.to_string())
        .into_iter()
        .chain(segments(error.instance_path().as_str()))
        .collect()
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
fn place_found(events: &[Event<'_>], found: &[Found]) -> Vec<(usize, String)> {
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
fn place(events: &[Event<'_>], targets: &[(Vec<String>, Vec<String>)]) -> Vec<(usize, String)> {
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

/// The registry jsonschema builds for `schema`, with every resource and anchor in it by its
/// absolute URI, and no way to fetch another.
fn registry(schema: &Value) -> Result<Registry<'_>, ReferencingError> {
    let resource = ResourceRef::new(schema, Draft::default().detect(schema));
    Registry::new()
        .draft(resource.draft())
        .add(resource.id().unwrap_or(DEFAULT_BASE), resource)?
        .prepare()
}

/// How deep and how big a schema gets when its `$ref`s are followed, resolved as jsonschema
/// resolves them: against the base each `$id` sets. A `$ref` back to a schema on the way to it
/// is recursion, which jsonschema handles: it adds nothing.
struct Reach<'r> {
    root: &'r Value,
    /// Each object and array under the root: its parent, and its key or index there.
    parents: HashMap<*const Value, (&'r Value, Cow<'r, str>)>,
    /// The levels at and under each schema (or, with `false`, each value that is not one, such
    /// as an `enum`) already measured, and its size, by address. Only for a schema measured
    /// without skipping a `$ref` back to a schema above it, as that one is shorter where the
    /// skipped schema is not open.
    measured: HashMap<(*const Value, bool), Measure>,
    /// The schemas on the way to the current one, with their levels.
    open: HashMap<*const Value, usize>,
    /// Each `$ref` on the way: its object and key.
    refs: Vec<(&'r Value, &'static str)>,
    /// Whether every `$ref` so far points into one of [`NOT_CHECKS`] at the root.
    kept: bool,
}

/// How a value is reached: as a schema, with the resolver for its base and its draft, or as a
/// value under a schema, whose subschemas are `subs`, sorted by address.
#[derive(Clone, Copy)]
enum At<'a, 'r> {
    Schema(&'a Resolver<'r>, Draft),
    Under(&'a Resolver<'r>, Draft, &'a [*const Value]),
}

impl<'r> Reach<'r> {
    fn new(root: &'r Value) -> Self {
        let mut reach = Reach {
            root,
            parents: HashMap::new(),
            measured: HashMap::new(),
            open: HashMap::new(),
            refs: Vec::new(),
            kept: true,
        };
        reach.collect(root);
        reach
    }

    /// Note the parent of each object and array in `value`.
    fn collect(&mut self, value: &'r Value) {
        for (key, child) in children(value) {
            if child.is_object() || child.is_array() {
                self.parents.insert(child, (value, key));
                self.collect(child);
            }
        }
    }

    /// The path of `value` in the file, or `None` if it is not in the file, as in a
    /// meta-schema.
    fn path(&self, mut value: &'r Value) -> Option<Vec<String>> {
        let mut path = Vec::new();
        while !std::ptr::eq(value, self.root) {
            let (parent, key) = self.parents.get(&std::ptr::from_ref(value))?;
            path.push(key.clone().into_owned());
            value = parent;
        }
        path.reverse();
        Some(path)
    }

    /// Whether every `$ref` points into one of [`NOT_CHECKS`] at the root, or where a `$ref`
    /// leads past [`MAX_SCHEMA_DEPTH`] levels or [`MAX_SCHEMA_SIZE`], or cannot be resolved.
    fn measure(mut self, registry: &'r Registry<'r>) -> Result<bool, Over> {
        let draft = Draft::default().detect(self.root);
        let resource = ResourceRef::new(self.root, draft);
        let base = referencing::uri::from_str(resource.id().unwrap_or(DEFAULT_BASE))
            .map_err(|error| self.over_at(None, Problem::Unresolved(error.to_string())))?;
        let resolver = registry
            .resolver(base)
            .in_subresource(resource)
            .map_err(|error| self.over_at(None, Problem::Unresolved(error.to_string())))?;
        self.visit(self.root, 1, At::Schema(&resolver, draft))?;
        Ok(self.kept)
    }

    /// An error about `spot`, a key in an object, or else about the innermost `$ref` on the way
    /// that is in the file.
    fn over_at(&self, spot: Option<(&'r Value, &str)>, problem: Problem) -> Over {
        let refs = self.refs.iter().rev().map(|&(at, key)| (at, key));
        let (path, key) = spot
            .into_iter()
            .chain(refs)
            .find_map(|(at, key)| Some((self.path(at)?, key.to_owned())))
            // Containers alone stay within the parser's depth limit, so a `$ref` is on the way.
            .unwrap_or_else(|| (Vec::new(), "$ref".to_owned()));
        Over { path, key, problem }
    }

    /// Follow each `$ref` in the schema `value`, which is `level` levels deep, into `below`,
    /// and say how big what they point at is.
    fn follow(
        &mut self,
        value: &'r Value,
        level: usize,
        resolver: &Resolver<'r>,
        draft: Draft,
        below: &mut Visit,
    ) -> Result<usize, Over> {
        let mut size: usize = 0;
        for key in REF_KEYS {
            let Some(reference) = value.get(key).and_then(Value::as_str) else {
                continue;
            };
            if key != "$ref" && !draft.is_known_keyword(key) {
                continue;
            }
            let lookup = match key {
                "$recursiveRef" => resolver.lookup_recursive_ref(),
                _ => resolver.lookup(reference),
            };
            let (target, target_resolver, target_draft) = match lookup {
                Ok(target) => target.into_inner(),
                Err(error) => {
                    let problem = Problem::Unresolved(error.to_string());
                    return Err(self.over_at(Some((value, key)), problem));
                }
            };
            let top = self.path(target).and_then(|path| path.into_iter().next());
            self.kept &= top.is_some_and(|top| NOT_CHECKS.contains(&top.as_str()));
            if let Some(&open) = self.open.get(&std::ptr::from_ref(target)) {
                below.cut = below.cut.min(open);
                continue;
            }
            self.refs.push((value, key));
            let child = self.visit(
                target,
                level + 1,
                At::Schema(&target_resolver, target_draft),
            )?;
            size = size.saturating_add(child.measure.size);
            if below.add(child) {
                return Err(self.over_at(None, Problem::Big));
            }
            self.refs.pop();
        }
        Ok(size)
    }

    /// `value`, which is `level` levels deep, measured, or where it is too deep or too big.
    fn visit(&mut self, value: &'r Value, level: usize, at: At<'_, 'r>) -> Result<Visit, Over> {
        if !(value.is_object() || value.is_array()) {
            return Ok(Visit {
                measure: Measure::default(),
                cut: usize::MAX,
            });
        }
        let address: *const Value = value;
        let memo = (address, matches!(at, At::Schema(..)));
        let known = self.measured.get(&memo).copied();
        if level + known.map_or(1, |m| m.height) - 1 > MAX_SCHEMA_DEPTH {
            return Err(self.over_at(None, Problem::Deep));
        }
        if let Some(measure) = known {
            return Ok(Visit {
                measure,
                cut: usize::MAX,
            });
        }
        self.open.insert(address, level);
        let mut below = Visit {
            measure: Measure { height: 0, size: 1 },
            cut: usize::MAX,
        };
        let (resolver, draft, subs) = match at {
            At::Schema(resolver, draft) => {
                let mut subs: Vec<*const Value> = draft
                    .subresources_of(value)
                    .map(std::ptr::from_ref)
                    .collect();
                subs.sort_unstable();
                (resolver, draft, Cow::Owned(subs))
            }
            At::Under(resolver, draft, subs) => (resolver, draft, Cow::Borrowed(subs)),
        };
        let schema = matches!(at, At::Schema(..));
        // The size of what an `unevaluated*` keyword here checks again.
        let mut again: usize = 0;
        for (key, member) in children(value) {
            let mut child = if subs.binary_search(&std::ptr::from_ref(member)).is_ok() {
                // A subschema with an `$id` sets a new base for what is under it.
                let draft = draft.detect(member);
                let resolver = resolver
                    .in_subresource(ResourceRef::new(member, draft))
                    .map_err(|e| {
                        self.over_at(Some((member, "$id")), Problem::Unresolved(e.to_string()))
                    })?;
                self.visit(member, level + 1, At::Schema(&resolver, draft))?
            } else {
                self.visit(member, level + 1, At::Under(resolver, draft, &subs))?
            };
            // jsonschema may compare a value with each one in an `enum`, and copies them all
            // into each error.
            if schema && matches!(key.as_ref(), "enum" | "const") {
                child.measure.size = child.measure.size.saturating_add(scalars(member));
            }
            if schema && IN_PLACE.contains(&key.as_ref()) {
                again = again.saturating_add(child.measure.size);
            }
            if below.add(child) {
                let spot = self.refs.is_empty().then_some((value, key.as_ref()));
                return Err(self.over_at(spot, Problem::Big));
            }
        }
        if schema {
            again = again.saturating_add(self.follow(value, level, resolver, draft, &mut below)?);
            // Nested in each other's `anyOf`s, each level of these doubles the work below it.
            if let Some(key) = UNEVALUATED
                .into_iter()
                .find(|key| value.get(key).is_some() && draft.is_known_keyword(key))
            {
                below.measure.size = below.measure.size.saturating_add(again);
                if below.measure.size > MAX_SCHEMA_SIZE {
                    return Err(self.over_at(Some((value, key)), Problem::Twice));
                }
            }
        }
        self.open.remove(&address);
        below.measure.height += 1;
        // A skipped `$ref` back to `value` itself is skipped wherever `value` is reached.
        if below.cut >= level {
            below.cut = usize::MAX;
            self.measured.insert(memo, below.measure);
        }
        Ok(below)
    }
}

/// How many values in `value` are neither objects nor arrays.
fn scalars(value: &Value) -> usize {
    match value {
        Value::Object(_) | Value::Array(_) => {
            children(value).map(|(_, child)| scalars(child)).sum()
        }
        _ => 1,
    }
}

/// A schema's levels, counting each `$ref` followed as one, and its size: its objects and arrays
/// with each `$ref` replaced by what it points at, and the values in its `enum`s and `const`s.
#[derive(Clone, Copy, Default)]
struct Measure {
    height: usize,
    size: usize,
}

/// A schema measured, and the level of the highest schema above it that a skipped `$ref` points
/// back at, or `usize::MAX`.
#[derive(Clone, Copy)]
struct Visit {
    measure: Measure,
    cut: usize,
}

impl Visit {
    /// Take in a schema under this one, and say whether this one is now past
    /// [`MAX_SCHEMA_SIZE`].
    fn add(&mut self, child: Visit) -> bool {
        self.measure.height = self.measure.height.max(child.measure.height);
        self.measure.size = self.measure.size.saturating_add(child.measure.size);
        self.cut = self.cut.min(child.cut);
        self.measure.size > MAX_SCHEMA_SIZE
    }
}

/// Why [`Reach`] turns a schema away.
enum Problem {
    Deep,
    Big,
    /// Too big once the schemas beside an `unevaluated*` keyword count twice.
    Twice,
    Unresolved(String),
}

/// Where a schema is turned away, and why: the path of an object and a key in it.
struct Over {
    path: Vec<String>,
    key: String,
    problem: Problem,
}

impl Over {
    fn message(&self) -> String {
        let key = &self.key;
        match &self.problem {
            Problem::Deep => format!(
                "this `{key}` leads more than {MAX_SCHEMA_DEPTH} levels deep, counting each \
                 `$ref` followed as a level"
            ),
            Problem::Big => format!(
                "this `{key}` takes the schema past {MAX_SCHEMA_SIZE} objects and arrays, \
                 counting what each `$ref` points at every time it is followed"
            ),
            Problem::Twice => format!(
                "this `{key}` takes the schema past {MAX_SCHEMA_SIZE} objects and arrays, \
                 counting twice the schemas beside it that jsonschema checks again for it"
            ),
            Problem::Unresolved(error) => format!("this `{key}` cannot be resolved: {error}"),
        }
    }
}

/// An open container while the events are folded into values, with an object's current key
/// (as text, its offset, and as written) and each earlier key holding U+FFFD, by text, as written.
enum Open<'e> {
    Array(Vec<Value>),
    Object(
        Map<String, Value>,
        (String, usize, &'e [u8]),
        HashMap<String, &'e [u8]>,
    ),
}

/// Fold events into documents: one, or one per line in JSON Lines.
fn documents(events: &[Event<'_>]) -> Vec<Document> {
    let mut documents = Vec::new();
    let mut first_event = 0;
    let mut stack: Vec<Open<'_>> = Vec::new();
    let mut unchecked = None;
    for (i, event) in events.iter().enumerate() {
        let value = match &event.kind {
            EventKind::Key(key) => {
                if let Some(Open::Object(_, current, _)) = stack.last_mut() {
                    *current = (key_text(key).into_owned(), event.offset, &**key);
                }
                continue;
            }
            EventKind::ObjectStart => {
                stack.push(Open::Object(
                    Map::new(),
                    (String::new(), 0, &[]),
                    HashMap::new(),
                ));
                continue;
            }
            EventKind::ArrayStart => {
                stack.push(Open::Array(Vec::new()));
                continue;
            }
            EventKind::End => match stack.pop() {
                Some(Open::Array(items)) => Value::Array(items),
                Some(Open::Object(members, ..)) => Value::Object(members),
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
            Some(Open::Object(members, (key, offset, raw), seen)) => {
                let key = std::mem::take(key);
                // Real duplicates are the duplicate-key rule's; keys that only become the same
                // once lone surrogates are U+FFFD are not.
                if key.contains('\u{FFFD}') {
                    match seen.get(&key) {
                        Some(earlier) if earlier != raw => {
                            unchecked.get_or_insert((*offset, SAME_KEYS));
                        }
                        Some(_) => {}
                        None => {
                            seen.insert(key.clone(), raw);
                        }
                    }
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
    use super::{
        DEFAULT_BASE, MAX_MESSAGE, MAX_SCHEMA_SIZE, Measure, Over, Problem, Reach, Schema, Visit,
        cut, documents, fits, found, message, number, place, place_found, registry, segments,
        split,
    };
    use jsonschema::Draft;
    use jsonvet::{Config, Dialect};
    use proptest::prelude::*;
    use serde_json::{Value, json};

    #[test]
    fn a_schema_of_exactly_the_size_limit_fits() {
        let visit = |size| Visit {
            measure: Measure { height: 1, size },
            cut: usize::MAX,
        };
        let mut schema = visit(MAX_SCHEMA_SIZE - 1);
        assert!(!schema.add(visit(1)));
        assert!(schema.add(visit(1)));
    }

    /// Where `schema` is turned away, if it is.
    fn over(schema: &Value) -> Option<Over> {
        let registry = registry(schema).expect("a registry");
        Reach::new(schema).measure(&registry).err()
    }

    /// The path in `schema` of what `reference`, at the root, points at.
    fn target(schema: &Value, reference: &str) -> Option<String> {
        let registry = registry(schema).ok()?;
        let resource = referencing::ResourceRef::new(schema, Draft::default().detect(schema));
        let base = referencing::uri::from_str(resource.id().unwrap_or(DEFAULT_BASE)).ok()?;
        let resolver = registry.resolver(base).in_subresource(resource).ok()?;
        let lookup = resolver.lookup(reference).ok()?;
        let path = Reach::new(schema).path(lookup.contents())?;
        Some(path.join("/"))
    }

    #[test]
    fn refs_resolve_by_pointer_anchor_and_id_against_their_base() {
        let schema = json!({
            "$id": "https://example.com/root.json",
            "$defs": {
                "a/b": {"anyOf": [{"$anchor": "x"}, {"type": "string"}]},
                "%": {"type": "null"},
                "sub": {"$id": "sub.json", "$defs": {"c": {}}},
                // Two resources with the same relative `$id`, each against its own base.
                "p": {"$id": "p/", "$defs": {"l": {"$id": "l", "type": "string"}}},
                "q": {"$id": "q/", "$defs": {"l": {"$id": "l", "type": "number"}}},
            },
        });
        let path = |reference| target(&schema, reference);
        assert_eq!(path("#").as_deref(), Some(""));
        assert_eq!(
            path("#/$defs/a~1b/anyOf/1").as_deref(),
            Some("$defs/a/b/anyOf/1")
        );
        assert_eq!(path("#/$defs/%25").as_deref(), Some("$defs/%"));
        assert_eq!(path("#x").as_deref(), Some("$defs/a/b/anyOf/0"));
        assert_eq!(path("root.json#/$defs").as_deref(), Some("$defs"));
        assert_eq!(
            path("https://example.com/sub.json#/$defs/c").as_deref(),
            Some("$defs/sub/$defs/c")
        );
        assert_eq!(path("q/l").as_deref(), Some("$defs/q/$defs/l"));
        assert_eq!(path("p/l").as_deref(), Some("$defs/p/$defs/l"));
        assert_eq!(path("l"), None);
        assert_eq!(path("#/$defs/none"), None);
        assert_eq!(path("other.json"), None);
        // Before 2019-09, an `$id` of `#name` is an anchor.
        let old = json!({"$schema": "http://json-schema.org/draft-07/schema#", "definitions": {"%": {"$id": "#old"}}});
        assert_eq!(target(&old, "#old").as_deref(), Some("definitions/%"));
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
        let errors: Vec<_> = validator
            .iter_errors(&documents[0].value)
            .flat_map(|e| found(None, &documents[0].value, &e))
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
            over(schema)
                .filter(|over| matches!(over.problem, Problem::Deep))
                .map(|over| (over.path.join("/"), over.key))
        };
        // `$defs/long` is first reached near the root, then again under 100 levels of `allOf`.
        let long = nested(60, json!({"type": "integer"}));
        let schema = json!({
            "$defs": {"long": long},
            "allOf": [nested(100, json!({"$ref": "#/$defs/long"}))],
        });
        let path = format!("allOf/0{}", "/allOf/0".repeat(50));
        assert_eq!(too_deep(&schema), Some((path, "$ref".to_owned())));
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

    /// `x{i}`'s `additionalProperties` goes through `y{i}` back to `x{i}`, and its `allOf`, 60
    /// levels deep, ends in a `$ref` to `y{i-1}`: `k` such pairs.
    fn cycles(k: usize) -> Value {
        let mut defs = serde_json::Map::new();
        for i in 0..k {
            let end = match i {
                0 => json!({"type": "integer"}),
                _ => json!({"$ref": format!("#/$defs/y{}", i - 1)}),
            };
            defs.insert(
                format!("x{i}"),
                json!({"additionalProperties": {"$ref": format!("#/$defs/y{i}")}, "allOf": [nested(120, end)]}),
            );
            defs.insert(format!("y{i}"), json!({"$ref": format!("#/$defs/x{i}")}));
        }
        json!({"$defs": defs, "$ref": format!("#/$defs/y{}", k - 1)})
    }

    #[test]
    fn a_height_measured_while_a_ref_back_up_was_skipped_is_not_reused() {
        // `y0` is first measured inside `x1`'s `allOf`, while `y1` and `x1` are open and the
        // `$ref` back to them is skipped. From the root, `y1` to `y0` is 2 + 122 + 122 levels.
        let deep = over(&cycles(2)).is_some_and(|over| matches!(over.problem, Problem::Deep));
        assert!(deep);
        assert!(over(&cycles(1)).is_none());
    }

    #[test]
    fn a_schema_too_big_once_its_refs_are_followed_is_caught() {
        // Each `d{i}` points twice at `d{i+1}`, so following the `$ref`s doubles it 24 times:
        // through an array, through an object's members, and through two reference keywords.
        type Shape = fn(String) -> Value;
        let fan_out = |levels: usize, shape: Shape| {
            let mut defs = serde_json::Map::new();
            for i in 0..levels {
                defs.insert(format!("d{i}"), shape(format!("#/$defs/d{}", i + 1)));
            }
            defs.insert(format!("d{levels}"), json!({"type": "string"}));
            json!({"$defs": defs, "$ref": "#/$defs/d0"})
        };
        let shapes: [(Shape, &str); 4] = [
            (
                |next| json!({"anyOf": [{"$ref": next}, {"$ref": next}]}),
                "$ref",
            ),
            (
                |next| json!({"allOf": [{"$ref": next}, {"$ref": next}]}),
                "$ref",
            ),
            (
                |next| json!({"properties": {"a": {"$ref": next}, "b": {"$ref": next}}}),
                "$ref",
            ),
            (
                |next| json!({"$ref": next, "$dynamicRef": next}),
                "$dynamicRef",
            ),
        ];
        for (shape, key) in shapes {
            let big = over(&fan_out(24, shape))
                .is_some_and(|over| matches!(over.problem, Problem::Big) && over.key == key);
            assert!(big);
            assert!(over(&fan_out(16, shape)).is_none());
        }
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

    /// Schemas that only `items` checks the elements of, and so are split, then ones that are not.
    fn split_schemas() -> [(Value, bool); 10] {
        [
            (
                json!({"items": {"type": "string"}, "minItems": 2, "uniqueItems": true}),
                true,
            ),
            (
                json!({
                    "items": {"$ref": "#/$defs/p"},
                    "contains": {"type": "null"},
                    "minContains": 2,
                    "$defs": {"p": {"properties": {"a": {"type": "integer"}}, "additionalProperties": false}},
                }),
                true,
            ),
            (
                json!({
                    "$schema": "http://json-schema.org/draft-07/schema#",
                    "$id": "https://example.com/r.json",
                    "items": {"$ref": "#/definitions/n"},
                    "definitions": {"n": {"type": ["array", "integer"], "items": {"$ref": "#/definitions/n"}}},
                }),
                true,
            ),
            (
                json!({"items": {"additionalProperties": false}, "maxItems": 3}),
                true,
            ),
            (json!({"items": false}), true),
            (json!({"items": {"$ref": "#"}, "type": "array"}), false),
            (
                json!({"items": {"anyOf": [{"$ref": "#"}]}, "type": "array"}),
                false,
            ),
            (
                json!({"items": {"type": "string"}, "allOf": [{"items": {"minLength": 1}}]}),
                false,
            ),
            (
                json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "items": [{"type": "string"}]}),
                false,
            ),
            (
                json!({"items": {"type": "string"}, "$defs": {"a": {"$ref": "#/items"}}, "contains": {"$ref": "#/$defs/a"}}),
                false,
            ),
        ]
    }

    /// What [`Schema::errors`] returns.
    type Errors = Vec<(usize, String, bool)>;

    /// `text`'s schema errors under `schema`, checked in one pass and, if it can be, split.
    fn one_pass_and_split(schema: &Value, text: &str) -> (Errors, Option<Errors>) {
        let options = jsonschema::options();
        let checked = |items, members| {
            let schema = Schema {
                validator: options.build(schema).expect("a schema"),
                items,
                members,
                key_patterns: Vec::new(),
                string_patterns: Vec::new(),
            };
            let (_, events) = jsonvet::parse(text, &Config::new(Dialect::Json));
            schema.errors(&events.unwrap_or_default())
        };
        let registry = registry(schema).expect("a registry");
        let kept = Reach::new(schema).measure(&registry).is_ok_and(|kept| kept);
        let items = split(schema, kept, false, &options);
        let members = split(schema, kept, true, &options);
        let split = (items.is_some() || members.is_some()).then(|| checked(items, members));
        (checked(None, None), split)
    }

    #[test]
    fn a_split_schema_reports_what_one_pass_does_past_the_cap() {
        // 150 elements, each failing `items`, beside errors about the whole array.
        let text = format!("[{}]", vec!["{\"a\": 1, \"b\": 1}"; 150].join(","));
        for (schema, splits) in split_schemas() {
            let (one_pass, split) = one_pass_and_split(&schema, &text);
            assert_eq!(split.is_some(), splits, "{schema}");
            if let Some(split) = split {
                assert_eq!(split, one_pass, "{schema}");
            }
        }
    }

    /// Schemas that only `properties`, `patternProperties` and `additionalProperties` check the
    /// members of, and so are split, then ones that are not.
    fn member_schemas() -> [(Value, bool); 7] {
        [
            (
                json!({"properties": {"a": {"type": "string"}, "b": {"const": 1}}, "required": ["c"], "maxProperties": 2}),
                true,
            ),
            (
                json!({"patternProperties": {"^a": {"type": "integer"}}, "additionalProperties": false, "propertyNames": {"maxLength": 2}}),
                true,
            ),
            (
                json!({"additionalProperties": false, "minProperties": 1}),
                true,
            ),
            (
                json!({"additionalProperties": {"$ref": "#/$defs/n"}, "$defs": {"n": {"type": ["object", "null"], "additionalProperties": {"$ref": "#/$defs/n"}}}}),
                true,
            ),
            (
                json!({"properties": {"a": true}, "unevaluatedProperties": false}),
                false,
            ),
            (
                json!({"properties": {"a": {"type": "string"}}, "anyOf": [{"required": ["a"]}]}),
                false,
            ),
            (
                json!({"additionalProperties": {"$ref": "#/properties/a"}, "properties": {"a": {"type": "null"}}}),
                false,
            ),
        ]
    }

    #[test]
    fn a_split_object_schema_reports_as_many_errors_past_the_cap() {
        // 150 members, each failing, beside errors about the whole object. Past the cap, the
        // first 100 are by member, not by keyword.
        let members: Vec<String> = (0..150)
            .map(|i| format!("\"a{i}\": {{\"b\": 1}}"))
            .collect();
        let text = format!("{{{}}}", members.join(","));
        for (schema, splits) in member_schemas() {
            let (one_pass, split) = one_pass_and_split(&schema, &text);
            assert_eq!(split.is_some(), splits, "{schema}");
            if let Some(split) = split {
                assert_eq!(split.len(), one_pass.len(), "{schema}");
                assert_eq!(split.last(), one_pass.last(), "{schema}");
            }
        }
    }

    #[test]
    fn enum_and_const_values_count_toward_the_size() {
        // 18 levels, each with two `$ref`s to the next, reach the last 262,144 times: about 1.6
        // million objects and arrays, and 6.8 million values with the 20 numbers at the end.
        let fan_out = |last: Value| {
            let mut defs = serde_json::Map::new();
            for i in 0..18 {
                let next = format!("#/$defs/d{}", i + 1);
                defs.insert(
                    format!("d{i}"),
                    json!({"anyOf": [{"$ref": next}, {"$ref": next}]}),
                );
            }
            defs.insert("d18".to_owned(), last);
            json!({"$defs": defs, "$ref": "#/$defs/d0"})
        };
        let numbers: Vec<usize> = (0..20).collect();
        for last in [json!({"enum": numbers}), json!({"const": [numbers]})] {
            let big = over(&fan_out(last)).is_some_and(|over| matches!(over.problem, Problem::Big));
            assert!(big);
        }
        assert!(over(&fan_out(json!({"type": "string"}))).is_none());
    }

    proptest! {
        #[test]
        fn a_split_object_schema_reports_what_one_pass_does(members in prop::collection::vec(("[ab]{1,2}", any_json()), 0..6)) {
            let object: serde_json::Map<String, Value> = members.into_iter().collect();
            let text = serde_json::to_string(&object).unwrap_or_default();
            for (schema, _) in member_schemas() {
                let (one_pass, split) = one_pass_and_split(&schema, &text);
                if let Some(split) = split {
                    prop_assert_eq!(split, one_pass, "{}", schema);
                }
            }
        }

        #[test]
        fn a_split_schema_reports_what_one_pass_does(items in prop::collection::vec(any_json(), 0..6)) {
            let text = serde_json::to_string(&items).unwrap_or_default();
            for (schema, _) in split_schemas() {
                let (one_pass, split) = one_pass_and_split(&schema, &text);
                if let Some(split) = split {
                    prop_assert_eq!(split, one_pass, "{}", schema);
                }
            }
        }

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
