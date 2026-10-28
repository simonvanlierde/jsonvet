//! `--schema`: each file's values checked against one JSON Schema, every error placed at the
//! value it is about. The schema's size limits are in `limits`, placing errors at their values
//! in `place`, and folding events into values in `fold`.

mod fold;
mod limits;
mod place;
pub(crate) mod select;

use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap};
use std::ops::Range;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use jsonschema::error::ValidationErrorKind;
use jsonschema::{PatternOptions, ReferencingError, ValidationError, ValidationOptions, Validator};
use jsonvet::{Config, Dialect, Event, Report, Rule, Severity};
use serde_json::{Map, Value};

use crate::messages;
use crate::settings::Settings;
use fold::{Fold, documents};
use limits::{MAX_SCHEMA_DEPTH, PATTERN_BYTES, Reach, registry};
use place::{Place, place, place_found, pointer_path, segments};

/// How deep a file checked against a schema is read. Past it, the file has a syntax error.
/// jsonschema validates recursively, so depth costs stack (see `STACK` in lib.rs). Its memory for
/// an error deep under a recursive `$ref` grows linearly with depth: 19 MB for a 60 KB file
/// nested 10,000 levels.
pub(crate) const SCHEMA_MAX_DEPTH: usize = 512;

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
    /// in one pass; one element at a time, it holds at most one element's errors. Built the
    /// first time a document needs it, as its parts compile the schema's regexes again, so a
    /// file whose documents all pass a `linear` schema never pays for that.
    items: OnceLock<Option<Split>>,
    /// The same for an object whose members only `properties`, `patternProperties` and
    /// `additionalProperties` check, each member checked as an object of its own.
    members: OnceLock<Option<Split>>,
    /// Whether every `$ref` in the schema points into one of [`NOT_CHECKS`] at the root, as
    /// [`split`] needs.
    kept: bool,
    /// `{"pattern": p}` for each regex in the schema tried on keys: `patternProperties` and a
    /// `pattern` under `propertyNames`, to find the keys a regex error on an object is about.
    /// Empty for a `linear` schema, which has no such errors.
    key_patterns: Regexes,
    /// `{"pattern": p}` for each `pattern` in the schema, to name the one a regex error on a
    /// string is about when its schema path does not. Empty for a `linear` schema.
    string_patterns: Regexes,
    /// Whether every regex the schema uses compiles for the `regex` crate, which runs in linear
    /// time, so no match stops at [`BACKTRACK_LIMIT`]. Such a schema is built with it alone.
    linear: bool,
    /// For a schema the project picked, whose errors quote nothing from it (ADR 0012).
    quiet: Option<Quiet>,
}

/// What a quiet schema's errors say instead of its text: its name, and where in it each keyword
/// that holds such text is, as line and column by schema path.
struct Quiet {
    name: String,
    places: HashMap<String, (usize, usize)>,
    /// The root's base URI without a fragment. A schema path is relative to the nearest `$id`,
    /// so `places` holds only for errors whose keyword resolves against this.
    base: Option<String>,
}

/// The keywords whose values a jsonschema error message can quote, and those whose members' can.
/// Every other keyword's errors quote only the value checked, a type or a known format name.
const QUOTED: [&str; 21] = [
    "const",
    "enum",
    "pattern",
    "required",
    "not",
    "maximum",
    "minimum",
    "exclusiveMaximum",
    "exclusiveMinimum",
    "maxItems",
    "minItems",
    "maxLength",
    "minLength",
    "maxProperties",
    "minProperties",
    "multipleOf",
    "$ref",
    "$dynamicRef",
    "$recursiveRef",
    "dependentRequired",
    "dependencies",
];
const QUOTED_MEMBERS: [&str; 2] = ["dependentRequired", "dependencies"];

/// The keywords whose members are named by the schema's author, not keywords themselves.
const NAMES: [&str; 6] = [
    "properties",
    "patternProperties",
    "$defs",
    "definitions",
    "dependentSchemas",
    "dependencies",
];

