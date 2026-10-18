# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html). Bump the minor
version for a new option or output, the patch version for a fix.

## [Unreleased]
### Fixed
- A duplicate key with a combining mark or a symbol such as U+093E or U+24B6 in its name is now
  quoted in the path (`$["a\u093e"]`), as check-json5 1.4.1 does. It was printed bare.

## [3.0.0] - unreleased

This is a major release because of the three **Breaking** changes below. To upgrade, set
`rev: v3.0.0` (or `uses: simonvanlierde/jsonvet@v3`), then:

1. If you run jsonvet from another directory than the one holding `jsonvet.toml` or the
   `pyproject.toml` with `[tool.jsonvet]`, or pass absolute paths, write its `dialects`
   patterns relative to that file's directory.
2. If a script passes an option jsonvet does not know, expect exit 2 instead of exit 1.
3. Rust library users: treat `Rule::ALL` and `Dialect::ALL` as slices, add a `_` arm to a
   `match` on `Severity`, build a `Location` from `Location::START` instead of a struct literal,
   and read the text of `UnknownRule` and `UnknownDialect` with `id()` and `name()`.

Hook and command users who do neither of the first two need no other change.

### Added

- `--format sarif` prints one SARIF 2.1.0 document instead of text lines, for GitHub code
  scanning. A duplicate key also points at its first copy, and a file that cannot be read is a
  result under the rule `unreadable`. Exit codes and `--max-warnings` work as before. It keeps
  the first 25,000 results, the most GitHub keeps in full, and counts the rest.
- A GitHub Action: `uses: simonvanlierde/jsonvet@v3` checks the repository's tracked JSON files,
  apart from symlinks and submodules, or the `files` you give it (`**` matches at any depth on
  every runner). With `sarif:` it also writes a SARIF file for code scanning, and fails with a
  message when the installed jsonvet cannot. It installs jsonvet from PyPI and runs on Linux,
  macOS and Windows runners. Its `version` input takes `X.Y.Z` (with an optional pre- or
  post-release suffix such as `rc1` or `.post1`), `X.*`, `X.Y.*` or `latest`. The `v3` tag moves
  to each new 3.x release once its wheels are on PyPI.
