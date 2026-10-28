# jsonvet design

What jsonvet promises, and why it is built the way it is. How to use it is in the
[reference](usage.md). Each choice where we turned down a measured or argued alternative has a
record in [decisions/](decisions/README.md), with the numbers. Items marked *since 3.0* changed
after 2.0.0.

## The contract with 1.4

jsonvet 2.0 replaced check-json5 1.4 (Python) in place, with one hand-written parser
([0001](decisions/0001-one-parser-and-the-1.4-contract.md)). These stay the same:

- the verdict for every file in the two dialects 1.4 knows: JSON5 (the default for `.json`,
  `.json5`, `.geojson`, `.jsonld`, `.webmanifest`, `.babelrc` and any other extension) and strict
  JSON through `--strict`;
- the exit code: 0 when every file passes, 1 when any fails, 2 on a usage error;
- a duplicate-key report: the position of the repeated key and its JSONPath;
- `--strict GLOB`, now an alias for `--dialect json:GLOB`, and `--strict '!GLOB'` for
  `--dialect auto:GLOB`.

These change, and the CHANGELOG marks them **Breaking**:

- syntax errors use jsonvet's own wording;
- the output line gains a severity and a rule id;
- the wheel has no importable Python module, only the commands;
- a line also ends at a lone CR, and in JSON5 at U+2028 and U+2029, so positions after one differ;
- *since 3.0*, a duplicate-key JSONPath over 256 bytes keeps the segments that fit in 128 bytes at
  each end, with `…` between them, so it is at most 259 bytes;
- *since 3.0*, a quoted key in a JSONPath keeps non-ASCII text as written (`$["日本"]`). Control
  and invisible characters, `…` and lone surrogates stay escaped;
- *since 3.0*, a missing file whose name starts with `-` is an unknown option and exits 2;
- the hook also checks `.jsonl` and `.ndjson` files;
- `.jsonc`, `.code-workspace` and `.code-snippets` files are checked as JSONC, and `.hujson` and
  `.jwcc` files as JWCC, so JSON5-only syntax in them now fails.

A syntax error points at the first character that cannot continue the document, or at the end of
the input. Two point earlier: a missing `,` points just after the value before it, and a number
with a leading zero at that zero. A word such as `True` is named whole, at its first letter. Any
difference from 1.4 that is not listed below is a bug.

### Known differences from 1.4

`scripts/differential.py` compares jsonvet with check-json5 1.4.1 file by file. These differences
are intended, besides the line ends and paths above:

- Raw U+2028 and U+2029 inside strings pass, as the JSON5 spec allows. 1.4 rejected them.
- Nesting deeper than 10,000 levels is a syntax error (`Config::max_depth` changes it). 1.4
  stopped at 9,998 levels, or 9,994 with JSON5 syntax, so only files 9,995 to 10,000 levels deep
  differ.
- A `\u` escape in an unquoted key must stand for a character the key could hold literally, as
  ECMAScript 5.1 says. 1.4 passed any escape, such as `{\u0301: 1}` or `{\ud800: 1}`. Literal
  characters agree: both go by general category, as the spec and the JS reference do, not by
  Unicode's `ID_Start`.
- After a duplicate key, jsonvet also reports a later syntax error. 1.4 stops at the duplicates.
  The verdict is the same.

## Speed

<!-- speed:start -->
Median of 40 runs from `uv run scripts/bench.py`, on 12 cores of a Ryzen 7 H 255 under Linux
(2026-10-28), and of 3 for 1.0.1. Every tool gives the same verdict in every case:

| Case                       | check-json5 1.0.1 | check-json5 1.4.1 | jsonvet 3.2 | Speed-up over 1.4.1 | over 1.0.1 |
| -------------------------- | ----------------: | ----------------: | ----------: | ------------------: | ---------: |
| 1,100 config files         |          1,828 ms |             40 ms |      4.3 ms |                  9x |       423x |
| 2.7 MB JSON                |         10,076 ms |             41 ms |      9.2 ms |                  4x |     1,095x |
| 2.7 MB JSON, duplicate key |         10,071 ms |            322 ms |     11.8 ms |                 27x |       851x |
| 120 KB JSONC               |            498 ms |             20 ms |      2.2 ms |                  9x |       231x |
| 1.8 MB JSON5               |         11,407 ms |         11,541 ms |      8.3 ms |              1,395x |     1,379x |
<!-- speed:end -->