impl Quiet {
    /// The places of the keywords in [`QUOTED`] in `schema`, found in its `events`, with the
    /// line and column of each from `report`.
    fn new(
        name: &str,
        schema: &Value,
        events: &[Event<'_>],
        report: &jsonvet::Report<'_>,
    ) -> Quiet {
        let mut targets = Vec::new();
        quoted(schema, &mut Vec::new(), None, &mut targets);
        let places = place(events, targets.clone())
            .into_iter()
            .zip(targets)
            .map(|((offset, _), (path, _))| {
                let at = report.location(offset);
                let mut pointer = String::new();
                for segment in path {
                    pointer.push('/');
                    pointer.push_str(&segment.replace('~', "~0").replace('/', "~1"));
                }
                (pointer, (at.line, at.column))
            })
            .collect();
        Quiet {
            name: name.to_owned(),
            places,
            base: limits::base(schema),
        }
    }

    /// `error`'s message without text from the schema, unless its kind quotes none: the keyword
    /// it fails, and where that is in the schema if known. For the part of a split schema checked
    /// on each element or member, `moved` is where that part came from.
    fn message(&self, error: &ValidationError<'_>, moved: Option<&str>) -> String {
        use ValidationErrorKind as K;
        match error.kind() {
            K::PropertyNames { error } => return self.message(error, moved),
            K::AdditionalItems { .. }
            | K::AdditionalProperties { .. }
            | K::AnyOf { .. }
            | K::OneOfMultipleValid { .. }
            | K::OneOfNotValid { .. }
            | K::Contains
            | K::ContentEncoding { .. }
            | K::ContentMediaType { .. }
            | K::FalseSchema
            | K::Format { .. }
            | K::FromUtf8 { .. }
            | K::Type { .. }
            | K::UnevaluatedItems { .. }
            | K::UnevaluatedProperties { .. }
            | K::UniqueItems => return message(error),
            K::BacktrackLimitExceeded { .. }
            | K::RegexEngineFailure { .. }
            | K::Constant { .. }
            | K::Custom { .. }
            | K::Enum { .. }
            | K::ExclusiveMaximum { .. }
            | K::ExclusiveMinimum { .. }
            | K::Maximum { .. }
            | K::Minimum { .. }
            | K::MaxItems { .. }
            | K::MaxLength { .. }
            | K::MaxProperties { .. }
            | K::MinItems { .. }
            | K::MinLength { .. }
            | K::MinProperties { .. }
            | K::MultipleOf { .. }
            | K::Not { .. }
            | K::Pattern { .. }
            | K::Required { .. }
            | K::Referencing(_) => {}
        }
        // A custom keyword's name is the schema's own; jsonvet adds none.
        let keyword = match error.kind() {
            K::Custom { .. } => messages::text("schema.custom-keyword"),
            kind => kind.keyword(),
        };
        let path = error.schema_path().as_str();
        let path = match (moved, path.strip_prefix(&format!("/{EACH}"))) {
            (Some(moved), Some(rest)) => Cow::Owned(format!("{moved}{rest}")),
            _ => Cow::Borrowed(path),
        };
        let name = &self.name;
        let at_root = error
            .absolute_keyword_location()
            .is_none_or(|uri| uri.as_str().split('#').next() == self.base.as_deref());
        match self.places.get(path.as_ref()).filter(|_| at_root) {
            Some((line, column)) => messages::fill(
                "schema.value-fails-at",
                &[
                    ("keyword", &keyword),
                    ("path", name),
                    ("line", line),
                    ("column", column),
                ],
            ),
            None => messages::fill(
                "schema.value-fails",
                &[("keyword", &keyword), ("path", name)],
            ),
        }
    }
}

/// The paths in `schema`, under `path`, of the keywords in [`QUOTED`], and of the members of
/// those in [`QUOTED_MEMBERS`], as targets for [`place`]. `parent` is the keyword `schema` is
/// the value of, if any.
fn quoted(
    schema: &Value,
    path: &mut Vec<String>,
    parent: Option<&str>,
    targets: &mut Vec<(Vec<String>, Vec<String>)>,
) {
    let members = parent.is_some_and(|parent| QUOTED_MEMBERS.contains(&parent));
    let names = parent.is_some_and(|parent| NAMES.contains(&parent));
    for (key, member) in children(schema) {
        path.push(key.clone().into_owned());
        if members || (!names && QUOTED.contains(&key.as_ref())) {
            targets.push((path.clone(), Vec::new()));
        }
        // An `enum` or `const` holds values, not schemas, and can be large. A name is no keyword.
        if names || !["enum", "const"].contains(&key.as_ref()) {
            quoted(member, path, (!names).then_some(key.as_ref()), targets);
        }
        path.pop();
    }
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
    /// For the part of a split schema checked on each element or member: the path in the
    /// whole schema that its [`EACH`] stands for.
    moved: Option<&'static str>,
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
        || messages::text("schema.a-pattern").to_owned(),
        |r| messages::fill("schema.the-pattern", &[("regex", &r)]),
    );
    messages::fill(
        &format!("schema.costly.{what}"),
        &[("pattern", &pattern), ("limit", &BACKTRACK_LIMIT)],
    )
}

