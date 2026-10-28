//! How deep and how big a schema gets with its `$ref`s followed, as jsonschema follows them,
//! and how many regexes it holds.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use jsonschema::{Draft, ReferencingError, Registry};
use referencing::{Resolver, ResourceRef};
use serde_json::Value;

use super::{NOT_CHECKS, children};
use crate::messages;

/// How deep a schema may nest, counting each local `$ref` followed as one more level; real
/// schemas stay well under it.
pub(super) const MAX_SCHEMA_DEPTH: usize = 128;

/// How many objects and arrays a schema may hold once each `$ref` is replaced by what it points
/// at, with the values in each `enum` and `const`. jsonschema may check every one of them for
/// one value, so `$ref`s that each point twice at the next schema, 24 levels deep, took 12.9 s
/// and 11.4 GB.
const MAX_SCHEMA_SIZE: usize = 5_000_000;

/// How many bytes one regex may take compiled, for the `regex` crate's program and, again, for
/// the cache it fills while matching. Its default of 10 MB let a 9.4 KB schema of 200 patterns
/// such as `(?:[a-z]{1000}){100}N` take 11 s and 2.8 GB to compile. The largest regex on
/// SchemaStore takes 0.7 MB.
pub(super) const PATTERN_BYTES: usize = 2 << 20;

/// How many distinct regexes a schema may hold, each up to [`PATTERN_BYTES`] compiled. The most
/// on SchemaStore is 457, in one schema. 1,000 regexes of nearly 2 MB each, a 44 KB schema, took
/// 7 s and 2.6 GB to compile.
const MAX_PATTERNS: usize = 1_000;

/// How many regexes the schemas one run reads may hold between them, each schema's distinct
/// ones counted on their own, as each schema compiles its own. Every schema stays in memory
/// until the run ends, so three schemas of 1,000 near-cap regexes took 21 s and 7.9 GB. The
/// 43 schemas on SchemaStore with the most regexes hold 994 between them, and all 1,008
/// hold 2,015.
pub(super) const RUN_PATTERNS: usize = 1_000;

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

/// The registry jsonschema builds for `schema`, with every resource and anchor in it by its
/// absolute URI, and no way to fetch another.
pub(super) fn registry(schema: &Value) -> Result<Registry<'_>, ReferencingError> {
    let resource = ResourceRef::new(schema, Draft::default().detect(schema));
    Registry::new()
        .draft(resource.draft())
        .add(resource.id().unwrap_or(DEFAULT_BASE), resource)?
        .prepare()
}

/// The base URI jsonschema resolves `schema`'s root against, without a fragment.
pub(super) fn base(schema: &Value) -> Option<String> {
    let resource = ResourceRef::new(schema, Draft::default().detect(schema));
    let uri = referencing::uri::from_str(resource.id().unwrap_or(DEFAULT_BASE)).ok()?;
    uri.as_str().split('#').next().map(str::to_owned)
}

