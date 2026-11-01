//! A document's values as an arena that borrows its strings from the text, which jsonschema
//! validates through its `Json` trait instead of a `serde_json::Value` copy of the document.

use std::borrow::Cow;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::OnceLock;

use jsonschema::JsonType as T;
use jsonschema::json::cmp;
use jsonschema::json::{Array, Json, Node, NodeIdentity, Object};
use jsonschema_value::LazyInstance;

use super::fold::documents;
use jsonvet::{Config, Dialect, PathStep, json_path};
use serde_json::{Map, Number, Value};

// One entry per value: a change that grows it costs memory on every number in a large file.
const _: () = assert!(std::mem::size_of::<Entry>() <= 16);

/// jsonschema's name for this representation of a document.
pub(super) struct Own;

/// One value. A container holds the range of its children in [`Tree::elems`] or
/// [`Tree::members`].
enum Entry {
    Null,
    Bool(bool),
    /// A number, in the three forms `serde_json::Number` keeps, so that it takes 8 bytes.
    Pos(u64),
    Neg(i64),
    Float(f64),
    /// Its index in [`Tree::strings`], which keeps the entry small.
    String(u32),
    /// The range of its elements, and the offset just past its `]`.
    Array(u32, u32, u32),
    /// Its members are sorted by key with duplicates gone, as a `serde_json::Map` keeps them.
    /// The same for its members and its `}`.
    Object(u32, u32, u32),
}

/// One document's values. Strings are borrowed from the text unless they hold an escape.
#[derive(Default)]
pub(super) struct Tree<'e> {
    entries: Vec<Entry>,
    strings: Vec<Cow<'e, str>>,
    /// The offset of each entry's first byte in the text.
    at: Vec<u32>,
    elems: Vec<u32>,
    members: Vec<Member<'e>>,
    /// The key as written, by the entry of its value, for a key that holds U+FFFD: a lone
    /// surrogate in the text is U+FFFD in the key's text.
    raws: HashMap<u32, Cow<'e, [u8]>>,
    root: u32,
    /// The text the offsets are in, and its dialect's index in [`Dialect::ALL`], so that a
    /// container can be read again from its span. Empty when the tree is not to be read again.
    source: &'e str,
    dialect: u32,
    /// The offset of the document's first byte, which the offsets in the tree count from, so that
    /// only a document itself, not its place in a long file, is limited to 32 bits.
    base: usize,
}

/// An offset below the start of the document, as the tree keeps it. [`MAX_OFFSET`] is the most a
/// tree is built for, one under what fits, so that the end of a container still does.
pub(super) const MAX_OFFSET: usize = u32::MAX as usize - 1;

pub(super) fn offset(at: usize) -> u32 {
    u32::try_from(at).unwrap_or(u32::MAX)
}

/// An object member: its key, its value's entry, and the offset of its key in the text.
type Member<'e> = (Cow<'e, str>, u32, u32);

/// What [`Tree`] is built with while a document's events come in: the children of every open
/// container, until it ends.
#[derive(Default)]
pub(super) struct Builder<'e> {
    pub(super) tree: Tree<'e>,
    pub(super) elems: Vec<u32>,
    pub(super) members: Vec<Member<'e>>,
}

impl<'e> Builder<'e> {
    /// A builder for trees of the document `source`, in `dialect`.
    pub(super) fn new(source: &'e str, dialect: Dialect) -> Self {
        let dialect = Dialect::ALL.iter().position(|d| *d == dialect);
        let tree = Tree {
            source,
            dialect: dialect.and_then(|d| u32::try_from(d).ok()).unwrap_or(0),
            ..Tree::default()
        };
        Builder {
            tree,
            ..Builder::default()
        }
    }

    /// Begin a document whose first byte is at `base`.
    pub(super) fn begin(&mut self, base: usize) {
        self.tree.base = base;
    }

    /// An offset in the text, as the open document's tree keeps it.
    pub(super) fn relative(&self, at: usize) -> u32 {
        offset(at - self.tree.base)
    }

    /// Keep the key of the member whose value is `node` as written.
    pub(super) fn raw(&mut self, node: u32, raw: Cow<'e, [u8]>) {
        self.tree.raws.insert(node, raw);
    }

    pub(super) fn string(&mut self, text: Cow<'e, str>, at: usize) -> u32 {
        let index = u32::try_from(self.tree.strings.len()).unwrap_or(u32::MAX);
        self.tree.strings.push(text);
        self.add(Entry::String(index), at)
    }

