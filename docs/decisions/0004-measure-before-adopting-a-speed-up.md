# 4. Measure a speed-up first, adopt it only past 5%

Status: accepted (2026-10-19)

## Context

Every speed-up costs something: a dependency, a slower release build, more code to fuzz. Most
ideas for JSON speed come from other projects' numbers. ruff measured PGO at 8 to 14% and uv at
about 9%, but nobody had measured any of them on a JSON linter with exact positions.

## Options

The rule we set: build it, time it with `hyperfine -N` against the same build without it, and
keep it only when it is at least 5% faster end to end, with the same output.

Measured on 2026-10-19 and turned down:

- **simdutf8** for UTF-8 validation: 3.6 ms with the standard library against 1.8 ms on an 82 MB
  file, about 1% of the 211 ms run. Not worth a dependency.
- **PGO** (rustc 1.99, fat LTO both ways, trained on 289 files kept apart from the benchmark
  inputs): within σ of the plain build on every `scripts/bench.py` case. For example, 1,100 files
  took 4.36 against 4.41 ms, and 1.8 MB of JSON5 10.26 against 10.29 ms. Large inputs were
  within σ too: 78 MB of JSON Lines took 294 against 287 ms. PGO doubled ruff's release pipeline,
  and our cross-built ARM64 Linux wheel would need a native runner first.
- **A whitespace run skip** at the top of `trivia`, the loop that skips whitespace and
  comments: about 3% on pretty JSON (648 to about 670 MB/s on one core). A related change
  returns early unless the next byte can start whitespace or a comment. It cut instructions by
  19% on minified input, and its wall-clock timing is still to do.

Argued, not measured:

- **Memory maps.** They need `unsafe` (see [0002](0002-no-unsafe-code.md)), and a file truncated
  while mapped raises SIGBUS. ripgrep measured plain reads faster on many small files, which is
  the hook's workload.
- **Release-profile changes.** The profile already has fat LTO, one codegen unit,
  `panic = "abort"` and strip, the same as uv and prek.

## Decision

Apply the 5% rule to every speed-up, and record a turned-down one with its numbers.

## Consequences

- Kept under the rule: checking files on every core (#62, 1,100 files 14.2 to 4.0 ms) and
  splitting large JSON Lines files across cores (#68, 78 MB 264 to 89 ms).
- Speed claims need a quiet machine and the median, as
  [CONTRIBUTING.md](../../CONTRIBUTING.md#tests) says. A busy machine can hide a 5% change.

## Revisit if

- the parser grows enough branches that a PGO profile could differ, or a new compiler changes
  the PGO result;
- a profile shows UTF-8 validation or file reading above 5% of a run.
