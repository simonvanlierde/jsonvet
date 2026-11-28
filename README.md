# jsonvet

[![CI](https://github.com/simonvanlierde/jsonvet/actions/workflows/ci.yml/badge.svg)](https://github.com/simonvanlierde/jsonvet/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/simonvanlierde/jsonvet/branch/main/graph/badge.svg)](https://codecov.io/gh/simonvanlierde/jsonvet)
[![CodSpeed](https://img.shields.io/endpoint?url=https://codspeed.io/badge.json)](https://app.codspeed.io/simonvanlierde/jsonvet?utm_source=badge)
[![PyPI](https://img.shields.io/pypi/v/jsonvet)](https://pypi.org/project/jsonvet/)
[![crates.io](https://img.shields.io/crates/v/jsonvet)](https://crates.io/crates/jsonvet)
[![Marketplace](https://img.shields.io/badge/marketplace-jsonvet-blue?logo=github)](https://github.com/marketplace/actions/jsonvet)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-green)](#credits-and-license)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/simonvanlierde/jsonvet/badge)](https://scorecard.dev/viewer/?uri=github.com/simonvanlierde/jsonvet)
[![OpenSSF Best Practices](https://www.bestpractices.dev/projects/15798/badge)](https://www.bestpractices.dev/projects/15798)
[![DOI](https://zenodo.org/badge/DOI/10.5281/zenodo.23461825.svg)](https://doi.org/10.5281/zenodo.23461825)

A linter and formatter for JSON, I-JSON, JSONC, HuJSON (JWCC), JSON5 and JSON Lines. It finds
syntax errors and duplicate keys and reports the exact line and column.

[Try it in your browser](https://simonvanlierde.com/jsonvet/). Edit the sample, or paste your own
text to check, fix, or format it on your machine.

```text
config.json:4:5: error[duplicate-key] duplicate key $.server.port (first at 2:5)
package.json:2:14: error[syntax] missing `,`
events.jsonl:12:1: error[syntax] blank line; JSON Lines needs a value on every line
```

<!-- speed:start -->
On a 1.8 MB JSON5 file, jsonvet 4.0 took 7.1 ms and check-json5 1.4.1 took 14.9 s.
On 1,100 config files, they took 4.1 ms and 34 ms.
<!-- speed:end -->
See the [benchmark method and other results](https://github.com/simonvanlierde/jsonvet/blob/main/docs/design.md#speed).
If you use the old hook, see [how to switch](#coming-from-check-json5).

- **Six dialects, one parser.** The filename sets the default. Plain `.json` uses JSON5.
  For strict JSON, use `--dialect json:package.json`.
- **Fixes.** `--fix` repairs what has a safe fix, such as a stray byte order mark.
- **Formatting that keeps comments.** `jsonvet fmt` lays out all six dialects
  and leaves your comments where they are.
- **JSON Schema with positions.** Each schema error points at its value's line and column.
  Default runs stay offline; `--schema-catalog` opts into SchemaStore downloads.
- **Fits CI.** A pre-commit or prek hook, a GitHub Action, a command or a Rust library.
  The CLI writes text, SARIF, JSON, or GitLab Code Quality reports.

## Quick start

### Pre-commit or prek hook

Save this as `.pre-commit-config.yaml` for [pre-commit](https://pre-commit.com) or
[prek](https://github.com/j178/prek). If you have a config, add the repository entry to `repos`.

```yaml
repos:
  - repo: https://github.com/simonvanlierde/jsonvet
    rev: v4.0.0
    hooks:
      - id: jsonvet
```

Run `prek run jsonvet --all-files` or `pre-commit run jsonvet --all-files` to try the hook.
To format on commit, add `- id: jsonvet-format` as another hook in the same `hooks` list.
The formatter rewrites files. If it changes any, stage the result and commit again.
To apply safe fixes, add `args: [--fix]` to `jsonvet`. See
[Fixing](https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md#fixing).

It checks files by name: `.json`, `.jsonc`, `.json5`, `.jsonl`, `.ndjson`, `.geojson`, `.jsonld`,
`.hujson`, `.jwcc`, `.code-workspace`, `.code-snippets`, `.webmanifest` and `.babelrc`.

Options go in `args`, such as `args: [--dialect, 'json:package.json']` to hold `package.json` to strict
JSON. The hook installs a prebuilt binary from PyPI, so you need no Rust. On Linux the wheel needs glibc
2.28 or newer; older systems build from the source distribution, which needs Rust.

### GitHub Action

Add these steps to a job in your workflow.

```yaml
steps:
  - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
    with:
      persist-credentials: false
  - uses: simonvanlierde/jsonvet@v4
```

The step fails when a file has an error. The default text output also emits annotations, subject
to GitHub's display limits. On a runner with `gh`, the Action checks the wheel's attestation
before installing it by default. See the inputs and verification conditions in the
[reference](https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md#github-action).

To check formatting too, append these steps to the job's `steps` list.

```yaml
- uses: astral-sh/setup-uv@07a047e608db84b3d8c06cd9a7c7dcc9c355c18e # v10.3.2
- run: uvx jsonvet fmt --check .
```

To see the problems in GitHub code scanning, use the
[SARIF workflow](https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md#code-scanning).

### Trunk Code Quality

Use the [custom linter example](https://simonvanlierde.com/jsonvet/docs/usage.html#trunk-code-quality)
to run jsonvet from Trunk.

### Command line

```bash
uvx jsonvet .                    # check every JSON-like file under a folder, no install needed
uvx jsonvet --fix .              # also fix what has a safe fix, in place
uvx jsonvet fmt .                # format the files, keeping their comments
uvx jsonvet fmt --check .        # list files that need formatting; exit 1 if any
uv tool install jsonvet          # install it (or `pipx install jsonvet`)
cargo install jsonvet-cli        # build it from source
cargo binstall jsonvet-cli       # download the binary from the GitHub release
jsonvet lsp                      # the language server, for your editor
```

[uv](https://docs.astral.sh/uv/getting-started/installation/) provides `uvx`. Each
[GitHub release](https://github.com/simonvanlierde/jsonvet/releases) also has a binary
for Linux, macOS and Windows, with checksums and an SBOM
([verifying a release](https://github.com/simonvanlierde/jsonvet/blob/main/SECURITY.md#verifying-a-release)).
A folder stands for the JSON-like files under it, skipping hidden and ignored files, and `-` reads
standard input. The [reference](https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md#fixing)
covers `--fix`, [formatting](https://github.com/simonvanlierde/jsonvet/blob/main/docs/format.md)
covers `jsonvet fmt`, and [editors](https://github.com/simonvanlierde/jsonvet/blob/main/docs/editors.md)
covers `jsonvet lsp` in Neovim, Helix and other editors, and building the VS Code extension.
`jsonvet --help` lists every option, and the
[reference](https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md) has the details.

### Rust library

```bash
cargo add jsonvet
```

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
| `json`   | nothing ([RFC 8259](https://www.rfc-editor.org/rfc/rfc8259))                  | none; choose it with `--dialect json`                       |
| `i-json` | stricter content ([RFC 7493](https://www.rfc-editor.org/rfc/rfc7493))         | none                                                                 |
| `jsonc`  | `//` and `/* */` comments                                                    | `.jsonc`, `.code-workspace`, `.code-snippets`                        |
| `jwcc`   | comments and trailing commas ([HuJSON](https://github.com/tailscale/hujson)) | `.hujson`, `.jwcc`                                                   |
| `json5`  | the [JSON5 1.0.0](https://spec.json5.org/) grammar                            | `.json`, `.json5`, `.geojson`, `.jsonld`, `.webmanifest`, `.babelrc` |
| `jsonl`  | one JSON value per line ([JSON Lines](https://jsonlines.org/))               | `.jsonl`, `.ndjson`                                                  |

Any other extension is checked as JSON5. `.json` defaults to JSON5 because most `.json` files that
fail as JSON are config files with comments. A first-line [JSONC modeline](docs/usage.md#jsonc-modelines)
selects JSONC. To check a file as strict JSON, use `--dialect json:package.json`.

See the [JSON5 suite](https://github.com/simonvanlierde/jsonvet/blob/main/crates/jsonvet/tests/conformance.rs)
and [scored JSON5 cases](https://simonvanlierde.com/jsonvet/docs/conformance.html#json5).
The [rule tests](https://github.com/simonvanlierde/jsonvet/blob/main/crates/jsonvet/tests/cases.rs)
include I-JSON.

## Rules

| Rule                    | Reports                                                           | Default                                          |
| ----------------------- | ----------------------------------------------------------------- | ------------------------------------------------ |
| `syntax`                | the first syntax error; checking stops there                      | error                                            |
| `duplicate-key`         | a key repeated in one object, with both positions and its path    | error                                            |
| `invalid-number`        | a number a double cannot hold, or an integer past ±(2⁵³ − 1)      | error in `i-json`, off elsewhere                 |
| `invalid-code-point`    | a lone surrogate or a noncharacter in a string                    | error in `i-json`, off elsewhere                 |
| `byte-order-mark`       | a byte order mark at the start of the file                        | warning in `json` and `i-json`, error in `jsonl` |
| `missing-final-newline` | a JSON Lines file that does not end with a newline                | warning in `jsonl`                               |
| `mixed-line-endings`    | a line ending that differs from the file's first one              | warning                                          |
| `unused-suppression`    | a `jsonvet-ignore` comment that hides nothing                     | warning in `jsonc`, `jwcc` and `json5`           |
| `unreadable`            | a file that is missing, not a regular file, or not readable UTF-8 | error, always                                    |
| `schema`                | a value the file's JSON Schema rejects                            | error, always                                    |

Change a rule's severity with `--error`, `--warn` or `--ignore`; `syntax`, `unreadable` and
`schema` stay errors, whatever a setting says. In `jsonc`, `jwcc` and `json5`, a comment
`// jsonvet-ignore duplicate-key` turns a rule off for the next line. Warnings fail a run only past
`--max-warnings N`.

## Configuration

To keep settings out of the command line, put them in `jsonvet.toml`, or in a `[tool.jsonvet]`
table in `pyproject.toml`:

```toml
rules = { byte-order-mark = "ignore", missing-final-newline = "error" }

# For the files a glob matches, after `rules`; the last matching entry wins.
per-file-rules = [{ files = "legacy/*.json", duplicate-key = "ignore" }]

# Checked in order; the last matching entry wins.
dialects = [
  { files = ".vscode/*.json", dialect = "jsonc" },
  { files = "package.json", dialect = "json" },
]

# Skipped when jsonvet walks a folder; files named on the command line are still checked.
exclude = ["node_modules", "tests/fixtures/*"]

# Used by `jsonvet fmt`.
[format]
indent = 4
width = 100

[[per-file-format]]
files = "generated/*.json"
indent = 2
```

Each file takes the nearest config file in its folder or a parent, up to the repository root, so
each package in a monorepo can have its own. Options on the command line win. The reference says
[where jsonvet looks](https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md#configuration-file)
and [how globs match](https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md#globs).
[Formatting](https://github.com/simonvanlierde/jsonvet/blob/main/docs/format.md#settings) lists the
`[format]` settings and how `.editorconfig` fits in.

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

Such a schema must stay inside the project. A `$schema` URL is skipped, and every `$ref` must
point inside the schema file. `--schema-catalog` enables SchemaStore lookup by file name when
no local schema applies, with downloads cached for seven days. Schema errors are always errors. The rules for
schema paths and the limits are in the
[reference](https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md#json-schema-limits).

## Coming from check-json5

Use the `jsonvet` package, command and hook ID in 4.0. See the
[migration guide](https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md#coming-from-check-json5)
for hook and flag changes.

jsonvet gives the same pass or fail as 1.4.1, apart from a few
[edge cases](https://github.com/simonvanlierde/jsonvet/blob/main/docs/design.md#known-differences-from-14).
One change can fail a file that passed: `.jsonc`, `.code-workspace` and `.code-snippets` files
are now checked as JSONC, and `.hujson` and `.jwcc` files as JWCC. To keep the old JSON5 check for
`.jsonc` files, pass the dialect override in the hook's arguments.

```yaml
- id: jsonvet
  args: [--dialect, 'json5:*.jsonc']
```

## How it is tested

The tests run on Linux, macOS and Windows, with fuzzing, mutation testing and a comparison with
check-json5 1.4.1. The [conformance report](https://simonvanlierde.com/jsonvet/docs/conformance.html)
runs the same files against 18 other parsers in four languages. Its 27 runs include jsonvet and
the duplicate-key variants. Use the [interactive matrix](https://simonvanlierde.com/jsonvet/docs/matrix.html)
to see what each parser did on an individual file.

## Development

You need [Rust](https://rustup.rs) and [just](https://just.systems).

```sh
git submodule update --init --quiet
cargo test --workspace
```

See [CONTRIBUTING.md](https://github.com/simonvanlierde/jsonvet/blob/main/CONTRIBUTING.md) for
the full check and lint suite.

## More

- [Reference](https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md),
  [formatting](https://github.com/simonvanlierde/jsonvet/blob/main/docs/format.md),
  [editors](https://github.com/simonvanlierde/jsonvet/blob/main/docs/editors.md),
  [design](https://github.com/simonvanlierde/jsonvet/blob/main/docs/design.md),
  [architecture](https://github.com/simonvanlierde/jsonvet/blob/main/docs/architecture.md),
  [accessibility](https://github.com/simonvanlierde/jsonvet/blob/main/docs/accessibility.md) and
  [CHANGELOG](https://github.com/simonvanlierde/jsonvet/blob/main/CHANGELOG.md).
- The same docs as web pages: <https://simonvanlierde.com/jsonvet/docs/>.
- [CONTRIBUTING.md](https://github.com/simonvanlierde/jsonvet/blob/main/CONTRIBUTING.md),
  [roadmap](https://github.com/simonvanlierde/jsonvet/blob/main/docs/roadmap.md) and
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
repository by Anthony Sottile and contributors, both MIT-licensed.

Licensed under either of [Apache License, Version 2.0](https://github.com/simonvanlierde/jsonvet/blob/main/LICENSE-APACHE) or [MIT license](https://github.com/simonvanlierde/jsonvet/blob/main/LICENSE-MIT)
at your option. Ben Mares's copyright notice stays in the MIT text.

Unless you explicitly state otherwise, any contribution you intentionally submit for inclusion in
jsonvet, as defined in the Apache-2.0 license, is dual licensed as above, without any additional
terms or conditions.
