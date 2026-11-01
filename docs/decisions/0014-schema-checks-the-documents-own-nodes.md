# 14. `--schema` checks the document's own nodes, through jsonschema's `Json` trait

Status: accepted (2026-11-01)

## Context

`--schema` folded each document into a `serde_json::Value` and handed that to jsonschema. The copy
held every string and key again, and most of a check's memory was that copy. jsonschema 0.60 can
validate any representation that implements its `jsonschema::json::Json`, `Node`, `Object` and
`Array` traits, with the same keywords, formats, `$ref`s and `unevaluated*` handling. The traits
are new and its author expects them to change.

## Options

- **Keep `serde_json::Value`.** No risk from the traits, and the copy stays.
- **Implement the traits over a tree jsonvet builds.** The tree borrows strings and keys from the
  text and keeps numbers as `serde_json::Number`. The schema itself stays a `Value`, because
  jsonschema compiles from one.
- **Wait for the traits to settle.** A release or two, with no gain meanwhile.

## Decision

The second. `crates/jsonvet-cli/src/schema/nodes.rs` holds the tree and the trait impls. The
validators are built with `jsonschema::options_for::<Own>()`. Objects keep their members sorted by
key with later duplicates replacing earlier ones, so member order, counts and lookups match what
`serde_json::Map` gave. The part of a schema checked on each member of an object sees a view of
the object with one member, instead of a one-member copy. Errors still carry only an instance
path, so offsets still come from the second walk over the events (`place.rs`); the tree has no
byte offsets yet.

## Consequences

An escaped string is still decoded by the parser, so only strings without escapes are borrowed.
`to_value` builds a `Value` for a failing node when jsonschema quotes it in an error, which is a
copy of that node only. The traits are pinned by the exact `jsonschema` version in `Cargo.toml`
and `Cargo.lock`. When a release changes them, the build fails in `nodes.rs` and nowhere else:
the changes to expect are a new required method on `Node`, `Object` or `Array`, a changed
signature of `with_string_node`, or the traits moving out of `jsonschema::json`. If one cannot be
met cheaply, go back to the first option: `Tree::value` already builds the `Value`, so the change is to
validate that instead of the tree, as the code did before this record.

## Confirmation

`cargo nextest run -p jsonvet-cli` runs the unit test in `nodes.rs`, the folding property tests in
`fold.rs`, which compare the tree's values with what `serde_json` reads, the split-schema property
tests in `mod.rs`, the `schemastore` snapshots and the heap bounds in `tests/schema_memory.rs`.

## Revisit if

A jsonschema release changes the traits in a way that costs more than a day, or its error type
starts to carry the failing node, which would let offsets come from the tree and retire the second
walk.
