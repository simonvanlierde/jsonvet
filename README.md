# jsonvet

[![CI](https://github.com/simonvanlierde/jsonvet/actions/workflows/ci.yml/badge.svg)](https://github.com/simonvanlierde/jsonvet/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/simonvanlierde/jsonvet/branch/main/graph/badge.svg)](https://codecov.io/gh/simonvanlierde/jsonvet)
[![pre-commit](https://img.shields.io/badge/pre--commit-enabled-brightgreen?logo=pre-commit&logoColor=white)](https://github.com/pre-commit/pre-commit)
[![License: MIT](https://img.shields.io/badge/license-MIT-green)](https://github.com/simonvanlierde/jsonvet/blob/main/LICENSE)
[![release](https://img.shields.io/github/v/release/simonvanlierde/jsonvet)](https://github.com/simonvanlierde/jsonvet/releases/latest)

jsonvet checks JSON, I-JSON, JSONC, HuJSON (JWCC), JSON5 and JSON Lines files for syntax errors,
duplicate keys and the rules of each dialect. Every problem comes with its file, line, column and
rule, and a duplicate key with the JSONPath and position of both copies:

```text
config.json:4:5: error[duplicate-key] duplicate key $.server.port (first at 2:5)
package.json:3:3: error[syntax] expected `,` or `}`, found `"`
events.jsonl:12:1: error[syntax] blank line; JSON Lines needs a value on every line
```

It is a pre-commit and prek hook, a command, and a Rust library. Until 2.0 it was check-json5, a
Python hook; 2.0 is a rewrite in Rust that keeps its verdicts for JSON5 and strict JSON (see
[Coming from check-json5](#coming-from-check-json5)).

## Use it as a hook

In `.pre-commit-config.yaml` (pre-commit or [prek](https://github.com/j178/prek)):

```yaml
- repo: https://github.com/simonvanlierde/jsonvet
  rev: v3.0.0
  hooks:
    - id: jsonvet
```

Options go in `args`. For example, to hold `package.json` to strict JSON and check `.vscode`
files as JSONC:

```yaml
    - id: jsonvet
      args: [--strict, package.json, --dialect, "jsonc:.vscode/*.json"]
```

The hook installs a prebuilt binary from PyPI, so it needs no Rust. It checks files by name:
`.json`, `.jsonc`, `.json5`, `.jsonl`, `.ndjson`, `.geojson`, `.jsonld`, `.hujson`, `.jwcc`,
`.code-workspace`, `.code-snippets`, `.webmanifest` and `.babelrc`. In GitHub Actions each problem
also becomes an annotation on the pull request.

The hook has no [`--schema`](#json-schema): the PyPI binary leaves it out to stay small, until it
shows its worth.

## Use it as a GitHub Action

```yaml
- uses: simonvanlierde/jsonvet@v3
```

The action installs jsonvet from PyPI into a virtual environment, so it runs on Linux, macOS and
Windows runners with no other setup, and the step fails when a file has an error. Inputs, all
optional:

| Input     | What it does                                                                         |
| --------- | ------------------------------------------------------------------------------------ |
| `files`   | paths or globs to check, separated by spaces; `**` matches folders at any depth on every runner; symlinks are skipped; by default every tracked file the hook would check, apart from symlinks and submodules |
| `args`    | extra options, separated by spaces, such as `--strict package.json`                  |
| `sarif`   | also write a SARIF file to this path (see [GitHub code scanning](#github-code-scanning)) |
| `version` | the jsonvet version to install: `X.Y.Z` (also with a suffix such as `rc1` or `.post1`), `X.*`, `X.Y.*` or `latest`; anything else stops the step |

Releases are tagged `vX.Y.Z`, and `v3` moves to the newest 3.x release. By default the action
installs the jsonvet version of its own tag: `@v3` the newest 3.x, `@v3.0.0` exactly 3.0.0. Pinned
to a commit SHA or a branch, it installs the release named in that commit's `Cargo.toml`. Between
releases that is the last release, not the code in the commit; the step log says which version it
installed. Pinning the action does not pin the wheel's bytes: pip installs `jsonvet==X.Y.Z`
from PyPI without a hash check. Set `version:` to say which release you want. It
installs the PyPI binary, so it has no [`--schema`](#json-schema) either.

## Dialects

Each file is checked in the dialect its extension implies, and `--dialect` or the config file can
choose another. Going from JSON to JSONC, JWCC and JSON5 adds syntax at each step. The one thing that
does not carry over is where a `//` comment ends: JWCC ends it at LF only, JSONC also at a lone
CR, and JSON5 also at U+2028 and U+2029.

| Dialect  | Adds to JSON                                                                 | Default for                                                        |
| -------- | ---------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| `json`   | nothing ([RFC 8259](https://www.rfc-editor.org/rfc/rfc8259))                  | none; choose it with `--dialect` or `--strict`                     |
| `i-json` | stricter content ([RFC 7493](https://www.rfc-editor.org/rfc/rfc7493)): lone surrogates, noncharacters and numbers a double cannot hold are errors | none |
| `jsonc`  | `//` and `/* */` comments                                                    | `.jsonc`, `.code-workspace`, `.code-snippets`                      |
| `jwcc`   | comments and trailing commas ([HuJSON](https://github.com/tailscale/hujson)) | `.hujson`, `.jwcc`                                                 |
| `json5`  | the [JSON5 1.0.0](https://spec.json5.org/) grammar                            | `.json`, `.json5`, `.geojson`, `.jsonld`, `.webmanifest`, `.babelrc` |
| `jsonl`  | one JSON value per line ([JSON Lines](https://jsonlines.org/))               | `.jsonl`, `.ndjson`                                                |

A file with any other extension is checked as JSON5. Extensions match in lower case only:
`A.JSONC` is JSON5, and so is `A.JSON`.

`.json` files default to JSON5, because most `.json` files that fail as JSON are config files
with comments. Hold the ones a strict parser reads, such as `package.json`, to JSON with
`--dialect json:package.json`.

## Rules

| Rule                    | Reports                                                       | Default                                          |
| ----------------------- | ------------------------------------------------------------- | ------------------------------------------------ |
| `syntax`                | the first syntax error; checking stops there                  | error                                            |
| `duplicate-key`         | a key repeated in one object, with both positions and its path | error                                            |
| `invalid-number`        | a number a double cannot hold, or an integer past ±(2⁵³ − 1)   | error in `i-json`, off elsewhere                 |
| `invalid-code-point`    | a lone surrogate or a noncharacter in a string                | error in `i-json`, off elsewhere                 |
| `byte-order-mark`       | a byte order mark at the start of the file                    | warning in `json` and `i-json`, error in `jsonl` |
| `missing-final-newline` | a JSON Lines file that does not end with a newline            | warning in `jsonl`                               |

Warnings don't fail a run unless there are more than `--max-warnings N`. A file jsonvet cannot
read is reported under `unreadable`, which is not a rule you can set: it always fails the run.

## Options and configuration

```text
--dialect DIALECT[:GLOB]   check files matching GLOB (every file, without one) as DIALECT,
                           or as `auto`, the extension's default; the last match wins
--strict GLOB              the same as --dialect json:GLOB, kept from check-json5
--error RULE, --warn RULE, --ignore RULE
--max-warnings N
--format text|sarif        text lines (the default), or one SARIF 2.1.0 document on stdout
--schema FILE              also check every file against the JSON Schema in FILE
--config FILE, --no-config
```

A glob without `/` matches the file name at any depth, and one with `/` matches the whole path.
`*` also matches `/`. A glob given with `--dialect` or `--strict` matches the path as you typed it
on the command line, so relative to the working directory; pre-commit runs from the repository
root. A glob in a config file is relative to the directory that holds the file: `data/*.json` in
the root's `jsonvet.toml` matches `data/x.json` whether jsonvet runs in the root or in `data/`, and
when the path is absolute. A file outside that directory matches none of the file's globs.

jsonvet reads regular files only. It follows a symlink and checks it when the target is a regular
file. A directory, device, FIFO or socket prints `error: not a regular file` (rule `unreadable`)
and makes the run exit 1. The hook (`types: [text]`) and the action skip tracked symlinks and
submodules.

pre-commit passes file names without a `--`, so a name that starts with `-` is read as an option
unless it is a file that exists: `-x.json` is a name, and so is an existing `--x.json`. An unknown
`--name` that is not an existing file is a usage error (`unknown option`). Put `--` before the
file names when you cannot rule out the confusion: `jsonvet -- --x.json`.

Settings can also live in `jsonvet.toml`, or in a `[tool.jsonvet]` table in `pyproject.toml`.
jsonvet looks in the working directory and its parents up to the repository root, and the command
line wins:

```toml
rules = { byte-order-mark = "ignore", missing-final-newline = "error" }

# Checked in order; the last matching entry wins.
dialects = [
  { files = ".vscode/*.json", dialect = "jsonc" },
  { files = "package.json", dialect = "json" },
]
```

A key other than `rules` and `dialects`, in the file or in a `dialects` entry, is an error
(exit status 2), so a typo is never ignored. A severity is `error`, `warning` (or `warn`), or
`ignore` (or `off`).

Exit status: 0 when no file has an error, 1 when one does, 2 for a usage error, which includes an
unknown option. A file jsonvet cannot read, or that does not exist, prints `name: error: <reason>`
with no line or column and makes the run exit 1. With no files, it exits 0. When the output pipe
closes early, as in `jsonvet … | head -1`, jsonvet stops quietly and exits with the verdict of the
files it had checked so far.

### Output

File names and messages come from the files you check. jsonvet escapes what could break the line
or a terminal. A control character prints as `\u001b`, with CR and LF as `\r` and `\n`, and
a Unicode bidirectional control or invisible character (U+061C, U+200B to U+200F, U+2028 to
U+202E, U+2066 to U+2069, U+FEFF) as `\u202e`. A name that starts with `::`, after any white space, prints as `./::…`. A `[`
straight after `##` prints as `\u005b`. So no output line looks like a workflow command, old (`##[command]`) or new
(`::command`). The `file=` value of an annotation holds the real name, percent-encoded as GitHub requires.

## JSON Schema

`--schema FILE` checks each file's values against one JSON Schema, and reports each error at the
line and column of the value it is about, in every dialect, comments and JSON5 syntax included.
It comes with `cargo install jsonvet-cli` only: the PyPI wheels, and so the hook and the GitHub
Action, are built without it, because it makes the binary about 5 MB instead of 0.6 MB. There,
`--schema` exits with status 2 and says why. The exit status is the contract, not the wording.

```text
$ jsonvet --schema server.schema.json server.json5
server.json5:3:9: error[schema] "80" is not of type "integer" (at $.port)
server.json5:6:3: error[schema] Additional properties are not allowed ('debug' was unexpected) (at $)
```

Each message ends with the JSONPath of the value, written as duplicate-key paths are: `$` is the
root, `$.tags[1]` an array element, `$[""]` the empty key.

The errors come under the rule `schema` and are errors, in text, GitHub annotations and SARIF alike.
They are always errors: `--error`, `--warn`, `--ignore` and the settings file do not take `schema`,
and exit with status 2 if given it. A message quotes the value unless that would make it long; then
it says `value`. A message, with its path, is cut at 200 bytes and then ends in `...`. Each document
lists at most 100 schema errors: the first 100 that jsonschema finds, which takes an object's keys
in sorted order, not in file order. They are printed in file order, then one more line, `... and N
more schema errors in this document`, at the document's start. SARIF output leaves that line out.
The rule id `schema` and the position are the stable part of each error; the message wording comes
from the jsonschema crate and may change between releases. A file with a syntax error gets no schema
errors, and with `--schema` a file nested deeper than 512 levels has one: jsonschema takes memory
that grows with the square of the depth to report an error deep inside a recursive schema. Each line
of a JSON Lines file is checked on its own. `NaN`, `Infinity` and numbers past a double's range have
no JSON Schema value, so a document holding one is reported once and not checked. So is a document
with two keys in one object that differ only in lone surrogates such as `"\ud800"`, which both
become U+FFFD. In the schema file, either is a usage error.

With `--schema`, jsonvet holds each file's values in memory: about 23 times the file's size, so
a 14 MB file takes about 320 MB. Without it, memory stays near the file's size.

A pattern with a back-reference or lookaround may backtrack 5,000 times per string. Past that,
`pattern` reports an error, but `patternProperties` and an `additionalProperties` next to it
treat the key as not matching: the jsonschema crate gives them no way to report it.

The schema file may itself be in any dialect. Its `$schema` picks the draft, from draft 4 to
2020-12. A `$ref` must point inside the schema file: jsonvet fetches nothing over the network or
from disk. A schema that cannot be read or used, such as one with a `$ref` to another file or one
nested more than 128 levels deep, is a usage error with exit status 2. Each `$ref` followed inside
the file counts as a level, so a chain of them deeper than 128 is one too; a `$ref` back to a
schema it is inside, as in `{"items": {"$ref": "#"}}`, is recursion and is fine.

## GitHub code scanning

`--format sarif` prints one [SARIF 2.1.0](https://docs.oasis-open.org/sarif/sarif/v2.1.0/)
document instead of text lines, which GitHub code scanning and other tools read. Each problem is a
result with its rule, severity, message and position, and a duplicate key also points at the
first copy. A file jsonvet cannot read is a result of its own, under the rule `unreadable`. The
exit status and `--max-warnings` work as with text output, and no workflow annotations are
printed. GitHub keeps only 5,000 results of a run with more than 25,000, so jsonvet writes the
first 25,000 and counts the rest in a tool notification and on stderr.

```bash
jsonvet --format sarif $(git ls-files '*.json') > jsonvet.sarif
```

The action's `sarif` input writes that file next to the usual log and annotations. Upload it with
`github/codeql-action/upload-sarif`, which needs `security-events: write`:

```yaml
jobs:
  jsonvet:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      security-events: write
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - uses: simonvanlierde/jsonvet@v3
        with:
          sarif: jsonvet.sarif
      # Upload when jsonvet found problems too: that is when the step fails.
      - uses: github/codeql-action/upload-sarif@24c54180a607b1449ed407dd24f251e4e9147c8d # v4.38.3
        if: success() || failure()
        with:
          sarif_file: jsonvet.sarif
```

## Other ways to run it

```bash
uvx jsonvet config/*.json        # from PyPI, without installing; no --schema
pip install jsonvet              # the same checks, without --schema, as a Python package
cargo install jsonvet-cli        # from source, with --schema
```

The library is the `jsonvet` crate:

```rust
use jsonvet::{check, Config, Dialect};

let report = check("{\"a\": 1, \"a\": 2}", &Config::new(Dialect::Json));
for problem in report.problems() {
    let at = report.location(problem.offset());
    println!("{}:{}: {}[{}] {problem}", at.line, at.column, problem.severity(), problem.rule());
}
```

## Coming from check-json5

Bump `rev` to the newest tag. The `check-json5` hook id, the `check-json5` command and `--strict`
keep working through 3.x, and the `check-json5` package on PyPI now installs jsonvet.

For `.json`, `.json5`, `.geojson`, `.jsonld`, `.webmanifest` and `.babelrc` files, and in strict
mode, 2.0 gives the same pass or fail as 1.4.1, the same exit codes and the same duplicate-key
paths. The evidence, a comparison on 20,184 files, is in [docs/design.md](https://github.com/simonvanlierde/jsonvet/blob/main/docs/design.md#known-differences-from-14).
What changes is listed in the
[CHANGELOG](https://github.com/simonvanlierde/jsonvet/blob/main/CHANGELOG.md):

- each line gains the severity and rule, as in `error[duplicate-key]`, and syntax errors use
  jsonvet's own wording;
- the hook also checks `.jsonl` and `.ndjson` files;
- `.jsonc`, `.code-workspace` and `.code-snippets` files are now checked as JSONC, and `.hujson`
  and `.jwcc` files as JWCC. 1.4.1 read every file as JSON5, so JSON5-only syntax such as
  `{'a': 1, b: 2}` passed there and fails now. To keep the old verdict for such a file, pass
  `--dialect json5:GLOB` (or add a `dialects` entry to the config file); see [Dialects](#dialects);
- lines end at every JSON5 line terminator (LF, CR, CRLF, U+2028, U+2029), so positions after a
  lone CR or U+2028 differ;
- the importable Python module is gone; the wheel brings the `jsonvet` and `check-json5` commands.

## Speed

Median wall time of one run over the files, from `uv run scripts/bench.py` on 12 cores of a
Ryzen 7 H 255 under Linux. Both tools give the same verdict in every case:

| Case                       | check-json5 1.4.1 | jsonvet 2.0 | Speed-up |
| -------------------------- | ----------------: | ----------: | -------: |
| 1,100 config files         |             40 ms |     12.5 ms |       3x |
| 2.7 MB JSON                |             41 ms |      9.8 ms |       4x |
| 2.7 MB JSON, duplicate key |            330 ms |     11.6 ms |      29x |
| 120 KB JSONC               |             22 ms |      1.5 ms |      15x |
| 1.8 MB JSON5               |         11,641 ms |      8.3 ms |   1,398x |

1.4 sent JSON, and JSONC with its comments stripped, to Python's C parser, so those were
already fast. Any other JSON5 syntax went to a pure-Python parser, and so did the second pass
that finds a duplicate key's position. jsonvet does everything in one pass of one parser.

## How it is tested

- the [json5-tests](https://github.com/json5/json5-tests) suite, all 114 cases;
- a reviewed snapshot of edge cases in every dialect;
- a [conformance matrix](https://github.com/simonvanlierde/jsonvet/blob/main/docs/conformance.md) against 17 other parsers in four languages;
- property tests: no input makes it panic, every position lies on a character boundary and
  matches a naive recount;
- fuzzing with cargo-fuzz, against serde_json and the json5 crate as references;
- mutation testing with cargo-mutants: every surviving mutant is one that changes no behaviour;
- the file-by-file comparison with check-json5 1.4.1 above, in CI;
- CI on Linux, macOS and Windows, plus the oldest supported Rust (1.88).

### Other parsers on the same files

<!-- conformance:start -->

| dialect | files | share of files where each parser gives the expected verdict |
|---|---|---|
| JSON5 | 166 | `js-json5` 100%, `jsonvet-json5` 100%, `rs-json5` 100%, `py-json5` 99%, `py-pyjson5` 99%, `rs-serde-json5` 94%, `go-json5` 91% |
| JSONC | 9 | `js-jsonc-parser` 100%, `jsonvet-jsonc` 100%, `go-tidwall-jsonc` 89%, `js-strip-json-comments` 89%, `rs-jsonc-parser` 78% |
| JWCC (HuJSON) | 10 | `jsonvet-jwcc` 100%, `go-hujson` 90% |
| JSON Lines | 11 | `jsonvet-jsonl` 100%, `py-jsonlines` 82%, `go-json-stream` 64%, `rs-serde-json-stream` 64% |
| Strict JSON (baseline) | 129 | `js-json-parse` 100%, `jsonvet-json` 100%, `rs-serde-json` 98%, `py-json` 96% |

Run on 2026-10-14. The full grid, and where each expected verdict comes from: [docs/conformance.md](https://github.com/simonvanlierde/jsonvet/blob/main/docs/conformance.md).

<!-- conformance:end -->

## Development

See [CONTRIBUTING.md](https://github.com/simonvanlierde/jsonvet/blob/main/CONTRIBUTING.md) for
setup, the checks, snapshots, the conformance matrix, fuzzing and releases. In short: clone with
`--recurse-submodules`, run `just setup` once, and `just check` before a pull request.

## Credits

I (Simon van Lierde) started this as a fork of [check-json5](https://gitlab.com/bmares/check-json5)
by Ben Mares, which is based on the [pre-commit-hooks](https://github.com/pre-commit/pre-commit-hooks)
repository by Anthony Sottile and contributors. 2.0 is a rewrite in Rust.

## License

[MIT](https://github.com/simonvanlierde/jsonvet/blob/main/LICENSE), like the projects it started from
([Ben Mares](https://gitlab.com/bmares/check-json5/-/blob/main/LICENSE),
[pre-commit-hooks](https://github.com/pre-commit/pre-commit-hooks/blob/main/LICENSE)).
