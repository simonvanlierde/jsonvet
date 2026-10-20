# 6. `--schema` on the jsonschema crate, offline, in every build

Status: accepted (2026-10-18)

## Context

check-jsonschema, the usual schema hook, reports no line numbers (its issue #359). jsonvet knows
every value's offset in six dialects, so it can report a schema error at its line and column.
`--schema` started as a trial in #50.

## Options

- **Leave schemas to other tools.** No positioned schema errors anywhere.
- **jsonschema with remote `$ref` and a SchemaStore catalog.** Network access in hooks and CI,
  and a cache to manage. Waits for a network policy.
- **jsonschema offline** (default features off): the CLI builds `serde_json` values from the
  parser's events and maps each error's JSON Pointer back to an offset.

Then where it ships. A cargo feature first kept it out of the wheels (670 KB against 4.26 MB),
but nearly every install comes from PyPI: about 900 a week, against 11 crates.io downloads in
total (2026-10-18).

## Decision

`--schema` uses jsonschema offline, so every `$ref` stays inside the schema file. It is in every
build since 3.0.0, the wheels, the hook and the Action included (#54).

## Consequences

- The manylinux x86-64 wheel grew from 0.67 to 4.33 MB, still under prek (6.1 MB) and ruff
  (10.1 MB). A plain check starts in 1.1 ms either way (#54).
- About 70 more crates, run only with `--schema`. One, `num-cmp`, last released in 2017 and has
  one owner, and jsonschema cannot drop it.
- A crafted schema can cost far more than its size, so `--schema` has limits, each from a case
  measured in #50 and #52 and explained in [design.md](../design.md#--schema). Building values
  takes 14 to 27 times the file's size.

## Confirmation

The tests in `cli.rs`'s `mod schema` hold each limit, such as `a_long_ref_chain_is_a_usage_error`
and `a_backtracking_pattern_is_cut_short_and_reported`. The `packaging` CI job builds the wheels
the release ships.

## Revisit if

- users ask for schemas picked by file name: that needs a network policy, then a cached catalog;
- jsonschema's `Json` trait settles, so it can read jsonvet's events instead of a copy;
- the wheel size draws complaints, or crates.io installs come near PyPI's.