    pub(super) fn scalar(&mut self, value: Value, at: usize) -> u32 {
        let entry = match value {
            Value::Bool(b) => Entry::Bool(b),
            Value::Number(n) => match (n.as_u64(), n.as_i64(), n.as_f64()) {
                (Some(u), ..) => Entry::Pos(u),
                (None, Some(i), _) => Entry::Neg(i),
                (None, None, Some(f)) => Entry::Float(f),
                _ => Entry::Null,
            },
            _ => Entry::Null,
        };
        self.add(entry, at)
    }

    fn add(&mut self, entry: Entry, at: usize) -> u32 {
        self.tree.entries.push(entry);
        self.tree.at.push(offset(at - self.tree.base));
        u32::try_from(self.tree.entries.len() - 1).unwrap_or(u32::MAX)
    }

    /// End the array whose children began at `start` in [`Builder::elems`].
    pub(super) fn end_array(&mut self, start: usize, at: usize, end: usize) -> u32 {
        let from = u32::try_from(self.tree.elems.len()).unwrap_or(u32::MAX);
        self.tree.elems.extend(self.elems.drain(start..));
        let to = u32::try_from(self.tree.elems.len()).unwrap_or(u32::MAX);
        self.add(Entry::Array(from, to, offset(end - self.tree.base)), at)
    }

    /// End the object whose members began at `start` in [`Builder::members`]. A later member of
    /// a key replaces an earlier one.
    pub(super) fn end_object(&mut self, start: usize, at: usize, end: usize) -> u32 {
        let from = u32::try_from(self.tree.members.len()).unwrap_or(u32::MAX);
        self.members[start..].sort_by(|a, b| a.0.cmp(&b.0));
        for member in self.members.drain(start..) {
            match self.tree.members[from as usize..].last_mut() {
                // The later value, at the first copy of the key.
                Some(last) if last.0 == member.0 => last.1 = member.1,
                _ => self.tree.members.push(member),
            }
        }
        let to = u32::try_from(self.tree.members.len()).unwrap_or(u32::MAX);
        self.add(Entry::Object(from, to, offset(end - self.tree.base)), at)
    }

    pub(super) fn finish(&mut self, root: u32) -> Tree<'e> {
        self.elems.clear();
        self.members.clear();
        let (source, dialect) = (self.tree.source, self.tree.dialect);
        let mut tree = std::mem::take(&mut self.tree);
        tree.root = root;
        self.tree.source = source;
        self.tree.dialect = dialect;
        tree
    }
}

impl<'e> Tree<'e> {
    /// The document's root.
    pub(super) fn root(&self) -> Nd<'_> {
        Nd::At(self, self.root, None)
    }

    /// The values as `serde_json` reads them.
    pub(super) fn value(&self) -> Value {
        self.root().to_value().into_owned()
    }

    /// Where each target is: given as a value's path and keys in it, the first of those keys'
    /// offset, or else the value's. With it, the value's JSONPath, written as duplicate-key
    /// paths are. A later duplicate key's value is the one found, at the key's first copy.
    pub(super) fn place(&self, targets: &[(Vec<String>, Vec<String>)]) -> Vec<(usize, String)> {
        targets
            .iter()
            .map(|(path, keys)| self.place_one(path, keys))
            .collect()
    }

    fn place_one(&self, path: &[String], keys: &[String]) -> (usize, String) {
        let mut node = self.root;
        let mut steps = Vec::with_capacity(path.len());
        let mut lost = false;
        for segment in path {
            let next = match (lost, &self.entries[node as usize]) {
                (false, Entry::Array(from, to, _)) => segment
                    .parse::<usize>()
                    .ok()
                    .filter(|i| i.to_string() == *segment && (*from as usize + i) < *to as usize)
                    .map(|i| (self.elems[*from as usize + i], PathStep::Index(i))),
                (false, Entry::Object(from, to, _)) => {
                    let members = self.members_of(*from, *to);
                    let found =
                        members.binary_search_by(|(k, ..)| k.as_ref().cmp(segment.as_str()));
                    found.ok().map(|at| {
                        let child = members[at].1;
                        let raw = self.raws.get(&child).map_or(segment.as_bytes(), |raw| raw);
                        (child, PathStep::Key(raw))
                    })
                }
                _ => None,
            };
            if let Some((child, step)) = next {
                node = child;
                steps.push(step);
            } else {
                lost = true;
                steps.push(PathStep::Key(segment.as_bytes()));
            }
        }
        let offset = match (&self.entries[node as usize], lost) {
            (Entry::Object(from, to, _), false) => {
                let members = self.members_of(*from, *to);
                keys.iter()
                    .filter_map(|key| {
                        let at = members.binary_search_by(|(k, ..)| k.as_ref().cmp(key.as_str()));
                        at.ok().map(|at| self.base + members[at].2 as usize)
                    })
                    .min()
            }
            _ => None,
        };
        (
            offset.unwrap_or(self.base + self.at[node as usize] as usize),
            json_path(steps),
        )
    }

    /// The text of the container at `index`, if the tree keeps its text.
    fn span(&self, index: u32) -> Option<&'e [u8]> {
        let (Entry::Array(_, _, end) | Entry::Object(_, _, end)) = self.entries[index as usize]
        else {
            return None;
        };
        self.source
            .as_bytes()
            .get(self.base + self.at[index as usize] as usize..self.base + end as usize)
    }

    fn members_of(&self, from: u32, to: u32) -> &[Member<'e>] {
        &self.members[from as usize..to as usize]
    }

    fn value_at(&self, index: u32) -> Value {
        match &self.entries[index as usize] {
            Entry::Null => Value::Null,
            Entry::Bool(b) => Value::Bool(*b),
            Entry::Pos(u) => Value::Number(Number::from(*u)),
            Entry::Neg(i) => Value::Number(Number::from(*i)),
            Entry::Float(f) => Number::from_f64(*f).map_or(Value::Null, Value::Number),
            Entry::String(s) => Value::String(self.strings[*s as usize].to_string()),
            Entry::Array(from, to, _) => Value::Array(
                self.elems[*from as usize..*to as usize]
                    .iter()
                    .map(|&child| self.value_at(child))
                    .collect(),
            ),
            Entry::Object(from, to, _) => Value::Object(
                self.members_of(*from, *to)
                    .iter()
                    .map(|(key, child, _)| (key.to_string(), self.value_at(*child)))
                    .collect::<Map<_, _>>(),
            ),
        }
    }
}

