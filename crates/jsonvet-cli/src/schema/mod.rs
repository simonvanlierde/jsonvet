//! `--schema`: each file's values checked against one JSON Schema, every error placed at the
//! value it is about. The schema's size limits are in `limits`, placing errors at their values
//! in `place`, and folding events into values in `fold`.

mod fold;
mod limits;
mod place;

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::path::Path;

use jsonschema::error::ValidationErrorKind;
use jsonschema::{PatternOptions, ReferencingError, ValidationError, ValidationOptions, Validator};
use jsonvet::{Dialect, Event, Severity};
use serde_json::{Map, Value};

use crate::settings::Settings;
use fold::documents;
use limits::{MAX_SCHEMA_DEPTH, Reach, registry};
use place::{place, place_found, pointer_path, segments};

/// How many schema errors one document reports, in the order jsonschema finds them; one more
/// line says there are more.
const MAX_ERRORS: usize = 100;

/// Past this many bytes, a message says "value" instead of quoting the value, and past it
/// still, it is cut short.
const MAX_MESSAGE: usize = 200;

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
    compiled: Compiled,
    /// For a schema whose elements only `items` checks: the schema without `items`, checked
    /// once on the whole array, and `items` alone, checked on each element. jsonschema collects
    /// every error before handing out the first, so a 10 MB array failing everywhere took 2.0 GB
    /// in one pass; one element at a time, it holds at most one element's errors.
    items: Option<Split>,
    /// The same for an object whose members only `properties`, `patternProperties` and
    /// `additionalProperties` check, each member checked as an object of its own.
    members: Option<Split>,
    /// `{"pattern": p}` for each regex in the schema tried on keys: `patternProperties` and a
    /// `pattern` under `propertyNames`, to find the keys a regex error on an object is about.
    key_patterns: Vec<(String, Validator)>,
    /// `{"pattern": p}` for each `pattern` in the schema, to name the one a regex error on a
    /// string is about when its schema path does not.
    string_patterns: Vec<(String, Validator)>,
    /// Whether every regex in the schema compiles for the `regex` crate. fancy-regex hands such
    /// a regex to it, which runs in linear time, so no match stops at [`BACKTRACK_LIMIT`].
    linear: bool,
}

/// A schema split in two: the part checked on the whole value, and the part checked on each
/// element or member.
struct Split {
    whole: Compiled,
    each: Compiled,
}

/// A validator and the schema it was built from, which its errors' schema paths point into.
struct Compiled {
    validator: Validator,
    schema: Value,
}

