# 8. A Rust binary in PyPI wheels, installed through a `setup.py` placeholder

Status: accepted (2026-10-13)

## Context

check-json5 1.4 was a `language: python` pre-commit hook, and its users pin it by `repo:` URL
and hook id. A Rust rewrite has to install without a Rust toolchain and keep those URLs. The
Python version also paid for its own startup on every run: measured on one pinned core on
2026-10-13, a whole run took 13.3 ms in Python (5.5 ms of it the interpreter), against 0.7 ms
for a Rust binary that only reads the file.

## Options

- **`language: rust`.** pre-commit builds the crate with cargo on first install, so every user
  needs Rust and waits for a compile.
- **A separate mirror repository with prebuilt binaries.** Changes every user's `repo:` URL.
- **maturin wheels with `bindings = "bin"`, and a root `setup.py` that pins them**, the way
  typos ships. pre-commit runs `pip install .` on the repository, which installs the pinned wheel
  and puts the binary on `PATH`.

For the old name: drop `check-json5` from PyPI, or keep it as a package that only installs
jsonvet.

## Decision

Ship the binary in maturin wheels for eight targets (Linux glibc and musl, macOS and Windows,
on x86-64 and ARM64), behind the root placeholder. Keep `check-json5` on PyPI as a shim with no
code of its own, together with the `check-json5` hook id, command and `--strict`, through 3.x.
4.0 drops them in one change.

## Consequences

- A hook install downloads a prebuilt binary and compiles nothing. The placeholder trial passed
  under pre-commit 4.7.0 and prek 0.5.5 on 2026-10-13, and CI installs the hook with both on
  Linux, macOS and Windows. That job once caught a placeholder that could not build.
- A release sets the version by hand in `setup.py` and `compat/check-json5/pyproject.toml`, and
  `scripts/check-versions.py` checks both against the tag.
- The GitHub release archives hold the binary from each wheel, so PyPI and the release ship the
  same bytes.
- Until 4.0 the shim, the second binary and their tests stay, about 120 lines.

## Revisit if

- pre-commit or prek gain a way to install a prebuilt binary without Python packaging;
- the hook install fails on a platform the wheels cover;
- 4.0 comes: then remove the shim.