/// A value of a [`Tree`], or a string alone; for an object, perhaps only one of its members.
#[derive(Clone, Copy)]
pub(super) enum Nd<'a> {
    At(&'a Tree<'a>, u32, Option<u32>),
    Text(&'a str),
}

impl<'a> Nd<'a> {
    /// The object with only its member at `position`, as the part of a schema checked on each
    /// member sees it.
    pub(super) fn only(&self, position: u32) -> Nd<'a> {
        match self {
            Nd::At(tree, index, _) => Nd::At(tree, *index, Some(position)),
            Nd::Text(text) => Nd::Text(text),
        }
    }

    /// The value at the JSON Pointer segments `path` below this one.
    pub(super) fn at<'p>(&self, path: impl IntoIterator<Item = &'p str>) -> Option<Nd<'a>> {
        let mut node = *self;
        for segment in path {
            node = match node.entry()? {
                Entry::Object(..) => node.as_object()?.get(&segment.to_owned())?,
                Entry::Array(..) => node.as_array()?.elements().nth(segment.parse().ok()?)?,
                _ => return None,
            };
        }
        Some(node)
    }

    /// The keys of an object, in the order its members are kept.
    pub(super) fn keys(&self) -> Vec<String> {
        self.as_object()
            .map(|object| object.members().map(|(key, _)| key.to_owned()).collect())
            .unwrap_or_default()
    }

    fn entry(&self) -> Option<&'a Entry> {
        match self {
            Nd::At(tree, index, _) => Some(&tree.entries[*index as usize]),
            Nd::Text(_) => None,
        }
    }
}

impl Json for Own {
    type Node<'a> = Nd<'a>;
    type PreparedKey = String;
    type StringBuffer = ();

    fn prepare_key(key: &str) -> String {
        key.to_owned()
    }

    fn with_string_node<T>(_buffer: &mut (), string: &str, f: impl FnOnce(Nd<'_>) -> T) -> T {
        f(Nd::Text(string))
    }
}

/// An object, or one of its members: the slice of members it shows.
#[derive(Clone, Copy)]
pub(super) struct Members<'a>(&'a Tree<'a>, &'a [Member<'a>]);

pub(super) struct MemberIter<'a>(&'a Tree<'a>, std::slice::Iter<'a, Member<'a>>);

impl<'a> Iterator for MemberIter<'a> {
    type Item = (&'a str, Nd<'a>);
    fn next(&mut self) -> Option<Self::Item> {
        let (key, child, _) = self.1.next()?;
        Some((key.as_ref(), Nd::At(self.0, *child, None)))
    }
}

impl<'a> Object<'a, Own> for Members<'a> {
    type Node = Nd<'a>;
    type MemberName = &'a str;
    type MembersIter = MemberIter<'a>;

    fn len(&self) -> usize {
        self.1.len()
    }

    fn get(&self, key: &String) -> Option<Nd<'a>> {
        let at = self
            .1
            .binary_search_by(|(k, ..)| k.as_ref().cmp(key.as_str()))
            .ok()?;
        Some(Nd::At(self.0, self.1[at].1, None))
    }