/// A validator for `schema`, offline, so a `$ref` to anything outside the schema file fails, and
/// whether every regex it uses suits the `regex` crate. Each build compiles every regex it uses,
/// so such a schema is built with that crate, which fancy-regex would hand them to anyway, rather
/// than compiled a second time to find that out.
fn build(schema: &Value) -> (Result<Validator, ValidationError<'_>>, bool) {
    if let Ok(validator) = options(true).build(schema) {
        return (Ok(validator), true);
    }
    (options(false).build(schema), false)
}

/// The options a schema is built with: for the `regex` crate if it is `linear`, else [`fancy`].
fn options(linear: bool) -> ValidationOptions<'static> {
    if !linear {
        return fancy();
    }
    jsonschema::options().offline().with_pattern_options(
        PatternOptions::regex()
            .size_limit(PATTERN_BYTES)
            .dfa_size_limit(PATTERN_BYTES),
    )
}

/// The options for a schema with a regex the `regex` crate cannot compile. fancy-regex
/// backtracks for back-references and lookaround, and its default limit of a million lets one
/// 30-byte string take 30 ms.
fn fancy() -> ValidationOptions<'static> {
    jsonschema::options().offline().with_pattern_options(
        PatternOptions::fancy_regex()
            .backtrack_limit(BACKTRACK_LIMIT)
            .size_limit(PATTERN_BYTES)
            .dfa_size_limit(PATTERN_BYTES),
    )
}

/// A regex of a schema that is not `linear`, as the schema `{"pattern": p}`, compiled the first
/// time a match past [`BACKTRACK_LIMIT`] needs it named. Compiled when the schema is, each would
/// cost again what the schema's build already did, for nothing unless such a match comes.
struct Regex {
    text: String,
    validator: OnceLock<Option<Validator>>,
}

impl Regex {
    fn validator(&self) -> Option<&Validator> {
        self.validator
            .get_or_init(|| {
                let schema = serde_json::json!({ "pattern": self.text });
                fancy().build(&schema).ok()
            })
            .as_ref()
    }
}

/// Regexes in order of their text.
type Regexes = Vec<Arc<Regex>>;

