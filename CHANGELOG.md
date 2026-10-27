# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html). Bump the minor
version for a new option or output, the patch version for a fix.

## [Unreleased]

Most users need no change. Check these if they apply to you:

1. If a file has a top-level `$schema` that is a path, jsonvet now checks the file against that
   schema. A `$schema` that starts with `/` or `//`, or climbs out of the repository with `..`, is
   now an error even if nothing is there. Make it a URL or a path inside the repository, or pass
   `--schema`.
2. If a script reads piped text output while `FORCE_COLOR` is set, the lines now carry color codes.
   Pass `--color never` to get plain lines.

### Added

- A config file can give files a schema: `schemas = [{ files = "config/*.json", schema =
  "server.schema.json" }]` checks the matching files as `--schema` would. A file's own `$schema`
  that names a schema on disk, relative to the file, works too. jsonvet still fetches nothing, so a
  `$schema` URL, or a relative path inside the project to a file that does not exist, means no
  schema check. Both kinds of path must stay inside the checked file's own project (its repository,
  or where the config search for it stops), unless `--config` named the config file, and an error in
  such a schema names its place but quotes nothing from it, so a pull request cannot make jsonvet
  read or show a file outside the project.
- `--color auto|always|never` colors the code frames. `auto`, the default, colors them on a
  terminal unless `NO_COLOR` is set. `FORCE_COLOR` or `--color always` color piped output too,
  which stays one line per problem with the severity and rule in color.
- SARIF results and GitHub annotations now have an `endColumn`, so code scanning and pull
  requests mark the whole duplicate key, number or word, not only its first character.
- The library's `Problem::end()` gives the byte offset where the problem's text ends, and
  `Report::text()` gives the text a report is about.
- A playground at <https://simonvanlierde.github.io/jsonvet/>. Paste text, pick a dialect, and see
  each problem with its line, column and rule as you type. It runs jsonvet in your browser as
  WebAssembly, so the text never leaves your machine. It has no `--schema` checks.

### Changed

- With `FORCE_COLOR` set, as many Node CI setups do, piped line output now carries color codes
  around the severity and the rule. `--color never` gives plain lines.
- `--schema` checks a JSON Lines file of 1 MB or more on several cores, one part of its lines on
  each, and folds and checks one line at a time instead of the whole file at once. On 12 cores,
  a 102 MB file checks 2.7x faster and 80 MB of nested records 6 to 9x faster, in 0.91 GB
  instead of 3.4 GB. The output is unchanged.
- `--schema` uses jsonschema 0.60.1. It shares the values of an `enum` or `const` between errors
  instead of copying them. A schema with an `anyOf` around `additionalProperties` and a
  200,000-value `enum` took 6.3 GB and 2.7 s on 1,000 failing keys, and now takes 40 MB and 0.03 s.
  The binary is 135 KB larger.
- On a terminal, each problem now shows as a code frame: the message, the line of the file with
  the problem marked under it, and the related place, such as a duplicate key's first copy.
  pre-commit in a terminal shows frames too. CI, pre-commit.ci and pipes keep one line per
  problem, and `| cat` forces lines.
- Strings check faster. A 9 MB file of short keys and values checks 8% faster, and files full of
  escapes or of JSON5 strings with tabs 10 to 25% faster. Pretty-printed JSON gains about 3%.
  The problems reported are unchanged.
- `--schema` takes less memory on large files. jsonvet used to keep every value the parser read
  until the whole file was folded into values. Now it folds them as it reads, and checks each
  document as soon as it is complete. On a 12 MB file of small records, peak memory went from
  378 MB to 266 MB. A JSON Lines file of 1 MB or more is checked in parts, as before. A file with
  schema errors is read a second time to place them, which costs about 2% more time there. The
  output is the same.

### Fixed

- `--schema` no longer fails a file because a string in it is too costly to match against a
  pattern the schema never applies there. jsonvet tried every pattern on every string, so any Expo
  `app.json` with a string of 5,000 characters or more failed against SchemaStore's schema.
- The crates on crates.io and the wheels on PyPI now carry the MIT license text, not just its name.

### Security