- Library: `Rule::description()`, `Problem::related()` with `Related` and `RelatedKind` (what the
  second position points at), `Problem::path()` (a duplicate key's JSONPath),
  `Dialect::EXTENSIONS`, `Dialect::FILE_NAMES` and `Location::START`.
- A `CITATION.cff`, and a Zenodo DOI for each release from this one on, so jsonvet and its
  conformance matrix can be cited.
- `--schema FILE` checks each file against one JSON Schema and reports each error under the rule
  `schema`, at the line and column of the value it is about, in every dialect and output format.
  Each message ends with the value's JSONPath, as duplicate-key messages write it, with `$` for the
  root. The rule id and the position are the stable part; the message wording comes from the
  jsonschema crate and may change between releases. A file with a syntax error gets no schema
  errors, and with `--schema` a file nested deeper than 512 levels has one, since jsonschema's
  memory use grows with the square of the depth there. A `$ref` must stay inside the schema file:
  jsonvet fetches nothing. Each document lists at most 100 schema errors, the first jsonschema finds
  (it takes object keys in sorted order), then a count of the rest, which SARIF output leaves out. A
  document holding `NaN`, `Infinity` or a number past a double's range has no JSON Schema value, nor
  does one with two keys in an object that differ only in lone surrogates, which both become U+FFFD;
  such a document gets one schema error and is not checked. Schema errors are always errors:
  `--error`, `--warn`, `--ignore` and settings files do not take `schema`. SARIF output lists
  `schema` among its rules in builds that have `--schema`. A schema that cannot be read or used,
  nests more than 128 levels, or holds such a number or keys exits with status 2, as does one whose
local `$ref`s chain more than 128 deep, counting the nesting along the way. A regex in it may
  backtrack 5,000 times per match; past that, `pattern` reports an error, but `patternProperties`
  and an `additionalProperties` beside it count the key as not matching. With `--schema`, memory use
  is about 23 times the file's size.
- `--schema` comes with `cargo install jsonvet-cli`, where the binary grows from about 0.6 MB to
  about 5 MB on Linux x86-64. The PyPI wheels, which the hook and the GitHub Action install, are
  built without it and stay at about 0.65 MB; there `--schema` exits with status 2 and says how to
  get it. Building the crate with `--no-default-features` leaves it out too.
- The library's `parse` returns a text's values as positioned events (the new types `Event` and
  `EventKind`), next to the report `check` gives.
- The library's `json_path` writes a path of steps (the new type `PathStep`) as duplicate-key
  messages do, long paths shortened the same way.

### Changed

- **Breaking:** `dialects` patterns in `jsonvet.toml` or `[tool.jsonvet]` match the path relative
  to the directory of that file, not the working directory, so running from a subdirectory or with
  absolute paths picks the same dialect. Patterns given with `--dialect` or `--strict` still match
  the path as typed.
- **Breaking:** an unknown `--option` that is not an existing file is a usage error (exit 2,
  "unknown option") instead of a missing file (exit 1). An existing file named like `--x.json` is
  still checked.
- **Breaking** (library): `Rule::ALL` and `Dialect::ALL` are slices, not fixed-size arrays, and
  `Severity` and `Location` are `#[non_exhaustive]`, so later minor releases can add rules,
  dialects, severities and position fields. The text inside `UnknownRule` and `UnknownDialect` is
  private; read it with `UnknownRule::id()` and `UnknownDialect::name()`.
- Directories, devices and FIFOs are reported as `not a regular file` under the rule `unreadable`
  (exit 1). A symlink to a regular file is still checked.
- A duplicate-key path longer than 256 bytes keeps as many segments from each end as fit, with
  `…` in place of the rest, and a single segment too long to fit is cut, so a path is at most
  259 bytes (256 and the `…`). Shorter paths print as before.
- A JSON Lines syntax error at the end of a line says "found the end of the line" instead of
  "found U+000A".
- The `--dialect` help says that files with any other extension are checked as JSON5, and that
  extensions match in lower case only.
- The `check-json5` hook id, the `check-json5` command and `--strict` keep working through 3.x,
  not only until 3.0.
- Unquoted JSON5 keys follow Unicode 17, up from 16, so they may use the letters it added, such
  as the 4,298 ideographs of CJK Extension J.

### Fixed

- In JWCC (HuJSON) files, a `//` comment ends only at a line feed, as the JWCC text says. A lone
  carriage return no longer ends it, so `[1, // c` + CR + `2]` fails as it should.
- A FIFO no longer hangs a run, and a symlink to `/dev/zero` no longer fills memory.
- A deep document full of duplicate keys could print hundreds of megabytes; the output now grows
  only with the number of duplicates.
- Finding the column of a problem on a long single line is no longer quadratic, with or without
  non-ASCII text on it, so a minified file with many duplicate keys takes milliseconds instead
  of seconds.
- In JSON Lines files with CRLF line endings, an error at the end of a line gets the same column
  as with LF.
- `jsonvet … | head` exits with the verdict instead of printing "Broken pipe" and exiting 2.

### Security

- File names and messages can no longer start a GitHub Actions workflow command or hold the older
  `##[command]` form, which could hide or forge annotations. A name starting with `::` prints as
  `./::…`, and the `[` after `##` is escaped.
- Control characters, Unicode bidi controls, zero-width characters (U+200B to U+200F), the line
  and paragraph separators and the byte order mark in file names, in messages and in errors that
  quote a config file print as escapes such as `\u001b` and `\u202e`, so a file cannot send
  terminal escape sequences, hide part of the line or reorder it.
- Library: a syntax error that shows the character it found prints a bidi control as a code
  point, such as `U+202E`, as it already did for control characters, so `Display` output cannot
  reorder the line either.

## [2.0.0] - 2026-10-13

check-json5 is now **jsonvet**, rewritten in Rust. In JSON5 and strict mode it gives the same
pass or fail as 1.4.1, apart from the differences listed below. I ran both on 20,000 generated
files plus the 114 json5-tests cases in each mode, and they agreed on every one. With the 70-file
corpus added later and a stricter comparison (20,184 files, 40,368 runs), 385 runs on 383 files
differ, all on purpose: a `\u` escape that cannot start an unquoted key, the depth limit, and a
syntax error reported after a duplicate key.

Upgrade: set `rev:` to the new tag. The repository moved to `simonvanlierde/jsonvet`, and the old
URL still works. The `check-json5` hook id, the `check-json5` command and `--strict` keep working
until 3.0. To change anything else, switch to the `jsonvet` hook id.

### Added

- Dialects: `json`, `i-json`, `jsonc`, `jwcc` (HuJSON), `json5` and `jsonl` (JSON Lines), picked
  by file extension or with `--dialect DIALECT[:GLOB]`. `.json` files stay JSON5 by default.
- Rules with ids and severities: `syntax`, `duplicate-key`, `invalid-number`,
  `invalid-code-point`, `byte-order-mark` and `missing-final-newline`, changed with `--error`,
  `--warn` and `--ignore`, and `--max-warnings N`.
- Settings in `jsonvet.toml`, or `[tool.jsonvet]` in `pyproject.toml`, found up to the
  repository root.
- A duplicate key's report gives the position of the first copy too.
- The hook also checks `.jsonl` and `.ndjson` files.
- The `jsonvet` Rust library and the `jsonvet-cli` crate.

### Changed

- **Breaking:** each output line gains the severity and rule, as in
  `file:4:5: error[duplicate-key] duplicate key $.a (first at 2:5)`. Syntax errors use jsonvet's
  own wording. GitHub annotations are `::error` or `::warning` by severity.
- **Breaking:** lines end at every JSON5 line terminator (LF, CR, CRLF, U+2028 and U+2029), so
  positions after a lone CR, U+2028 or U+2029 differ from 1.x.
- **Breaking:** `.jsonc`, `.code-workspace` and `.code-snippets` files are checked as JSONC, and
  `.hujson` and `.jwcc` files as JWCC. 1.4.1 read every file as JSON5, so JSON5-only syntax such
  as `{'a': 1, b: 2}` passed there and fails now. To keep the old verdict, pass
  `--dialect json5:GLOB` or add a `dialects` entry to the config file.
- The `check-json5` package on PyPI now only installs `jsonvet`. The wheel brings the `jsonvet`
  and `check-json5` commands.
- The hook installs a prebuilt binary, from wheels for Linux, macOS and Windows on x86-64 and
  arm64. It needs no Rust, and no longer starts Python on every run.
- Raw U+2028 and U+2029 inside strings pass in every JSON5 file, as the spec allows. 1.x
  rejected them in files that used more of JSON5 than comments and trailing commas.
- Nesting deeper than 10,000 levels is a syntax error, about where 1.4 stopped (9,998 levels).
  Library users can change it with `Config::max_depth`.
- Numbers of any length pass; 1.x capped integers at 1,000,000 digits.
- A `\u` escape in an unquoted JSON5 key fails when the character it stands for could not appear
  there literally, as in `{\u0301: 1}`. 1.x passed any escape.

### Deprecated

- The `check-json5` hook id, the `check-json5` command and `--strict` keep working until 3.0.
  Switch to the `jsonvet` hook id and `--dialect json:GLOB`.

### Removed

- **Breaking:** the importable Python module is gone.

## [1.4.1] - 2026-10-13

Upgrade: set `rev: v1.4.1`. Nothing else is needed.

### Fixed

- `--strict` passed a file where `NaN` or `Infinity` runs on into other text, such as
  `[NaN-2.5e3` or `Infinitynul }`. Those files now fail, with the constant's position.
- An unquoted JSON5 key written with a `\u` escape, such as `\u0061`, was compared with its
  escape still in place, so it did not match `a` or `'a'`, and its path showed the escape. Keys
  now compare by their decoded name.

## [1.4.0] - 2026-10-13

Upgrade: set `rev: v1.4.0`. Nothing else is needed.

### Added

- The hook also runs on Tailscale's `.hujson` and `.jwcc`, VS Code's `.code-workspace` and
  `.code-snippets`, `.webmanifest` and `.babelrc` files: all JSON, some with comments.

### Changed

- The hook checks JSON with only comments and trailing commas, such as `tsconfig.json`, with the
  C-speed `json` parser instead of `json5`. Whole hook runs are 11× faster on a 50 KB file and
  56× faster on a 2 MB one. Files that use more of JSON5 still go through `json5`.
- A number may have up to 1,000,000 digits. 1.3.0 lifted Python's limit of 4300 altogether,
  which let one file with a huge number stall a run. A longer number now fails its file with a
  message.

### Fixed

- A JSON5 file with comments and an unescaped U+2028 or U+2029 inside a string now passes, as
  the JSON5 spec allows. The `json5` package rejected it; strict JSON files already passed.
- A file with a duplicate key and an undecodable string after it crashed the run with a
  traceback and left the remaining files unchecked. It now reports the duplicate.
- A newline in a file name is printed as `\n`, so a file name can no longer inject a GitHub
  Actions workflow command into CI output.
- Checking a file with many duplicate keys took time that grew with the square of their
  number. 200,000 duplicates took 42 s; they now take under a second.
- `--strict` patterns match `./sub/a.json` like `sub/a.json`.

## [1.3.0] - 2026-10-13

Upgrade: set `rev: v1.3.0`. Nothing else is needed for the hook. Only a script that parses the
hook's output, or imports `pre_commit_hooks.check_json5`, needs a change (see Changed).

### Added

- Every failure prints as `file:line:col: message`, so editors and terminals link to it.
- A duplicate key reports its JSON path (`$.server.port`), and every repeated key in a file is
  listed, not only the first.
- On GitHub Actions, each failure is also an annotation on the pull request diff.
- `--strict GLOB` holds matching files to strict JSON: no comments, trailing commas or `NaN`.
  Repeat it to add patterns, and `!GLOB` exempts files.
- Published on PyPI, so `uvx check-json5 FILE...` works without pre-commit.

### Changed

- **Breaking:** the output format changed. `f.json: Failed to json decode (<string>:4
  Unexpected "," at column 8)` is now `f.json:4:8: Unexpected ","`.
- **Breaking:** the module moved from `pre_commit_hooks.check_json5` to
  `check_json5.check_json5`, so the PyPI package no longer collides with `pre-commit-hooks`. The
  `check-json5` command is unchanged.
- Tested on Python 3.15.

### Fixed

- A file nested deeper than Python's recursion limit crashed the run and left the remaining files
  unchecked. It now fails on its own.
- Valid JSON with an integer of more than 4300 digits failed.

## [1.2.1] - 2026-10-11

### Changed

- Faster start: a run over strict JSON no longer imports `json5`, `argparse`, `pathlib` or
  `typing`.

## [1.2.0] - 2026-07-06

### Added

- Strict JSON is parsed with the stdlib `json` parser first, falling back to `json5` only for
  files that use JSON5 features: about 500× faster on large files.
- The hook also runs on `.jsonc`, `.geojson` and `.jsonld` files.

## [1.1.0] - 2025-09-26

### Changed

- Packaging moved from Poetry to uv.

## [1.0.1] - 2025-12-26

### Changed

- No upper bound on the `json5` dependency.
- The hook also runs on `.json5` files.

## [1.0.0] - 2021-12-04

### Changed

- Renamed from `pre-commit-check-json5` to `check-json5`.

[Unreleased]: https://github.com/simonvanlierde/jsonvet/compare/v3.0.0...HEAD
[3.0.0]: https://github.com/simonvanlierde/jsonvet/compare/v2.0.0...v3.0.0
[2.0.0]: https://github.com/simonvanlierde/jsonvet/compare/v1.4.1...v2.0.0
[1.4.1]: https://github.com/simonvanlierde/jsonvet/compare/v1.4.0...v1.4.1
[1.4.0]: https://github.com/simonvanlierde/jsonvet/compare/v1.3.0...v1.4.0
[1.3.0]: https://github.com/simonvanlierde/jsonvet/compare/v1.2.1...v1.3.0
[1.2.1]: https://github.com/simonvanlierde/jsonvet/compare/v1.2.0...v1.2.1
[1.2.0]: https://github.com/simonvanlierde/jsonvet/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/simonvanlierde/jsonvet/compare/v1.0.1...v1.1.0
[1.0.1]: https://github.com/simonvanlierde/jsonvet/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/simonvanlierde/jsonvet/releases/tag/v1.0.0