/// The regexes in `schema` tried on keys, and those tried on strings, as [`Schema`] keeps them.
/// One tried on both is one [`Regex`], so it is compiled at most once.
fn regexes<'v>(schema: &'v Value) -> (Regexes, Regexes) {
    let (mut keys, mut strings) = (BTreeSet::new(), BTreeSet::new());
    patterns(schema, false, &mut keys, &mut strings);
    let mut all: HashMap<&'v str, Arc<Regex>> = HashMap::new();
    let mut each = |regexes: BTreeSet<&'v str>| {
        regexes
            .into_iter()
            .map(|regex| {
                let shared = all.entry(regex).or_insert_with(|| {
                    Arc::new(Regex {
                        text: regex.to_owned(),
                        validator: OnceLock::new(),
                    })
                });
                Arc::clone(shared)
            })
            .collect()
    };
    (each(keys), each(strings))
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
fn found(
    index: Option<usize>,
    value: &Value,
    error: &ValidationError<'_>,
    message: impl FnOnce() -> String,
) -> Vec<Found> {
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
        message: message(),
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

/// Open a place for `found`'s document at its first `event`, in `slot`, and finish the place
/// there before it, so one is held at a time: a place takes more memory than its errors. Kept
/// out of the walk's closure, so the closure saves no registers on each of the file's events.
#[cold]
#[inline(never)]
fn open<'e>(
    slot: &mut Option<Place<'e>>,
    found: &[Found],
    spots: &mut Vec<Vec<(usize, String)>>,
    event: &Event<'e>,
) {
    spots.extend(slot.take().map(Place::finish));
    let place = slot.insert(Place::new(place::found_targets(found)));
    place.event(event);
}

/// A file's documents checked as their events come in.
struct Checking<'s, 'e> {
    schema: &'s Schema,
    fold: Fold<'e>,
    /// Each document with something to report, in order.
    documents: Vec<Checked>,
}

/// What one document has to report: why it is not checked, or its errors and where its events
/// and its first event are, to place them.
enum Checked {
    Unchecked(usize, &'static str),
    Found(Range<usize>, usize, Errors),
}

impl<'e> Checking<'_, 'e> {
    /// Take the file's next event, and check the document it completes, if any.
    fn event(&mut self, event: &Event<'e>) {
        let Some(document) = self.fold.event(event) else {
            return;
        };
        if let Some((offset, reason)) = document.unchecked {
            self.documents.push(Checked::Unchecked(offset, reason));
            return;
        }
        let found = self.schema.collect(&document.value);
        if !found.kept.is_empty() {
            self.documents
                .push(Checked::Found(document.events, document.offset, found));
        }
    }