    fn members(&self) -> MemberIter<'a> {
        MemberIter(self.0, self.1.iter())
    }
}

#[derive(Clone, Copy)]
pub(super) struct Elems<'a>(&'a Tree<'a>, &'a [u32]);

pub(super) struct ElemIter<'a>(&'a Tree<'a>, std::slice::Iter<'a, u32>);

impl<'a> Iterator for ElemIter<'a> {
    type Item = Nd<'a>;
    fn next(&mut self) -> Option<Nd<'a>> {
        Some(Nd::At(self.0, *self.1.next()?, None))
    }
}

impl<'a> Array<'a, Own> for Elems<'a> {
    type Node = Nd<'a>;
    type ElementsIter = ElemIter<'a>;

    fn len(&self) -> usize {
        self.1.len()
    }

    fn elements(&self) -> ElemIter<'a> {
        ElemIter(self.0, self.1.iter())
    }

    fn is_unique(&self) -> bool {
        let items: Vec<Nd<'a>> = self.elements().collect();
        // As jsonschema does: pairwise for a few, else by hash.
        if items.len() <= 15 {
            return items
                .iter()
                .enumerate()
                .all(|(i, a)| items[i + 1..].iter().all(|b| !equal(a, b)));
        }
        let mut seen: HashMap<u64, Vec<usize>> = HashMap::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            let mut hasher = DefaultHasher::new();
            hash(item, &mut hasher);
            let bucket = seen.entry(hasher.finish()).or_default();
            if bucket.iter().any(|&j| equal(&items[j], item)) {
                return false;
            }
            bucket.push(i);
        }
        true
    }
}

