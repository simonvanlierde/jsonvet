# jsonvet reference

The details behind the [README](../README.md). Why things work this way is in
[design.md](design.md).

## Options

```text
--dialect DIALECT[:GLOB]   check files matching GLOB (every file, without one) as DIALECT,
                           or as `auto`, the extension's default; the last match wins
--strict GLOB              the same as --dialect json:GLOB, kept from check-json5
--error RULE, --warn RULE, --ignore RULE
--max-warnings N
--format text|sarif        text lines (the default), or one SARIF 2.1.0 document on stdout
--schema FILE              also check every file against the JSON Schema in FILE
--stdin-filename PATH      the name standard input (`-`) goes by
--config FILE, --no-config
```

`jsonvet --help` prints the same list with more detail.

## Output

Each problem is one line: `file:line:col: severity[rule] message`.

- Lines and columns count from 1. Columns count Unicode code points, and a byte order mark is
  column 1.
- A duplicate key gets its JSONPath and the position of the first copy. `$` is the root,
  `$.tags[1]` an array element, `$[""]` the empty key.
- A file jsonvet cannot read gets a line with no position and no rule, such as
  `gone.json: error: no such file`. It always fails the run, and you cannot change that.
- In GitHub Actions, each problem is also printed as an annotation.

jsonvet escapes control, bidirectional and invisible characters in file names and messages. So no
output line can drive a terminal or pass for a GitHub workflow command. The exact forms are in
[design.md](design.md#output).

## Exit status and files

| Exit | When                                                     |
| ---: | -------------------------------------------------------- |
|    0 | no file has an error, or there were no files             |
|    1 | a file has an error, or a file cannot be read            |
|    2 | a usage error, such as an unknown option or a bad schema |

jsonvet reads regular files only, and follows a symlink to one. A directory, device, FIFO or
socket prints `error: not a regular file`. The hook (`types: [text]`) skips tracked symlinks and
submodules.

When the output pipe closes early, as in `jsonvet … | head -1`, jsonvet stops quietly. It exits
with the verdict for the files it has checked.

## Standard input

`-` as a file reads standard input: `cat a.json | jsonvet -`. Output names it `-`, and it is
checked as JSON5, the default for an unknown extension. `--stdin-filename PATH` names it `PATH`
instead, in the output and when picking its dialect and settings, so
`jsonvet --stdin-filename tsconfig.json -` checks it as a `.json` file. With `--stdin-filename`
and no file, jsonvet reads standard input without the `-`.

Empty input is checked like an empty file, which is a syntax error. `-` can be given only once;
to check a file named `-`, pass `./-`.

jsonvet does not read standard input unless you ask, as ruff, Biome and taplo do. A run with no
files and input piped in, such as `echo '{}' | jsonvet`, checks nothing and exits 0, but it
says on stderr to pass `-`.

## File names that look like options

pre-commit passes file names with no `--` before them. So jsonvet reads an unknown `-x` or
`--name`, or a known option with a value such as `--dialect=json`, as a file name when a file of
exactly that name exists. Otherwise an unknown option is a usage error (`unknown option`).

To be sure, put `--` before the file names: `jsonvet -- --x.json`.

## Dialects

- Extensions match in lower case only, so `DATA.JSONL` is checked as JSON5, not JSON Lines.
- JSONC, JWCC and JSON5 each add syntax to the one before, except where a `//` comment ends.
  JWCC ends it at LF only, JSONC also at a lone CR, and JSON5 also at U+2028 and U+2029.
- In `i-json`, lone surrogates, noncharacters and numbers that a double cannot hold are errors.

## Rules and severities

A severity is `error`, `warning` (or `warn`), or `ignore` (or `off`). An ignored rule is not
checked at all. Warnings fail a run only when there are more than `--max-warnings N`.

## Globs

- A glob without `/` matches the file name at any depth.
- A glob with `/` matches the whole path, and `*` also matches `/`.
- A glob compares exact code points, with no Unicode normalization. A `\` in a path is read as
  `/`.
- On the command line, a glob matches the path as you typed it, so it is relative to the working
  directory. pre-commit runs from the repository root.
- In a config file, a glob is relative to the directory that holds the file. So `data/*.json` in
  the root's `jsonvet.toml` matches `data/x.json` when jsonvet runs in the root, in `data/`, or
  with an absolute path. A file outside that directory matches none of its globs.

## Configuration file

jsonvet reads `jsonvet.toml`, or a `[tool.jsonvet]` table in `pyproject.toml`. It looks in the
working directory and its parents, and stops at the repository root. Options on the command line
win. `--config FILE` reads that file instead, and `--no-config` reads none.

The file takes two keys, `rules` and `dialects`. Any other key, at the top or in a `dialects`
entry, is a usage error, so a typo never goes unnoticed.

## GitHub Action

All inputs are optional:

| Input     | What it does                                                                                |
| --------- | ------------------------------------------------------------------------------------------- |
| `files`   | paths or globs to check, separated by spaces; by default every tracked file the hook checks |
| `args`    | extra options, separated by spaces, such as `--strict package.json`                         |
| `sarif`   | also write a SARIF file to this path, besides the usual log and annotations                 |
| `version` | the jsonvet version to install: `X.Y.Z` (also with a suffix such as `rc1` or `.post1`), `X.*`, `X.Y.*` or `latest` |

In `files`, `**` matches folders at any depth on every runner. The Action skips symlinks, and by
default also submodules. Any other `version` value stops the step.

By default the Action installs the release that matches its own tag: `@v3` gets the newest 3.x,
and `@v3.0.0` gets 3.0.0. Pinned to a commit SHA or a branch, it installs the release named in
that commit's `Cargo.toml`. Between releases that is the last release, not the code in the
commit. The step log names the version it installed.

Pinning the Action does not pin the wheel's bytes: pip installs `jsonvet==X.Y.Z` from PyPI without
a hash check.

## SARIF

`--format sarif` prints one SARIF 2.1.0 document instead of text lines:

```bash
jsonvet --format sarif $(git ls-files '*.json') > jsonvet.sarif
```

- Each problem is a result with its rule, severity, message and position. A duplicate key also
  points at the first copy. A file jsonvet cannot read is a result under the rule `unreadable`.
- The exit status and `--max-warnings` work as with text. No annotations are printed.
- GitHub keeps only 5,000 results of a run with more than 25,000. So jsonvet writes the first
  25,000, and counts the rest in a tool notification and on stderr.

Uploading needs `github/codeql-action/upload-sarif` and the `security-events: write` permission.
The README has a [workflow](../README.md#github-code-scanning).

## JSON Schema limits

These hold for `--schema`. Why each one exists is in [design.md](design.md#--schema).

- **Severity.** Schema errors are always errors, in text, annotations and SARIF. `--error`,
  `--warn`, `--ignore` and the config file do not take `schema`, and exit 2 if you give it to
  them.
- **Errors.** A document lists at most 100 schema errors. They are the first 100 that jsonschema
  finds, and it takes an object's keys in sorted order, not file order. They print in file order.
  Then one more line, `... and N more schema errors in this document`, sits at the document's
  start. SARIF leaves that line out. `additionalProperties` gives one error per key it does not
  allow, at that key, and each one counts toward the 100.
- **Messages.** Each message ends with the JSONPath of the value, as duplicate keys do. A message
  quotes the value unless that makes it long; then it says `value`. A message, with its path, is
  cut at 200 bytes and ends in `...`. The wording comes from the jsonschema crate and may change
  between releases. The rule id `schema` and the position are stable.
- **Depth.** With `--schema`, a file nested deeper than 512 levels is a syntax error. A file with a
  syntax error gets no schema errors.
- **Values.** Each line of a JSON Lines file is checked on its own. `NaN`, `Infinity` and numbers
  past a double's range have no JSON Schema value. So a document that holds one is reported once
  and not checked. The same goes for a document with two keys in one object that differ only in
  lone surrogates such as `"\ud800"`, since both become U+FFFD. In the schema file, either one is
  a usage error.
- **Memory.** jsonvet holds each file's values in memory: 14 to 27 times the file's size, the most
  for many small records such as JSON Lines. Without `--schema`, memory stays near the file's
  size.
- **Regexes.** A pattern with a back-reference or lookaround may backtrack 5,000 times per string.
  A key or string that needs more is an error at that key or string: the pattern could not be
  checked. jsonvet tries each `patternProperties` pattern on every key, and each `pattern` on
  every string, so this error can name a string the pattern does not apply to.
- **The schema file.** It may be in any dialect. Its `$schema` picks the draft, from draft 4 to
  2020-12. A `$ref` must point inside the file: jsonvet fetches nothing, from the network or from
  disk. A schema jsonvet cannot read or use is a usage error (exit 2). That covers:
  - a `$ref` to another file;
  - nesting deeper than 128 levels, where each `$ref` followed counts as a level;
  - more than 5,000,000 objects and arrays once each `$ref` is replaced by what it points at,
    every time it is followed. SchemaStore's 1.6 MB `azure-pipelines.json` comes to about
    2,560,000.

  A `$ref` back to a schema it is inside, as in `{"items": {"$ref": "#"}}`, is recursion and is
  fine: jsonvet does not follow it again.

## Coming from check-json5

The `check-json5` hook id, the `check-json5` command, `--strict` and the `check-json5` package on
PyPI (which now installs jsonvet) keep working through 3.x.

Besides the dialect change in the README, the output line, the syntax-error wording, positions
after a lone CR or U+2028, the new `.jsonl` and `.ndjson` files and the removed Python module
changed. The CHANGELOG has them under 2.0.0 and 3.0.0, and
[design.md](design.md#the-contract-with-14) has the full list.
