# 9. Where the config search stops, and what a bad config may show

Status: accepted (2026-10-20)

## Context

The search for a file's config ([0005](0005-nearest-config-file-replaces.md)) climbed to `/` when
no folder had a `.git` entry, as with actions/checkout's tarball fallback. A `jsonvet.toml`
planted above the job's folder on a shared runner then applied to every job. And an error in a
config file quoted the bad value: a `jsonvet.toml` symlinked to a secret printed the secret into
CI logs (`unknown severity 'ghp_...'`). Both came from the review before 3.1.0.

## Options

- **Keep climbing to `/`**, as ruff and 3.0 do. Relies on runners not being shared.
- **Stop at the working directory when there is no `.git`.**
- For symlinked configs: **refuse them** (breaks repos that share one config through links),
  **follow them only inside the repository** (more code), or **follow them and quote nothing**.
- For users who want one config everywhere: **nothing**, or **a user config as a fallback**, as
  ruff has.

## Decision

- Without a `.git` above a file, the search stops at the working directory. A file outside it
  with no `.git` above uses only its own folder.
- A config file that is itself a symlink is still read, but its errors name only the file, line
  and column, never a key or value.
- When the search finds nothing, `jsonvet/jsonvet.toml` under `$XDG_CONFIG_HOME` (or `~/.config`;
  `%APPDATA%` on Windows) applies, with globs relative to the working directory. It replaces
  nothing a project file sets, and `--config` and `--no-config` skip it.

## Consequences

- A repository with no `.git` that relied on a config above the working directory no longer
  finds it. The changelog lists this under Changed.
- A config under a symlinked folder, rather than a symlinked file, still shows values: macOS's
  `/tmp` and `/var` are symlinks, and hiding errors there would hurt ordinary use.
- Tests point `XDG_CONFIG_HOME` and `APPDATA` at a missing folder (`.cargo/config.toml`), so no
  test reads a real home folder.

## Confirmation

`without_git_the_search_stops_at_the_working_directory`,
`an_error_in_a_symlinked_config_file_shows_no_value_from_it` and
`the_user_config_applies_only_without_a_project_config` in `crates/jsonvet-cli/tests/cli.rs`.

## Revisit if

- users ask for configs above the working directory without git: add an explicit root marker;
- a symlinked config folder leaks a value in practice.
