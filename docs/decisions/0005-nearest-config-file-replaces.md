# 5. The nearest config file applies, and replaces the ones above it

Status: accepted (2026-10-20)

## Context

Up to 3.0, one `jsonvet.toml` (or `[tool.jsonvet]` in `pyproject.toml`), found from the working
directory, applied to every file, so a run from a monorepo's root ignored each package's own. In
2.x its `dialects` patterns also matched the path as typed, so a file got a different dialect
when run from a subfolder.

## Options

- **One file from the working directory** (2.x, 3.0). Wrong for monorepos.
- **Merge every file from the root down.** A package's settings then depend on files outside it.
  Argued, not built.
- **The nearest file replaces any above it**, as in ruff, with an `extend` key if anyone asks.

## Decision

Each file takes the nearest config file in its folder or a parent, up to the first folder with a
`.git` entry, and nothing is merged (#73). A config file's patterns match paths relative to its
folder (since 3.0.0, #51); `--dialect` and `--strict` still match the path as typed.

## Consequences

- A package's settings are exactly what its file says. One that relied on root settings must
  copy them, as the changelog says. We call this Changed, not Breaking: before, a nested file
  applied only when run from its folder, so it was already meant for those files.
- A folder walk applies each folder's nearest `exclude`, and every config file is read before
  any file is checked, so a bad nested one still exits 2 with no output.
- Cost, from #73: the `scripts/bench.py` cases and a walk of 10,000 files in 200 folders stayed
  within σ, once a memo of the last folder took the walk from 27% slower back.

## Confirmation

`each_file_takes_the_nearest_config_file` and `a_walk_takes_each_folders_nearest_exclude` in
`crates/jsonvet-cli/tests/cli.rs`.

## Revisit if

- users want shared settings across packages: add `extend`, not merging;
- the lookup shows up past 5% on a real repository.
