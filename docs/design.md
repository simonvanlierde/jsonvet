# jsonvet design

What jsonvet promises, and why it is built the way it is. jsonvet 2.0 rewrote check-json5 1.4
(Python) in Rust. Items marked *since 3.0* changed after 2.0.0. For how to use it, see the
[reference](usage.md).

## The contract with 1.4

jsonvet 2.0 replaces check-json5 1.4 in place. These stay the same:

- the pass or fail verdict for every file in the two dialects 1.4 knows: JSON5 (the default for
  `.json`, `.json5`, `.geojson`, `.jsonld`, `.webmanifest` and `.babelrc` files, and for any other
  extension) and strict JSON through `--strict`;
- the exit code: 0 when every file passes, 1 when any fails, 2 on a usage error;
- what a duplicate-key report says: the position of the repeated key and its JSONPath (but see
  below for non-ASCII keys);
- the meaning of `--strict GLOB`, now an alias for `--dialect json:GLOB`, with `--strict '!GLOB'`
  mapping to `--dialect auto:GLOB`.

These change, and the changelog marks them **Breaking**:

- the wording of syntax errors: jsonvet has its own messages, not the json5 package's or
  Python's;
- the output line, which gains a severity and a rule id;
- the importable Python module, which is gone: the wheel brings only the commands;
- line ends: a line also ends at a lone CR, and in JSON5 at U+2028 and U+2029, so positions
  differ from 1.4 after one of them. In 2.0, U+2028 and U+2029 ended a line in every dialect;
- *since 3.0*, a duplicate-key JSONPath longer than 256 bytes is shortened. jsonvet keeps the
  segments that fit in 128 bytes at each end, with `…` between them, so a path is at most 259
  bytes. 1.4 and 2.0 printed the whole path;
- *since 3.0*, a quoted key in a JSONPath keeps non-ASCII text as written: `$["日本"]`, not
  `$["\u65e5\u672c"]`. Control and invisible characters, `…` and lone surrogates such as
  `\ud800` stay escaped;
- *since 3.0*, a missing file whose name starts with `-`, such as `--x` or `-x`, is an unknown
  option and exits 2. 1.4.1 and 2.0 exit 1;
- the hook also checks `.jsonl` and `.ndjson` files;
- `.jsonc`, `.code-workspace` and `.code-snippets` files are checked as JSONC, and `.hujson` and
  `.jwcc` files as JWCC. 1.4 read every file as JSON5, so JSON5-only syntax in those files, such
  as `{'a': 1, b: 2}`, passed in 1.4 and fails now. `--dialect json5:GLOB`, or a `dialects` entry
  in the config file, keeps the old verdict.

A syntax error points at the first character that cannot continue the document, or at the end of
the input. Two point earlier: a missing `,` points just after the value before it, and a number
with a leading zero at that zero. A word that cannot continue it, such as `True`, is named whole,
at its first letter. Where that differs from 1.4, the difference and its reason are listed below. Any other
difference is a bug.

### Known differences from 1.4

`scripts/differential.py` compares jsonvet with check-json5 1.4.1 file by file. Its first run,
against 1.4.0, found two bugs in 1.4.0, fixed in 1.4.1: strict mode passed `NaN` or `Infinity`
running on into other text (`[NaN-2.5e3`), and unquoted JSON5 keys were compared with their
`\u` escapes undecoded.

The last full run against 1.4.1 (`--cases 20000 --seed 7`, 2026-10-15) checked 20,184 files in
each of JSON5 and strict mode: 20,000 generated, the 114 json5-tests cases and the 70 corpus
files. Of the 40,368 runs, every verdict and duplicate-key report matches except 385 runs on 383
files. Those are 381 files whose unquoted key starts with a `\u` escape that cannot start a
key (in JSON5 mode), the depth-10,000 file, and a file with a syntax error after a duplicate key
(in both modes). Besides the line ends and the non-ASCII paths in the list above, these
differences are intended:

- Raw U+2028 and U+2029 inside strings pass in jsonvet, as the JSON5 spec allows. 1.4's JSON5
  path rejected them.
