# 2. No unsafe code

Status: accepted (2026-10-19)

## Context

jsonvet parses strangers' files in hooks and CI, and [SECURITY.md](../../SECURITY.md) treats a
crash on a crafted file as a security bug. Without `unsafe`, the worst a bad file can do is a
panic. Parsers usually reach for `unsafe` for speed: SIMD, unchecked indexing, memory maps.

## Options

- **Allow it in hot paths.** Nothing we measured needs it: simdutf8 saved about 1%
  ([0004](0004-measure-before-adopting-a-speed-up.md)), #62's 16-byte string skip compiles to SIMD
  in safe code, and the gap to serde_json is instructions per token (237M against 77M on 8 MB,
  2026-10-19), which `unsafe` doesn't remove.
- **An opt-in flag or a cargo feature.** The code still ships, or fuzzing has twice as much to
  cover.
- **Forbid it.**

## Decision

`unsafe_code = "forbid"` in the workspace [Cargo.toml](../../Cargo.toml).

## Consequences

No memory-safety bug can come from our own code; `cargo deny` watches dependencies like memchr
that do use `unsafe`. Memory maps are out.

## Confirmation

`forbid` makes any `unsafe` block a compile error.

## Revisit if

Only `unsafe` can give a speed-up past 5% end to end, and we can fuzz it and run Miri on it on
every target.
