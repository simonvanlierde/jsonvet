# 8. A Rust binary in PyPI wheels, installed through a `setup.py` placeholder

Status: accepted (2026-10-13)

## Context

check-json5 1.4 was a Python pre-commit hook, pinned by `repo:` URL and hook id. The rewrite has
to install without a Rust toolchain and keep those URLs. Python also cost 13.3 ms a run against
0.7 ms for a Rust binary (one pinned core, 2026-10-13).

## Options

- **`language: rust`.** Every user needs Rust and waits for a compile.
- **A mirror repository with prebuilt binaries.** Changes every user's `repo:` URL.
- **maturin wheels with `bindings = "bin"`, pinned by a root `setup.py`**, as typos does.

## Decision

Wheels for eight targets behind the placeholder. Releases 2.x and 3.x also published an empty
`check-json5` shim and provided its hook ID, command and `--strict`. Version 4.0 retires these
([migration guide](../usage.md#coming-from-check-json5)).

## Consequences

A hook install downloads a binary and compiles nothing. A release sets the version by hand in
`setup.py` and the shim, and the release archives hold the binary from each wheel, so PyPI and
GitHub ship the same bytes. The shim costs about 120 lines until compatibility is removed.

## Confirmation

The `hook` CI job installs with pre-commit and prek on three OSes, and the release's `versions`
job checks the versions against the tag.

## Revisit if

pre-commit or prek can install a prebuilt binary without Python packaging, or a hook install fails
on a platform the wheels cover.
