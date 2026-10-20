# 1. One hand-written parser for six dialects, under a contract with check-json5 1.4

Status: accepted (2026-10-13)

## Context

jsonvet 2.0 replaced check-json5 1.4, a Python hook, in place. A linter needs what a deserializer
skips: an exact position for every problem, and every duplicate key with both positions and its
JSONPath. And 1.4's users need the hook to pass and fail the same files.

## Options

- **Build on serde_json, json5 or jsonc-parser.** In a harness on 2026-10-13 (rustc 1.99, 46 to
  476 KiB config files), every crate turned `{a:1,a:2}` into `{"a": 2}`, so none could report a
  duplicate. jsonc-parser is lax by design and accepted missing commas. json5 1.3.1 went
  quadratic on hex numbers (0.5 MiB/s at 479 KiB). serde_json reads strict JSON only.
- **One parser per dialect.** Six grammars to keep in step, though each dialect extends the last.
- **One byte scanner for all six** (JSON, I-JSON, JSONC, JWCC, JSON5, JSON Lines), with the crates
  kept as fuzz oracles and baselines.

## Decision

One scanner for all six, written for linting: no values, an explicit frame stack, lazy
positions. The contract with 1.4 has two levels: verdicts, exit codes and duplicate-key reports
stay equal; syntax-error wording is ours, marked **Breaking** in the changelog.

## Consequences

- We own every edge case, so the parser carries a corpus, json5-tests (114 of 114), proptest,
  and fuzz targets against serde_json and json5.
- The differential run proves the contract: all 40,228 runs agreed with 1.4.1 at 2.0 (#34). On
  2026-10-15, 385 of 40,368 differed, all intended and listed in
  [design.md](../design.md#known-differences-from-14).
- In the conformance matrix only four of 27 parser runs report duplicate keys, and jsonvet is
  the only Rust one ([conformance.md](../conformance.md#duplicate-keys)).

## Confirmation

The `differential` CI job checks the contract against 1.4.1 on every pull request, and the weekly
fuzz workflow compares the parser with serde_json and json5.

## Revisit if

- a maintained Rust crate offers strict dialect modes with byte offsets and duplicate reports,
  and passes our corpus;
- 4.0 drops check-json5 compatibility, which ends the verdict half of the contract.
