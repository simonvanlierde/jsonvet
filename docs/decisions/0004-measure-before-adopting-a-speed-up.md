# 4. Measure a speed-up first, adopt it only past 5%

Status: accepted (2026-10-19)

## Context

Every speed-up costs something: a dependency, a slower release build, more code to fuzz. Most
ideas come with other projects' numbers (ruff measured PGO at 8 to 14%), and nobody had measured
them on a JSON linter with exact positions.

## Decision

Build it, time it with `hyperfine -N` against the same build without it, and keep it only when
it is at least 5% faster end to end with the same output. Record a rejected one here with its
numbers.

## Turned down

Measured on 2026-10-19:

- **simdutf8:** UTF-8 validation of 82 MB took 1.8 ms against 3.6 ms, about 1% of the 211 ms run.
- **PGO** (rustc 1.99, fat LTO both ways, trained on 289 files apart from the bench inputs):
  within σ on every `scripts/bench.py` case (1,100 files 4.36 against 4.41 ms) and on 78 MB of
  JSON Lines (294 against 287 ms). It would also double the release build.
- **A whitespace-run skip in `trivia`**, the loop that skips whitespace and comments: about 3% on
  pretty JSON.

Argued, not measured:

- **Memory maps:** they need `unsafe` ([0002](0002-no-unsafe-code.md)), a file truncated while
  mapped raises SIGBUS, and ripgrep found plain reads faster on many small files, the hook's case.
- **Release-profile changes:** fat LTO, one codegen unit, `panic = "abort"` and strip are already
  on, as in uv and prek.

## Consequences

- Kept under the rule: checking files on every core (#62, 1,100 files 14.2 to 4.0 ms) and
  splitting large JSON Lines files across cores (#68, 78 MB 264 to 89 ms).
- A 5% change needs a quiet machine and the median, as
  [CONTRIBUTING.md](../../CONTRIBUTING.md#tests) says.

## Confirmation

A speed-up pull request shows its `hyperfine -N` numbers against main. The `bench` CI job counts
the instructions of `check` and fails past +1%, and the `scaling` job fails on a path that turns
quadratic.

## Revisit if

- the parser changes enough that a PGO profile could differ, or a new compiler changes the
  result;
- a profile puts UTF-8 validation or file reading above 5% of a run.
