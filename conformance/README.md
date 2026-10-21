# Conformance matrix

This runs one set of input files through many parsers of JSON5, JSONC, JWCC (HuJSON), JSON Lines
and strict JSON, in four languages, and renders a pass/fail grid per dialect in
[docs/conformance.md](../docs/conformance.md). It is the same idea as Nicolas Seriot's "Parsing
JSON is a minefield", for the JSON dialects.

## Running it

You need docker and uv. Third-party parsers never run on the host; see below.

```sh
cargo build --release -p jsonvet-cli
uv run conformance/run.py      # writes conformance/results/
uv run conformance/report.py   # writes docs/conformance.md
```

The first run pulls four images and builds the drivers into `conformance/.work/` (ignored by git).
With the images present, a full run takes about 20 seconds, the builds included. `--work DIR` puts the build cache
elsewhere, and `--only PARSER ...` reruns some parsers and keeps the other results.

Go makes its module cache in `conformance/.work/go` read-only, and a worktree holds the
json5-tests submodule, so `git worktree remove` fails on a worktree that ran this. Remove one
with `chmod -R u+w <worktree> && rm -rf <worktree> && git worktree prune`.

## How third-party code is kept off the host

Every parser except jsonvet is installed and run inside an official image (`node:26-slim`,
`python:3.14-slim`, `golang:1.27`, `rust:1`) in a container named `tmp-matrix-*`, removed when it
exits, limited to 4 CPUs and run as your user. The setup step, which needs the network to fetch
packages, mounts only the driver directory (read-only) and the build cache. The parsing step has
no network and mounts the inputs read-only. npm runs with `--ignore-scripts`, and pip installs
wheels only, so Python stays at the newest version every pinned package has wheels for: 3.15
had none for pyjson5 2.0 in October 2026. The current hujson needs Go 1.26 or later.

Versions are pinned exactly: `drivers/node/package.json` with its lock file,
`drivers/python/requirements.txt`, `drivers/go/go.mod` with `go.sum`, and `drivers/rust/Cargo.toml`
with `Cargo.lock`. `run.py --lock` refreshes the lock files after you change a pin, inside the
same containers. `results/versions.json` records the version of every parser in a run.

## What the verdicts mean

Each driver reads file paths on stdin, parses each file with the library's most standard API and
default options, and prints `<path>\t<verdict>`:

- **accept**: the parser returned a value without an error.
- **reject**: the parser reported its documented parse error.
- **crash**: any other exception, a panic, or the process died (a stack overflow, say).
- **timeout**: no answer within 10 seconds. The runner then kills the container.

A limit the library enforces, such as a nesting depth or Python's 4,300-digit cap on integers, is a
reject when it raises the parser's own error type and a crash otherwise. That is why `py-json`
crashes and `py-json5` rejects on `integer-5000-digits.json`.

After a crash or a timeout the runner starts a fresh container at the next file. Libraries that take
bytes get the raw file. For libraries that take text, the driver decodes the file as strict UTF-8
first and counts a file that is not UTF-8 as a reject. A leading byte order mark stays in the text.

Some choices a reader should know about:

- Microsoft's `jsonc-parser` never throws: it recovers and collects errors. Any collected error
  counts as a reject. The Rust `jsonc-parser` is lenient by design with its default options.
- The Rust `jsonc-parser` returns no value and no error for a file without a value, such as one
  holding only a comment. That counts as an accept.
- Go's `json.Decoder` and serde_json's `StreamDeserializer` stand in for JSON Lines readers in those
  languages. They split values on any white space, not on lines. JSON Lines has no reference
  implementation.
- Most parsers keep one value of a duplicate key without a word, so duplicates count as valid
  syntax, and jsonvet runs with `--ignore duplicate-key`. The `*-dupes` runs turn detection on where
  a parser can do it, for the separate duplicate-key table.

## Expected verdicts

`report.py` takes each file's expected verdict from the first of these that applies:

1. `SPEC` in `report.py`: corpus files whose verdict the dialect's spec decides, with the section.
   "either" means the spec allows both verdicts (a nesting limit, an optional feature), so the row
   is shown but not scored.
2. The [json5-tests](https://github.com/json5/json5-tests) file extension. In the JSON5 table,
   `.json` and `.json5` files are valid and `.js` and `.txt` files are invalid. In the strict JSON
   table only `.json` files are valid. `comments/irregular-block-comment.json` is named `.json` but
   holds a block comment, so it counts as invalid JSON (json5/json5-tests#4).
3. `extra/<dialect>/expect.tsv`, for the cases in `extra/`: the verdict and the spec rule behind it.
4. jsonvet's reviewed corpus snapshot,
   `crates/jsonvet-cli/tests/snapshots/corpus__corpus.snap`, where a file whose only errors are
   duplicate keys counts as valid.

The spec wins over jsonvet: where they differ, jsonvet's column shows a disagreement like any other.

## Adding a parser

1. Add an entry to the `parsers` map of the driver for its language, and pin the package in that
   driver's manifest.
2. Run `uv run conformance/run.py --lock` to update the lock file.
3. Add a row to `PARSERS` in `run.py`: its language, its package, and the dialects it claims.
4. Run `uv run conformance/run.py --only <parser>`, then `uv run conformance/report.py`.

A new language needs a driver that follows the protocol above and an entry in `LANGS` in `run.py`.

To add an input, put the file in `extra/<dialect>/` and add a line to that folder's `expect.tsv`.
Keep the bytes exact: the repository's hooks and `.gitattributes` leave that folder alone.

## Parsers left out

- `github.com/flynn/json5` now redirects to `github.com/titanous/json5`, which is in the matrix.
- `github.com/muhammadmuzzammil1998/jsonc`: last release v1.0.0 in 2022.
- `github.com/titanous/json5` itself is barely maintained (v1.0.0 in 2023), but it is the Go JSON5
  library people find, so it stays.