- `--schema` no longer takes tens of seconds on strings too costly for a pattern under an `allOf`:
  5,000 such strings took 26.3 s and now take 0.05 s, and one beside 100,000 that fail the pattern
  took over 120 s and now takes 2.55 s. A much larger file can still be slow: SECURITY.md has the
  numbers.

## [3.1.1] - 2026-10-21

### Fixed

- `cargo binstall jsonvet-cli` found the 3.1.0 release binaries but stopped, because each archive held
  only `jsonvet` and not the crate's other binary, `check-json5`. The archives now hold both.

## [3.1.0] - 2026-10-21

3.1 checks the files under a folder, takes the nearest config file for each file, explains its
rules and prints JSON for scripts. Each release now has standalone binaries with build provenance
attestations, and the Action checks the wheel's attestation before it installs it. Runs over many
files or large files are faster, and a crafted schema or repository can no longer hang jsonvet or
fill memory.

To upgrade, set `rev: v3.1.0`; `uses: simonvanlierde/jsonvet@v3` takes it by itself. Most users
need no other change. Check these if they apply to you:

1. If a subfolder has its own `jsonvet.toml` or `[tool.jsonvet]`, its files now follow only that
   file, not the one at the repository root. Copy any root settings they need into it.
2. If you run jsonvet with no `.git` above the files, as after actions/checkout's tarball
   fallback, a `jsonvet.toml` above the working directory no longer applies. Run jsonvet from the
   folder that holds it, or pass `--config`.
3. If a script reads the count in `... and N more schema errors`, change it: the line is now
   `... and more schema errors in this document`, with no count.
4. If your schema has a `$ref` that points at nothing, fix it: jsonvet now refuses the schema
   with exit 2, also when nothing uses that `$ref`. Nested `unevaluatedProperties` or
   `unevaluatedItems`, and the values in `enum` and `const`, now count toward the schema size
   limit, so a schema that passed before can now be too big.
5. On GitHub Enterprise Server, set the Action's `verify` input to `false`.

With `--schema`, a file of 3.2 MB or more is now checked on its own, not beside others. A run over
many large files takes longer and much less memory; see Changed.

### Added

- A folder as an argument checks the files under it with an extension or name jsonvet knows, in
  sorted order. The walk skips hidden files and what `.gitignore` and `.ignore` name, as ripgrep
  does. Files named on the command line are checked as before. Before, jsonvet reported a folder
  as `not a regular file`.
- An `exclude` setting: globs, relative to the config file, for paths a folder walk skips.
- A user config file, as ruff has: `jsonvet/jsonvet.toml` in `$XDG_CONFIG_HOME` (or `~/.config`)
  on Linux and macOS, or in `%APPDATA%` on Windows. It applies only when no project config file
  is found, and its globs are relative to the working directory. `--config` and `--no-config`
  skip it.
- `-` as a file reads standard input, and `--stdin-filename PATH` names it for the output, the
  dialect and the settings. With `--stdin-filename` and no file, jsonvet reads standard input.
- `--list-rules` lists the rules with their default severity in each dialect, and
  `--explain RULE` explains one, with a bad and a good example. A mistyped rule gets a
  suggestion, such as ``(did you mean `duplicate-key`?)``.
- `--format json` prints one JSON document for scripts and editors: a `version` and a list of
  diagnostics. Each has the file, rule, severity, message, line, column and byte offset, and the
  related place where there is one, such as a duplicate key's first copy. Unreadable files and
  `--schema` errors are in the list too. docs/usage.md describes every field.
- SARIF output gives each rule its help text and a link to its section of docs/usage.md, so
  GitHub code scanning shows them on each alert.
- When stdout is a terminal, text output ends with a count, such as `3 errors in 2 of 5 files`.
  Piped output, as in the hook and in CI, is unchanged.
- Clearer syntax errors. Verdicts, exit codes and duplicate-key reports are unchanged:
  - A missing comma says ``missing `,` `` and points just after the value before it, not at the
    next key.
  - An object or array still open at the end of the file says which one and where it opened:
    ``unclosed `{`: expected `,` or `}`, found the end of the file (opened at 1:1)``. SARIF adds
    the opening bracket as a related location.
  - A bad word shows whole, with a hint for other languages' literals:
    ``expected a value, found `True`; did you mean `true`?``.
  - A number such as `01` says `leading zeros are not allowed`.
  - Syntax that a stricter dialect rejects names the dialect that allows it:
    `comments are not allowed in json; the jsonc dialect allows this`, and for a trailing comma
    `the jwcc and json5 dialects allow this`. The hint names a dialect only when that dialect
    reads the syntax in full, so `[NaNx]` or an unclosed `/*` gets none.