- Nesting deeper than 10,000 levels is a syntax error in jsonvet (`Config::max_depth` changes
  it). 1.4 stopped at 9,998 levels, or 9,994 in files with JSON5 syntax, so only files from 9,995
  to 10,000 levels deep get a different verdict.
- A `\u` escape in an unquoted key must stand for a character the key could hold literally,
  as ECMAScript 5.1 says. 1.4 passed any escape, so `{\u0301: 1}` passed there, and so did
  `{\ud800: 1}`, though a lone surrogate is no character at all. Literal characters agree:
  both go by general category (letters to start a key, then marks, digits and connectors too), as
  the spec and the JS reference do, not by Unicode's `ID_Start`.
- After a duplicate key, 1.4 stops at the duplicates. jsonvet goes on and also reports a later
  syntax error. The verdict is the same.
- Syntax-error wording, and some syntax-error positions, follow the rule above.

## Speed

Median wall time of one run over the files, from `uv run scripts/bench.py` on 12 cores of a
Ryzen 7 H 255 under Linux. Both tools give the same verdict in every case:

| Case                       | check-json5 1.4.1 | jsonvet 3.0 | Speed-up |
| -------------------------- | ----------------: | ----------: | -------: |
| 1,100 config files         |             40 ms |     12.5 ms |       3x |
| 2.7 MB JSON                |             41 ms |      9.8 ms |       4x |
| 2.7 MB JSON, duplicate key |            330 ms |     11.6 ms |      29x |
| 120 KB JSONC               |             22 ms |      1.5 ms |      15x |
| 1.8 MB JSON5               |         11,641 ms |      8.3 ms |   1,398x |

1.4 sent JSON, and JSONC with its comments stripped, to Python's C parser, so those were already
fast. Other JSON5 syntax went to a pure-Python parser, and so did the second pass that finds a
duplicate key's position. jsonvet does everything in one pass of one parser.

A JSON Lines file of 1 MB or more is cut at newlines into one part for each core its chunk of
files leaves free, and the parts are checked at once. A newline inside a JSON string must be
escaped, and JSON Lines has no comments, so every raw newline ends a record. Each part counts
offsets from the start of the file, problems stop at the first syntax error as in one pass, and
the output is the same. With `hyperfine` on the same machine, 78 MB of nested records took 264 ms
before and 89 ms after (3.0x). 100 MB of records that are mostly one long string took 83 ms
before and 64 ms after: the string scan is already fast, and reading the file (19 ms for `cat`)
stays on one thread.

## Dialects

