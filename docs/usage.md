
# jsonvet reference

The details behind the [README](../README.md). Why things work this way is in
[design.md](design.md).

## Options

```text
--dialect DIALECT[:GLOB]   check files matching GLOB (every file, without one) as DIALECT,
                           or as `auto`, the extension's default; the last match wins
--strict GLOB              the same as --dialect json:GLOB, kept from check-json5;
                           --strict '!GLOB' means --dialect auto:GLOB
--error RULE, --warn RULE, --ignore RULE
--max-warnings N
--format text|sarif|json   text lines (the default), one SARIF 2.1.0 document, or one JSON
                           document on stdout
--color auto|always|never  color text output; auto colors it on a terminal unless NO_COLOR
                           is set, and anywhere when FORCE_COLOR is
--schema FILE              check each file against the JSON Schema in FILE, instead of the
                           schemas from settings or a file's own local $schema
--stdin-filename PATH      the name standard input (`-`) goes by
--config FILE, --no-config
--list-rules               list the rules with their default severities
--explain RULE             explain a rule, with an example
```

`jsonvet --help` prints the same list with more detail.


## Folders

A folder on the command line stands for the files under it with an extension or name from the
README's [dialect table](../README.md#dialects), so `jsonvet .` does not check `README.md`. The
walk works as ripgrep's does:

- It skips hidden files and folders, and what `.gitignore` (inside a git repository), `.ignore`
  and git's global and `info/exclude` files name.
- It skips what the `exclude` setting names, and everything under a folder it names.
- It does not follow symlinks.
- It stops with a usage error at a `.gitignore` or `.ignore` that is not a regular file or holds
  more than 16 MB.
- It lists each folder's files in sorted order.

A file or folder you name is always checked, even when an ignore file or `exclude` names it. So
`exclude` has no effect on the hook or the Action, which name each file: use the hook's own
`exclude:` and the Action's `files` input there.

## Output

Each problem is one line: `file:line:col: severity[rule] message`. On a terminal, each problem
shows as a code frame instead, with its line of the file and any related place marked, and a last
line counts them. pre-commit runs hooks under a terminal when its color is on, so a commit made
from a terminal shows frames. CI, pre-commit.ci and pipes get one line per problem, and `| cat`
forces lines.

Frames are in color unless `NO_COLOR` is set or you pass `--color never`. `--color always` or
`FORCE_COLOR` also color the severity and rule of piped lines. `FORCE_COLOR` set to `0` or `false`
counts as unset. `NO_COLOR` wins over `FORCE_COLOR`, and `--color` wins over both.

- Lines and columns count from 1, and columns count Unicode code points. A byte order mark is
  column 1.
- A duplicate key gets its JSONPath and the position of the first copy: `$` is the root,
  `$.tags[1]` an array element, `$[""]` the empty key.
- A file jsonvet cannot read gets a line with no position and no rule, such as
  `gone.json: error: no such file`. It always fails the run.
- A syntax error about two places names the second: `(started at 1:10)` for an unterminated
  string or comment, `(opened at 1:1)` for an object or array still open at the end.
- Some syntax errors end with a hint after a `;`, as in
  `comments are not allowed in json; the jsonc dialect allows this`.
- A UTF-16 or UTF-32 file fails with `file is UTF-16 (LE); jsonvet reads UTF-8`. Any other file
  that is not UTF-8 fails with `not valid UTF-8 (byte N)`.
- In GitHub Actions, each problem is also an annotation.

