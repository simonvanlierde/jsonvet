# 2. No unsafe code

Status: accepted (2026-10-19)

## Context

jsonvet parses files it did not write, in hooks and in CI on pull requests from strangers.
[SECURITY.md](../../SECURITY.md) treats a crafted file that crashes or hangs it as a security
bug. With no `unsafe`, a bad file can at worst cause a panic, not memory corruption, and
`check` is tested never to panic. The workspace sets `unsafe_code = "forbid"` in
[Cargo.toml](../../Cargo.toml).

The usual reasons for `unsafe` in a parser are speed: SIMD loads, unchecked indexing, memory
maps.

## Options

- **Allow `unsafe` in the hot paths.** The speed-ups measured so far do not need it:
  - UTF-8 validation with simdutf8 saved 1.8 ms of a 211 ms run on 82 MB, about 1%
    (2026-10-19, see [0004](0004-measure-before-adopting-a-speed-up.md)).
  - Strings skip plain bytes in 16-byte blocks since #62. The fixed-size block compiles to SSE2
    or NEON compares in safe code. Since Rust 1.87, intrinsics that need only a target feature
    are safe to call.
  - On one pinned core, best of 7 on 83 MB of pretty JSON (2026-10-19), `check` ran at 648 MB/s
    against 2,101 MB/s for serde_json into `IgnoredAny`. Callgrind on 8 MB put most of that gap in
    instructions per token: 237M against 77M on minified input. `unsafe` does not remove that
    work.
- **An opt-in runtime flag.** The unsafe code would still ship in every binary.
- **A cargo feature.** Doubles what fuzzing and mutation runs must cover.
- **Forbid it.**

## Decision

Keep `unsafe_code = "forbid"` in every crate.

## Consequences

- A memory-safety bug in jsonvet's own code cannot happen. Dependencies such as memchr still
  use `unsafe`, and `cargo deny` and the advisory job watch them.
- Memory maps are out (see [0004](0004-measure-before-adopting-a-speed-up.md)), and any SIMD has
  to stay in what the compiler vectorizes or what safe intrinsics allow.
- The gap to serde_json stays open until per-token work shrinks. The open speed work aims
  there, all in safe code.

## Revisit if

- a profile shows a speed-up past 5% end to end that only `unsafe` can give, and we can afford
  fuzzing and Miri for that code on every target.