One parser covers six dialects; the [README](../README.md#dialects) has the table. JSON, JSONC,
JWCC and JSON5 each add syntax to the one before, so a file that passes in one passes in the next.

The one exception is where a `//` comment ends. JWCC ends it at LF only, JSONC also at a lone CR,
and JSON5 also at U+2028 and U+2029. So `new-lines/comment-cr.json5` passes as JSONC and fails as
JWCC.

Lines end at LF, CR and CRLF. In JSON5 they also end at U+2028 and U+2029, because the JSON5 spec
counts them as line terminators. The other dialects treat both as ordinary characters.

`.json` defaults to JSON5, as in 1.4. `auto` means "the default for this file's extension".

JSONC rejects trailing commas, with no option to allow them. The [JSONC spec](https://jsonc.org/)
says parsers "MAY" accept them, and its reference parser, Microsoft's `jsonc-parser`, and prek's
`check-jsonc` reject them by default. Files whose own tools accept them, such as VS Code's
settings and `tsconfig.json`, are JWCC, which `dialects` selects per glob.

## Rules

Every problem belongs to a rule, and each rule has a default severity per dialect. The
[README](../README.md#rules) lists them.

`invalid-number` and `invalid-code-point` are the I-JSON rules, off outside `i-json`. They report
an overflow, and an integer written as digits alone past ±(2⁵³ − 1). RFC 7493 leaves two cases
open, and these rules do not flag them:

- a number that underflows to zero (`1e-400`);
- an integer written with a fraction or an exponent past 2⁵³ (`9007199254740993.0`).

In GitHub Actions each problem is also printed as a `::warning` or `::error` annotation.
`--format sarif` prints one SARIF 2.1.0 document instead, with no annotations. jsonvet writes its
JSON by hand.

### Output

File names and messages come from the files you check, so jsonvet escapes anything that could
break the line or a terminal. A control character prints as `\u001b`, with CR and LF as `\r` and
`\n`. A bidirectional control or invisible character (U+00AD, U+061C, U+180E, U+200B to U+200F,
U+2028 to U+202E, U+2060 to U+2064, U+2066 to U+2069, U+FEFF, U+E0000 to U+E007F) prints as
`\u202e`, past U+FFFF as a UTF-16 pair such as `\udb40\udc01`. SARIF strings escape the same
characters the same way. A name that starts with `::`, after any white space, prints as `./::…`,
and a `[` straight after `##` prints as `\u005b`. So no line looks like a GitHub workflow
command, old (`##[command]`) or new (`::command`). An annotation's `file=` value holds the real
name, percent-encoded as GitHub requires.

### `--schema`

`--schema FILE` adds a seventh rule, `schema`, which belongs to the CLI: the library knows no
schemas. The library's `parse` returns the file's values as events, each with its byte offset.
The CLI folds them into `serde_json` values and validates those with the `jsonschema` crate, with
default features off so nothing is fetched. It then maps each error's instance path, a JSON
Pointer, back to the offset of its value. One walk over a document's events finds every error's
value, with no pointer kept per value.

The message names the value by JSONPath, through the same code as `duplicate-key`, so one file's
problems use one path style. A pointer reads the same for `[0]` and `["0"]`, so a segment counts
as an index only where the value it steps into is an array. Schema errors follow the file's other
problems. A file with a syntax error has no events, so it gets none.

Every build has `--schema`, the PyPI wheels too, so the hook and the Action can use it.
jsonschema and its regex engines take the binary from about 0.7 MB to about 5 MB on Linux
x86-64, still smaller than prek's or ruff's wheel, and a plain check starts no slower. It adds
about 70 crates, whose code runs only with `--schema`. One build means one help text and one
set of tests.

The limits are listed in the [reference](usage.md#json-schema-limits). Why each exists:

- **File depth.** jsonschema validates recursively. Its memory grows with the square of the depth
  when an error sits deep under a recursive `$ref`: 1.3 GB for a 50 KB file nested 10,000 levels.
  Every run starts a thread with a 64 MB stack, because the 512-level read took up to 2.2 MB in a
  debug build, more than Windows' 1 MB main thread. Only the stack a run uses is committed, so a
  plain check still works under `ulimit -v 60000`. Where the thread cannot start, jsonvet runs on
  the main thread. The threads that check files in parallel get the same stack, and any that
  cannot start leave their files to the others.
- **Memory.** Folded values take 14 to 27 times the file's size, the most for many small records
  such as JSON Lines. jsonschema collects every error before handing out the first, so a 10 MB
  array of 5,000,000 failing numbers took 2.0 GB, against 0.37 GB when it passes. When nothing
  but `items` checks a top-level array's elements, and every `$ref` points into `$defs` or
  `definitions`, jsonvet checks the schema without `items` on the whole array, then `items` on
  one element at a time: 0.37 GB both ways, with the same output. jsonschema runs `contains`
  after `items` and every other keyword before it, so the errors keep that order. Any other
  schema is checked in one pass.
- **Schema depth and `$ref` chains.** Without the check, 1,000 `allOf` and `$ref` hops overflow the
  64 MB stack at once against a 512-level array. A `$ref` back to a schema on the way to it is
  recursion and adds nothing. The error lands at the `$ref` that crosses the line. A `$ref`
  jsonvet can follow inside the file is a JSON Pointer, an anchor, or a URI naming an `$id` there.
- **Schema size.** jsonschema may check every schema a value reaches, so a schema where each level
  `$ref`s the next twice doubles the work per level, well under the depth cap: 24 levels took
  12.9 s and 11.4 GB for one `{}`. jsonvet counts the objects and arrays with each `$ref` replaced
  by what it points at, and stops at 5,000,000. That lets 18 such levels through (about 3.9
  million, 1 s and 340 MB) and SchemaStore's 1.6 MB `azure-pipelines.json` (about 2.56 million).
  A schema measured while a `$ref` back up to a schema above it was skipped is measured again
  wherever it is reached, as it can be deeper there.
- **Regex backtracking.** Schema regexes with back-references or lookaround run on fancy-regex,
  which backtracks. With the cap, `^(a|a)*\1b` over 5,000 strings takes under a second, not
  minutes. `patternProperties` (and `additionalProperties` beside it) count an over-cap key as not
  matching, because jsonschema 0.58 drops the regex error there (`is_match(..).unwrap_or(false)`),
  and so does `pattern` under `not`, `if` or `oneOf`. So jsonvet tries every `patternProperties`
  regex, and every `pattern` under `propertyNames`, on every key, and every `pattern` on every
  string, with the same cap. An over-cap match is an error at that key or string. Trying each
  regex everywhere, not only where the schema applies it, keeps the check short but can fail a
  string the regex never checks. With three lookaround patterns, a 5.9 MB file takes 241 ms
  instead of 105 ms.

## Configuration

jsonvet reads `jsonvet.toml`, or a `[tool.jsonvet]` table in `pyproject.toml`. It looks in the
working directory and its parents. It stops at the first directory that has a `.git` entry, so
it never uses a stray file above the repository. A pattern given with `--dialect` or `--strict`
matches the path as typed, as in 1.4. The [reference](usage.md#globs) has the glob rules.

## Architecture

```text
crates/jsonvet/        library: the parser, rules and positions. Depends on memchr and unicode-properties.
crates/jsonvet-cli/    binaries jsonvet and check-json5: arguments, config, output, and --schema
                       (jsonschema, serde_json). Built into the PyPI wheels by maturin.
compat/check-json5/    the check-json5 package on PyPI: depends on jsonvet, nothing else
setup.py               placeholder that pre-commit installs: pins the jsonvet wheel
action.yml             the GitHub Action: installs the jsonvet wheel from PyPI and runs it
fuzz/  tests/  scripts/
```

### The library interface

The public API is `check`, `Config`, `Dialect`, `UnknownDialect`, `Rule`, `Severity`,
`UnknownRule`, `Location`, `Report`, `Problem`, `Related` and `RelatedKind`. Each type is
`#[non_exhaustive]` or has private fields, so adding a variant or field is not a breaking change.
`parse`, `Event`, `EventKind`, `json_path` and `PathStep` are public for jsonvet-cli only. They
are not API and can change in any release. The full interface is on
[docs.rs](https://docs.rs/jsonvet).

The text is `&str`: the caller has already checked UTF-8, so every offset falls on a character
boundary and columns are well defined. `check` never panics and has no error mode: the problems
in the file are its output. A `Report` is `Send` but not `Sync`, because `location` caches line
starts and character counts inside it.

Behind that interface:

- a single-pass byte scanner, with memchr to skip through comments, and a UTF-8 decode only for
  bytes from 0x80 up;
- an explicit stack of frames, not recursion, so the depth limit is a setting, not the call
  stack's size. The frames also hold each object's keys for duplicate detection and give the
  JSONPath of a duplicate key;
- keys compared by their decoded value, borrowed from the input unless they contain an escape;
- an object's keys checked by a scan up to 16 keys, then through a hash index. The index uses the
  standard library's SipHash with keys from the OS, so a crafted file cannot make its keys collide
  and the check stays linear in the file's size, as [SECURITY.md](../SECURITY.md) promises;
- line starts found once with memchr, and only when the first location is asked for;
- in text that is not all ASCII, a count of the code points before every 4 KiB, built only as far
  as a lookup needs, so finding a column counts the characters of at most two chunks.

### Python packaging

pre-commit installs a `language: python` hook by running `pip install .` on the repository. The
root `setup.py` is a placeholder that depends on the exact released `jsonvet` wheel, so a hook
install downloads a prebuilt binary and never compiles Rust. typos ships the same way. maturin
builds the wheels with `bindings = "bin"`, for Linux (glibc and musl, x86-64 and arm64), macOS
(x86-64 and arm64) and Windows (x64 and arm64).

## Testing

How to run each layer is in [CONTRIBUTING.md](../CONTRIBUTING.md#tests). CI runs on Linux, macOS
and Windows, plus the oldest supported Rust (1.88). Each layer exists for one reason:

- **Conformance** (json5-tests, all 114 cases): the suite is the spec's own answer for what must
  pass or fail.
- **Corpus:** one file per case keeps every edge case, survey finding and fuzz finding from
  coming back. One reviewed snapshot pins every exit code and output line.
- **Conformance matrix:** the same files through 17 other parsers in four languages, in
  [docs/conformance.md](conformance.md).
- **Differential:** the promise to 1.4 is a verdict match, so it is checked against 1.4 itself.
- **Fuzzing:** finds panics, offsets inside a character, and disagreements with serde_json and the
  json5 crate that hand-written cases miss. The json5 target skips inputs that hit json5's own
  limits, not the grammar's, each found by fuzzing: integers past 64 bits, lone surrogate
  escapes, U+2028 and U+2029 in strings, and identifier characters from a Unicode release one side
  has and the other lacks.
- **API changes:** `cargo semver-checks` reports and does not block, since 3.0 changes the public
  API.
- **Properties:** proptest checks laws no single case shows: no input panics, and every location
  lies on a character boundary and matches a naive recount.
- **Mutations:** show which behaviour no test pins. The mutants that survive change no behaviour,
  such as speed-only branches. `.cargo/mutants.toml` lists each one with its reason.

## Publishing

A release publishes to PyPI and crates.io from `release.yml`, with trusted publishing on both.
A pushed `vX.Y.Z` tag starts it, not a published GitHub release: the release is immutable, so the
workflow drafts it, attaches the binary archives, the SBOM and `SHA256SUMS`, and publishes it after
the packages are up. The archives hold the binary from each wheel, not a second build, so PyPI and
the GitHub release ship the same bytes. Each file gets a build provenance attestation, and the
PyPI upload adds PEP 740 attestations. The binaries are not built with `cargo auditable`: maturin
cannot run it without a cargo wrapper inside every build container, so the SBOM lists the
dependencies instead.
The `check-json5` package uploads after the `jsonvet` wheels, and `jsonvet-cli` after the
`jsonvet` crate. crates.io only allows trusted publishing for a crate that already exists, so
2.0.0 went up by hand.

## Dependencies and pins

Every version in the repository falls in one of five groups, and `.github/renovate.json5`
encodes them.

- **Bumped monthly**, in one Renovate pull request once a release is 7 days old. Majors get their
  own pull request. This covers:
  - the workspace crates: ranges in `Cargo.toml`, versions in `Cargo.lock`, which also refreshes
    indirect dependencies;
  - the fuzz crate's dependencies;
  - GitHub Actions, pinned by commit with the version in a comment;
  - pre-commit hook revisions;
  - the CI tools: ruff, pre-commit and prek through `uvx`, the cargo tools through
    `install-action`, uv, and the maturin that builds the wheels.
  - The fuzz crate pins json5 exactly, because the `diff_json5` target compares with it. A bump
    changes `fuzz/`, which runs the fuzz workflow.
  - ruff is pinned in one place, the `rev` in `.pre-commit-config.yaml`, and CI runs its hooks.
  - `check-json5` builds with `uv_build`, pinned to the uv version CI uses. Renovate moves the two
    together.
- **Bumped quarterly, with a re-run:** the conformance matrix's parsers, pinned exactly with their
  lock files, and its images, pinned by tag and digest. CI's `conformance/run.py --check` fails
  until `results/versions.json` matches the pins, so the pull request cannot merge until someone
  re-runs the matrix. The lock files are never refreshed on their own.
- **Fixed:**
  - `check-json5==1.4.1` on Python 3.13 in `scripts/`, the reference the contract is checked
    against;
  - `rust-version = "1.88"` and the MSRV job, raised by hand when a dependency or a language
    feature needs it;
  - jsonvet's own version in `setup.py` and `compat/`. A release commit sets it by hand, and
    `scripts/check-versions.py` checks it against the tag.
- **Pinned to the release:** the images and the Rust that build the release wheels, by digest and
  version, so a wheel does not change with an upstream image. This is an exception to the next
  group.
- **Floating:** Rust stable for lint and tests and nightly for fuzzing, so new lints and compiler
  changes show up at once, and the hosted runner images.

## Not supported

JSON text sequences (RFC 7464) and Hjson. Hjson's unquoted strings need a tokenizer of their own.
