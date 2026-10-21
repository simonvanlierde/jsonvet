# 1. One hand-written parser for six dialects, under a contract with check-json5 1.4

Status: accepted (2026-10-13)

## Context

jsonvet 2.0 replaced check-json5 1.4 in place. A linter needs what a deserializer skips: an exact
position for every problem, and every duplicate key with both positions. 1.4's users also need
the hook to pass and fail the same files.

## Options

- **Build on serde_json, json5 or jsonc-parser.** On 2026-10-13 every one turned `{a:1,a:2}` into
  `{"a": 2}`, so none could report a duplicate. jsonc-parser is lax by design, and json5 1.3.1
  went quadratic on hex numbers.
- **One parser per dialect.** Six grammars to keep in step.
- **One byte scanner for JSON, I-JSON, JSONC, JWCC, JSON5 and JSON Lines**, with the crates as
  fuzz oracles.

## Decision

One scanner, built for linting. Verdicts, exit codes and duplicate-key reports stay equal to 1.4;
syntax-error wording is ours, marked **Breaking**.

## Consequences

- We own every edge case, so the parser has a corpus, json5-tests (114 of 114), proptest and fuzz
  targets.
- All 40,228 differential runs agreed with 1.4.1 at 2.0 (#34); the 385 intended differences since
  are listed in [design.md](../design.md#known-differences-from-14).

## Confirmation

The `differential` CI job on every pull request, and the weekly fuzz run against serde_json and
json5.

## Revisit if

A maintained crate offers strict dialect modes with offsets and duplicate reports and passes our
corpus, or 4.0 drops check-json5 compatibility.