jsonvet escapes control, bidirectional and invisible characters in file names and messages, so no
output line can drive a terminal or pass for a GitHub workflow command
([the exact forms](design.md#output)). A file name that is not UTF-8 shows with U+FFFD in text,
JSON and annotations, and exactly in SARIF.

## Exit status and files

| Exit | When                                                     |
| ---: | -------------------------------------------------------- |
|    0 | no file has an error, or there were no files             |
|    1 | a file has an error, or a file cannot be read            |
|    2 | a usage error, such as an unknown option or a bad schema |

jsonvet reads regular files, and follows a symlink to one. A device, FIFO or socket prints
`error: not a regular file`. When the output pipe closes early, as in `jsonvet … | head -1`,
jsonvet stops quietly, with the verdict for the files it has checked.

## Standard input

`-` reads standard input, named `-` and checked as JSON5. `--stdin-filename PATH` names it `PATH`
instead, in the output and for picking its dialect and settings, and makes the `-` optional. Empty
input is a syntax error. To check a file named `-`, pass `./-`. jsonvet reads standard input only
when asked, so `echo '{}' | jsonvet` checks nothing and exits 0.

## File names that look like options

pre-commit passes file names with no `--` before them. So an unknown `-x` or `--name`, or a known
option with a value such as `--dialect=json`, is read as a file name when a file of exactly that
name exists. To be sure, put `--` before the file names: `jsonvet -- --x.json`.

## Dialects

- Extensions match in lower case only, so `DATA.JSONL` is checked as JSON5.
- `--dialect` and the config file also take `ijson`, `hujson` (for `jwcc`), and `ndjson` and
  `json-lines` (for `jsonl`). Dialect names are case-sensitive.
- JSONC rejects trailing commas, as the [JSONC spec](https://jsonc.org/) and Microsoft's
  reference parser do by default. For `tsconfig.json`, VS Code's `settings.json` and other files
  that use them, select `jwcc`.
- JSONC, JWCC and JSON5 each add syntax to the one before, except where a `//` comment ends. JWCC
  ends it at LF only, JSONC also at a lone CR, and JSON5 also at U+2028 and U+2029.

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

jsonvet could not check the file at all: it does not exist, is a device, FIFO or socket, could
not be read, or is not UTF-8. It always fails the run, and no option changes that.
Text output shows no rule for it; SARIF and JSON output file it under `unreadable`.

### schema

A value the file's JSON Schema rejects, such as a missing required key or a string where the
schema wants a number. The schema comes from `--schema`, the config file's `schemas` or a local
`$schema` in the file. Always an error: `--error`, `--warn`, `--ignore` and settings files do not
take this rule.


## Globs

- A glob without `/` matches the file name at any depth. A glob with `/` matches the whole path,
  and `*` also matches `/`.
- A glob compares exact code points. A `\` in a path is read as `/`.
- On the command line, a glob is relative to the working directory. pre-commit runs from the
  repository root.
- In a config file, a glob is relative to the folder that holds the file. With `--config`, a file
  outside that folder matches none of its globs.

## Configuration file

jsonvet reads `jsonvet.toml`, or a `[tool.jsonvet]` table in `pyproject.toml`. Each file takes the
nearest one in its folder or a parent, which replaces any further up: nothing is merged. The
search stops at the first folder with a `.git` entry, or without one at the working directory. A
file outside the working directory takes only its own folder's. Standard input is looked up by
its `--stdin-filename`, or from the working directory.

When the search finds nothing, jsonvet reads your own `jsonvet/jsonvet.toml` in
`$XDG_CONFIG_HOME` (or `~/.config`) on Linux and macOS, or `%APPDATA%` on Windows, with globs
relative to the working directory. `--config FILE` reads that file for every file instead, and
`--no-config` reads none. Options on the command line win.

A config file that is not a regular file is skipped, and one may hold at most 16 MB. An error in
a symlinked config file names its line and column, but not the key or value. An unknown key is a
usage error that names the closest known one.

```toml
[rules]
byte-order-mark = "error"

[[dialects]]
files = ".vscode/*.json"
dialect = "jwcc"
```

- `rules` sets a severity per rule.
- `dialects` picks a dialect per glob. The last matching entry wins.
- `exclude` lists globs for a folder walk to skip. A walk uses each folder's nearest config file.
- `schemas` checks the files a glob matches against a schema on disk, as `--schema` does:
  `schemas = [{ files = "config/*.json5", schema = "schemas/server.schema.json" }]`. The schema
  path is relative to the config file's folder, and the last matching entry wins. `--schema`
  replaces them all. A schema here that jsonvet cannot read or use is a usage error, even when no
  file matches its glob. In a config file the search finds, the schema must be inside the project
  ([JSON Schema limits](#json-schema-limits)).

## GitHub Action

All inputs are optional:

| Input     | What it does                                                                                |
| --------- | ------------------------------------------------------------------------------------------- |
| `files`   | paths or globs to check, separated by spaces; by default every tracked file the hook checks |
| `args`    | extra options, separated by spaces, such as `--strict package.json`                         |
| `sarif`   | also write a SARIF file to this path, besides the usual log and annotations                 |
| `version` | the jsonvet version to install: `X.Y.Z` (also with a suffix such as `rc1` or `.post1`), `X.*`, `X.Y.*` or `latest` |
| `verify`  | `true` (the default) checks the wheel's attestation before installing it; `false` skips that |

In `files`, `**` matches folders at any depth on every runner, and a glob matches files only. The
Action skips symlinks, and by default submodules. With the default `files`, run `actions/checkout`
first. With no JSON files, the step passes with a warning. Windows limits a command line to 32,767
characters, so with thousands of JSON files on a Windows runner, narrow `files`.

By default the Action installs the release that matches its own tag: `@v3` gets the newest 3.x on
PyPI, and `@v3.0.0` gets 3.0.0. Pinned to a commit or a branch, it installs the release named in
that commit's `Cargo.toml`, not the code in it. Before it installs the wheel, it checks that the
wheel came from jsonvet's `release.yml` ([how](../SECURITY.md#verifying-a-release)). On a runner
without `gh`, or for a release before 3.1.0, it installs the wheel unchecked, with a notice. Set
`verify: false` on GitHub Enterprise Server.

## SARIF

`--format sarif` prints one SARIF 2.1.0 document instead of text lines:

```bash
jsonvet --format sarif $(git ls-files '*.json') > jsonvet.sarif
```

- Each problem is a result with its rule, severity, message and position. The position ends
  (`endColumn`, and `endLine` when needed) just past the problem's text. A problem between two
  characters, such as a missing `,`, has no end, and nor has a schema error. A file jsonvet cannot
  read is a result under the rule `unreadable`.
- Each rule carries its help text and a link to its section above.
- GitHub keeps only 5,000 results of a run with more than 25,000. So jsonvet writes the first
  25,000, and counts the rest in a tool notification and on stderr.

The exit status and `--max-warnings` work as with text, and no annotations are printed. The README
has an [upload workflow](../README.md#github-code-scanning).

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


- Strings are escaped as in SARIF, so a JSON reader gets the name and message back unchanged.
- Each diagnostic is on a line of its own, written as files are checked. The exit status and
  `--max-warnings` work as with text. No annotations or count are printed.

## JSON Schema limits

These hold for every schema: from `--schema`, a config file or a file's `$schema`. Why each one
exists is in [design.md](design.md#--schema).

- **Which schema.** `--schema` applies to every file. Without it, a file takes the schema of the
  last matching `schemas` entry in its config file. Without one, a file whose top-level object has
  a `$schema` path, not a URL, is checked against that file, relative to the file's folder. A
  fragment, as in `s.json#`, is dropped. A `$schema` URL, a `$schema` in JSON Lines, and a
  `$schema` path to a missing file are skipped. A `$schema` file that exists but cannot be read or
  used is a schema error at the `$schema` value. A schema named by `--schema` or a config file
  that is missing or cannot be used is a usage error (exit 2).
- **Inside the project.** A schema named by a `$schema`, or by a config file the search finds, must
  be inside the checked file's own project: the folder where the [config
  search](#configuration-file) stops. A path that is absolute, has a drive or network prefix such as
  `C:\x` or `\\host\share`, climbs out with `..`, or leads out through a symlink is refused, on
  every OS and whether or not the file exists. For a `$schema`, that is a schema error at its value.
  For a config file entry, it is a usage error at the entry.

  Errors from such a schema quote nothing from it, since the file may not be yours to show. They
  name the keyword and, where jsonvet can tell, its place, as in
  `value fails "enum" (the schema s.json:3:11)`. That hides `const` and `enum` values, patterns,
  the property names `required` and `dependentRequired` ask for, the limits of `minimum`,
  `maxLength` and the like, and the schema under `not`. The checked
  file's own keys and values, type names and format names may still show.

  `--schema`, `--config FILE` and your own config file may name a schema anywhere, and their
  errors quote it. So on pull requests, do not pass `--config` a file the pull request can change:
  it could name any file on the runner, and parts of it could show in the log
  ([0012](decisions/0012-schema-paths-stay-in-the-project.md)).
- **Severity.** Schema errors are always errors. `--error`, `--warn`, `--ignore` and the config
  file do not take `schema` (exit 2).
- **Errors.** A document lists at most 100 schema errors: the first 100 jsonschema finds, printed
  in file order. If there are more, a line `... and more schema errors in this document` follows
  at the document's start, with no count. SARIF and JSON leave that line out.
  `additionalProperties` gives one error per key it does not allow.
- **Messages.** Each message ends with the value's JSONPath, and is cut at 200 bytes. The wording
  comes from the jsonschema crate and may change between releases. The rule id and the position
  are stable.
- **Depth.** With a schema, a file nested deeper than 512 levels is a syntax error. A file with a
  syntax error gets no schema errors.
- **Values.** Each line of a JSON Lines file is checked on its own. `NaN`, `Infinity` and numbers
  past a double's range have no JSON Schema value, so a document that holds one is reported once
  and not checked. So is an object with two keys that differ only in lone surrogates, since both
  become U+FFFD.
- **Memory.** jsonvet holds one document's values at a time: 4 to 22 times its size, the most for
  many small records. A JSON Lines file holds one line's values, or one per thread, and each line
  with errors adds about 700 bytes. When only `items` checks a top-level array's elements, or only
  `properties`, `patternProperties` and `additionalProperties` check a top-level object's members,
  jsonvet checks one element or member at a time. Otherwise jsonschema holds every error in the
  document at once: about 420 bytes each, and more for one that holds part of the schema, such as
  an `anyOf` error.
- **Regexes.** A pattern with a back-reference or lookaround may backtrack 5,000 times per string.
  A string or key that needs more, where the pattern decides the result, is an error saying the
  pattern could not be checked. A document that also has an error either way reports only that
  one.
- **The schema file.** It may be in any dialect. Its `$schema` picks the draft, from draft 4 to
  2020-12. A schema jsonvet cannot read or use is a usage error (exit 2). That covers:
  - a `$ref` to another file, or to nothing in the file, even one nothing uses. A `$ref` resolves
    as jsonschema resolves it, against the base the `$id`s above it set;
  - nesting deeper than 128 levels, where each `$ref` followed counts as a level;
  - more than 5,000,000 objects and arrays with each `$ref` replaced by what it points at, every
    time it is followed, and every value in each `enum` and `const`. Beside
    `unevaluatedProperties` or `unevaluatedItems`, the schemas that apply to the same value
    (`allOf`, `anyOf`, `oneOf`, `if`, `then`, `else`, `dependentSchemas` and `$ref`s) count
    twice. SchemaStore's 1.6 MB `azure-pipelines.json` comes to about 3,210,000.

  A `$ref` back to a schema it is inside, as in `{"items": {"$ref": "#"}}`, is recursion and is
  fine.

## Coming from check-json5

The `check-json5` package on PyPI keeps working through 3.x: it installs jsonvet. Besides the
dialect change in the README, the output line, syntax-error wording, positions after a lone CR or
U+2028, the hook's `.jsonl` and `.ndjson` files and the Python module changed
([full list](design.md#the-contract-with-14)).