- A UTF-16 or UTF-32 file, with or without a byte order mark, says
  `file is UTF-16 (LE); jsonvet reads UTF-8` instead of `not valid UTF-8 (byte 0)` or a syntax
  error at its first zero byte. jsonvet tells the encoding from the zero bytes in the first four,
  so no UTF-8 file that passed before fails now.
- An unknown rule, key, severity or dialect in the config file gets its line and column and the
  closest known name: ``jsonvet.toml:2:1: unknown rule `duplicate-keys` (did you mean
  `duplicate-key`?)``. Unknown values for `--dialect`, `--format`, `--error`, `--warn` and
  `--ignore` get the same hint, also for a wrong case such as `--format SARIF`.
- `jsonvet --help`, the reference and the unknown-dialect error list the dialect aliases
  `ijson`, `hujson`, `ndjson` and `json-lines`, which jsonvet already accepted.
- The Action checks the wheel's build provenance attestation with `gh attestation verify` before
  it installs jsonvet 3.1.0 or later, and stops if the check fails. On a runner without `gh` it
  installs the wheel unchecked, with a notice. A new `verify` input turns the check off, as
  GitHub Enterprise Server needs.
- Each GitHub release has the `jsonvet` binary for eight targets (Linux glibc and musl, macOS and
  Windows, on x86-64 and ARM64) as `jsonvet-<target>.tar.gz` or `.zip`, with a `SHA256SUMS` file
  and a CycloneDX SBOM of its dependencies. `cargo binstall jsonvet-cli` downloads that binary
  instead of compiling it.
- Build provenance attestations for the wheels, the sdist and the release files, which
  `gh attestation verify` checks. `SECURITY.md` shows how.
- `SECURITY.md`: how to report a file that crashes or hangs jsonvet, privately through GitHub.
- Library: `RelatedKind::Opened`, the related location of an unclosed object or array, and
  `Dialect::ALIASES`.

### Changed

- Each file takes the nearest `jsonvet.toml` or `[tool.jsonvet]` above it, in its own folder or a
  parent up to the repository root, so each package in a monorepo can have its own settings.
  Before, jsonvet found one file from the working directory for all files. The nearest file
  replaces any further up; nothing is merged. A folder walk skips what each folder's nearest file
  excludes, and `--schema` reads the schema with the settings nearest to it. `--config` and
  `--no-config` work as before.
- Without a `.git` anywhere above a file, as after actions/checkout's tarball fallback, the config
  search stops at the working directory, or for a file outside it at the file's own folder.
  Before, it climbed to `/`, so a `jsonvet.toml` above the job's folder on a shared runner
  applied.
- A document with more than 100 schema errors now ends with `... and more schema errors in this
  document`, without a count. Counting the rest meant finding every one of them.
- A schema with a `$ref` that points at nothing in the file is a usage error at that `$ref`, also
  when nothing uses it. Before, an unused one passed and a used one was reported at the schema's
  first line.
- Files are checked on every core, so a run over many files is about 4x faster on 12 cores. The
  output is unchanged and in the order the files were given. Memory now covers the files checked
  together: about 64 MB of text, or one file per core when files are larger than that. With
  `--schema`, a file counts 20 times its size, so a file of 3.2 MB or more is checked on its own:
  twelve 10 MB files take 0.30 GB instead of 3.4 GB, in 2.6 s instead of 1.3 s.
- `--schema` checks a top-level array one element at a time when only `items` checks its
  elements. A 10 MB array failing everywhere now takes as much memory as a valid one, 0.37 GB
  instead of 2.0 GB, and checks 2.7x faster. The errors reported are unchanged.
- `--schema` also checks a top-level object one member at a time when only `properties`,
  `patternProperties` and `additionalProperties` check its members. Up to 100 errors, the output
  is unchanged. Past that, the 100 reported are the first by member, in key order, instead of by
  keyword.
- A JSON Lines file of 1 MB or more is split at newlines and checked on the cores its chunk leaves
  free: 78 MB of nested records check 3x faster on 12 cores. The output is unchanged.