impl<'a> Node<'a, Own> for Nd<'a> {
    type Object = Members<'a>;
    type Array = Elems<'a>;
    type Number = Number;

    fn as_object(&self) -> Option<Members<'a>> {
        match self {
            Nd::At(tree, _, only) => match self.entry()? {
                Entry::Object(from, to, _) => {
                    let all = tree.members_of(*from, *to);
                    let shown = match only {
                        Some(i) => &all[*i as usize..=*i as usize],
                        None => all,
                    };
                    Some(Members(tree, shown))
                }
                _ => None,
            },
            Nd::Text(_) => None,
        }
    }

    fn as_array(&self) -> Option<Elems<'a>> {
        match (self, self.entry()?) {
            (Nd::At(tree, ..), Entry::Array(from, to, _)) => {
                Some(Elems(tree, &tree.elems[*from as usize..*to as usize]))
            }
            _ => None,
        }
    }

    fn as_string(&self) -> Option<Cow<'a, str>> {
        match self {
            Nd::Text(text) => Some(Cow::Borrowed(text)),
            Nd::At(..) => match self.entry()? {
                Entry::String(s) => match self {
                    Nd::At(tree, ..) => Some(Cow::Borrowed(tree.strings[*s as usize].as_ref())),
                    Nd::Text(_) => None,
                },
                _ => None,
            },
        }
    }

    fn as_number(&self) -> Option<Number> {
        match self.entry()? {
            Entry::Pos(u) => Some(Number::from(*u)),
            Entry::Neg(i) => Some(Number::from(*i)),
            Entry::Float(f) => Number::from_f64(*f),
            _ => None,
        }
    }

    fn as_boolean(&self) -> Option<bool> {
        match self.entry()? {
            Entry::Bool(b) => Some(*b),
            _ => None,
        }
    }

    fn is_null(&self) -> bool {
        matches!(self.entry(), Some(Entry::Null))
    }

    fn json_type(&self) -> jsonschema::JsonType {
        use jsonschema::JsonType as T;
        match self.entry() {
            None | Some(Entry::String(_)) => T::String,
            Some(Entry::Null) => T::Null,
            Some(Entry::Bool(_)) => T::Boolean,
            Some(Entry::Pos(_) | Entry::Neg(_) | Entry::Float(_)) => T::Number,
            Some(Entry::Array(..)) => T::Array,
            Some(Entry::Object(..)) => T::Object,
        }
    }

    fn equals_value(&self, expected: &Value) -> bool {
        match (self.entry(), expected) {
            (None, Value::String(expected)) => matches!(self, Nd::Text(text) if text == expected),
            (Some(Entry::Null), Value::Null) => true,
            (Some(Entry::Bool(a)), Value::Bool(b)) => a == b,
            (Some(Entry::Pos(_) | Entry::Neg(_) | Entry::Float(_)), Value::Number(b)) => {
                self.as_number().is_some_and(|a| cmp::equal_numbers(&a, b))
            }
            (Some(Entry::String(_)), Value::String(b)) => {
                self.as_string().is_some_and(|a| a == b.as_str())
            }
            (Some(Entry::Array(..)), Value::Array(items)) => self.as_array().is_some_and(|array| {
                array.len() == items.len()
                    && array
                        .elements()
                        .zip(items)
                        .all(|(node, item)| node.equals_value(item))
            }),
            (Some(Entry::Object(..)), Value::Object(members)) => {
                self.as_object().is_some_and(|object| {
                    object.len() == members.len()
                        && members.iter().all(|(key, item)| {
                            object.get(key).is_some_and(|node| node.equals_value(item))
                        })
                })
            }
            _ => false,
        }
    }

    fn lazy_value(&self) -> LazyInstance<'a> {
        // A container is read again from the text if an error asks for it, so errors that
        // never show their value cost no copy of it.
        match self {
            Nd::At(tree, index, None) => tree.span(*index).map_or_else(
                || LazyInstance::Ready(self.to_value()),
                |bytes| LazyInstance::Deferred {
                    bytes,
                    tag: tree.dialect,
                    make: read_again,
                    cell: OnceLock::new(),
                },
            ),
            _ => LazyInstance::Ready(self.to_value()),
        }
    }

    fn to_value(&self) -> Cow<'a, Value> {
        Cow::Owned(match self {
            Nd::Text(text) => Value::String((*text).to_owned()),
            Nd::At(tree, index, None) => tree.value_at(*index),
            Nd::At(..) => Value::Object(
                self.as_object()
                    .into_iter()
                    .flat_map(|object| object.members().collect::<Vec<_>>())
                    .map(|(key, child)| (key.to_owned(), child.to_value().into_owned()))
                    .collect::<Map<_, _>>(),
            ),
        })
    }

    fn identity(&self) -> Option<NodeIdentity> {
        match self {
            Nd::At(tree, index, only) => Some(NodeIdentity::tagged(
                std::ptr::from_ref::<Tree<'_>>(tree) as usize,
                // A member's view of an object is another node than the object.
                index
                    .wrapping_mul(2)
                    .wrapping_add(u32::from(only.is_some())),
            )),
            // The text's own address, which no tree's is: a `$ref` loop through a name is found.
            Nd::Text(text) => Some(NodeIdentity::tagged(text.as_ptr() as usize, u32::MAX)),
        }
    }
}

/// Two values are equal as JSON Schema says: numbers by value, objects without their order.
fn equal(a: &Nd<'_>, b: &Nd<'_>) -> bool {
    match (a.json_type(), b.json_type()) {
        (T::Number, T::Number) => match (a.as_number(), b.as_number()) {
            (Some(a), Some(b)) => cmp::equal_numbers(&a, &b),
            _ => false,
        },
        (T::String, T::String) => a.as_string() == b.as_string(),
        (T::Boolean, T::Boolean) => a.as_boolean() == b.as_boolean(),
        (T::Null, T::Null) => true,
        (T::Array, T::Array) => match (a.as_array(), b.as_array()) {
            (Some(a), Some(b)) => {
                a.len() == b.len() && a.elements().zip(b.elements()).all(|(a, b)| equal(&a, &b))
            }
            _ => false,
        },
        (T::Object, T::Object) => match (a.as_object(), b.as_object()) {
            (Some(a), Some(b)) => {
                a.len() == b.len()
                    && a.members()
                        .all(|(key, a)| b.get(&key.to_owned()).is_some_and(|b| equal(&a, &b)))
            }
            _ => false,
        },
        _ => false,
    }
}