    /// The errors, placed in a second walk over the file's events. `walk` gets the event range
    /// and offset of each document with errors, and hands its argument at least those
    /// documents' events, in order, each with its index from the first walk. It is called only if
    /// there is an error to place.
    fn errors(
        self,
        walk: impl FnOnce(&[(Range<usize>, usize)], &mut dyn FnMut(usize, &Event<'e>)),
    ) -> Vec<(usize, String, bool)> {
        let pending: Vec<(Range<usize>, &[Found])> = self
            .documents
            .iter()
            .filter_map(|checked| match checked {
                Checked::Found(events, _, found) => Some((events.clone(), &found.kept[..])),
                Checked::Unchecked(..) => None,
            })
            .collect();
        let failing: Vec<(Range<usize>, usize)> = self
            .documents
            .iter()
            .filter_map(|checked| match checked {
                Checked::Found(events, offset, _) => Some((events.clone(), *offset)),
                Checked::Unchecked(..) => None,
            })
            .collect();
        let mut spots = Vec::with_capacity(pending.len());
        // The place of the document the walk is in.
        let mut place: Option<Place<'e>> = None;
        if !pending.is_empty() {
            let mut next = 0;
            walk(&failing, &mut |i, event| {
                while pending.get(next).is_some_and(|(events, _)| events.end <= i) {
                    next += 1;
                }
                if let Some((events, found)) = pending.get(next)
                    && events.contains(&i)
                {
                    if i == events.start {
                        open(&mut place, found, &mut spots, event);
                    } else if let Some(place) = &mut place {
                        place.event(event);
                    }
                }
            });
        }
        // The walk reaches every document, so this is the last one's place.
        spots.extend(place.map(Place::finish));
        debug_assert_eq!(spots.len(), pending.len());
        let mut spots = spots.into_iter();
        let mut errors = Vec::new();
        for checked in self.documents {
            let (offset, found) = match checked {
                Checked::Unchecked(offset, reason) => {
                    errors.push((
                        offset,
                        messages::fill("schema.unchecked", &[("reason", &reason)]),
                        false,
                    ));
                    continue;
                }
                Checked::Found(_, offset, found) => (offset, found),
            };
            let spots = spots.next().unwrap_or_default();
            let spots = place::found_spots(&found.kept, spots);
            let mut placed: Vec<(usize, String, bool)> = found
                .kept
                .into_iter()
                .zip(spots)
                .map(|(found, (offset, at))| {
                    let message = messages::fill(
                        "schema.found-at",
                        &[("message", &found.message), ("at", &at)],
                    );
                    (offset, cut(message), false)
                })
                .collect();
            // Stable, so errors at one value keep the schema's order.
            placed.sort_by_key(|(offset, ..)| *offset);
            errors.append(&mut placed);
            if found.more {
                errors.push((offset, messages::text("schema.more").to_owned(), true));
            }
        }
        errors
    }
}

impl Schema {
    /// Read and compile the schema at `path`, in its dialect under `settings`. Any problem
    /// with it is a usage error. A `quiet` one names only the place, never text from the
    /// schema, as for a symlinked config file: a pull request picks a `$schema` or config
    /// schema, which could be a file that holds a secret (ADR 0012). `admit` takes the number of
    /// distinct regexes in it before any is compiled, and may refuse it.
    pub(crate) fn load(
        path: &Path,
        settings: &Settings,
        quiet: bool,
        admit: impl FnOnce(usize) -> Result<(), String>,
    ) -> Result<Schema, String> {
        let name = path.to_string_lossy();
        let text = crate::read_small(path, crate::MAX_SCHEMA_BYTES)
            .map_err(|e| messages::fill("schema.unreadable", &[("path", &name), ("error", &e)]))?;
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
            let place: [(&str, &dyn std::fmt::Display); 3] =
                [("path", &name), ("line", &at.line), ("column", &at.column)];
            if quiet {
                return messages::fill("schema.unusable", &place);
            }
            messages::fill(
                "schema.problem",
                &[place[0], place[1], place[2], ("problem", problem)],
            )
        };
        if let Some(problem) = report
            .problems()
            .iter()
            .find(|p| p.severity() == Severity::Error)
        {
            return Err(at(problem.offset(), &messages::problem(problem)));
        }
        let events = events.unwrap_or_default();
        let mut documents: Vec<_> = documents(&events).collect();
        if documents.len() != 1 {
            return Err(messages::fill("schema.one-value", &[("path", &name)]));
        }
        let document = documents.swap_remove(0);
        let events = &events[document.events.clone()];
        if let Some((offset, reason)) = document.unchecked {
            return Err(at(offset, &reason));
        }
        let schema = &document.value;
        let outside = |error: &ReferencingError| match error {
            ReferencingError::Unretrievable { .. } if quiet => {
                Some(messages::fill("schema.ref-outside", &[("path", &name)]))
            }
            ReferencingError::Unretrievable { uri, .. } => Some(messages::fill(
                "schema.ref-outside-to",
                &[("path", &name), ("uri", uri)],
            )),
            _ => None,
        };
        // The same index of `$id`s and anchors jsonschema builds, offline.
        let registry = registry(schema)
            .map_err(|error| outside(&error).unwrap_or_else(|| at(events[0].offset, &error)))?;
        // jsonschema also validates recursively through each `$ref`, so a long chain of them
        // takes the stack or hours, however shallow the schema is.
        let refuse = |over: limits::Over| {
            let (offset, _) = place(events, vec![(over.path.clone(), vec![over.key.clone()])])[0];
            at(offset, &over.message())
        };
        let kept = Reach::new(schema).measure(&registry).map_err(refuse)?;
        admit(limits::patterns(schema).map_err(refuse)?)?;
        let (built, linear) = build(schema);
        let error = match built {
            Ok(validator) => {
                let (key_patterns, string_patterns) = if linear {
                    (Vec::new(), Vec::new())
                } else {
                    regexes(schema)
                };
                return Ok(Schema {
                    compiled: Compiled {
                        validator,
                        schema: schema.clone(),
                        moved: None,
                    },
                    items: OnceLock::new(),
                    members: OnceLock::new(),
                    kept,
                    key_patterns,
                    string_patterns,
                    linear,
                    quiet: quiet.then(|| Quiet::new(&name, schema, events, &report)),
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
        let (offset, _) = place_found(events, &found(None, schema, &error, String::new))[0];
        // A regex too large to compile is reported as no regex at all.
        if matches!(error.kind(), ValidationErrorKind::Format { format } if format == "regex") {
            let mb = PATTERN_BYTES >> 20;
            return Err(at(
                offset,
                &messages::fill("schema.regex-too-big", &[("error", &error), ("mb", &mb)]),
            ));
        }
        Err(at(offset, &error))
    }

    /// `text`'s report under `config`, read no deeper than [`SCHEMA_MAX_DEPTH`], and its schema
    /// errors as (offset, message, summary): each document's in order, then a summary if it has
    /// more than [`MAX_ERRORS`]. A text with a syntax error gets none. The values are checked as the text is read, a document at a time,
    /// and the text is read again only to place errors, so its events are never all held at once.
    pub(crate) fn check<'t>(
        &self,
        text: &'t str,
        config: &Config,
    ) -> (Report<'t>, Vec<(usize, String, bool)>) {
        let config = config.clone().max_depth(SCHEMA_MAX_DEPTH);
        self.check_with(
            |sink| jsonvet::parse_with(text, &config, sink),
            |report| report.problems().iter().any(|p| p.rule() == Rule::Syntax),
            |_, sink| {
                let mut i = 0;
                jsonvet::parse_with(text, &config, &mut |event| {
                    sink(i, &event);
                    i += 1;
                });
            },
        )
    }

    /// The `lines` of a JSON Lines `text`, one part of a split file, checked as [`Schema::check`]
    /// checks a whole text. Each line is a document, so only the lines with errors are read again.
    pub(crate) fn check_lines(
        &self,
        text: &str,
        lines: Range<usize>,
        config: &Config,
    ) -> (jsonvet::Lines, Vec<(usize, String, bool)>) {
        let config = config.clone().max_depth(SCHEMA_MAX_DEPTH);
        self.check_with(
            |sink| jsonvet::parse_lines(text, lines.clone(), &config, Some(sink)),
            jsonvet::Lines::syntax_error,
            |failing, sink| {
                // Documents with no events between them are read again as one range.
                let mut documents = failing.iter().peekable();
                while let Some((events, offset)) = documents.next() {
                    let mut last = (events.end, *offset);
                    while let Some((next, offset)) =
                        documents.next_if(|(next, _)| next.start == last.0)
                    {
                        last = (next.end, *offset);
                    }
                    let start = text[..*offset].rfind('\n').map_or(0, |at| at + 1);
                    let end = text[last.1..]
                        .find('\n')
                        .map_or(text.len(), |at| last.1 + at + 1);
                    let mut i = events.start;
                    jsonvet::parse_lines(
                        text,
                        start..end,
                        &config,
                        Some(&mut |event| {
                            sink(i, &event);
                            i += 1;
                        }),
                    );
                }
            },
        )
    }

    /// What `read` gives, checking each event it hands its sink, and the schema errors unless
    /// `syntax_error` says it found one. `again` runs only if there is an error to place, as
    /// [`Checking::errors`]' walk.
    fn check_with<'t, R>(
        &self,
        read: impl FnOnce(&mut dyn FnMut(Event<'t>)) -> R,
        syntax_error: impl FnOnce(&R) -> bool,
        again: impl FnOnce(&[(Range<usize>, usize)], &mut dyn FnMut(usize, &Event<'t>)),
    ) -> (R, Vec<(usize, String, bool)>) {
        let mut checking = self.checking();
        let first = read(&mut |event| checking.event(&event));
        if syntax_error(&first) {
            return (first, Vec::new());
        }
        (first, checking.errors(again))
    }

    fn checking<'e>(&self) -> Checking<'_, 'e> {
        Checking {
            schema: self,
            fold: Fold::default(),
            documents: Vec::new(),
        }
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
        let parts = match value {
            Value::Array(_) => self.split(false),
            Value::Object(_) => self.split(true),
            _ => None,
        };
        match (value, parts) {
            (Value::Array(items), Some(Split { whole, each })) => {
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
            (Value::Object(members), Some(Split { whole, each })) => {
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

    /// The schema split for arrays, or with `members` for objects, if it can be, built on first
    /// need. Threads checking parts of one file at once share it, and build it at most once.
    fn split(&self, members: bool) -> Option<&Split> {
        let parts = if members { &self.members } else { &self.items };
        parts
            .get_or_init(|| {
                let options = options(self.linear);
                split(&self.compiled.schema, self.kept, members, &options)
            })
            .as_ref()
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
            return found(index, value, error, || match &self.quiet {
                Some(quiet) => quiet.message(error, compiled.moved),
                None => message(error),
            });
        }
        let path = pointer_path(index, error);
        let at = pattern_at(&compiled.schema, error.schema_path().as_str());
        // The pattern at the error's schema path, if it is one of `patterns`; else, as a path
        // relative to a subschema's own `$id` points elsewhere, the first that is costly too.
        let named = |patterns: &[Arc<Regex>], subject: &str| {
            let subject = Value::String(subject.to_owned());
            let known = patterns
                .iter()
                .any(|regex| Some(&regex.text) == at.as_ref());
            patterns
                .iter()
                .filter(|regex| !known || Some(&regex.text) == at.as_ref())
                .find(|regex| {
                    regex
                        .validator()
                        .is_some_and(|v| v.validate(&subject).is_err_and(|e| backtracked(&e)))
                })
                .map(|regex| regex.text.clone())
        };
        // A quiet schema's regex is never named.
        let shown = |regex: Option<String>| regex.filter(|_| self.quiet.is_none());
        let keys: Vec<Found> = match error.instance().as_ref() {
            Value::Object(members) => members
                .keys()
                .filter_map(|key| {
                    let regex = named(&self.key_patterns, key)?;
                    Some(Found {
                        path: path.clone(),
                        message: costly("key", shown(Some(regex)).as_deref()),
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
            Value::String(string) => costly(
                "string",
                shown(named(&self.string_patterns, string)).as_deref(),
            ),
            _ => costly("value", shown(at.clone()).as_deref()),
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
/// NOTE: each part compiles its regexes again, as jsonschema keeps compiled regexes per build, so
/// [`Schema::split`] calls this only once a document needs it; [`limits::patterns`] bounds what
/// that costs.
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
    let compile = |schema: Map<String, Value>, moved| {
        let schema = Value::Object(schema);
        let validator = options.build(&schema).ok()?;
        Some(Compiled {
            validator,
            schema,
            moved,
        })
    };
    Some(Split {
        whole: compile(whole, None)?,
        each: compile(each, Some(if members { "" } else { "/items" }))?,
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
    use std::sync::OnceLock;

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

    /// The schema errors [`Schema::check`] returns.
    type Errors = Vec<(usize, String, bool)>;

    /// `text`'s schema errors under `schema`, checked in one pass and, if it can be, split.
    fn one_pass_and_split(schema: &Value, text: &str) -> (Errors, Option<Errors>) {
        let options = jsonschema::options();
        let registry = registry(schema).expect("a registry");
        let kept = Reach::new(schema).measure(&registry).is_ok_and(|kept| kept);
        let checked = |items, members| {
            let schema = Schema {
                compiled: Compiled {
                    validator: options.build(schema).expect("a schema"),
                    schema: schema.clone(),
                    moved: None,
                },
                items: OnceLock::from(items),
                members: OnceLock::from(members),
                kept,
                key_patterns: Vec::new(),
                string_patterns: Vec::new(),
                linear: true,
                quiet: None,
            };
            schema.check(text, &Config::new(Dialect::Json)).1
        };
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
