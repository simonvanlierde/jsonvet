# 12. Schema paths from a project stay in it, and their errors quote nothing

Status: accepted (2026-10-23)

## Context

A config file's `schemas` and a file's own `$schema` name a schema on disk. Both come from the
repository, so a pull request from a stranger picks which file jsonvet reads. Before this record,
`{"$schema": "/etc/passwd"}` printed ``expected a value, found `root` ``, a `..` path or a symlink
reached any file, and a JSON file outside the repository had its values quoted in schema compile
errors, in text, JSON and SARIF. On Windows, a `\\host\share` path would open a network
connection. Forty spellings of one 6.9 MB schema (`x/../s.json`, ...) were each read once, which
took 36.7 s and 15.5 GB.

## Options

- **Follow `$schema` only on request**, with a flag: safe, but the feature is off where it is
  most useful. Argued, not measured.
- **Confine the paths to the project and quote nothing from such a schema**, as
  [0009](0009-where-the-config-search-stops.md) does for a symlinked config file.
- **Confine only**, and keep quoting: a secret file in the working tree, such as a `.env` a CI
  step wrote, is inside the project too.

## Decision

A schema path from a project's config file or a file's `$schema` must stay inside the project:
the repository root, or the working directory without a `.git`, as in 0009. A path with a root
or a Windows prefix (`/x`, `//x`, `C:\x`, `\\host\share`) is refused before anything touches
the disk. Then `..` is folded by name and the path made canonical, so a `..` or a symlink that
leads out is refused too, whether or not the file exists. A refused `$schema` is a schema error
at its value; a refused config entry is a usage error at the entry. An error in such a schema
names its line and column, never its text. `--schema` and the user's own config file are the
user's choice, so they may name any file and their errors quote it. Schemas are cached by
canonical path, so every spelling of one is read once.

## Consequences

A project that keeps its schemas outside the repository, or names them by absolute path, must
move them in or pass `--schema`. A schema error from `$schema` or a project config says only
where it is, so the author opens the schema to see why. A symlink inside the project that leads
to a missing file outside is skipped like any missing `$schema`, so it tells whether that file
exists.

## Confirmation

`a_schema_outside_the_project_is_refused_and_never_quoted`,
`a_broken_schema_named_by_the_file_or_a_config_shows_no_text_from_it` in
`crates/jsonvet-cli/tests/cli.rs`'s `mod schema`, and
`every_spelling_of_a_schema_path_reads_it_once` in `crates/jsonvet-cli/src/lib.rs`.

## Revisit if

Users need schemas outside the project without `--schema` (add a setting for extra roots), or
the hidden messages make broken schemas too hard to fix.
