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
--format text|sarif|json   text lines (the default), one SARIF 2.1.0 document, or one JSON
                           document on stdout
--schema FILE              also check every file against the JSON Schema in FILE
--stdin-filename PATH      the name standard input (`-`) goes by
--config FILE, --no-config
--list-rules               list the rules with their default severities
--explain RULE             explain a rule, with an example
```

`jsonvet --help` prints the same list with more detail.

## Folders

A folder on the command line stands for the files under it that jsonvet checks by default: those
with an extension or name from the README's dialect table. Other files are skipped without a
word, so `jsonvet .` does not check `README.md` as JSON5. The walk works as ripgrep's does:

- It skips hidden files and folders, such as `.vscode/` and `.babelrc`, and what `.gitignore`
  (inside a git repository), `.ignore` and git's global and `info/exclude` files name.
- It skips paths the `exclude` setting names. A pattern that matches a folder skips everything
  under it.
- It does not follow symlinks.
- It stops with a usage error at a `.gitignore` or `.ignore`, in the folder or one above it, that
  is not a regular file or holds more than 16 MB.
- It lists the files under each folder in sorted order, so output is stable. Arguments keep the
  order you give them.

A file or folder you name is always checked, even when an ignore file or `exclude` names it. So
`exclude` has no effect on the pre-commit hook or the GitHub Action, which name each file: use
the hook's own `exclude:` and the Action's `files` input there.

## Output

Each problem is one line: `file:line:col: severity[rule] message`. When stdout is a terminal,
a last line counts them, such as `3 errors in 2 of 5 files`. Piped or redirected output, as in
the hook and in CI, has no such line.

- Lines and columns count from 1. Columns count Unicode code points, and a byte order mark is
  column 1.
- A duplicate key gets its JSONPath and the position of the first copy. `$` is the root,
  `$.tags[1]` an array element, `$[""]` the empty key.
- A file jsonvet cannot read gets a line with no position and no rule, such as
  `gone.json: error: no such file`. It always fails the run, and you cannot change that.
- A syntax error that is about two places also names the second: `(started at 1:10)` for an
  unterminated string or comment, `(opened at 1:1)` for an object or array still open at the end
  of the file. A missing `,` points just after the value before it.
- Some syntax errors end with a hint after a `;`: the dialect that would accept the syntax, as in
  `comments are not allowed in json; the jsonc dialect allows this`, or `did you mean `true`?` for
  a word such as `True`. Select a dialect with `--dialect jsonc:GLOB`, or a `dialects` entry in
  the configuration file.
- A UTF-16 or UTF-32 file fails with `file is UTF-16 (LE); jsonvet reads UTF-8`. jsonvet knows
  one by its byte order mark, or without one by the zero bytes in its first four bytes. A
  little-endian file without a mark that starts with a quote is the exception: it fails as a
  syntax error, because JSON5 allows a raw U+0000 in a string, so UTF-8 can start the same way.
  Any other
  file that is not UTF-8 fails with `not valid UTF-8 (byte N)`.
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

jsonvet reads regular files only, and follows a symlink to one. A device, FIFO or socket prints
`error: not a regular file`; a directory is walked (see [Folders](#folders)). The hook (`types: [text]`) skips tracked symlinks and
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

jsonvet reads standard input only when asked, as ruff, Biome and taplo do. A run with no files,
such as `echo '{}' | jsonvet`, checks nothing and exits 0.

## File names that look like options

pre-commit passes file names with no `--` before them. So jsonvet reads an unknown `-x` or
`--name`, or a known option with a value such as `--dialect=json`, as a file name when a file of
exactly that name exists. Otherwise an unknown option is a usage error (`unknown option`).

To be sure, put `--` before the file names: `jsonvet -- --x.json`.

## Dialects

- Extensions match in lower case only, so `DATA.JSONL` is checked as JSON5, not JSON Lines.
- `--dialect` and the configuration file also take the names `ijson` (for `i-json`), `hujson`
  (for `jwcc`), and `ndjson` and `json-lines` (for `jsonl`). Dialect names are case-sensitive: `--dialect JSON` is an error that suggests `json`.
- JSONC rejects trailing commas, as the [JSONC spec](https://jsonc.org/) and Microsoft's
  reference parser do by default. For files that use them, such as `tsconfig.json` or VS Code's
  `settings.json`, select `jwcc`, which is JSONC with trailing commas.
- JSONC, JWCC and JSON5 each add syntax to the one before, except where a `//` comment ends.
  JWCC ends it at LF only, JSONC also at a lone CR, and JSON5 also at U+2028 and U+2029.
- In `i-json`, lone surrogates, noncharacters and numbers that a double cannot hold are errors.

## Rules and severities

A severity is `error`, `warning` (or `warn`), or `ignore` (or `off`). An ignored rule is not
checked at all. Warnings fail a run only when there are more than `--max-warnings N`.

`jsonvet --list-rules` lists the rules and `jsonvet --explain RULE` prints one of the sections
below. SARIF output links each rule to its section here.

### syntax

The file is not valid in its dialect: a missing or extra comma, a comment in strict JSON, an
unclosed string. jsonvet reports the first one and stops there, since what follows cannot be read
reliably. Always an error.

