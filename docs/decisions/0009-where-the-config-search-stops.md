# 9. Where the config search stops, and what a bad config may show

Status: accepted (2026-10-20)

## Context

With no `.git` anywhere, as with actions/checkout's tarball fallback, the config search
([0005](0005-nearest-config-file-replaces.md)) climbed to `/`, so a `jsonvet.toml` planted above
the job's folder on a shared runner applied. And a config error quoted the bad value: a
`jsonvet.toml` symlinked to a secret printed it into CI logs.

## Options

- **Keep climbing to `/`**, as ruff and 3.0 do, or **stop at the working directory**.
- For symlinked configs: **refuse them** (breaks shared configs), **follow them only inside the
  repository** (more code), or **follow them and quote nothing**.
- **A user config as a fallback**, as ruff has, or none.

## Decision

Without a `.git` above a file, the search stops at the working directory. A symlinked config file
is read, but its errors name only the file, line and column. When nothing is found,
`jsonvet/jsonvet.toml` under `$XDG_CONFIG_HOME` or `~/.config` (`%APPDATA%` on Windows) applies,
with globs relative to the working directory; `--config` and `--no-config` skip it.

## Consequences

A repository without `.git` that relied on a config above the working directory loses it
(Changed in the changelog). A config under a symlinked folder still shows values, since macOS's
`/tmp` and `/var` are symlinks. Tests point `XDG_CONFIG_HOME` and `APPDATA` at a missing folder.

## Confirmation

`without_git_the_search_stops_at_the_working_directory`,
`an_error_in_a_symlinked_config_file_shows_no_value_from_it` and
`the_user_config_applies_only_without_a_project_config` in `crates/jsonvet-cli/tests/cli.rs`.

## Revisit if

Users need configs above the working directory without git (add a root marker), or a symlinked
folder leaks a value in practice.
