# 1. One hand-written parser for six dialects, under a contract with check-json5 1.4

Status: accepted (2026-10-13)

## Context

jsonvet 2.0 replaced check-json5 1.4, a Python hook, in place. A linter needs things a
deserializer does not: exact positions for every problem, every duplicate key with both
positions and its JSONPath, and strict verdicts in each dialect. Users of 1.4 also need the
hook to keep passing and failing the same files.

## Options

- **Build on serde_json, json5 or jsonc-parser.** We surveyed the Rust crates and ran them in a
  harness on 2026-10-13 (rustc 1.99, 46 to 476 KiB config-shaped files). jsonc-parser ran at 125
  to 155 MiB/s and json5 1.3.1 at 113 to 135 MiB/s into a value. Every crate turned
  `{a:1,a:2}` into `{"a": 2}`, so none could report a duplicate. On 89 edge-case probes json5
  disagreed with the spec once and jsonc-parser 10 or 11 times. jsonc-parser is designed as a
  lax superset: it accepted missing commas by default. json5 1.3.1 also went quadratic on hex
  numbers, down to 0.5 MiB/s at 479 KiB. serde_json reads strict JSON only. Each choice meant adding
  strictness or positions to someone else's design.
- **One parser per dialect.** Six grammars to keep in step, though JSON, JSONC, JWCC and JSON5
  each add syntax to the one before.
- **One hand-written byte scanner for all six** (JSON, I-JSON, JSONC, JWCC, JSON5, JSON Lines),
  with the crates kept as fuzz oracles and benchmark baselines.

For the contract with 1.4, the choice was between matching its output byte for byte and
matching what scripts and hooks depend on.

## Decision

One parser for all six dialects, written for linting: no values, an explicit frame stack,
lazy positions. The contract with 1.4 has two levels. Verdicts, exit codes and duplicate-key
reports stay equal. Syntax-error wording is ours, and the changelog marks it **Breaking**.

## Consequences

- One grammar change covers every dialect. The cost is that we own every edge case, so the
  parser carries a corpus, json5-tests (114 of 114), proptest, and fuzz targets against
  serde_json and json5.
- The differential run is the proof of the contract. At the 2.0 release all 40,228 runs agreed
  with 1.4.1 (#34). In the last full run, on 2026-10-15, 385 of 40,368 runs differed, on 383
  files, all listed as intended in [design.md](../design.md#known-differences-from-14).
- The conformance matrix (2026-10-19) shows the gap the survey found: of 27 parser runs, only
  four can report duplicate keys. jsonvet is the only Rust one, and the only one that does so in
  JSON Lines ([conformance.md](../conformance.md#duplicate-keys)).

## Revisit if

- a maintained Rust crate offers strict modes for these dialects with byte offsets and
  duplicate-key reports, and matches our corpus;
- 4.0 drops the check-json5 compatibility, which ends the verdict half of the contract.