impl Compiled {
    /// `value`'s errors, after a quicker check that it has any.
    fn errors<'i>(&'i self, value: &'i Value) -> impl Iterator<Item = ValidationError<'i>> {
        let invalid = !self.validator.is_valid(value);
        invalid
            .then(|| self.validator.iter_errors(value))
            .into_iter()
            .flatten()
    }
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

/// Whether `error` says a regex match ran past [`BACKTRACK_LIMIT`] where its outcome decides
/// the result.
fn backtracked(error: &ValidationError<'_>) -> bool {
    matches!(
        error.kind(),
        ValidationErrorKind::BacktrackLimitExceeded { .. }
    )
}

/// The regex at `schema_path` in `schema`: a `patternProperties` key, or a `pattern`. For a path
/// relative to a subschema's own `$id`, this can be the wrong one, or none.
fn pattern_at(schema: &Value, schema_path: &str) -> Option<String> {
    let segments: Vec<String> = segments(schema_path).collect();
    match segments.as_slice() {
        [.., parent, regex] if parent == "patternProperties" => Some(regex.clone()),
        _ => schema.pointer(schema_path)?.as_str().map(str::to_owned),
    }
}

/// The message for a `what` that could not be matched on within [`BACKTRACK_LIMIT`], naming
/// `regex` if known.
fn costly(what: &str, regex: Option<&str>) -> String {
    let pattern = regex.map_or_else(
        || "a pattern".to_owned(),
        |r| format!("the pattern \"{r}\""),
    );
    format!(
        "this {what} could not be checked against {pattern}: matching it takes more than \
         {BACKTRACK_LIMIT} backtracking steps"
    )
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

/// `error`, on the element at `index` if any, as the errors to report: one per key for
/// unexpected properties, up to one more than [`MAX_ERRORS`]; otherwise one.
fn found(index: Option<usize>, value: &Value, error: &ValidationError<'_>) -> Vec<Found> {
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
    /// with it is a usage error. A `quiet` one names only the place, never text from the
    /// schema, as for a symlinked config file: a pull request picks a `$schema` or config
    /// schema, which could be a file that holds a secret (ADR 0012).
    pub(crate) fn load(path: &Path, settings: &Settings, quiet: bool) -> Result<Schema, String> {
        let name = path.to_string_lossy();
        let text = crate::read_small(path, crate::MAX_SCHEMA_BYTES)
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
            if quiet {
                return format!("the schema {name}:{}:{} cannot be used", at.line, at.column);
            }
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
            ReferencingError::Unretrievable { .. } if quiet => Some(format!(
                "the schema {name} has a `$ref` outside the file; jsonvet fetches no schemas"
            )),
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
                let regex = jsonschema::options()
                    .offline()
                    .with_pattern_options(PatternOptions::regex());
                let linear = keys.iter().chain(&strings).all(|pattern| {
                    regex
                        .build(&serde_json::json!({ "pattern": pattern }))
                        .is_ok()
                });
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
                    compiled: Compiled {
                        validator,
                        schema: schema.clone(),
                    },
                    items: split(schema, kept, false, &options),
                    members: split(schema, kept, true, &options),
                    key_patterns: compile(keys),
                    string_patterns: compile(strings),
                    linear,
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

    /// The first [`MAX_ERRORS`] errors in `value`, which stop being asked for once there is one
    /// too many.
    fn collect(&self, value: &Value) -> Errors {
        let mut errors = Errors::default();
        // One pass over the whole document finds a valid one quickest. A regex that can stop at
        // the backtrack limit makes that pass slow instead, as explained below.
        if self.linear && self.compiled.validator.is_valid(value) {
            return errors;
        }
        // Each error is turned into what is reported as it comes, as jsonschema's own holds
        // much of the schema: an `enum`'s values, or every branch's errors for an `anyOf`.
        // A split schema is also checked for validity a part at a time: jsonschema checks a
        // value again for each outcome of a regex match it cannot finish, so one pass over a
        // whole document repeats every costly match in it, past the error cap.
        match (value, &self.items, &self.members) {
            (Value::Array(items), Some(Split { whole, each }), _) => {
                // jsonschema checks `contains` after `items`, and the rest before.
                let mut after = Vec::new();
                for error in whole.errors(value) {
                    let found = self.found(whole, None, value, &error);
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
                    for error in each.errors(item) {
                        if !errors.add(self.found(each, Some(i), item, &error)) {
                            return errors;
                        }
                    }
                }
                errors.add(after);
            }
            (Value::Object(members), _, Some(Split { whole, each })) => {
                for error in whole.errors(value) {
                    if !errors.add(self.found(whole, None, value, &error)) {
                        return errors;
                    }
                }
                for (key, member) in members {
                    let one = Value::Object(Map::from_iter([(key.clone(), member.clone())]));
                    for error in each.errors(&one) {
                        if !errors.add(self.found(each, None, &one, &error)) {
                            return errors;
                        }
                    }
                }
            }
            _ => {
                for error in self.compiled.errors(value) {
                    if !errors.add(self.found(&self.compiled, None, value, &error)) {
                        break;
                    }
                }
            }
        }
        errors
    }

    /// [`found`], except for a regex match past [`BACKTRACK_LIMIT`] that decides the result,
    /// which jsonschema reports once, at the object for a key. That one names its pattern, and
    /// on an object it is one error per key that pattern could not be matched on.
    fn found(
        &self,
        compiled: &Compiled,
        index: Option<usize>,
        value: &Value,
        error: &ValidationError<'_>,
    ) -> Vec<Found> {
        if !backtracked(error) {
            return found(index, value, error);
        }
        let path = pointer_path(index, error);
        let at = pattern_at(&compiled.schema, error.schema_path().as_str());
        // The pattern at the error's schema path, if it is one of `patterns`; else, as a path
        // relative to a subschema's own `$id` points elsewhere, the first that is costly too.
        let named = |patterns: &[(String, Validator)], subject: &str| {
            let subject = Value::String(subject.to_owned());
            let known = patterns.iter().any(|(regex, _)| Some(regex) == at.as_ref());
            patterns
                .iter()
                .filter(|(regex, _)| !known || Some(regex) == at.as_ref())
                .find(|(_, regex)| regex.validate(&subject).is_err_and(|e| backtracked(&e)))
                .map(|(regex, _)| regex.clone())
        };
        let keys: Vec<Found> = match error.instance().as_ref() {
            Value::Object(members) => members
                .keys()
                .filter_map(|key| {
                    Some(Found {
                        path: path.clone(),
                        message: costly("key", Some(&named(&self.key_patterns, key)?)),
                        keys: vec![key.clone()],
                        key: true,
                    })
                })
                .take(MAX_ERRORS + 1)
                .collect(),
            _ => Vec::new(),
        };
        if !keys.is_empty() {
            return keys;
        }
        let message = match error.instance().as_ref() {
            Value::String(string) => {
                costly("string", named(&self.string_patterns, string).as_deref())
            }
            _ => costly("value", at.as_deref()),
        };
        vec![Found {
            path,
            keys: Vec::new(),
            key: false,
            message,
        }]
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
    let compile = |schema: Map<String, Value>| {
        let schema = Value::Object(schema);
        let validator = options.build(&schema).ok()?;
        Some(Compiled { validator, schema })
    };
    Some(Split {
        whole: compile(whole)?,
        each: compile(each)?,
    })
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

#[cfg(test)]
mod tests {
    use super::limits::{Reach, registry};
    use super::{Compiled, MAX_MESSAGE, Schema, cut, fits, message, split};
    use jsonvet::{Config, Dialect};
    use proptest::prelude::*;
    use serde_json::{Value, json};

    use super::fold::tests::any_json;

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
                compiled: Compiled {
                    validator: options.build(schema).expect("a schema"),
                    schema: schema.clone(),
                },
                items,
                members,
                key_patterns: Vec::new(),
                string_patterns: Vec::new(),
                linear: true,
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
    }
}