/// A hash that equal values share: `0` and `-0.0` alike, and integers as the doubles they equal.
fn hash(node: &Nd<'_>, state: &mut DefaultHasher) {
    match node.json_type() {
        T::Null => state.write_u8(0),
        T::Boolean => node.as_boolean().hash(state),
        T::Number => {
            if let Some(number) = node.as_number() {
                match number.as_f64() {
                    Some(0.0) => 0_u64.hash(state),
                    Some(f) => f.to_bits().hash(state),
                    None => number.as_u64().hash(state),
                }
            }
        }
        T::String => node.as_string().hash(state),
        T::Array => {
            for item in node.as_array().into_iter().flat_map(|a| a.elements()) {
                hash(&item, state);
            }
        }
        _ => {
            let mut all = 0_u64;
            for (key, value) in node.as_object().into_iter().flat_map(|o| o.members()) {
                let mut member = DefaultHasher::new();
                key.hash(&mut member);
                hash(&value, &mut member);
                all ^= member.finish();
            }
            state.write_u64(all);
        }
    }
}

/// The value of a container's text, read as `LazyInstance` asks: with the dialect at index `tag`.
fn read_again(bytes: &[u8], tag: u32) -> Value {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Value::Null;
    };
    let dialect = match Dialect::ALL.get(tag as usize) {
        Some(Dialect::JsonLines) | None => Dialect::Json,
        Some(dialect) => *dialect,
    };
    let config = Config::new(dialect).max_depth(super::SCHEMA_MAX_DEPTH);
    let (_, events) = jsonvet::parse(text, &config);
    let events = events.unwrap_or_default();
    documents(&events)
        .next()
        .map_or(Value::Null, |document| document.tree.value())
}

#[cfg(test)]
mod tests {
    use super::super::fold::documents;
    use super::*;
    use jsonvet::{Config, Dialect};
    use serde_json::json;

    #[test]
    fn the_end_of_a_container_at_the_most_does_not_saturate() {
        assert_eq!(offset(MAX_OFFSET + 1) as usize, MAX_OFFSET + 1);
    }

    #[test]
    fn duplicate_keys_count_once_and_one_member_can_be_shown_alone() {
        let text = r#"{"b": 1, "a": [true], "b": "last"}"#;
        let (_, events) = jsonvet::parse(text, &Config::new(Dialect::Json));
        let events = events.unwrap_or_default();
        let document = documents(&events).next().expect("a document");
        let root = document.tree.root();
        let members = root.as_object().expect("an object");
        assert_eq!(members.len(), 2);
        assert_eq!(root.keys(), ["a", "b"]);
        assert_eq!(root.only(1).to_value().into_owned(), json!({"b": "last"}));
        assert_eq!(root.only(0).as_object().map(|o| o.len()), Some(1));
        let schema = json!({"propertyNames": {"maxLength": 1}, "maxProperties": 2});
        let validator = jsonschema::options_for::<Own>()
            .build(&schema)
            .expect("a schema");
        assert!(validator.is_valid(root));
    }
}

#[cfg(test)]
#[allow(
    clippy::unreadable_literal,
    clippy::needless_raw_string_hashes,
    clippy::cast_possible_truncation,
    clippy::unicode_not_nfc,
    reason = "test inputs are written as they are in files"
)]
mod differential {
    use super::super::fold::Fold;
    use super::*;
    use jsonschema::json::SerdeJson;
    use jsonvet::{Config, Dialect};
    use serde_json::json;
    use std::path::Path;

    fn errs_serde(v: &jsonschema::Validator<SerdeJson>, x: &Value) -> Vec<String> {
        v.iter_errors(x)
            .map(|e| {
                format!(
                    "{} | {} | {} | {}",
                    e.instance_path().as_str(),
                    e.schema_path().as_str(),
                    e,
                    e.instance()
                )
            })
            .collect()
    }
    fn errs_own(v: &jsonschema::Validator<Own>, x: Nd<'_>) -> Vec<String> {
        v.iter_errors(x)
            .map(|e| {
                format!(
                    "{} | {} | {} | {}",
                    e.instance_path().as_str(),
                    e.schema_path().as_str(),
                    e,
                    e.instance()
                )
            })
            .collect()
    }

