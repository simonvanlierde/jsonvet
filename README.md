# jsonvet

[![CI](https://github.com/simonvanlierde/jsonvet/actions/workflows/ci.yml/badge.svg)](https://github.com/simonvanlierde/jsonvet/actions/workflows/ci.yml)
[![PyPI](https://img.shields.io/pypi/v/jsonvet)](https://pypi.org/project/jsonvet/)
[![crates.io](https://img.shields.io/crates/v/jsonvet)](https://crates.io/crates/jsonvet)
[![codecov](https://codecov.io/gh/simonvanlierde/jsonvet/branch/main/graph/badge.svg)](https://codecov.io/gh/simonvanlierde/jsonvet)
[![License: MIT](https://img.shields.io/badge/license-MIT-green)](https://github.com/simonvanlierde/jsonvet/blob/main/LICENSE)
[![DOI](https://zenodo.org/badge/DOI/10.5281/zenodo.23461825.svg)](https://doi.org/10.5281/zenodo.23461825)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/simonvanlierde/jsonvet/badge)](https://scorecard.dev/viewer/?uri=github.com/simonvanlierde/jsonvet)

A linter for JSON, I-JSON, JSONC, HuJSON (JWCC), JSON5 and JSON Lines. It finds syntax errors and
duplicate keys, and tells you the exact line and column:

```text
config.json:4:5: error[duplicate-key] duplicate key $.server.port (first at 2:5)
package.json:3:3: error[syntax] expected `,` or `}`, found `"`
events.jsonl:12:1: error[syntax] blank line; JSON Lines needs a value on every line
```

- **Six dialects, one parser.** Each file is checked in the dialect its extension implies.
- **JSON Schema with positions.** `--schema` points each schema error at its value's line and
  column.
- **Fits CI.** Use it as a pre-commit or prek hook, a GitHub Action, a command or a Rust library.
- **Fast.** It was check-json5, a Python hook, until 2.0 rewrote it in Rust. It is up to 1,398
  times faster.

## Quick start

### Pre-commit or prek hook

Add this to `.pre-commit-config.yaml` (for [pre-commit](https://pre-commit.com) or
[prek](https://github.com/j178/prek)):

```yaml
- repo: https://github.com/simonvanlierde/jsonvet
  rev: v3.0.0
  hooks:
    - id: jsonvet
```

It checks files by name: `.json`, `.jsonc`, `.json5`, `.jsonl`, `.ndjson`, `.geojson`, `.jsonld`,
`.hujson`, `.jwcc`, `.code-workspace`, `.code-snippets`, `.webmanifest` and `.babelrc`.

Options go in `args`. This holds `package.json` to strict JSON:

```yaml
    - id: jsonvet
      args: [--strict, package.json]
```

The hook installs a prebuilt binary from PyPI, so you need no Rust.

### GitHub Action

```yaml
- uses: simonvanlierde/jsonvet@v3
```

The step fails when a file has an error, and each problem shows up as an annotation on the pull
request. It runs on Linux, macOS and Windows runners. Its optional inputs are in the
[reference](https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md#github-action).

### Command line

```bash
uvx jsonvet config/*.json        # run it from PyPI without installing
uvx jsonvet .                    # check every JSON-like file under a folder
pip install jsonvet              # install it as a Python package
cargo install jsonvet-cli        # build it from source
cargo binstall jsonvet-cli       # download the binary from the GitHub release
```

Each [GitHub release](https://github.com/simonvanlierde/jsonvet/releases) after 3.0.0 also has
the `jsonvet` binary for Linux (glibc and musl), macOS and Windows, on x86-64 and ARM64, as
`jsonvet-<target>.tar.gz` (`.zip` on Windows), with a `SHA256SUMS` file and a CycloneDX SBOM.
To check that a file came from this repository's release workflow, see
[SECURITY.md](https://github.com/simonvanlierde/jsonvet/blob/main/SECURITY.md#verifying-a-release).

A folder stands for the files under it with an extension or name from the dialect table below,
skipping hidden files and what `.gitignore` names, as ripgrep does. A file you name is always
checked. It exits 0 when no file has an error, 1 when one does, and 2 for a usage error.
`jsonvet --help` lists every option.

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

For each file, jsonvet uses the nearest one in the file's folder or a parent, up to the repository
root, so each package in a monorepo can have its own. With none, it uses your own
`~/.config/jsonvet/jsonvet.toml`, if you have one
([where](https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md#configuration-file)).
Options on the command line win. How globs
match is in the
[reference](https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md#globs).

## JSON Schema

`--schema FILE` checks each file against one JSON Schema, and reports each error at its value:

```text
$ jsonvet --schema server.schema.json server.json5
server.json5:3:9: error[schema] "80" is not of type "integer" (at $.port)
server.json5:6:3: error[schema] Additional properties are not allowed ('debug' was unexpected) (at $.debug)
```

In the hook, give each schema its own entry:

```yaml
    - id: jsonvet
      name: jsonvet (server schema)
      files: ^config/server\.json5$
      args: [--schema, schemas/server.schema.json]
```

Schema errors are always errors. jsonvet never fetches a schema from the network or another file,
so every `$ref` must point inside the schema file. The other limits are in the
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

For the files check-json5 knew, jsonvet gives the same pass or fail as 1.4.1, apart from a few
edge cases listed in
[docs/design.md](https://github.com/simonvanlierde/jsonvet/blob/main/docs/design.md#known-differences-from-14).
One change can fail a file that passed: `.jsonc`, `.code-workspace` and `.code-snippets` files are now
checked as JSONC, and `.hujson` and `.jwcc` files as JWCC. To keep the old verdict, add
`--dialect json5:GLOB`. The other changes are in the
[CHANGELOG](https://github.com/simonvanlierde/jsonvet/blob/main/CHANGELOG.md) under 2.0.0 and
3.0.0.

## How it is tested

jsonvet runs the json5-tests suite, edge-case snapshots, property tests, fuzzing, mutation testing
and a comparison with check-json5 1.4.1, on Linux, macOS and Windows. A
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

Run on 2026-10-19. The full grid, and where each expected verdict comes from: [docs/conformance.md](https://github.com/simonvanlierde/jsonvet/blob/main/docs/conformance.md).

<!-- conformance:end -->

## More

- [Reference](https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md): every option and
  limit in detail.
- [Design](https://github.com/simonvanlierde/jsonvet/blob/main/docs/design.md): why it works the
  way it does, with the speed figures.
- [CHANGELOG](https://github.com/simonvanlierde/jsonvet/blob/main/CHANGELOG.md): what changed in
  each release.
- [CONTRIBUTING.md](https://github.com/simonvanlierde/jsonvet/blob/main/CONTRIBUTING.md): how to
  build, test and release it.

Found a file jsonvet gets wrong? Please
[open an issue](https://github.com/simonvanlierde/jsonvet/issues) with the file and the command
you ran. If it crashes or hangs jsonvet, report it privately as
[SECURITY.md](https://github.com/simonvanlierde/jsonvet/blob/main/SECURITY.md) says.

To cite jsonvet, use the DOI
[10.5281/zenodo.23461825](https://doi.org/10.5281/zenodo.23461825) or GitHub's "Cite this
repository" button.

## Credits and license

I (Simon van Lierde) started this as a fork of [check-json5](https://gitlab.com/bmares/check-json5)
by Ben Mares, which is based on the [pre-commit-hooks](https://github.com/pre-commit/pre-commit-hooks)
repository by Anthony Sottile and contributors. It is
[MIT-licensed](https://github.com/simonvanlierde/jsonvet/blob/main/LICENSE), like both of them.