1.4 sent strict JSON and JSONC to Python's C parser, so those were already fast, and everything
else through pure Python. 1.0.1, the original hook that pre-commit configs pin from GitHub, sent
every file through pure Python. jsonvet reads each file once with one parser, and checks files on every
core.

A JSON Lines file of 1 MB or more is cut at newlines into parts, one for each core that is free,
and the parts are checked at once. That is safe because every raw newline ends a record: a JSON
string must escape it, and JSON Lines has no comments. Each part counts offsets from the start of
the file and stops at its first syntax error, so the output matches one pass. With `--schema`,
each line is its own document and the error cap counts per document, so the parts' errors joined
in order are the ones one pass finds.

A speed-up must measure faster with the same output, and at least 5% faster end to end if it
adds a dependency, `unsafe` or a slower build
([0004](decisions/0004-measure-before-adopting-a-speed-up.md)).

## Dialects

One parser covers six dialects. What each accepts is in the [README](../README.md#dialects) and
the [reference](usage.md#dialects). Three choices behind them:

- `.json` defaults to JSON5, as in 1.4.
- JSON5 also ends lines at U+2028 and U+2029, because its spec counts them as line terminators.
- JSONC rejects trailing commas, with no option to allow them. The JSONC spec says parsers "MAY"
  accept them, and its reference parser and prek's `check-jsonc` reject them by default. Files
  whose tools accept them are JWCC.

## Rules

Every problem belongs to a rule with a default severity per dialect
([README](../README.md#rules)). The I-JSON rules leave alone the two cases RFC 7493 leaves open: a
number that underflows to zero (`1e-400`), and an integer written with a fraction or an exponent
past 2⁵³ (`9007199254740993.0`).

jsonvet writes SARIF and JSON by hand, so every format escapes strings by one rule
([0007](decisions/0007-hand-written-output-with-one-escaping-rule.md)). SARIF holds its results
until the end. JSON and text write each problem as they get it, but the library holds a file's
problems until its report is done, about 110 bytes each: 20 million duplicate keys in a 120 MB
file took 2.3 GB.

### Output

File names and messages come from the files you check, so jsonvet escapes anything that could
break the line or drive a terminal:

- A control character prints as `\u001b`, with CR and LF as `\r` and `\n`.
- A bidirectional control or invisible character (U+00AD, U+061C, U+180E, U+200B to U+200F,
  U+2028 to U+202E, U+2060 to U+2064, U+2066 to U+2069, U+FEFF, U+E0000 to U+E007F) prints as
  `\u202e`, and past U+FFFF as a UTF-16 pair such as `\udb40\udc01`.
- A name that starts with `::`, after any white space, gets `./` in front, and a `[` right after
  `##` prints as `\u005b`. So no line passes for a GitHub workflow command, old (`##[command]`)
  or new (`::command`).

Text output, code frames and annotations can be translated, and SARIF and JSON stay English
([0013](decisions/0013-messages-in-a-plain-catalog.md)). SARIF and JSON strings escape the same
characters the same way. An annotation's `file=` value is
the escaped name, then percent-encoded as GitHub requires.

A code frame also prints a line of the file. There, each of those characters prints as U+FFFD
instead, so the line keeps one character per character of the file and the marks under it line
up. A frame shows at most 160 characters each side of what it marks, with `…` where it cuts, so a
minified file is not copied whole for each problem on its one line.

### `--schema`

`schema` is a seventh rule, and it belongs to the CLI: the library knows no schemas. jsonschema
runs with default features off, so nothing is fetched, and every build has it
([0006](decisions/0006-offline-json-schema-in-every-build.md)). Schemas a project picks stay inside
it and quote nothing in their errors
([0012](decisions/0012-schema-paths-stay-in-the-project.md)).

The library's `parse_with` hands the CLI each value as an event with its byte offset. The CLI
folds the events into `serde_json` values and validates each document as soon as it is complete,
keeping no events. If a document has errors, the CLI reads it again and maps each error's JSON
Pointer back to the offset of its value. A pointer reads the same for `[0]` and `["0"]`, so a
segment counts as an index only where it steps into an array. Messages name the value by
JSONPath, through the same code as `duplicate-key`.

A crafted schema or file can cost far more than its size. The limits are fixed caps, not a
timeout, so a file gets the same verdict on every machine
([0010](decisions/0010-schema-limits-are-deterministic-caps.md)). Their values are in the
[reference](usage.md#json-schema-limits). Why each exists:

- **File depth.** jsonschema validates recursively. Every run starts a thread with a 64 MB stack,
  because a 512-level read took up to 2.2 MB in a debug build, more than Windows' 1 MB main
  thread. Only the stack a run uses is committed. Where the thread cannot start, jsonvet runs on
  the main thread.
- **Memory.** Folded values are most of what a check holds ([how
  much](usage.md#json-schema-limits)), so jsonvet folds and checks one document at a time: a JSON
  Lines file holds one line's values, or one per thread when split. jsonschema collects every error
  of a call before it hands out the first, so jsonvet also checks two shapes of schema one element
  or member at a time:
  - When nothing but `items` checks a top-level array's elements, and every `$ref` points into
    `$defs` or `definitions`: the schema without `items` on the array, then `items` on each
    element. jsonschema runs `contains` after `items` and every other keyword before it, so the
    errors keep that order.
  - When only `properties`, `patternProperties` and `additionalProperties` check a top-level
    object's members: each member as an object of its own. Its errors past the first 100 then come
    member by member instead of keyword by keyword.

  Any other schema is checked in one pass. Files are read a chunk at a time, up to 64 MB. With a
  schema each byte counts 20 times, and the chunk is cut from the files' sizes before any is read,
  so a file of 3.2 MB or more is checked on its own. Otherwise every core can take a large file at
  once: twelve 10 MB files took 3.4 GB instead of 0.30 GB. A file's own `$schema` is known only
  once the file is read. It then counts the same, and the texts other threads read in the meantime
  wait for the next chunk.
- **Errors.** A jsonschema error can hold far more than its message: an `anyOf` error holds every
  branch's errors. jsonvet keeps each error's message and path and drops the rest, and stops
  asking after 100. What one jsonschema call collects is still all in memory at once
  ([SECURITY.md](../SECURITY.md#known-limits)).
- **Schema depth and `$ref` chains.** Without the cap, 1,000 `allOf` and `$ref` hops overflow the
  64 MB stack against a 512-level array. jsonvet resolves each `$ref` with the `referencing` crate
  jsonschema uses, against the base the `$id`s above it set, so both follow the same schemas.
- **Schema size.** jsonschema may check every schema a value reaches, so a schema whose levels
  each `$ref` the next twice doubles the work per level, well under the depth cap: 24 levels took
  12.9 s and 11.4 GB for one `{}`. So jsonvet counts the schema with each `$ref` expanded. `enum`
  and `const` values count, because jsonschema may compare a value with each one. Beside
  `unevaluatedProperties` or `unevaluatedItems`, the schemas that apply to the same value count
  twice, because jsonschema checks them again to find what they evaluated.
- **Regex backtracking.** Patterns with back-references or lookaround run on fancy-regex, which
  backtracks. An over-cap match is neither a match nor a miss, so jsonschema checks the value
  under each outcome, up to 64 times, and reports it only where the outcomes disagree: at the
  string, or for a key at its object. jsonvet names the pattern from the error's schema path, and
  tries it on the object's keys to place an error at each one. A split schema is also checked for
  validity a part at a time, since one pass repeats every over-cap match once per outcome. When
  every regex compiles for the linear-time `regex` crate, no match can run past the cap, so
  jsonvet first checks the whole document for validity in one pass. The copy of each pattern that
  names it in such an error is compiled only when the first one comes, since compiling them all
  with the schema took half as much memory again.
- **Regex compiling.** A pattern of a few bytes, such as `(?:[a-z]{1000}){100}`, can take
  megabytes to compile, and every schema read stays in memory until the run ends. So both regex
  engines get a 2 MB limit per pattern, and jsonvet counts the different patterns of each schema
  before compiling any: at most 1,000 in one schema, and 1,000 across all the schemas a run reads.
  Three schemas of 1,000 near-cap patterns, each named by a `$schema`, took 21 s and 7.9 GB before
  the run cap. Which schema goes past it depends on which thread reads its file first, so it stops
  the run with a usage error instead of failing one file.

## Configuration

Each file takes the nearest config file, which replaces any above it
([0005](decisions/0005-nearest-config-file-replaces.md)). Where the search stops, and why a bad
config shows no value, is in [0009](decisions/0009-where-the-config-search-stops.md). The rules
users see are in the [reference](usage.md#configuration-file).

Config, schema and ignore files are read only when they are regular files under a size cap, as
checked files are: a committed link to `/dev/zero` would fill memory, and a FIFO would hang the
run. A config file may be a symlink that leads anywhere on the machine, so its errors show the
line and column only. The `ignore` crate reads `.gitignore` and `.ignore` with no such check, so
the walk checks those two in each folder before it enters it.

## Architecture

```text
crates/jsonvet/        library: the parser, rules and positions. Depends on hashbrown, memchr and
                       unicode-properties.
crates/jsonvet-cli/    binaries jsonvet and check-json5: arguments, config, output, and --schema
                       (jsonschema, serde_json). Built into the PyPI wheels by maturin.
crates/jsonvet-wasm/   the library as WebAssembly for the playground: one export, `check`
playground/            the browser playground: a static page around the wasm. Deployed by pages.yml.
compat/check-json5/    the check-json5 package on PyPI: depends on jsonvet, nothing else
setup.py               placeholder that pre-commit installs: pins the jsonvet wheel
action.yml             the GitHub Action: installs the jsonvet wheel from PyPI and runs it
fuzz/  tests/  scripts/
```

### The library interface

The public API is `check`, `Config`, `Dialect`, `UnknownDialect`, `Rule`, `Severity`, `UnknownRule`,
`Location`, `Report`, `Problem`, `Related`, `RelatedKind` and `MESSAGES`
([docs.rs](https://docs.rs/jsonvet)).
Each type is `#[non_exhaustive]` or has private fields, so a new variant or field is not a breaking
change. The `#[doc(hidden)]` items are public for jsonvet-cli only, and can change in any release:
`parse`, `parse_with`, `parse_lines`, `join_lines`, `line_parts`, `Lines`, `Event`, `EventKind`,
`json_path`, `PathStep`, `key_text`, `did_you_mean`, `closest`, `fill` and `is_hidden`.

The text is `&str`, so every offset falls on a character boundary. `check` never panics and has no
error mode: the problems in the file are its output. A `Report` is `Send` but not `Sync`, because
`location` caches line starts inside it.

The parser keeps an explicit stack of frames, not recursion, so the depth limit is a setting and
not the call stack's size. Past 16 keys, an object's keys go through a hash index keyed with std's
SipHash from the OS, so a crafted file cannot make the check quadratic
([0003](decisions/0003-siphash-for-duplicate-keys.md)). There is no `unsafe` code, so the worst a
crafted file can do is a panic ([0002](decisions/0002-no-unsafe-code.md)).

### Python packaging

pre-commit installs a `language: python` hook with `pip install .`. The root `setup.py` is a
placeholder that depends on the exact released `jsonvet` wheel, so a hook install downloads a
binary and never compiles Rust ([0008](decisions/0008-binary-wheels-behind-a-placeholder.md)).
maturin builds the wheels with `bindings = "bin"`, for Linux (glibc and musl), macOS and Windows,
each on x86-64 and arm64.

## Testing

How to run each layer is in [CONTRIBUTING.md](../CONTRIBUTING.md#tests). CI runs on Linux, macOS
and Windows, and on the oldest supported Rust (1.88). What each layer is for:

- **Conformance** (json5-tests, all 114 cases): the spec's own answer for what must pass or fail.
- **Corpus:** one file per edge case, survey finding and fuzz finding, and one reviewed snapshot
  of every exit code and output line.
- **Conformance matrix:** the same files through other parsers, in
  [docs/conformance.md](conformance.md).
- **Differential:** the promise to 1.4 is a verdict match, so it is checked against 1.4 itself.
- **Fuzzing:** panics, offsets inside a character, and disagreements with serde_json and the json5
  crate. The json5 target skips inputs that hit json5's own limits, not the grammar's: integers
  past 64 bits, lone surrogate escapes, U+2028 and U+2029 in strings, and identifier characters
  from a Unicode release one side lacks.
- **API changes:** `cargo semver-checks` reports and does not block.
- **Properties:** proptest checks that no input panics, and that every location lies on a
  character boundary and matches a naive recount.
- **Mutations:** show which behaviour no test pins. `.cargo/mutants.toml` lists each surviving
  mutant, such as a speed-only branch, with its reason.

## Publishing

A pushed `vX.Y.Z` tag starts `release.yml`, not a published GitHub release, so the release can be
immutable. The workflow publishes to PyPI and crates.io by trusted publishing, then publishes the
GitHub release it drafted, with the binary archives, the SBOM and `SHA256SUMS`. The archives hold
the binaries from the wheels, so PyPI and the GitHub release ship the same bytes. Every file gets
a build provenance attestation ([SECURITY.md](../SECURITY.md#verifying-a-release)). The binaries
are not built with `cargo auditable`, which maturin cannot run without a cargo wrapper in every
build container, so the SBOM lists the dependencies instead.

## Dependencies and pins

Every version falls in one of five groups, which `.github/renovate.json5` encodes.

- **Bumped monthly**, in one Renovate pull request once a release is 7 days old. Majors get their
  own pull request. This covers the workspace and fuzz crates, GitHub Actions (pinned by commit),
  pre-commit hook revisions, and the CI tools: ruff, pre-commit and prek through `uvx`, the cargo
  tools through `install-action`, uv, and maturin. The monthly `Cargo.lock` refresh, which also
  moves indirect dependencies, skips the 7-day wait, since a refresh has no release date to age
  against. So it can take a crate published that day, and it waits for review.
  - The fuzz crate pins json5 exactly, because `diff_json5` compares with it.
  - ruff is pinned in one place, the `rev` in `.pre-commit-config.yaml`.
  - `uv_build`, which builds `check-json5`, moves with the uv CI uses.
- **Bumped quarterly, with a re-run:** the conformance matrix's parsers, pinned exactly with their
  lock files, and its images, pinned by tag and digest. CI's `conformance/run.py --check` fails
  until `results/versions.json` matches the pins, so the pull request cannot merge until someone
  re-runs the matrix.
- **Fixed:** `check-json5==1.4.1` on Python 3.13 in `scripts/`, the reference for the contract;
  `rust-version = "1.88"`, raised by hand when a dependency or a language feature needs it; and
  jsonvet's own version in `setup.py` and `compat/`, which cargo-release sets in the release commit
  and `scripts/check-versions.py` checks against the tag.
- **Pinned to the release:** the images and the Rust that build the release wheels, by digest and
  version, so a wheel does not change with an upstream image.
- **Floating:** Rust stable for lint and tests, nightly for fuzzing, and the hosted runner images.

## Not supported

JSON text sequences (RFC 7464) and Hjson. Hjson's unquoted strings need a tokenizer of their own.