    /// Returns number of mismatches.
    fn compare(schema: &Value, text: &str, dialect: Dialect, label: &str) -> usize {
        let Ok(s) = jsonschema::options_for::<SerdeJson>()
            .offline()
            .build(schema)
        else {
            return 0;
        };
        let o = jsonschema::options_for::<Own>()
            .offline()
            .build(schema)
            .expect("own builds when serde does");
        let (_, events) = jsonvet::parse(text, &Config::new(dialect));
        let events = events.unwrap_or_default();
        // Trees that know their text, so error values are read again from it as in `--schema`.
        let mut fold = Fold::new(text, dialect);
        let docs: Vec<_> = events
            .iter()
            .filter_map(|event| fold.event(event))
            .collect();
        let mut bad = 0;
        for doc in docs {
            let value = doc.tree.value();
            let root = doc.tree.root();
            if s.is_valid(&value) != o.is_valid(root)
                || errs_serde(&s, &value) != errs_own(&o, root)
            {
                eprintln!(
                    "MISMATCH {label}\n serde: {:?}\n own:   {:?}",
                    errs_serde(&s, &value),
                    errs_own(&o, root)
                );
                bad += 1;
            }
            if let Value::Object(m) = &value {
                for (i, (k, v)) in m.iter().enumerate() {
                    let one = Value::Object(Map::from_iter([(k.clone(), v.clone())]));
                    let view = root.only(i as u32);
                    if s.is_valid(&one) != o.is_valid(view)
                        || errs_serde(&s, &one) != errs_own(&o, view)
                    {
                        eprintln!("MISMATCH one-member {label} key {k}");
                        bad += 1;
                    }
                }
            }
        }
        bad
    }

