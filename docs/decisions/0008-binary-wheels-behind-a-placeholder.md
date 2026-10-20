# 8. A Rust binary in PyPI wheels, installed through a `setup.py` placeholder

Status: accepted (2026-10-13)

## Context

check-json5 1.4 was a `language: python` pre-commit hook, pinned by `repo:` URL and hook id. The
rewrite has to install without a Rust toolchain and keep those URLs. Python also cost startup on
every run: on one pinned core (2026-10-13), a run took 13.3 ms in Python, 5.5 ms of it the
interpreter, against 0.7 ms for a Rust binary that only reads the file.

## Options

- **`language: rust`.** pre-commit compiles the crate on first install, so every user needs Rust.
- **A mirror repository with prebuilt binaries.** Changes every user's `repo:` URL.
- **maturin wheels with `bindings = "bin"`, pinned by a root `setup.py`**, as typos ships.
  pre-commit runs `pip install .`, which installs the wheel and puts the binary on `PATH`.

## Decision

Ship the binary in maturin wheels for eight targets (Linux glibc and musl, macOS, Windows; x86-64
and ARM64) behind the placeholder. Keep `check-json5` on PyPI as a shim with no code, along with
the `check-json5` hook id, command and `--strict`, through 3.x; 4.0 drops them together.

## Consequences

- A hook install downloads a binary and compiles nothing. CI installs the hook with pre-commit
  4.7.0 and prek 0.5.5 on Linux, macOS and Windows; that job once caught a placeholder that could
  not build.
- A release sets the version by hand in `setup.py` and `compat/check-json5/pyproject.toml`;
  `scripts/check-versions.py` checks both against the tag.
- The release archives hold the binary from each wheel, so PyPI and GitHub ship the same bytes.
- The shim and its tests cost about 120 lines until 4.0.

## Confirmation

The `hook` CI job installs the hook with pre-commit and prek on Linux, macOS and Windows, and the
release workflow's `versions` job runs `scripts/check-versions.py` against the tag.

## Revisit if

- pre-commit or prek can install a prebuilt binary without Python packaging;
- a hook install fails on a platform the wheels cover.
