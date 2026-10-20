# 2. No unsafe code

Status: accepted (2026-10-19)

## Context

jsonvet parses files it did not write, in hooks and in CI on strangers' pull requests.
[SECURITY.md](../../SECURITY.md) treats a crash or hang on a crafted file as a security bug.
Without `unsafe`, the worst a bad file can do is a panic, and `check` is tested never to panic.
The usual reasons for `unsafe` in a parser are speed: SIMD loads, unchecked indexing, memory maps.

## Options

- **Allow it in hot paths.** Nothing measured so far needs it. simdutf8 saved about 1% of a run
  (see [0004](0004-measure-before-adopting-a-speed-up.md)). The 16-byte string skip in #62
  compiles to SSE2 or NEON in safe code. The gap to serde_json is instructions per token, 237M
  against 77M on 8 MB of minified JSON (callgrind, 2026-10-19), which `unsafe` does not remove.
- **An opt-in flag.** The unsafe code still ships in every binary.
- **A cargo feature.** Doubles what fuzzing and mutation testing must cover.
- **Forbid it.**

## Decision

`unsafe_code = "forbid"` in the workspace [Cargo.toml](../../Cargo.toml).

## Consequences

- No memory-safety bug can come from jsonvet's own code. Dependencies such as memchr still use
  `unsafe`; `cargo deny` and the advisory job watch them.
- Memory maps are out, and SIMD stays within what the compiler vectorizes or safe intrinsics
  allow.

## Confirmation

`forbid` makes any `unsafe` block a compile error, so every build and `cargo lint` refuse it.

## Revisit if

- a profile shows a speed-up past 5% end to end that only `unsafe` can give, and we can fuzz it
  and run Miri on it on every target.