/// How deep and how big a schema gets when its `$ref`s are followed, resolved as jsonschema
/// resolves them: against the base each `$id` sets. A `$ref` back to a schema on the way to it
/// is recursion, which jsonschema handles: it adds nothing.
pub(super) struct Reach<'r> {
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
    pub(super) fn new(root: &'r Value) -> Self {
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
    pub(super) fn measure(mut self, registry: &'r Registry<'r>) -> Result<bool, Over> {
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
            // A `$recursiveRef` is `#`, and the dynamic scope it may move to is a schema on the
            // way, which adds nothing.
            let (target, target_resolver, target_draft) = match resolver.lookup(reference) {
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
            // jsonschema may compare a value with each one in an `enum`, so they cost time.
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
                let again = Visit {
                    measure: Measure {
                        height: 0,
                        size: again,
                    },
                    cut: usize::MAX,
                };
                if below.add(again) {
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

/// How many distinct regexes `schema` holds, counting each `pattern` and `patternProperties` key
/// anywhere in it, or where it holds one past [`MAX_PATTERNS`].
pub(super) fn patterns(schema: &Value) -> Result<usize, Over> {
    let mut path = Vec::new();
    let mut seen = HashSet::new();
    match past(schema, &mut path, &mut seen) {
        Some(key) => Err(Over {
            path,
            key,
            problem: Problem::Patterns,
        }),
        None => Ok(seen.len()),
    }
}

/// The key of the regex in `value` that takes `seen` past [`MAX_PATTERNS`], if one does, with
/// `path` left at the object that holds it.
fn past<'v>(
    value: &'v Value,
    path: &mut Vec<String>,
    seen: &mut HashSet<&'v str>,
) -> Option<String> {
    for (key, member) in children(value) {
        match (key.as_ref(), member) {
            ("patternProperties", Value::Object(regexes)) => {
                if let Some(regex) = regexes.keys().find(|regex| one_too_many(seen, regex)) {
                    path.push(key.into_owned());
                    return Some(regex.clone());
                }
            }
            ("pattern", Value::String(regex)) if one_too_many(seen, regex) => {
                return Some(key.into_owned());
            }
            _ => {}
        }
        path.push(key.into_owned());
        if let Some(key) = past(member, path, seen) {
            return Some(key);
        }
        path.pop();
    }
    None
}

/// Add `regex` to `seen`, and say whether it is one past [`MAX_PATTERNS`].
fn one_too_many<'v>(seen: &mut HashSet<&'v str>, regex: &'v str) -> bool {
    seen.insert(regex) && seen.len() > MAX_PATTERNS
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
    /// Past [`MAX_PATTERNS`] distinct regexes.
    Patterns,
}

/// Where a schema is turned away, and why: the path of an object and a key in it.
pub(super) struct Over {
    pub(super) path: Vec<String>,
    pub(super) key: String,
    problem: Problem,
}

impl Over {
    pub(super) fn message(&self) -> String {
        let key = &self.key;
        let fill = |id, limit: &usize| messages::fill(id, &[("key", key), ("limit", limit)]);
        match &self.problem {
            Problem::Deep => fill("schema.too-deep", &MAX_SCHEMA_DEPTH),
            Problem::Big => fill("schema.too-big", &MAX_SCHEMA_SIZE),
            Problem::Twice => fill("schema.too-big-twice", &MAX_SCHEMA_SIZE),
            Problem::Unresolved(error) => {
                messages::fill("schema.unresolved", &[("key", key), ("error", error)])
            }
            Problem::Patterns => messages::fill(
                "schema.too-many-patterns",
                &[("limit", &MAX_PATTERNS), ("mb", &(PATTERN_BYTES >> 20))],
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_BASE, MAX_PATTERNS, MAX_SCHEMA_SIZE, Measure, Over, Problem, Reach, Visit,
        patterns, registry,
    };
    use jsonschema::Draft;
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

    #[test]
    fn enum_and_const_values_count_toward_the_size() {
        // 16 levels, each with two `$ref`s to the next, reach the last 65,536 times: about 0.4
        // million objects and arrays, and 7 million values with the 100 numbers at the end.
        let fan_out = |last: Value| {
            let mut defs = serde_json::Map::new();
            for i in 0..16 {
                let next = format!("#/$defs/d{}", i + 1);
                defs.insert(
                    format!("d{i}"),
                    json!({"anyOf": [{"$ref": next}, {"$ref": next}]}),
                );
            }
            defs.insert("d16".to_owned(), last);
            json!({"$defs": defs, "$ref": "#/$defs/d0"})
        };
        let numbers: Vec<usize> = (0..100).collect();
        for last in [json!({"enum": numbers}), json!({"const": [numbers]})] {
            let big = over(&fan_out(last)).is_some_and(|over| matches!(over.problem, Problem::Big));
            assert!(big);
        }
        assert!(over(&fan_out(json!({"type": "string"}))).is_none());
        // Values jsonschema does not compare with, as in `default`, count only their arrays.
        assert!(over(&fan_out(json!({"default": numbers}))).is_none());
    }

    #[test]
    fn a_reference_keyword_of_another_draft_is_not_followed() {
        for (draft, key) in [
            ("http://json-schema.org/draft-07/schema#", "$dynamicRef"),
            ("http://json-schema.org/draft-07/schema#", "$recursiveRef"),
            (
                "https://json-schema.org/draft/2019-09/schema",
                "$dynamicRef",
            ),
        ] {
            let schema = json!({"$schema": draft, "properties": {"a": {key: "#/nowhere"}}});
            assert!(over(&schema).is_none(), "{draft} {key}");
        }
        let schema = json!({"properties": {"a": {"$dynamicRef": "#/nowhere"}}});
        let unresolved = over(&schema).is_some_and(|over| {
            matches!(over.problem, Problem::Unresolved(_)) && over.path == ["properties", "a"]
        });
        assert!(unresolved);
    }

    #[test]
    fn pattern_properties_keys_and_patterns_count_once_each() {
        // `^a` is both a key and a `pattern`, and counts once.
        let keys: serde_json::Map<String, Value> = (0..MAX_PATTERNS)
            .map(|i| (format!("^k{i}"), json!({})))
            .collect();
        let schema = |extra: &str| json!({"patternProperties": keys, "properties": {"a": {"pattern": "^k1"}, "b": {"pattern": extra}}});
        assert!(patterns(&schema("^k2")).is_ok());
        let over = patterns(&schema("^new")).expect_err("one past the cap");
        assert!(matches!(over.problem, Problem::Patterns));
        assert_eq!(
            (over.path.join("/"), over.key),
            ("properties/b".to_owned(), "pattern".to_owned())
        );
        let mut keys = keys.clone();
        keys.insert("^last".to_owned(), json!({}));
        let over = patterns(&json!({"patternProperties": keys})).expect_err("past the cap");
        assert_eq!(
            (over.path, over.key),
            (vec!["patternProperties".to_owned()], "^last".to_owned())
        );
    }
}
