# jsonvet

[![CI](https://github.com/simonvanlierde/jsonvet/actions/workflows/ci.yml/badge.svg)](https://github.com/simonvanlierde/jsonvet/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/simonvanlierde/jsonvet/branch/main/graph/badge.svg)](https://codecov.io/gh/simonvanlierde/jsonvet)
[![pre-commit](https://img.shields.io/badge/pre--commit-enabled-brightgreen?logo=pre-commit&logoColor=white)](https://github.com/pre-commit/pre-commit)
[![License: MIT](https://img.shields.io/badge/license-MIT-green)](https://github.com/simonvanlierde/jsonvet/blob/main/LICENSE)
[![release](https://img.shields.io/github/v/release/simonvanlierde/jsonvet)](https://github.com/simonvanlierde/jsonvet/releases/latest)

jsonvet checks JSON, I-JSON, JSONC, HuJSON (JWCC), JSON5 and JSON Lines files for syntax errors,
duplicate keys and the rules of each dialect. Each problem comes with its file, line, column and
rule. Columns count Unicode code points from 1, and a byte order mark is column 1. A duplicate
key also gets its JSONPath and the position of the first copy:

```text
config.json:4:5: error[duplicate-key] duplicate key $.server.port (first at 2:5)
package.json:3:3: error[syntax] expected `,` or `}`, found `"`
events.jsonl:12:1: error[syntax] blank line; JSON Lines needs a value on every line
```

A file jsonvet cannot read gets a line with no position and no rule, such as
`gone.json: error: no such file`; in SARIF it is a result under the rule `unreadable`.

It runs as a pre-commit or prek hook, a GitHub Action, a command and a Rust library. Up to 1.4 it
was check-json5, a Python hook. 2.0 rewrote it in Rust with the same verdicts (see
[Coming from check-json5](#coming-from-check-json5)).

## Use it as a hook

In `.pre-commit-config.yaml` (pre-commit or [prek](https://github.com/j178/prek)):

```yaml
- repo: https://github.com/simonvanlierde/jsonvet
  rev: v3.0.0
  hooks:
    - id: jsonvet
```

Options go in `args`. This holds `package.json` to strict JSON and checks `.vscode` files as
JSONC:

```yaml
    - id: jsonvet
      args: [--strict, package.json, --dialect, "jsonc:.vscode/*.json"]
```

It checks files by name: `.json`, `.jsonc`, `.json5`, `.jsonl`, `.ndjson`, `.geojson`, `.jsonld`,
`.hujson`, `.jwcc`, `.code-workspace`, `.code-snippets`, `.webmanifest` and `.babelrc`.

The hook and the Action install a prebuilt binary from PyPI, so they need no Rust. That binary
has no [`--schema`](#json-schema). In GitHub Actions each problem also becomes an annotation on
the pull request.

## Use it as a GitHub Action

```yaml
- uses: simonvanlierde/jsonvet@v3
```

The step fails when a file has an error. It runs on Linux, macOS and Windows runners with no other
setup. All inputs are optional:

| Input     | What it does                                                                        |
| --------- | ----------------------------------------------------------------------------------- |
| `files`   | paths or globs to check, separated by spaces; by default every tracked file the hook checks |
| `args`    | extra options, separated by spaces, such as `--strict package.json`                 |
| `sarif`   | also write a SARIF file to this path (see [GitHub code scanning](#github-code-scanning)) |
| `version` | the jsonvet version to install: `X.Y.Z` (also with a suffix such as `rc1` or `.post1`), `X.*`, `X.Y.*` or `latest` |

In `files`, `**` matches folders at any depth on every runner. The Action skips symlinks, and by
default also submodules. Any other `version` value stops the step.

By default the Action installs the release that matches its own tag: `@v3` gets the newest 3.x,
`@v3.0.0` gets 3.0.0. Pinned to a commit SHA or a branch, it installs the release named in that
commit's `Cargo.toml`. Between releases that is the last release, not the code in the commit.
The step log names the version it installed. Pinning the Action does not pin the wheel's bytes:
pip installs `jsonvet==X.Y.Z` from PyPI without a hash check.

## Use it from the command line

```bash
uvx jsonvet config/*.json        # from PyPI, without installing; no --schema
pip install jsonvet              # as a Python package; no --schema
cargo install jsonvet-cli        # from source, with --schema
```

The Rust library is the `jsonvet` crate:

```rust
use jsonvet::{check, Config, Dialect};

let report = check("{\"a\": 1, \"a\": 2}", &Config::new(Dialect::Json));
for problem in report.problems() {
    let at = report.location(problem.offset());
    println!("{}:{}: {}[{}] {problem}", at.line, at.column, problem.severity(), problem.rule());
}
```

## Options

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

A glob without `/` matches the file name at any depth. A glob with `/` matches the whole path,
and `*` also matches `/`. A glob compares exact code points, with no Unicode normalization, and
a `\` in a path is read as `/`. A glob on the command line matches the path as you typed it, so
it is relative to the working directory. pre-commit runs from the repository root.

pre-commit passes file names with no `--` before them. So an unknown `-x` or `--name`, or a known
option with a value such as `--dialect=json`, is a file name when a file of exactly that name
exists. Otherwise an unknown one is a usage error (`unknown option`). Put `--` before the file
names to be sure: `jsonvet -- --x.json`.

Exit status: 0 when no file has an error, 1 when one does, 2 for a usage error. With no files,
jsonvet exits 0. A file that does not exist or cannot be read prints `name: error: <reason>`, with
no line or column, and makes the run exit 1. jsonvet reads regular files only, and follows a
symlink to one. A directory, device, FIFO or socket prints `error: not a regular file`. The hook
(`types: [text]`) skips tracked symlinks and submodules. When the output pipe closes early, as in
`jsonvet … | head -1`, jsonvet stops quietly and exits with the verdict for the files it has
checked.

## Configuration

Settings can also live in `jsonvet.toml`, or in a `[tool.jsonvet]` table in `pyproject.toml`.
jsonvet looks in the working directory and its parents up to the repository root. The command
line wins.

```toml
rules = { byte-order-mark = "ignore", missing-final-newline = "error" }

# Checked in order; the last matching entry wins.
dialects = [
  { files = ".vscode/*.json", dialect = "jsonc" },
  { files = "package.json", dialect = "json" },
]
```

A glob in a config file is relative to the directory that holds the file. So `data/*.json` in the
root's `jsonvet.toml` matches `data/x.json` when jsonvet runs in the root, in `data/`, or with an
absolute path. A file outside that directory matches none of its globs.

A severity is `error`, `warning` (or `warn`), or `ignore` (or `off`). Any key other than `rules`
and `dialects`, at the top or in a `dialects` entry, is a usage error, so a typo never goes
unnoticed.

## Dialects

Each file is checked in the dialect its extension implies. `--dialect` or the config file can pick
another. JSONC, JWCC and JSON5 each add syntax to the one before. The exception is where a `//`
comment ends: JWCC ends it at LF only, JSONC also at a lone CR, and JSON5 also at U+2028 and
U+2029.

| Dialect  | Adds to JSON                                                                 | Default for                                                        |
| -------- | ---------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| `json`   | nothing ([RFC 8259](https://www.rfc-editor.org/rfc/rfc8259))                  | none; choose it with `--dialect` or `--strict`                     |
| `i-json` | stricter content ([RFC 7493](https://www.rfc-editor.org/rfc/rfc7493)): lone surrogates, noncharacters and numbers a double cannot hold are errors | none |
| `jsonc`  | `//` and `/* */` comments                                                    | `.jsonc`, `.code-workspace`, `.code-snippets`                      |
| `jwcc`   | comments and trailing commas ([HuJSON](https://github.com/tailscale/hujson)) | `.hujson`, `.jwcc`                                                 |
| `json5`  | the [JSON5 1.0.0](https://spec.json5.org/) grammar                            | `.json`, `.json5`, `.geojson`, `.jsonld`, `.webmanifest`, `.babelrc` |
| `jsonl`  | one JSON value per line ([JSON Lines](https://jsonlines.org/))               | `.jsonl`, `.ndjson`                                                |

Any other extension is checked as JSON5. Extensions match in lower case only, so `DATA.JSONL`
is checked as JSON5, not JSON Lines.

`.json` defaults to JSON5 because most `.json` files that fail as JSON are config files with
comments. Hold the files a strict parser reads to JSON: `--dialect json:package.json`.

## Rules

| Rule                    | Reports                                                       | Default                                          |
| ----------------------- | ------------------------------------------------------------- | ------------------------------------------------ |
| `syntax`                | the first syntax error; checking stops there                  | error                                            |
| `duplicate-key`         | a key repeated in one object, with both positions and its path | error                                            |
| `invalid-number`        | a number a double cannot hold, or an integer past ±(2⁵³ − 1)   | error in `i-json`, off elsewhere                 |
| `invalid-code-point`    | a lone surrogate or a noncharacter in a string                | error in `i-json`, off elsewhere                 |
| `byte-order-mark`       | a byte order mark at the start of the file                    | warning in `json` and `i-json`, error in `jsonl` |
| `missing-final-newline` | a JSON Lines file that does not end with a newline            | warning in `jsonl`                               |

Warnings fail a run only when there are more than `--max-warnings N`. A file jsonvet cannot read
is reported under `unreadable`. That is not a rule you can set: it always fails the run.

jsonvet escapes control, bidirectional and invisible characters in file names and messages, so
no output line can drive a terminal or pass for a GitHub workflow command. The exact forms are in
[docs/design.md](https://github.com/simonvanlierde/jsonvet/blob/main/docs/design.md#output).

## JSON Schema

`--schema FILE` checks each file's values against one JSON Schema. Each error is reported at the
line and column of its value, in every dialect, with comments and JSON5 syntax:

```text
$ jsonvet --schema server.schema.json server.json5
server.json5:3:9: error[schema] "80" is not of type "integer" (at $.port)
server.json5:6:3: error[schema] Additional properties are not allowed ('debug' was unexpected) (at $.debug)
```

It comes with `cargo install jsonvet-cli` only. The PyPI wheels, and so the hook and the Action,
leave it out: it makes the binary about 5 MB instead of 0.6 MB. There `--schema` exits with status
2 and says why. The exit status is the contract, not the wording.

Each message ends with the JSONPath of the value, written as duplicate-key paths are: `$` is the
root, `$.tags[1]` an array element, `$[""]` the empty key. The rule id `schema` and the position
are stable. The message wording comes from the jsonschema crate and may change between releases.

Schema errors are always errors, in text, annotations and SARIF. `--error`, `--warn`, `--ignore`
and the config file do not take `schema`, and exit 2 if given it. The limits:

- **Errors.** A document lists at most 100 schema errors: the first 100 jsonschema finds, which
  takes an object's keys in sorted order, not file order. They print in file order. Then one more
  line, `... and N more schema errors in this document`, sits at the document's start. SARIF
  leaves that line out. `additionalProperties` gives one error per key it does not allow, at
  that key, and each counts toward the 100.
- **Messages.** A message quotes the value unless that makes it long; then it says `value`. A
  message, with its path, is cut at 200 bytes and ends in `...`.
- **Depth.** With `--schema`, a file nested deeper than 512 levels is a syntax error. A file with
  a syntax error gets no schema errors.
- **Values.** Each line of a JSON Lines file is checked on its own. `NaN`, `Infinity` and numbers
  past a double's range have no JSON Schema value, so a document holding one is reported once and
  not checked. So is a document with two keys in one object that differ only in lone surrogates
  such as `"\ud800"`, since both become U+FFFD. In the schema file, either is a usage error.
- **Memory.** jsonvet holds each file's values in memory: 14 to 27 times the file's size, the
  most for many small records such as JSON Lines. Without `--schema`, memory stays near the
  file's size.
- **Regexes.** A pattern with a back-reference or lookaround may backtrack 5,000 times per string.
  A key or string that needs more is an error at that key or string: the pattern could not be
  checked. jsonvet tries each `patternProperties` pattern on every key, and each `pattern` on
  every string, so this error can name a string the pattern does not apply to.
- **The schema file.** It may be in any dialect. Its `$schema` picks the draft, from draft 4 to
  2020-12. A `$ref` must point inside the file: jsonvet fetches nothing, from the network or from
  disk. A schema jsonvet cannot read or use is a usage error (exit 2). That covers a `$ref` to
  another file, and nesting deeper than 128 levels, where each `$ref` followed counts as a level.
  It also covers a schema of more than 5,000,000 objects and arrays once each `$ref` is replaced
  by what it points at, every time it is followed. SchemaStore's 1.6 MB
  `azure-pipelines.json` comes to about 2,560,000. A `$ref` back to a schema it is inside, as in
  `{"items": {"$ref": "#"}}`, is recursion and is fine: it is not followed again.

Why each limit exists: [docs/design.md](https://github.com/simonvanlierde/jsonvet/blob/main/docs/design.md#rules).

## GitHub code scanning

`--format sarif` prints one [SARIF 2.1.0](https://docs.oasis-open.org/sarif/sarif/v2.1.0/)
document instead of text lines, for GitHub code scanning and other tools. Each problem is a result
with its rule, severity, message and position, and a duplicate key also points at the first copy.
A file jsonvet cannot read is a result under the rule `unreadable`. The exit status and
`--max-warnings` work as with text, and no annotations are printed. GitHub keeps only 5,000
results of a run with more than 25,000, so jsonvet writes the first 25,000 and counts the rest in
a tool notification and on stderr.

```bash
jsonvet --format sarif $(git ls-files '*.json') > jsonvet.sarif
```

The Action's `sarif` input writes that file as well as the usual log and annotations. Upload it
with `github/codeql-action/upload-sarif`, which needs `security-events: write`:

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

## Coming from check-json5

Bump `rev` to the newest tag. The `check-json5` hook id, the `check-json5` command and `--strict`
keep working through 3.x, and the `check-json5` package on PyPI now installs jsonvet.

For `.json`, `.json5`, `.geojson`, `.jsonld`, `.webmanifest` and `.babelrc` files, and in strict
mode, jsonvet gives the same pass or fail as 1.4.1, the same exit codes and the same duplicate-key
paths. The evidence, a comparison on 20,184 files, and the speed figures are in
[docs/design.md](https://github.com/simonvanlierde/jsonvet/blob/main/docs/design.md#known-differences-from-14).

One change can fail a file that passed: `.jsonc`, `.code-workspace` and `.code-snippets` files
are now checked as JSONC, and `.hujson` and `.jwcc` files as JWCC. 1.4.1 read every file as
JSON5, so JSON5-only syntax such as `{'a': 1, b: 2}` passed there. To keep the old verdict, pass
`--dialect json5:GLOB` or add a `dialects` entry to the config file. The output line, the
syntax-error wording, positions after a lone CR (or after U+2028 in JSON5), the new `.jsonl` and
`.ndjson` files and the removed Python module are in the
[CHANGELOG](https://github.com/simonvanlierde/jsonvet/blob/main/CHANGELOG.md) under 2.0.0 and
3.0.0.

## How it is tested

jsonvet runs the json5-tests suite, a snapshot of edge cases in every dialect, property tests,
fuzzing, mutation testing and the comparison with check-json5 1.4.1, on Linux, macOS and Windows.
A [conformance matrix](https://github.com/simonvanlierde/jsonvet/blob/main/docs/conformance.md)
runs the same files against 17 other parsers in four languages. Details:
[docs/design.md](https://github.com/simonvanlierde/jsonvet/blob/main/docs/design.md#testing).

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

Clone with `--recurse-submodules`, run `just setup` once, and run `just check` before a pull
request. Setup, the checks, snapshots, fuzzing and releases are in
[CONTRIBUTING.md](https://github.com/simonvanlierde/jsonvet/blob/main/CONTRIBUTING.md).

## Credits

I (Simon van Lierde) started this as a fork of [check-json5](https://gitlab.com/bmares/check-json5)
by Ben Mares, which is based on the [pre-commit-hooks](https://github.com/pre-commit/pre-commit-hooks)
repository by Anthony Sottile and contributors.

## License

[MIT](https://github.com/simonvanlierde/jsonvet/blob/main/LICENSE), like the projects it started from
([Ben Mares](https://gitlab.com/bmares/check-json5/-/blob/main/LICENSE),
[pre-commit-hooks](https://github.com/pre-commit/pre-commit-hooks/blob/main/LICENSE)).
