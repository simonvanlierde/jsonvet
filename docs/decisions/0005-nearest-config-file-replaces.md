# 5. The nearest config file applies, and replaces the ones above it

Status: accepted (2026-10-20)

## Context

Up to 3.0, one `jsonvet.toml`, found from the working directory, applied to every file, so a run
from a monorepo's root ignored each package's own.

## Options

- **One file from the working directory.** Wrong for monorepos.
- **Merge every file from the root down.** A package's settings would depend on files outside
  it. Argued, not built.
- **The nearest file replaces any above it**, as in ruff.

## Decision

Each file takes the nearest config file in its folder or a parent, up to the first `.git`, with
nothing merged (#73). Its patterns match paths relative to its own folder (3.0.0, #51).

## Consequences

A package's settings are exactly what its file says, so one that relied on the root's must copy
them. We call this Changed, not Breaking: a nested file used to apply only when run from its own
folder, so it was already meant for those files. The bench cases stayed within σ (#73).

## Confirmation

`each_file_takes_the_nearest_config_file` and `a_walk_takes_each_folders_nearest_exclude` in
`crates/jsonvet-cli/tests/cli.rs`.

## Revisit if

Users want shared settings across packages (add `extend`, not merging), or the lookup costs past
5% on a real repository.