- Strings are scanned 16 bytes at a time: files of long strings check up to 1.8x faster.
- Objects with many keys check up to 2.5x faster in a third less memory: one object of 2 million
  keys takes 0.34 s and 158 MB instead of 0.84 s and 246 MB. Files of many small objects check
  about 10% faster.
- The parser skips the space between tokens faster: 88 MB of minified JSON checks 21% faster,
  71 MB of pretty-printed JSON 7% faster, and the 2.7 MB benchmark file 5% faster.
- Thousands of files named on the command line start checking sooner, because jsonvet asks
  which are folders on every core. 10,000 named files check in 50 ms instead of 67 ms.
- `exclude` and `dialects` patterns cost about 60 times less per file. A folder walk over 10,000
  files with 10 `exclude` patterns takes 62 ms instead of 128 ms, and with 1,000 patterns 0.13 s
  instead of 9.9 s.
- The README is shorter and starts with a quick start. The details (globs, exit status, SARIF,
  the Action's inputs, the JSON Schema limits) moved to
  [docs/usage.md](https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md).

### Fixed

- `--schema` takes linear time on an object with many keys holding U+FFFD or lone surrogates. It
  was quadratic: 40,000 such keys took over a minute.
- `--dialect jsonl` with `--schema` no longer reads the schema as JSON Lines, which rejected any
  schema written over several lines.
- In JSON5, `{"a": None}` said ``expected `NaN`, found `o` ``. It now says
  ``expected a value, found `None`; did you mean `null`?``.
- `jsonvet --help` no longer says that `--schema` is missing from the PyPI wheels. It has been in
  them since 3.0.0.
- The Action no longer passes when the repository is not checked out: with the default `files`
  input it fails and says to run `actions/checkout`. It warns when it finds no JSON files.
- A `files` glob in the Action that also matched a folder failed that folder as not a regular
  file. Globs now match files only.

### Security

- A `pyproject.toml`, `--schema` file, `.gitignore` or `.ignore` that is a FIFO or a link to a
  device such as `/dev/zero` no longer hangs jsonvet or fills memory. jsonvet skips such a
  `pyproject.toml`, as it already skipped such a `jsonvet.toml`; a schema or ignore file like
  that is a usage error. Config and ignore files may hold up to 16 MB, a schema up to 64 MB.
- An error in a config file that is a symlink names its line and column but no longer quotes the
  key or value, which could be a secret from the file the link leads to.
- The Action runs Python isolated, so a `json.py`, `glob.py` or `venv/` folder in your repository
  never runs in place of Python's own.
- A crafted `--schema` file could make jsonvet run for minutes or take gigabytes of memory, past
  the limits docs/usage.md lists. A schema can sit in the repository a pull request edits, so
  this matters in CI. Fixed:
  - After 100 errors, jsonvet went on asking jsonschema for every other error just to count it:
    a 1 KB schema on a 15 KB file took 57 s. It now stops at the 101st: 1.7 s.
  - Each kept error held what jsonschema stores in it, such as every branch's errors for an
    `anyOf` or a copy of an `enum`'s values: 40 such errors took 3.2 GB. jsonvet now keeps only
    the message and the place.
  - The schema size limit left out the values in `enum` and `const`. It also left out that
    `unevaluatedProperties` and `unevaluatedItems` make jsonschema check the schemas beside them
    a second time: 27 nested levels of these took 22 s on `{"c": 1}`. Both count now.
  - A `$ref` to a relative `$id` was matched to the first `$id` with that text, not resolved
    against its base as jsonschema resolves it, so a small schema elsewhere could be measured in
    place of the real one. jsonvet now resolves `$ref`s with jsonschema's own code.
  - Looking up a `$ref` among the `$id`s was quadratic: 20,000 of them took 24 s, now 0.2 s.

## [3.0.0] - 2026-10-18

3.0 checks files against a JSON Schema, with each error at its line and column, writes SARIF
for GitHub code scanning, and ships as a GitHub Action.

Four changes below are **Breaking**. To upgrade, set `rev: v3.0.0` (or
`uses: simonvanlierde/jsonvet@v3`), then:

1. If you run jsonvet from a directory other than the one holding `jsonvet.toml` (or the
   `pyproject.toml` with `[tool.jsonvet]`), or pass absolute paths, write its `dialects` patterns
   relative to that file's directory.
2. If a script passes an option jsonvet does not know, expect exit 2 instead of exit 1.
3. If a script matches paths in the output, match non-ASCII keys as written, not as `\u`
   escapes.
4. Rust library users: treat `Rule::ALL` and `Dialect::ALL` as slices, add a `_` arm to a `match`
   on `Severity`, build a `Location` from `Location::START` instead of a struct literal, and read
   the text of `UnknownRule` and `UnknownDialect` with `id()` and `name()`.

Hook and command users who do none of the first three need no other change.

### Added

- `--schema FILE` checks each file against one JSON Schema. Each error comes under the rule
  `schema`, at the line and column of its value, in every dialect and output format, and ends
  with the value's JSONPath. Schema errors are always errors. A `$ref` must stay inside the
  schema file: jsonvet fetches nothing. A schema nested deeper than 128 levels, or holding more
  than 5,000,000 objects and arrays, counting each `$ref` followed, is a usage error. The limits
  on depth, size, error count, message length, regex backtracking and memory are in the
  [README](https://github.com/simonvanlierde/jsonvet#json-schema). A key or string whose pattern
  match needs more than 5,000 backtracking steps is an error, so `patternProperties` cannot let
  it pass unchecked. `additionalProperties` gives one error per key it does not allow, at that
  key, also for `"additionalProperties": false` on its own.
- `--schema` is in every build: the PyPI wheels, so the pre-commit hook and the GitHub Action,
  and `cargo install jsonvet-cli`. It grows the binary from about 0.7 MB to about 5 MB on Linux
  x86-64.
- `--format sarif` prints one SARIF 2.1.0 document instead of text lines, for GitHub code
  scanning. A duplicate key also points at its first copy, and a file that cannot be read is a
  result under the rule `unreadable`. Exit codes and `--max-warnings` work as with text. It keeps
  the first 25,000 results, the most GitHub keeps in full, and counts the rest.
- A GitHub Action, `uses: simonvanlierde/jsonvet@v3`. It checks the repository's tracked JSON
  files, apart from symlinks and submodules, or the `files` you give it (`**` matches at any depth
  on every runner). With `sarif:` it also writes a SARIF file for code scanning, and fails with a
  message when the installed jsonvet cannot. It installs jsonvet from PyPI and runs on Linux,
  macOS and Windows runners. Its `version` input takes `X.Y.Z` (with an optional suffix such as
  `rc1` or `.post1`), `X.*`, `X.Y.*` or `latest`. The `v3` tag moves to each new 3.x release once
  its wheels are on PyPI.
- Library: `Rule::description()`, `Problem::related()` with `Related` and `RelatedKind` (what the
  second position points at), `Problem::path()` (a duplicate key's JSONPath),
  `Dialect::EXTENSIONS`, `Dialect::FILE_NAMES` and `Location::START`.
- A `CITATION.cff`, and a Zenodo DOI for each release from this one on, so jsonvet and its
  conformance matrix can be cited.

### Changed

- **Breaking:** `dialects` patterns in `jsonvet.toml` or `[tool.jsonvet]` match the path relative
  to the directory of that file, not the working directory. Running from a subdirectory or with
  absolute paths now picks the same dialect. Patterns given with `--dialect` or `--strict` still
  match the path as typed.
- **Breaking:** non-ASCII keys in paths print as written, not as `\u` escapes:
  `$["日本"]`, not `$["\u65e5\u672c"]`, and an emoji key prints as itself, not as a surrogate
  pair. This covers duplicate-key paths and the `(at ...)` paths of schema errors. Control and
  invisible characters, `…` and lone surrogates such as `\ud800` stay escaped. Update scripts
  that match paths.
- **Breaking:** an unknown option, long such as `--x` or short such as `-x`, that is not an
  existing file is a usage error (exit 2, "unknown option"), not a missing file (exit 1). An
  existing file named like `--x.json`, `-x.json` or `--dialect=json` is still checked as a file:
  an argument is a file whenever a file of exactly that name exists, apart from `-h`, `-V`,
  `--help`, `--version` and a known option without `=`.
- **Breaking** (library): `Rule::ALL` and `Dialect::ALL` are slices, not fixed-size arrays.
  `Severity` and `Location` are `#[non_exhaustive]`, so later minor releases can add rules,
  dialects, severities and position fields. The text inside `UnknownRule` and `UnknownDialect` is
  private; read it with `UnknownRule::id()` and `UnknownDialect::name()`.
- Library: `parse`, `Event`, `EventKind`, `json_path`, `PathStep` and `is_hidden` are public
  only for the `jsonvet-cli` crate. They are hidden from the docs, are not part of the API and
  may change in any release.
- Directories, devices and FIFOs are reported as `not a regular file` under the rule `unreadable`
  (exit 1). A symlink to a regular file is still checked.
- A duplicate-key path longer than 256 bytes keeps as many segments from each end as fit, with
  `…` in place of the rest. A single segment too long to fit is cut. So a path is at most 259
  bytes (256 and the `…`). Shorter paths print as before.
- U+2028 and U+2029 end a line only in JSON5, whose spec counts them as line terminators. In
  JSON, I-JSON, JSONC, JWCC and JSON Lines, lines end at LF, CRLF and a lone CR only, so
  positions after a raw U+2028 in a string no longer move down a line.
- A JSON Lines syntax error at the end of a line says "found the end of the line" instead of
  "found U+000A".
- The `--dialect` help says that files with any other extension are checked as JSON5, and that
  extensions match in lower case only.
- The `check-json5` hook id, the `check-json5` command and `--strict` keep working through 3.x,
  not only until 3.0.
- Unquoted JSON5 keys follow Unicode 17, up from 16, so they may use the letters it added, such as
  the 4,298 ideographs of CJK Extension J.

### Deprecated

- `Problem::related_offset()`: use `related()`, which also says what the second position is.

### Fixed

- A key with a combining mark or a symbol such as U+093E or U+24B6 in its name now goes in
  brackets and quotes in the path, as check-json5 1.4.1 does. 2.0 printed it after a dot.
- In JWCC (HuJSON) files, a `//` comment ends only at a line feed, as the JWCC text says. A lone
  carriage return no longer ends it, so `[1, // c` + CR + `2]` fails as it should.
- A FIFO no longer hangs a run, and a symlink to `/dev/zero` no longer fills memory.
- A deep document full of duplicate keys could print hundreds of megabytes. The output now grows
  only with the number of duplicates.
- Finding a problem's column on a long single line is no longer quadratic, with or without
  non-ASCII text on it. A minified file with many duplicate keys takes milliseconds, not seconds.
- In JSON Lines files with CRLF line endings, an error at the end of a line gets the same column
  as with LF.
- `jsonvet … | head` exits with the verdict instead of printing "Broken pipe" and exiting 2.
- A file name such as `-x=a.json` keeps its `=`: 2.0 opened `-xa.json`. A file name that starts
  with `-` and is not valid UTF-8 is opened as given: 2.0 put U+FFFD in place of the bad bytes
  of some, such as `--x<0xff>.json`, and reported no such file.

### Security

- File names and messages can no longer start a GitHub Actions workflow command or hold the older
  `##[command]` form, which could hide or forge annotations. A name starting with `::` prints as
  `./::…`, and the `[` after `##` is escaped.
- Control characters, Unicode bidi controls, zero-width characters (U+200B to U+200F), the soft
  hyphen, U+180E, U+2060 to U+2064, the line and paragraph separators, the byte order mark and
  the tag characters U+E0000 to U+E007F print as escapes such as `\u001b` and `\u202e`. This
  covers file names, messages, and usage errors that quote a config file, a command-line
  argument or the schema file, and the same characters in SARIF strings. So a file cannot send
  terminal escape sequences, hide part of the line or reorder it.
- Library: a syntax error that shows the character it found prints a bidi control as a code
  point, such as `U+202E`, as it already did for control characters. So `Display` output cannot
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

[Unreleased]: https://github.com/simonvanlierde/jsonvet/compare/v3.1.1...HEAD
[3.1.1]: https://github.com/simonvanlierde/jsonvet/compare/v3.1.0...v3.1.1
[3.1.0]: https://github.com/simonvanlierde/jsonvet/compare/v3.0.0...v3.1.0
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
