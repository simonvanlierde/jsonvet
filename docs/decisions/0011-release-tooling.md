# 11. cargo-release bumps the version, the tag starts the rest

Status: accepted (2026-10-21)

## Context

A release changes the version in six places, dates the CHANGELOG section and adds its compare
link, all by hand. Main takes changes only through squash-merged pull requests, and release.yml
publishes the crates and wheels by trusted publishing once a `v*` tag lands.

## Options

- **release-please or release-plz**, argued: both write the CHANGELOG from commit subjects, and
  ours is written for users, with upgrade steps first. A tag they push with the default
  `GITHUB_TOKEN` starts no other workflow, so they need a PAT or a GitHub App with publish
  rights. release-plz would also publish the crates itself, next to release.yml's trusted
  publishing.
- **cargo-dist**, argued: it would have caught the 3.1.0 binstall bug, but it builds its own
  binaries, while our archives ship the same bytes as the PyPI wheels.
- **Let CI move the `v3` major tag**: the workflow token got HTTP 403 moving it past changes to
  `.github/workflows/` on 3.1.0, and a token that could would give CI write access to the
  workflows. The `major-tag` job was removed in #89.
- **cargo-release on the maintainer's machine, for the bump only.**

## Decision

`cargo release X.Y.Z --execute` on a release branch bumps the version, makes the replacements and
commits. `release.toml` sets `publish`, `tag` and `push` to false and refuses `main`. The
maintainer tags the squash commit by hand and moves `v3` by hand.

## Consequences

A release needs cargo-release installed, and a pre-release still needs the Python pins edited by
hand in PEP 440 form. When 4.0 drops the check-json5 compatibility, two replacements go away; the
decision stands.

## Confirmation

`scripts/check-versions.py` in CI and in release.yml's `versions` job.

## Revisit if

release-please gains a mode that keeps a hand-written CHANGELOG, jsonvet gets a second maintainer
who needs releases without the maintainer's machine, a second archive problem turns up, or we
want installers (cargo-dist).