```text
Bad (json):  {"a": 1,}
Good (json): {"a": 1}
```

### duplicate-key

Most parsers keep the last copy of a repeated key and drop the others without a word, so a value
is lost; RFC 8259 leaves the result undefined. The message gives the key's JSONPath and where its
first copy is. An error in every dialect.

```text
Bad:  {"port": 80, "port": 8080}
Good: {"port": 8080}
```

### invalid-number

I-JSON (RFC 7493) asks for numbers a double can hold, and for integers within 2^53 - 1 either
side of zero, so that every parser reads them the same. jsonvet reports an overflow, and an
integer written as digits alone past that limit. An error in `i-json`, off otherwise.

```text
Bad (i-json):  {"id": 9007199254740993}
Good (i-json): {"id": "9007199254740993"}
```

### invalid-code-point

I-JSON (RFC 7493) forbids lone surrogates and noncharacters in strings: many decoders reject them
or replace them with U+FFFD. An error in `i-json`, off otherwise.

```text
Bad (i-json):  {"a": "\ud800"}
Good (i-json): {"a": "\ud83d\ude00"}
```

### byte-order-mark

RFC 8259 says JSON text must not start with a byte order mark (U+FEFF), and many parsers fail on
one. Save the file as UTF-8 without a BOM. An error in `jsonl`, a warning in `json` and `i-json`,
off otherwise.

```text
Bad:  <U+FEFF>{"a": 1}
Good: {"a": 1}
```

### missing-final-newline

JSON Lines ends every record with a newline. Without one, the next record appended joins the last
line and breaks it. A warning in `jsonl`, off otherwise.

```text
Bad (jsonl):  {"a": 1}<end of file>
Good (jsonl): {"a": 1}<newline>
```

### unreadable

jsonvet could not check the file at all: it does not exist, is a directory, device, FIFO or
socket, could not be read, or is not UTF-8. It always fails the run, and no option changes that.
Text output shows no rule for it; SARIF and JSON output file it under `unreadable`.

### schema

A value the JSON Schema given with `--schema` rejects, such as a missing required key or a string
where the schema wants a number. Always an error: `--error`, `--warn`, `--ignore` and settings
files do not take this rule.

## Globs

