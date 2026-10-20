# 5. The nearest config file applies, and replaces the ones above it

Status: accepted (2026-10-20)

## Context

Up to 3.0, one `jsonvet.toml` (or `[tool.jsonvet]` in `pyproject.toml`), found from the working
directory, applied to every file. In a monorepo where each package has its own file, a run from
the root ignored them all. In 2.x the `dialects` patterns in that file also matched paths as
typed, so the same file got a different dialect when run from a subfolder or with an absolute
path.

## Options

- **One file from the working directory** (2.x and 3.0). Simple, but wrong for monorepos.
- **Merge every file from the repository root down**, each overriding the one above. A
  package's settings then depend on files outside it, and nobody can read them in one place.
  Argued, not built.
- **The nearest file replaces any above it**, as ruff does, with an `extend` key later if
  someone asks for one.

For `dialects` patterns: match the path as typed, or relative to the config file's folder.

## Decision

Each file takes the nearest config file in its own folder or a parent, up to the first folder
with a `.git` entry. Nothing is merged. Patterns in a config file match the path relative to
that file's folder (since 3.0.0, #51). Patterns given with `--dialect` or `--strict` still match
the path as typed, as in 1.4.

## Consequences

- A package's settings are exactly what its file says. A package that relied on root settings
  must copy them into its own file, as the changelog says.
- We list the nearest-file change under Changed, not Breaking (#73). Before, a nested file
  applied only when run from inside its folder, so it was already meant for those files. The
  relative patterns were Breaking in 3.0.0, with an upgrade step.
- A folder walk applies each folder's nearest `exclude`. Every config file is read before any
  file is checked, so a bad nested one still exits 2 with no output.
- The cost was measured with `hyperfine -N`, 40 runs, against a release build of main (#73).
  The `scripts/bench.py` cases stayed within σ. So did a walk over 10,000 files in 200 folders:
  33.1 ± 5.2 against 34.7 ± 1.0 ms. That needed a memo of the last folder looked up. Without it,
  the walk took 27% longer. Peak memory on the walk went from 8.4 to 9.3 MB.

## Revisit if

- users ask for shared settings across packages: add `extend`, not merging;
- a lookup per folder shows up past 5% on a real repository's run.
