//! Each schema error's value and keys as targets for [`super::nodes::Tree::place`], and the
//! segments of its pointer.

use jsonschema::ValidationError;

use super::Found;
use super::nodes::Tree;

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

/// The targets for the errors in `found`: for an error about a key, also its member.
pub(super) fn found_targets(found: &[Found]) -> Vec<(Vec<String>, Vec<String>)> {
    let mut targets = Vec::with_capacity(found.len() * 2);
    for found in found {
        targets.push((found.path.clone(), found.keys.clone()));
        if found.key {
            let mut member = found.path.clone();
            member.extend(found.keys.iter().cloned());
            targets.push((member, Vec::new()));
        }
    }
    targets
}

/// Where each error in `found` is, from the `spots` of its [`found_targets`], and that value's
/// path; for an error about a key, the path of the key's member.
pub(super) fn found_spots(found: &[Found], spots: Vec<(usize, String)>) -> Vec<(usize, String)> {
    let mut spots = spots.into_iter();
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

/// Where each error in `found` is in `tree`, and that value's path; for an error about a key, the
/// path of the key's member.
pub(super) fn place_found(tree: &Tree<'_>, found: &[Found]) -> Vec<(usize, String)> {
    found_spots(found, tree.place(&found_targets(found)))
}

#[cfg(test)]
mod tests {
    use super::super::fold::documents;
    use super::super::found;
    use super::{place_found, segments};
    use jsonvet::{Config, Dialect};
    use serde_json::json;

    #[test]
    fn a_later_duplicate_key_wins_in_the_value_and_its_position() {
        let text = r#"{"a": [1, {"b": 2}], "b\u00e9/~": "x", "a": "last"}"#;
        let (_, events) = jsonvet::parse(text, &Config::new(Dialect::Json));
        let events = events.unwrap_or_default();
        let documents: Vec<_> = documents(&events).collect();
        assert_eq!(
            documents[0].tree.value(),
            json!({"a": "last", "b\u{e9}/~": "x"})
        );
        let schema =
            json!({"properties": {"a": {"type": "integer"}, "b\u{e9}/~": {"type": "integer"}}});
        let validator = jsonschema::validator_for(&schema).expect("a schema");
        let errors: Vec<_> = validator
            .iter_errors(&documents[0].tree.value())
            .flat_map(|e| found(None, documents[0].tree.root(), &e, String::new))
            .collect();
        let paths: Vec<_> = errors.iter().map(|e| e.path.join("/")).collect();
        assert_eq!(paths, ["a", "b\u{e9}/~"]);
        let located: Vec<_> = place_found(&documents[0].tree, &errors)
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
        let document = documents(&events).next().expect("a document");
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
            assert_eq!(document.tree.place(&[target])[0].1, path, "{pointer}");
        }
    }
}
