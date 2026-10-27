# 4. Measure a speed-up first, and weigh it against what it costs

Status: accepted (2026-10-19), amended 2026-10-27: a speed-up with no cost needs no 5% bar

## Context

Every speed-up costs something: a dependency, a slower release build, more code to fuzz. Most
ideas come with other projects' numbers (ruff: PGO 8 to 14%), not ours.

## Decision

Build it, time it with `hyperfine -N` against the same build without it, and check the output is
the same. A speed-up that is only code in the crates we already have is kept when it measures
faster on the case it targets and no slower on the others. One that adds a cost (a dependency,
`unsafe`, a slower build, a platform-specific path) must pay for it: at least 5% faster end to
end. Record what we turn down here.

## Turned down

- **simdutf8** (2026-10-19): 1.8 against 3.6 ms on 82 MB, about 1% of the run.
- **PGO** (2026-10-19, fat LTO both ways): within σ on every `scripts/bench.py` case and on 78 MB
  of JSON Lines, for twice the release build time.
- **A whitespace-run skip in `trivia`** (2026-10-19): about 3%. An early return measured better
  instead.
- **Memory maps**, argued: they need `unsafe` ([0002](0002-no-unsafe-code.md)) and raise SIGBUS
  on a file truncated while mapped.

## Consequences

Kept under the rule: checking files on every core (#62, 1,100 files 14.2 to 4.0 ms) and splitting
large JSON Lines files (#68, 78 MB 264 to 89 ms), and a byte table for strings (#116: 9 MB of
short keys 27.5 to 25.3 ms, files full of escapes or JSON5 tabs 10 to 25% faster, up to 6% on
`scripts/bench.py`).
A speed-up past 5% on one kind of file counts, if it is no slower on the others. A 5% call needs
a quiet machine and the median. The 5% bar first applied to every speed-up; since 2026-10-27 it
applies only to one that adds a cost, as a small gain in plain code is still worth having.

## Confirmation

Speed-up PRs show `hyperfine -N` numbers; the `bench` job fails on +1% instructions, and
`scaling` on a path that turns quadratic.

## Revisit if

The parser or compiler changes enough that PGO could differ, or UTF-8 validation or file reading
passes 5% of a run.