    #[test]
    fn schemastore_corpus_matches_serde() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/schemastore");
        let (mut n, mut bad) = (0, 0);
        for dir in std::fs::read_dir(&root)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
        {
            let schema: Value =
                serde_json::from_str(&std::fs::read_to_string(dir.join("schema.json")).unwrap())
                    .unwrap();
            for part in ["valid", "invalid", "extra"] {
                let Ok(rd) = std::fs::read_dir(dir.join(part)) else {
                    continue;
                };
                for f in rd.flatten() {
                    let text = std::fs::read_to_string(f.path()).unwrap();
                    n += 1;
                    bad += compare(
                        &schema,
                        &text,
                        Dialect::Json5,
                        &f.path().display().to_string(),
                    );
                }
            }
        }
        eprintln!("compared {n} files, {bad} mismatches");
        assert_eq!(bad, 0);
    }

    #[test]
    fn edge_cases_match_serde() {
        let schemas = [
            json!({"enum": [1, 1.0, -0.0, {"a": 1}, [1], "\u{e9}"]}),
            json!({"items": {"enum": [0, 1, 1e2, 18446744073709551615u64]}}),
            json!({"uniqueItems": true}),
            json!({"items": {"multipleOf": 0.1}}),
            json!({"items": {"multipleOf": 3}}),
            json!({"const": {"a": 1, "b": [1.0, {}]}}),
            json!({"type": "object", "properties": {"a": {"type": "integer"}, "b": {"maxLength": 1}}, "required": ["c", "d", "e"], "additionalProperties": false}),
            json!({"propertyNames": {"maxLength": 1}, "minProperties": 2, "maxProperties": 1}),
            json!({"propertyNames": {"$ref": "#/$defs/l"}, "$defs": {"l": {"$ref": "#/$defs/l"}}}),
            json!({"$ref": "#/$defs/n", "$defs": {"n": {"anyOf": [{"type": "integer"}, {"type": "array", "items": {"$ref": "#/$defs/n"}}, {"type": "object", "additionalProperties": {"$ref": "#/$defs/n"}}]}}}),
            json!({"unevaluatedProperties": false, "properties": {"a": true}}),
            json!({"items": {"type": "integer"}}),
            json!({"maxLength": 2, "pattern": "^a"}),
            json!({"oneOf": [{"type": "object"}, {"required": ["a"]}]}),
            json!({"contains": {"const": 1}, "minContains": 2}),
            // A `$ref` loop under `propertyNames` ends in jsonschema's cycle guard.
            json!({"propertyNames": {"$ref": "#/$defs/l"}, "$defs": {"l": {"allOf": [{"$ref": "#/$defs/l"}], "minLength": 0}}}),
            // Several errors at the root, each quoting the whole document or not.
            json!({"allOf": (0..10).map(|n| json!({"maxItems": n})).collect::<Vec<_>>()}),
            // `enum` on containers.
            json!({"items": {"enum": [0, 1, 2, [], {}, {"a": 1}, [1.0], {"b": [1, {"c": null}]}]}}),
            json!({"enum": [[{"a": 1}, {"a": 2}], 5]}),
            json!({"const": [{"a": 1.0}, {"a": 2}]}),
            json!({"uniqueItems": true, "items": {"type": ["object", "array", "integer"]}}),
            json!({"dependentRequired": {"a": ["b"]}, "patternProperties": {"^x": {"type": "string"}}}),
        ];
        let docs = [
            r#"{"b": 1, "a": [true], "b": "last"}"#,
            r#"{"a": 1, "a": 2, "x1": 3, "": 0, "é": 1, "é": 2}"#,
            r#"[1, 1.0, -0, 0, -0.0, 1e2, 100, 1E+2, 18446744073709551615, 18446744073709551616, -9223372036854775809, 0.30000000000000004, 1e308]"#,
            r#"[{}, [], "", {"a":1,"b":2}, {"b":2,"a":1}, [1.0], [1]]"#,
            r#""\ud800 é \n \"""#,
            r#"{"\ud800": 1, "\udc00": 2}"#,
            r#"{}"#,
            r#"[]"#,
            r#""ab""#,
            r#"0.3"#,
            r#"[0.3, 0.7, 9, 9.0, 1e400]"#,
            r#"{"x": 1, "a": 1, "y": {"z": [1, {"q": "r"}]}}"#,
            r#"[{"a": 1}, {"a": 1.0}, {"a": 2}, [1], [1.0], {"b": [1, {"c": null}]}, {"b": [1, {"c": null}]}]"#,
            r#"[{"a": 1}, {"a": 2}, [], {}, 1, 2, 3, "x", "y", 1.5, 2.5, true, false, null, [0], [[0]], {"k": {}}]"#,
            r#"[[{"a": 1}, {"a": 2}], 5]"#,
        ];
        let json5 = [
            r#"[0x10, +1, .5, 5., Infinity, -Infinity, NaN, 0x7FFFFFFFFFFFFFFFFF]"#,
            r#"{a: 1, 'b': 2, a: 3}"#,
        ];
        let mut bad = 0;
        for s in &schemas {
            for d in &docs {
                bad += compare(s, d, Dialect::Json, &format!("{s} on {d}"));
            }
            for d in &json5 {
                bad += compare(s, d, Dialect::Json5, &format!("{s} on {d}"));
            }
        }
        assert_eq!(bad, 0);
    }

    /// Documents in every dialect, with the places the re-read of a container could go wrong: a
    /// bracket in a comment, trailing commas, JSON5 spellings, and records that do not start a line.
    #[test]
    fn dialect_cases_match_serde() {
        let schemas = [
            json!({"uniqueItems": true}),
            json!({"maxItems": 1, "maxProperties": 1}),
            json!({"type": "string"}),
            json!({"properties": {"a": {"type": "string"}}, "additionalProperties": {"type": "string"}}),
            json!({"items": {"enum": [1, [1], {"a": 2}, "x"]}}),
        ];
        let many = |pair: &str| format!("[{pair}, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]");
        let mut cases: Vec<(Dialect, String)> = vec![
            (Dialect::Jsonc, "{\"a\": [1, 2 /* ] } */], // } ]\n \"b\": {\"c\": 1 /* } */}}".into()),
            (Dialect::Jsonc, "[ /* [ */ {\"a\": 1}, // ]\n {\"a\": \"]\"} ]".into()),
            (Dialect::Jwcc, "[1, {\"a\": 2,}, [3,],]".into()),
            (Dialect::Jwcc, "{\"a\": [1, 1,], \"b\": {},}".into()),
            (Dialect::Json5, "{unquoted: 0x1F, 'single': +1, half: .5, \"s\": \"\\ud800\", /* } */ list: [1, 2,], // ]\n}".into()),
            (Dialect::Json5, "[0x10, +1, .5, 5., 'it\\'s', \"\\ud800\", {a: 1, a: 2, 'a': 3}, /* ] */ 0x10]".into()),
            (Dialect::JsonLines, "{\"a\": 1, \"a\": 2}\n  {\"k\": [1, 1]}\n\t[1, 1.0]\n{\"a\": \"x\", \"a\": 1}\n".into()),
            (Dialect::JsonLines, "  [1]  \n   {\"a\": [1, 2], \"a\": [1]}\n".into()),
        ];
        // Numbers that are equal as numbers, with and without the hash.
        for pair in [
            "1, 1.0",
            "0, -0.0",
            "9007199254740993, 9007199254740992",
            "18446744073709551615, 18446744073709551616",
            "-9223372036854775808, -9223372036854775808.0",
            "1e2, 100",
            "{\"a\": 1}, {\"a\": 1.0}",
        ] {
            cases.push((Dialect::Json, format!("[{pair}]")));
            cases.push((Dialect::Json, many(pair)));
        }
        let mut bad = 0;
        for schema in &schemas {
            for (dialect, text) in &cases {
                bad += compare(schema, text, *dialect, &format!("{schema} on {text}"));
            }
        }
        assert_eq!(bad, 0);
    }
}
