//! A document's values as an arena that borrows its strings from the text, which jsonschema
//! validates through its `Json` trait instead of a `serde_json::Value` copy of the document.

use std::borrow::Cow;
use std::collections::HashMap;

use jsonschema::json::{Array, Json, Node, NodeIdentity, Object};
use jsonvet::{PathStep, json_path};
use serde_json::{Map, Number, Value};

/// jsonschema's name for this representation of a document.
pub(super) struct Own;

/// One value. A container holds the range of its children in [`Tree::elems`] or
/// [`Tree::members`].
enum Entry<'e> {
    Null,
    Bool(bool),
    Number(Number),
    String(Cow<'e, str>),
    Array(u32, u32),
    /// Its members are sorted by key with duplicates gone, as a `serde_json::Map` keeps them.
    Object(u32, u32),
}

/// One document's values. Strings are borrowed from the text unless they hold an escape.
#[derive(Default)]
pub(super) struct Tree<'e> {
    entries: Vec<Entry<'e>>,
    /// The offset of each entry's first byte in the text.
    at: Vec<usize>,
    elems: Vec<u32>,
    members: Vec<Member<'e>>,
    /// The key as written, by the entry of its value, for a key that holds U+FFFD: a lone
    /// surrogate in the text is U+FFFD in the key's text.
    raws: HashMap<u32, Cow<'e, [u8]>>,
    root: u32,
}

/// An object member: its key, its value's entry, and the offset of its key in the text.
type Member<'e> = (Cow<'e, str>, u32, usize);

/// What [`Tree`] is built with while a document's events come in: the children of every open
/// container, until it ends.
#[derive(Default)]
pub(super) struct Builder<'e> {
    pub(super) tree: Tree<'e>,
    pub(super) elems: Vec<u32>,
    pub(super) members: Vec<Member<'e>>,
}

impl<'e> Builder<'e> {
    /// Keep the key of the member whose value is `node` as written.
    pub(super) fn raw(&mut self, node: u32, raw: Cow<'e, [u8]>) {
        self.tree.raws.insert(node, raw);
    }

    pub(super) fn string(&mut self, text: Cow<'e, str>, at: usize) -> u32 {
        self.add(Entry::String(text), at)
    }

    pub(super) fn scalar(&mut self, value: Value, at: usize) -> u32 {
        let entry = match value {
            Value::Bool(b) => Entry::Bool(b),
            Value::Number(n) => Entry::Number(n),
            _ => Entry::Null,
        };
        self.add(entry, at)
    }

    fn add(&mut self, entry: Entry<'e>, at: usize) -> u32 {
        self.tree.entries.push(entry);
        self.tree.at.push(at);
        u32::try_from(self.tree.entries.len() - 1).unwrap_or(u32::MAX)
    }

    /// End the array whose children began at `start` in [`Builder::elems`].
    pub(super) fn end_array(&mut self, start: usize, at: usize) -> u32 {
        let from = u32::try_from(self.tree.elems.len()).unwrap_or(u32::MAX);
        self.tree.elems.extend(self.elems.drain(start..));
        let to = u32::try_from(self.tree.elems.len()).unwrap_or(u32::MAX);
        self.add(Entry::Array(from, to), at)
    }

    /// End the object whose members began at `start` in [`Builder::members`]. A later member of
    /// a key replaces an earlier one.
    pub(super) fn end_object(&mut self, start: usize, at: usize) -> u32 {
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
        self.add(Entry::Object(from, to), at)
    }

    pub(super) fn finish(&mut self, root: u32) -> Tree<'e> {
        self.elems.clear();
        self.members.clear();
        let mut tree = std::mem::take(&mut self.tree);
        tree.root = root;
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
                (false, Entry::Array(from, to)) => segment
                    .parse::<usize>()
                    .ok()
                    .filter(|i| i.to_string() == *segment && (*from as usize + i) < *to as usize)
                    .map(|i| (self.elems[*from as usize + i], PathStep::Index(i))),
                (false, Entry::Object(from, to)) => {
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
            match next {
                Some((child, step)) => {
                    node = child;
                    steps.push(step);
                }
                None => {
                    lost = true;
                    steps.push(PathStep::Key(segment.as_bytes()));
                }
            }
        }
        let offset = match (&self.entries[node as usize], lost) {
            (Entry::Object(from, to), false) => {
                let members = self.members_of(*from, *to);
                keys.iter()
                    .filter_map(|key| {
                        let at = members.binary_search_by(|(k, ..)| k.as_ref().cmp(key.as_str()));
                        at.ok().map(|at| members[at].2)
                    })
                    .min()
            }
            _ => None,
        };
        (offset.unwrap_or(self.at[node as usize]), json_path(steps))
    }

    fn members_of(&self, from: u32, to: u32) -> &[Member<'e>] {
        &self.members[from as usize..to as usize]
    }

    fn value_at(&self, index: u32) -> Value {
        match &self.entries[index as usize] {
            Entry::Null => Value::Null,
            Entry::Bool(b) => Value::Bool(*b),
            Entry::Number(n) => Value::Number(n.clone()),
            Entry::String(s) => Value::String(s.to_string()),
            Entry::Array(from, to) => Value::Array(
                self.elems[*from as usize..*to as usize]
                    .iter()
                    .map(|&child| self.value_at(child))
                    .collect(),
            ),
            Entry::Object(from, to) => Value::Object(
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

    fn entry(&self) -> Option<&'a Entry<'a>> {
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
}

impl<'a> Node<'a, Own> for Nd<'a> {
    type Object = Members<'a>;
    type Array = Elems<'a>;
    type Number = &'a Number;

    fn as_object(&self) -> Option<Members<'a>> {
        match self {
            Nd::At(tree, _, only) => match self.entry()? {
                Entry::Object(from, to) => {
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
            (Nd::At(tree, ..), Entry::Array(from, to)) => {
                Some(Elems(tree, &tree.elems[*from as usize..*to as usize]))
            }
            _ => None,
        }
    }

    fn as_string(&self) -> Option<Cow<'a, str>> {
        match self {
            Nd::Text(text) => Some(Cow::Borrowed(text)),
            Nd::At(..) => match self.entry()? {
                Entry::String(s) => Some(Cow::Borrowed(s.as_ref())),
                _ => None,
            },
        }
    }

    fn as_number(&self) -> Option<&'a Number> {
        match self.entry()? {
            Entry::Number(n) => Some(n),
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
            Some(Entry::Number(_)) => T::Number,
            Some(Entry::Array(..)) => T::Array,
            Some(Entry::Object(..)) => T::Object,
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
            Nd::Text(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fold::documents;
    use super::*;
    use jsonvet::{Config, Dialect};
    use serde_json::json;

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
