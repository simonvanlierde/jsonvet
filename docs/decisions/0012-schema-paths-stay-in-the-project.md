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

A schema path from a config file the search finds or a file's `$schema` must stay inside the checked
file's own project: the folder where the config search for that file stops, as in 0009. That is the
nearest folder above the file with a `.git`, or without one the working directory, or the file's own
folder for a file outside the working directory. So a file in another repository, such as
`jsonvet ../other/x.json`, takes schemas from that repository, not the one jsonvet runs in.

jsonvet refuses a path with a root or a Windows prefix (`/x`, `//x`, `C:\x`, `\\host\share`)
before it touches the disk. Then it makes the path canonical, so it also refuses a `..` or a
symlink that leads out. For a file that does not exist, it makes the deepest existing part
canonical, following any dangling symlink on the way, and adds the rest by name. So a path that
would lead out is refused whether or not anything is there, and nothing outside can be probed.

A refused `$schema` is a schema error at its value. A refused config entry is a usage error at the
entry. An error in such a schema names its line and column, never its text.

`--schema`, `--config FILE` and the user's own config file are the choice of the person running
jsonvet. So they may name any file, with schema paths relative to the config file, and their
errors quote it. Schemas are cached by canonical path, so every spelling of one is read once.

## Consequences

A project that keeps its schemas outside the repository, or names them by absolute path, must
move them in or pass `--schema`. A schema error from `$schema` or a project config says only
where it is, so the author opens the schema to see why. A CI job that passes `--config` with a
path a pull request controls gives up the confinement for that config.

## Update 2026-10-25

"Quotes nothing" covers the errors a schema finds in a checked file, not only the errors that
stop it being used. A value that fails such a schema gets `value fails "<keyword>"` and the
keyword's line and column in the schema where jsonvet can find it, instead of jsonschema's
message. That hides `const` and `enum` values, patterns and regexes, required and dependent
property names, the limits of keywords such as `minimum` and `maxLength`, a `not` schema, and a
`$ref` target. Type names and format names are fixed words, so they stay, and so do the checked
file's own keys and values.

## Confirmation

`a_schema_outside_the_project_is_refused_and_never_quoted`,
`a_broken_schema_named_by_the_file_or_a_config_shows_no_text_from_it`,
`a_schema_named_by_the_file_or_a_config_quotes_nothing_from_it_in_its_errors`,
`a_config_file_outside_the_project_named_by_config_may_name_a_schema_beside_it`,
`a_file_in_another_repository_takes_schemas_from_its_own_project` in
`crates/jsonvet-cli/tests/cli.rs`'s `mod schema`,
`without_git_schemas_are_confined_to_where_the_search_stops` in its `mod file_reads`, and
`every_spelling_of_a_schema_path_reads_it_once` in `crates/jsonvet-cli/src/lib.rs`.

## Revisit if

Users need schemas outside the project without `--schema` (add a setting for extra roots), or
the hidden messages make broken schemas too hard to fix.