- A glob without `/` matches the file name at any depth.
- A glob with `/` matches the whole path, and `*` also matches `/`.
- A glob compares exact code points, with no Unicode normalization. A `\` in a path is read as
  `/`.
- On the command line, a glob matches the path as you typed it, so it is relative to the working
  directory. pre-commit runs from the repository root.
- In a config file, a glob is relative to the directory that holds the file. So `data/*.json` in
  the root's `jsonvet.toml` matches `data/x.json` when jsonvet runs in the root, in `data/`, or
  with an absolute path. With `--config`, a file outside that directory matches none of its globs.

## Configuration file

jsonvet reads `jsonvet.toml`, or a `[tool.jsonvet]` table in `pyproject.toml`. For each file it
looks in the file's folder and its parents, and stops at the repository root. The nearest file
wins, and it replaces any file further up: settings are not merged. Standard input is looked up by
its `--stdin-filename`, or from the working directory. Options on the command line win.
`--config FILE` reads that file for every file instead, and `--no-config` reads none.

The repository root is the first folder with a `.git` entry. With none above the file, the search
stops at the working directory, and for a file outside the working directory it reads only the
file's own folder.

When the search finds no file, jsonvet reads your own `jsonvet/jsonvet.toml`, if you have one:
in `$XDG_CONFIG_HOME`, or `~/.config` without it, on Linux and macOS, and in `%APPDATA%` on
Windows. Its globs are relative to the working directory. Any project file replaces it, and
`--config` and `--no-config` skip it.

A config file that is not a regular file, such as a FIFO, is skipped; a `--schema` file that is not
one is a usage error. A config file may hold at most 16 MB and a schema 64 MB. An error in a
config file that is a symlink names its line and column, but not the key or value, since the
link may lead to a file that is not yours to show.

A folder walk skips what the `exclude` setting of each folder's nearest file names.

The file takes three keys, `rules`, `dialects` and `exclude`. Any other key, at the top or in a
`dialects` entry, is a usage error, so a typo never goes unnoticed. The error names the line and
column in the file, and the closest known name: `jsonvet.toml:2:1: unknown rule
`duplicate-keys` (did you mean `duplicate-key`?); …`.

```toml
[rules]
byte-order-mark = "error"

[[dialects]]
files = ".vscode/*.json"
dialect = "jwcc"
```

`exclude` is an array of globs, relative to the config file's directory like the others, for
paths a folder walk skips (see [Folders](#folders)):

```toml
exclude = ["node_modules", "tests/fixtures/*"]
```

## GitHub Action

All inputs are optional:

| Input     | What it does                                                                                |
| --------- | ------------------------------------------------------------------------------------------- |
| `files`   | paths or globs to check, separated by spaces; by default every tracked file the hook checks |
| `args`    | extra options, separated by spaces, such as `--strict package.json`                         |
| `sarif`   | also write a SARIF file to this path, besides the usual log and annotations                 |
| `version` | the jsonvet version to install: `X.Y.Z` (also with a suffix such as `rc1` or `.post1`), `X.*`, `X.Y.*` or `latest` |
| `verify`  | `true` (the default) checks the wheel's attestation before installing it; `false` skips that |

In `files`, `**` matches folders at any depth on every runner. The Action skips symlinks, and by
default also submodules. Any other `version` value stops the step.

By default the Action installs the release that matches its own tag: `@v3` gets the newest 3.x,
and `@v3.0.0` gets 3.0.0. Pinned to a commit SHA or a branch, it installs the release named in
that commit's `Cargo.toml`. Between releases that is the last release, not the code in the
commit. The step log names the version it installed.

Pinning the Action does not pin the wheel's bytes, so before installing the wheel from PyPI the
Action checks its build provenance attestation with `gh attestation verify`: it must come from
jsonvet's `release.yml`. Hosted runners have `gh`; on a runner without it, and for releases before
3.1.0, which have no attestation, the wheel installs unchecked, with a notice in the log. Set
`verify: false` on GitHub Enterprise Server.

## SARIF

`--format sarif` prints one SARIF 2.1.0 document instead of text lines:

```bash
jsonvet --format sarif $(git ls-files '*.json') > jsonvet.sarif
```

- Each problem is a result with its rule, severity, message and position. A duplicate key also
  points at the first copy. A file jsonvet cannot read is a result under the rule `unreadable`.
- Each rule carries its help text and a link to its section above, which GitHub code scanning
  shows on each alert.
- The exit status and `--max-warnings` work as with text. No annotations are printed.
- GitHub keeps only 5,000 results of a run with more than 25,000. So jsonvet writes the first
  25,000, and counts the rest in a tool notification and on stderr.

Uploading needs `github/codeql-action/upload-sarif` and the `security-events: write` permission.
The README has a [workflow](../README.md#github-code-scanning).

## JSON

`--format json` prints one JSON document instead of text lines, for scripts and editors:

```bash
jsonvet --format json config.json5 | jq -r '.diagnostics[] | "\(.file):\(.line) \(.rule)"'
```

```json
{"version":1,"diagnostics":[
{"file":"dup.json","rule":"duplicate-key","severity":"error","message":"duplicate key $.a (first at 2:3)","line":3,"column":3,"offset":14,"related":[{"message":"first copy","line":2,"column":3,"offset":4}]},
{"file":"gone.json","rule":"unreadable","severity":"error","message":"no such file","line":null,"column":null,"offset":null,"related":[]}
]}
```

- `version` is the version of this layout, now `1`. A new field may come without a new version;
  a field that changes meaning or goes away gets one. Ignore fields you do not know.
- `diagnostics` holds one object per problem, in the order text output prints them. Each has:

  | Field      | What it holds                                                                     |
  | ---------- | --------------------------------------------------------------------------------- |
  | `file`     | the file name as given, or the `--stdin-filename`; bytes that are not UTF-8 show as U+FFFD |
  | `rule`     | the rule id, such as `duplicate-key`, or `schema` or `unreadable`                 |
  | `severity` | `error` or `warning`                                                              |
  | `message`  | the message as text output prints it, with its `(first at 2:3)` part              |
  | `line`     | the line, from 1; `null` for a file jsonvet could not read                        |
  | `column`   | the column, from 1, in code points as in text output; `null` with no line         |
  | `offset`   | the byte offset in the file, from 0; `null` with no line                          |
  | `related`  | other places the problem names, each with `message`, `line`, `column` and `offset`: a duplicate key's first copy (`first copy`), the bracket an unclosed value opened at (`opened here`), or where an unterminated comment or string starts (`starts here`). Often empty |

- Strings escape control characters, bidi controls and the `[` of `##[` as SARIF does (see
  [design.md](design.md#output)), so a JSON reader gets the name and message back unchanged.
- Each diagnostic is on a line of its own, and none is held back for the end: there is no limit
  on their number. Output comes in blocks as files are checked, as with text, so a reader cannot
  count on one line per file as it finishes.
- The exit status and `--max-warnings` work as with text. No annotations or count are printed.

## JSON Schema limits

These hold for `--schema`. Why each one exists is in [design.md](design.md#--schema).

- **Severity.** Schema errors are always errors, in text, annotations, SARIF and JSON. `--error`,
  `--warn`, `--ignore` and the config file do not take `schema`, and exit 2 if you give it to
  them.
- **Errors.** A document lists at most 100 schema errors. They are the first 100 that jsonschema
  finds, and it takes an object's keys in sorted order, not file order. They print in file order.
  Then one more line, `... and N more schema errors in this document`, sits at the document's
  start. SARIF and JSON leave that line out. `additionalProperties` gives one error per key it does not
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
  size. A top-level array that fails everywhere takes no more than a valid one when only `items`
  checks its elements, as in `{"type": "array", "items": {"$ref": "#/$defs/record"}}`. Otherwise
  each error takes about 340 bytes until the first 100 are picked: 1.7 GB for 5,000,000 errors.
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
