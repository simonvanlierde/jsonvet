# 6. `--schema` on the jsonschema crate, offline, in every build

Status: accepted (2026-10-18)

## Context

check-jsonschema, the usual schema hook, reports no line numbers (its issue #359). jsonvet
already knows the offset of every value in six dialects, so a schema error at its line and
column is something it can add. `--schema` started as a trial in #50 (merged 2026-10-17).

## Options

- **Leave schemas to other tools.** No work, and no positioned schema errors anywhere.
- **The `jsonschema` crate with remote `$ref` and a SchemaStore catalog.** Network access in a
  hook or CI step, and a cache to manage. Not decided yet; it waits for a network policy.
- **The `jsonschema` crate, offline** (default features off, so nothing is fetched): the CLI
  folds the parser's events into `serde_json` values and maps each error's JSON Pointer back to
  an offset.

Then where it ships. A cargo feature kept it out of the PyPI wheels at first (670 KB without
it, 4.26 MB with it). But almost every install comes from PyPI: about 900 a week, against 11
crates.io downloads in total (2026-10-18). Without it in the wheels, almost nobody could use it.

## Decision

`--schema` uses `jsonschema` offline: a `$ref` must stay inside the schema file. It is in every
build, the wheels, the hook and the Action included, since 3.0.0 (#54). The cargo feature is
gone.

## Consequences

- The manylinux x86-64 wheel went from 0.67 MB to 4.33 MB, still smaller than prek (6.1 MB),
  typos (7.7 MB) and ruff (10.1 MB). A plain check starts in 1.1 ms in both builds (#54).
- About 70 more crates, whose code runs only with `--schema`. One of them, `num-cmp`, had its
  last release in 2017 and has one owner; jsonschema has no feature to drop it.
- A crafted schema or file can cost far more than its size, so `--schema` has limits. Each one
  comes from a case measured in the reviews of #50 and #52, and
  [design.md](../design.md#--schema) says why each exists:
  - schema depth 128;
  - at most 5,000,000 objects and arrays, with each `$ref` followed. Without it, 24 levels that
    each `$ref` the next twice took 12.9 s and 11.4 GB;
  - 5,000 regex backtracking steps. A ReDoS case went from 2 min 31 s to 0.71 s;
  - 100 errors per document, and files at most 512 levels deep.
- Folded values take 14 to 27 times the file's size.

## Revisit if

- users or a downstream tool ask for schema association by file name; that needs a network
  policy first, then a SchemaStore catalog with a cache;
- jsonschema's `Json` trait settles, so jsonvet can hand it its own events instead of a copy;
- the wheel size draws complaints, or crates.io installs come near PyPI's.
