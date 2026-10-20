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
    use super::{MAX_MESSAGE, Schema, cut, fits, message, split};
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
