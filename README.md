# jsonvet

[![CI](https://github.com/simonvanlierde/jsonvet/actions/workflows/ci.yml/badge.svg)](https://github.com/simonvanlierde/jsonvet/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/simonvanlierde/jsonvet/branch/main/graph/badge.svg)](https://codecov.io/gh/simonvanlierde/jsonvet)
[![CodSpeed](https://img.shields.io/endpoint?url=https://codspeed.io/badge.json)](https://app.codspeed.io/simonvanlierde/jsonvet?utm_source=badge)
[![PyPI](https://img.shields.io/pypi/v/jsonvet)](https://pypi.org/project/jsonvet/)
[![crates.io](https://img.shields.io/crates/v/jsonvet)](https://crates.io/crates/jsonvet)
[![Marketplace](https://img.shields.io/badge/marketplace-jsonvet-blue?logo=github)](https://github.com/marketplace/actions/jsonvet)
[![License: MIT](https://img.shields.io/badge/license-MIT-green)](https://github.com/simonvanlierde/jsonvet/blob/main/LICENSE)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/simonvanlierde/jsonvet/badge)](https://scorecard.dev/viewer/?uri=github.com/simonvanlierde/jsonvet)
[![OpenSSF Best Practices](https://www.bestpractices.dev/projects/15798/badge)](https://www.bestpractices.dev/projects/15798)
[![DOI](https://zenodo.org/badge/DOI/10.5281/zenodo.23461825.svg)](https://doi.org/10.5281/zenodo.23461825)

A linter for JSON, I-JSON, JSONC, HuJSON (JWCC), JSON5 and JSON Lines. It finds syntax errors and
duplicate keys, and tells you the exact line and column:

```text
config.json:4:5: error[duplicate-key] duplicate key $.server.port (first at 2:5)
package.json:2:14: error[syntax] missing `,`
events.jsonl:12:1: error[syntax] blank line; JSON Lines needs a value on every line
```

**Try it in your browser:** the [playground](https://simonvanlierde.github.io/jsonvet/) checks the
text you paste as you type. Nothing to install, and the text stays on your machine.

- **Six dialects, one parser.** Each file is checked in the dialect its extension implies.
- **JSON Schema with positions.** Each schema error points at its value's line and column.
  jsonvet never fetches anything from the network.
- **Fits CI.** A pre-commit or prek hook, a GitHub Action, a command or a Rust library, with text,
  SARIF or JSON output.
- **Fast.** It was check-json5, a Python hook, until 2.0 rewrote it in Rust. It is up to 1,395
  times faster than check-json5 1.4.1
  ([figures](https://github.com/simonvanlierde/jsonvet/blob/main/docs/design.md#speed)).

## Quick start

### Pre-commit or prek hook

Add this to `.pre-commit-config.yaml` (for [pre-commit](https://pre-commit.com) or
[prek](https://github.com/j178/prek)):

```yaml
- repo: https://github.com/simonvanlierde/jsonvet
  rev: v3.2.0
  hooks:
    - id: jsonvet
```

It checks files by name: `.json`, `.jsonc`, `.json5`, `.jsonl`, `.ndjson`, `.geojson`, `.jsonld`,
`.hujson`, `.jwcc`, `.code-workspace`, `.code-snippets`, `.webmanifest` and `.babelrc`.

Options go in `args`, such as `args: [--strict, package.json]` to hold `package.json` to strict
JSON. The hook installs a prebuilt binary from PyPI, so you need no Rust.

### GitHub Action

```yaml
- uses: simonvanlierde/jsonvet@v3
```

The step fails when a file has an error, and each problem shows up as an annotation on the pull
request. Before it installs jsonvet from PyPI, it checks that the wheel came from this
repository's release workflow. Its inputs are in the
[reference](https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md#github-action).

### Command line

```bash
uvx jsonvet config/*.json        # run it from PyPI without installing
uvx jsonvet .                    # check every JSON-like file under a folder
pip install jsonvet              # install it as a Python package
cargo install jsonvet-cli        # build it from source
cargo binstall jsonvet-cli       # download the binary from the GitHub release
```

Each [GitHub release](https://github.com/simonvanlierde/jsonvet/releases) also has the binary
for Linux, macOS and Windows, with checksums and an SBOM
([verifying a release](https://github.com/simonvanlierde/jsonvet/blob/main/SECURITY.md#verifying-a-release)).
A folder stands for the JSON-like files under it, skipping hidden and ignored files, and `-` reads
standard input. `jsonvet --help` lists every option, and the
[reference](https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md) has the details.

### Rust library

```rust
use jsonvet::{check, Config, Dialect};

let report = check("{\"a\": 1, \"a\": 2}", &Config::new(Dialect::Json));
for problem in report.problems() {
    let at = report.location(problem.offset());
    println!("{}:{}: {}[{}] {problem}", at.line, at.column, problem.severity(), problem.rule());
}
```

The API is on [docs.rs](https://docs.rs/jsonvet).

## Dialects

| Dialect  | Adds to JSON                                                                 | Default for                                                          |
| -------- | ---------------------------------------------------------------------------- | -------------------------------------------------------------------- |
| `json`   | nothing ([RFC 8259](https://www.rfc-editor.org/rfc/rfc8259))                  | none; choose it with `--dialect` or `--strict`                       |
| `i-json` | stricter content ([RFC 7493](https://www.rfc-editor.org/rfc/rfc7493))         | none                                                                 |
| `jsonc`  | `//` and `/* */` comments                                                    | `.jsonc`, `.code-workspace`, `.code-snippets`                        |
| `jwcc`   | comments and trailing commas ([HuJSON](https://github.com/tailscale/hujson)) | `.hujson`, `.jwcc`                                                   |
| `json5`  | the [JSON5 1.0.0](https://spec.json5.org/) grammar                            | `.json`, `.json5`, `.geojson`, `.jsonld`, `.webmanifest`, `.babelrc` |
| `jsonl`  | one JSON value per line ([JSON Lines](https://jsonlines.org/))               | `.jsonl`, `.ndjson`                                                  |

Any other extension is checked as JSON5. `.json` defaults to JSON5 because most `.json` files that
fail as JSON are config files with comments. To check a file as strict JSON, use
`--dialect json:package.json`.

## Rules

| Rule                    | Reports                                                        | Default                                          |
| ----------------------- | -------------------------------------------------------------- | ------------------------------------------------ |
| `syntax`                | the first syntax error; checking stops there                   | error                                            |
| `duplicate-key`         | a key repeated in one object, with both positions and its path | error                                            |
| `invalid-number`        | a number a double cannot hold, or an integer past ±(2⁵³ − 1)   | error in `i-json`, off elsewhere                 |
| `invalid-code-point`    | a lone surrogate or a noncharacter in a string                 | error in `i-json`, off elsewhere                 |
| `byte-order-mark`       | a byte order mark at the start of the file                     | warning in `json` and `i-json`, error in `jsonl` |
| `missing-final-newline` | a JSON Lines file that does not end with a newline             | warning in `jsonl`                               |

Change a rule's severity with `--error`, `--warn` or `--ignore`. Warnings fail a run only past
`--max-warnings N`.

## Configuration

To keep settings out of the command line, put them in `jsonvet.toml`, or in a `[tool.jsonvet]`
table in `pyproject.toml`:

```toml
rules = { byte-order-mark = "ignore", missing-final-newline = "error" }

# Checked in order; the last matching entry wins.
dialects = [
  { files = ".vscode/*.json", dialect = "jsonc" },
  { files = "package.json", dialect = "json" },
]

# Skipped when jsonvet walks a folder; files named on the command line are still checked.
exclude = ["node_modules", "tests/fixtures/*"]
```

Each file takes the nearest config file in its folder or a parent, up to the repository root, so
each package in a monorepo can have its own. Options on the command line win. The reference says
[where jsonvet looks](https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md#configuration-file)
and [how globs match](https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md#globs).

## JSON Schema

`--schema FILE` checks each file against one JSON Schema, and reports each error at its value:

```text
$ jsonvet --schema server.schema.json server.json5
server.json5:3:9: error[schema] "80" is not of type "integer" (at $.port)
server.json5:6:3: error[schema] Additional properties are not allowed ('debug' was unexpected) (at $.debug)
```

A config file can give files a schema, and so can a file's own `$schema` path:

```toml
schemas = [{ files = "config/server.json5", schema = "schemas/server.schema.json" }]
```

```json5
{ $schema: "../schemas/server.schema.json", port: 8080 }
```

Such a schema must stay inside the project. A `$schema` URL is skipped: jsonvet fetches nothing,
so every `$ref` must point inside the schema file. Schema errors are always errors. The rules for
schema paths and the limits are in the
[reference](https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md#json-schema-limits).

## GitHub code scanning

`--format sarif` prints a [SARIF 2.1.0](https://docs.oasis-open.org/sarif/sarif/v2.1.0/) document
for GitHub code scanning. The Action's `sarif` input writes one for you to upload:

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
keep working through 3.x.

jsonvet gives the same pass or fail as 1.4.1, apart from a few
[edge cases](https://github.com/simonvanlierde/jsonvet/blob/main/docs/design.md#known-differences-from-14).
One change can fail a file that passed: `.jsonc`, `.code-workspace` and `.code-snippets` files
are now checked as JSONC, and `.hujson` and `.jwcc` files as JWCC. To keep the old verdict, add
`--dialect json5:GLOB`.

## How it is tested

The tests run on Linux, macOS and Windows, with fuzzing, mutation testing and a comparison with
check-json5 1.4.1. A
[conformance matrix](https://github.com/simonvanlierde/jsonvet/blob/main/docs/conformance.md)
runs the same files against 18 other parsers in four languages:

<!-- conformance:start -->

| dialect | files | share of files where each parser gives the expected verdict |
|---|---|---|
| JSON5 | 166 | `js-json5` 100%, `jsonvet-json5` 100%, `rs-json5` 100%, `py-json5` 99%, `py-pyjson5` 99%, `rs-serde-json5` 94%, `go-json5` 91% |
| JSONC | 9 | `js-jsonc-parser` 100%, `jsonvet-jsonc` 100%, `go-tidwall-jsonc` 89%, `js-strip-json-comments` 89%, `rs-jsonc-parser` 78% |
| JWCC (HuJSON) | 10 | `jsonvet-jwcc` 100%, `go-hujson` 90% |
| JSON Lines | 11 | `jsonvet-jsonl` 100%, `py-jsonlines` 82%, `go-json-stream` 64%, `rs-serde-json-stream` 64% |
| Strict JSON (baseline) | 129 | `js-json-parse` 100%, `jsonvet-json` 100%, `rs-serde-json` 98%, `py-json` 96%, `py-jiter` 95% |

Run on 2026-10-28. The full grid, and where each expected verdict comes from: [docs/conformance.md](https://github.com/simonvanlierde/jsonvet/blob/main/docs/conformance.md).

<!-- conformance:end -->

## More

- [Reference](https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md),
  [design](https://github.com/simonvanlierde/jsonvet/blob/main/docs/design.md) and
  [CHANGELOG](https://github.com/simonvanlierde/jsonvet/blob/main/CHANGELOG.md).
- [CONTRIBUTING.md](https://github.com/simonvanlierde/jsonvet/blob/main/CONTRIBUTING.md),
  [ROADMAP.md](https://github.com/simonvanlierde/jsonvet/blob/main/ROADMAP.md) and
  [GOVERNANCE.md](https://github.com/simonvanlierde/jsonvet/blob/main/GOVERNANCE.md).

Found a file jsonvet gets wrong? Please
[open an issue](https://github.com/simonvanlierde/jsonvet/issues) with the file and the command.
If it crashes or hangs jsonvet, report it privately as
[SECURITY.md](https://github.com/simonvanlierde/jsonvet/blob/main/SECURITY.md) says.

To cite jsonvet, use the DOI
[10.5281/zenodo.23461825](https://doi.org/10.5281/zenodo.23461825) or GitHub's "Cite this
repository" button.

## Credits and license

I (Simon van Lierde) started this as a fork of [check-json5](https://gitlab.com/bmares/check-json5)
by Ben Mares, which is based on the [pre-commit-hooks](https://github.com/pre-commit/pre-commit-hooks)
repository by Anthony Sottile and contributors. It is
[MIT-licensed](https://github.com/simonvanlierde/jsonvet/blob/main/LICENSE), like both of them.
