# jsonvet

[![CI](https://github.com/simonvanlierde/jsonvet/actions/workflows/ci.yml/badge.svg)](https://github.com/simonvanlierde/jsonvet/actions/workflows/ci.yml)
[![PyPI](https://img.shields.io/pypi/v/jsonvet)](https://pypi.org/project/jsonvet/)
[![crates.io](https://img.shields.io/crates/v/jsonvet)](https://crates.io/crates/jsonvet)
[![codecov](https://codecov.io/gh/simonvanlierde/jsonvet/branch/main/graph/badge.svg)](https://codecov.io/gh/simonvanlierde/jsonvet)
[![License: MIT](https://img.shields.io/badge/license-MIT-green)](https://github.com/simonvanlierde/jsonvet/blob/main/LICENSE)
[![DOI](https://zenodo.org/badge/DOI/10.5281/zenodo.23461825.svg)](https://doi.org/10.5281/zenodo.23461825)

A linter for JSON, I-JSON, JSONC, HuJSON (JWCC), JSON5 and JSON Lines. It finds syntax errors,
duplicate keys and breaks of each dialect's rules, and reports each one with its file, line,
column and rule:

```text
config.json:4:5: error[duplicate-key] duplicate key $.server.port (first at 2:5)
package.json:3:3: error[syntax] expected `,` or `}`, found `"`
events.jsonl:12:1: error[syntax] blank line; JSON Lines needs a value on every line
```

- **Exact positions.** Columns count Unicode code points from 1, and a byte order mark is
  column 1. A duplicate key also gets its JSONPath and the position of the first copy.
- **Six dialects, one parser.** Each file is checked in the dialect its extension implies, and you
  can pick another per glob.
- **JSON Schema with positions.** With `--schema`, each schema error points at the line and column
  of the value, in every dialect.
- **Fits CI.** It runs as a pre-commit or prek hook, a GitHub Action with annotations and SARIF, a
  command, and a Rust library.

Up to 1.4 this was check-json5, a Python hook. 2.0 rewrote it in Rust with the same verdicts, and
it is 1,398 times faster on a 1.8 MB JSON5 file (see
[Coming from check-json5](#coming-from-check-json5)).

## Quick start

### As a pre-commit or prek hook

In `.pre-commit-config.yaml` (for [pre-commit](https://pre-commit.com) or
[prek](https://github.com/j178/prek)):

```yaml
- repo: https://github.com/simonvanlierde/jsonvet
  rev: v3.0.0
  hooks:
    - id: jsonvet
```

Options go in `args`. This holds `package.json` to strict JSON and checks the `.vscode` files as
JSONC:

```yaml
    - id: jsonvet
      args: [--strict, package.json, --dialect, "jsonc:.vscode/*.json"]
```

It checks files by name: `.json`, `.jsonc`, `.json5`, `.jsonl`, `.ndjson`, `.geojson`, `.jsonld`,
`.hujson`, `.jwcc`, `.code-workspace`, `.code-snippets`, `.webmanifest` and `.babelrc`.

The hook installs a prebuilt binary from PyPI, so you need no Rust. In GitHub Actions it also
turns each problem into an annotation on the pull request.

### As a GitHub Action

```yaml
- uses: simonvanlierde/jsonvet@v3
```

The step fails when a file has an error, and each problem becomes an annotation on the pull
request. It runs on Linux, macOS and Windows runners with no other setup. The inputs (`files`,
`args`, `sarif`, `version`) are all optional and listed in the
[reference](https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md#github-action).

### From the command line

```bash
uvx jsonvet config/*.json        # run it from PyPI without installing
pip install jsonvet              # install it as a Python package
cargo install jsonvet-cli        # build it from source
```

Exit status is 0 when no file has an error, 1 when one does, and 2 for a usage error.

### As a Rust library

```rust
use jsonvet::{check, Config, Dialect};

let report = check("{\"a\": 1, \"a\": 2}", &Config::new(Dialect::Json));
for problem in report.problems() {
    let at = report.location(problem.offset());
    println!("{}:{}: {}[{}] {problem}", at.line, at.column, problem.severity(), problem.rule());
}
```

The API is on [docs.rs](https://docs.rs/jsonvet).

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

How globs match, file names that look like options, and what happens to unreadable files are in
the [reference](https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md).

## Configuration

Settings can also live in `jsonvet.toml`, or in a `[tool.jsonvet]` table in `pyproject.toml`.
jsonvet looks in the working directory and its parents, up to the repository root. Options on the
command line win.

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

A severity is `error`, `warning` (or `warn`), or `ignore` (or `off`). Any other key than `rules`
and `dialects`, at the top or in a `dialects` entry, is a usage error, so a typo never goes
unnoticed.

## Dialects

Each file is checked in the dialect its extension implies. `--dialect` or the config file can pick
another. JSONC, JWCC and JSON5 each add syntax to the one before, except for where a `//` comment
ends: JWCC ends it at LF only, JSONC also at a lone CR, and JSON5 also at U+2028 and U+2029.

| Dialect  | Adds to JSON                                                                 | Default for                                                          |
| -------- | ---------------------------------------------------------------------------- | -------------------------------------------------------------------- |
| `json`   | nothing ([RFC 8259](https://www.rfc-editor.org/rfc/rfc8259))                  | none; choose it with `--dialect` or `--strict`                       |
| `i-json` | stricter content ([RFC 7493](https://www.rfc-editor.org/rfc/rfc7493))         | none                                                                 |
| `jsonc`  | `//` and `/* */` comments                                                    | `.jsonc`, `.code-workspace`, `.code-snippets`                        |
| `jwcc`   | comments and trailing commas ([HuJSON](https://github.com/tailscale/hujson)) | `.hujson`, `.jwcc`                                                   |
| `json5`  | the [JSON5 1.0.0](https://spec.json5.org/) grammar                            | `.json`, `.json5`, `.geojson`, `.jsonld`, `.webmanifest`, `.babelrc` |
| `jsonl`  | one JSON value per line ([JSON Lines](https://jsonlines.org/))               | `.jsonl`, `.ndjson`                                                  |

In `i-json`, lone surrogates, noncharacters and numbers that a double cannot hold are errors.

Any other extension is checked as JSON5. Extensions match in lower case only, so `DATA.JSONL` is
checked as JSON5, not JSON Lines.

`.json` defaults to JSON5 because most `.json` files that fail as JSON are config files with
comments. Check the files that a strict parser reads as JSON: `--dialect json:package.json`.

## Rules

| Rule                    | Reports                                                        | Default                                          |
| ----------------------- | -------------------------------------------------------------- | ------------------------------------------------ |
| `syntax`                | the first syntax error; checking stops there                   | error                                            |
| `duplicate-key`         | a key repeated in one object, with both positions and its path | error                                            |
| `invalid-number`        | a number a double cannot hold, or an integer past ±(2⁵³ − 1)   | error in `i-json`, off elsewhere                 |
| `invalid-code-point`    | a lone surrogate or a noncharacter in a string                 | error in `i-json`, off elsewhere                 |
| `byte-order-mark`       | a byte order mark at the start of the file                     | warning in `json` and `i-json`, error in `jsonl` |
| `missing-final-newline` | a JSON Lines file that does not end with a newline             | warning in `jsonl`                               |

Warnings fail a run only when there are more than `--max-warnings N`. A file that jsonvet cannot
read is reported under `unreadable`. That is not a rule you can set: it always fails the run.

## JSON Schema

`--schema FILE` checks each file's values against one JSON Schema. Each error is reported at the
line and column of its value, in every dialect, comments and JSON5 syntax included:

```text
$ jsonvet --schema server.schema.json server.json5
server.json5:3:9: error[schema] "80" is not of type "integer" (at $.port)
server.json5:6:3: error[schema] Additional properties are not allowed ('debug' was unexpected) (at $.debug)
```

In the hook, give each schema its own entry, with `files` naming the files it covers:

```yaml
    - id: jsonvet
      name: jsonvet (server schema)
      files: ^config/server\.json5$
      args: [--schema, schemas/server.schema.json]
```

Each message ends with the JSONPath of the value, written as duplicate-key paths are: `$` is the
root, `$.tags[1]` an array element, `$[""]` the empty key.

Schema errors are always errors. `--error`, `--warn`, `--ignore` and the config file do not take
`schema`, and exit 2 if you give it to them.

jsonvet fetches nothing: a `$ref` must point inside the schema file. A document lists at most 100
schema errors. These and the other limits (depth, memory, regexes, schema size) are in the
[reference](https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md#json-schema-limits).

## GitHub code scanning

`--format sarif` prints one [SARIF 2.1.0](https://docs.oasis-open.org/sarif/sarif/v2.1.0/)
document instead of text lines, for GitHub code scanning and other tools:

```bash
jsonvet --format sarif $(git ls-files '*.json') > jsonvet.sarif
```

Each problem is a result with its rule, severity, message and position. A duplicate key also
points at the first copy. The exit status and `--max-warnings` work as with text, and no
annotations are printed. GitHub keeps only 5,000 results of a run with more than 25,000. So
jsonvet writes the first 25,000, and counts the rest in a tool notification and on stderr.

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
mode, jsonvet gives the same pass or fail as 1.4.1, with the same exit codes and the same
duplicate-key paths. The comparison on 20,184 files and the speed figures are in
[docs/design.md](https://github.com/simonvanlierde/jsonvet/blob/main/docs/design.md#known-differences-from-14).

One change can fail a file that passed: `.jsonc`, `.code-workspace` and `.code-snippets` files are
now checked as JSONC, and `.hujson` and `.jwcc` files as JWCC. 1.4.1 read every file as JSON5, so
JSON5-only syntax such as `{'a': 1, b: 2}` passed there. To keep the old verdict, pass
`--dialect json5:GLOB` or add a `dialects` entry to the config file.

The other changes (the output line, the syntax-error wording, positions after a lone CR or U+2028,
the new `.jsonl` and `.ndjson` files, the removed Python module) are in the
[CHANGELOG](https://github.com/simonvanlierde/jsonvet/blob/main/CHANGELOG.md) under 2.0.0 and
3.0.0.

## How it is tested

jsonvet runs the json5-tests suite, a snapshot of edge cases in every dialect, property tests,
fuzzing, mutation testing and the comparison with check-json5 1.4.1, on Linux, macOS and Windows.
A [conformance matrix](https://github.com/simonvanlierde/jsonvet/blob/main/docs/conformance.md)
runs the same files against 17 other parsers in four languages. Details are in
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

## Documentation

- [Reference](https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md): every option,
  globs, exit status, the Action's inputs, and the JSON Schema limits.
- [Design](https://github.com/simonvanlierde/jsonvet/blob/main/docs/design.md): why the rules are
  as they are, the speed figures, the architecture and the tests.
- [Conformance](https://github.com/simonvanlierde/jsonvet/blob/main/docs/conformance.md): the full
  grid against the other parsers.
- [CHANGELOG](https://github.com/simonvanlierde/jsonvet/blob/main/CHANGELOG.md): what changed in
  each release, with upgrade notes.

## Help and contributing

Found a bug or a file jsonvet gets wrong? Please
[open an issue](https://github.com/simonvanlierde/jsonvet/issues) with the file, or the smallest
part of it that still fails, and the command you ran.

To work on jsonvet, clone it with `--recurse-submodules`, run `just setup` once, and run
`just check` before a pull request. Setup, the checks, snapshots, fuzzing and releases are in
[CONTRIBUTING.md](https://github.com/simonvanlierde/jsonvet/blob/main/CONTRIBUTING.md).

## Citing

If you use jsonvet in research, please cite it. The concept DOI
[10.5281/zenodo.23461825](https://doi.org/10.5281/zenodo.23461825) always resolves to the newest
release, and [CITATION.cff](https://github.com/simonvanlierde/jsonvet/blob/main/CITATION.cff) has
the details. GitHub's "Cite this repository" button reads it.

## Credits

I (Simon van Lierde) started this as a fork of [check-json5](https://gitlab.com/bmares/check-json5)
by Ben Mares, which is based on the [pre-commit-hooks](https://github.com/pre-commit/pre-commit-hooks)
repository by Anthony Sottile and contributors.

## License

[MIT](https://github.com/simonvanlierde/jsonvet/blob/main/LICENSE), like the projects it started from
([Ben Mares](https://gitlab.com/bmares/check-json5/-/blob/main/LICENSE),
[pre-commit-hooks](https://github.com/pre-commit/pre-commit-hooks/blob/main/LICENSE)).
