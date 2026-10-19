# jsonvet reference

The fine print behind the [README](../README.md): every option, how globs and file names work, the
exit status, the GitHub Action's inputs, and the limits of `--schema`. Why the rules are as they
are is in [design.md](design.md).

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

`jsonvet --help` prints the same list with more detail.

## Globs

A glob without `/` matches the file name at any depth. A glob with `/` matches the whole path, and
`*` also matches `/`.

A glob compares exact code points, with no Unicode normalization. A `\` in a path is read as `/`.

A glob on the command line matches the path as you typed it, so it is relative to the working
directory. pre-commit runs from the repository root. A glob in a config file is relative to the
directory that holds that file (see [Configuration](../README.md#configuration)).

## File names that look like options

pre-commit passes file names with no `--` before them. So jsonvet reads an unknown `-x` or
`--name`, or a known option with a value such as `--dialect=json`, as a file name when a file of
exactly that name exists. Otherwise an unknown option is a usage error (`unknown option`).

To be sure, put `--` before the file names: `jsonvet -- --x.json`.

## Exit status and files

| Exit | When                                                       |
| ---: | ---------------------------------------------------------- |
|    0 | no file has an error, or there were no files               |
|    1 | a file has an error, or a file cannot be read              |
|    2 | a usage error, such as an unknown option or a bad schema   |

A file that does not exist or cannot be read prints `name: error: <reason>`, with no line or
column. In SARIF it is a result under the rule `unreadable`.

jsonvet reads regular files only, and follows a symlink to one. A directory, device, FIFO or
socket prints `error: not a regular file`. The hook (`types: [text]`) skips tracked symlinks and
submodules.

When the output pipe closes early, as in `jsonvet … | head -1`, jsonvet stops quietly. It exits
with the verdict for the files it has checked.

jsonvet escapes control, bidirectional and invisible characters in file names and messages, so no
output line can drive a terminal or pass for a GitHub workflow command. The exact forms are in
[design.md](design.md#output).

## GitHub Action

All inputs are optional:

| Input     | What it does                                                                                |
| --------- | ------------------------------------------------------------------------------------------- |
| `files`   | paths or globs to check, separated by spaces; by default every tracked file the hook checks |
| `args`    | extra options, separated by spaces, such as `--strict package.json`                         |
| `sarif`   | also write a SARIF file to this path (see [GitHub code scanning](../README.md#github-code-scanning)) |
| `version` | the jsonvet version to install: `X.Y.Z` (also with a suffix such as `rc1` or `.post1`), `X.*`, `X.Y.*` or `latest` |

In `files`, `**` matches folders at any depth on every runner. The Action skips symlinks, and by
default also submodules. Any other `version` value stops the step.

By default the Action installs the release that matches its own tag: `@v3` gets the newest 3.x,
and `@v3.0.0` gets 3.0.0. Pinned to a commit SHA or a branch, it installs the release named in
that commit's `Cargo.toml`. Between releases that is the last release, not the code in the
commit. The step log names the version it installed.

Pinning the Action does not pin the wheel's bytes: pip installs `jsonvet==X.Y.Z` from PyPI without
a hash check.

## JSON Schema limits

These hold for `--schema`. Why each one exists is in [design.md](design.md#--schema).

- **Errors.** A document lists at most 100 schema errors. They are the first 100 that jsonschema
  finds, and it takes an object's keys in sorted order, not file order. They print in file order.
  Then one more line, `... and N more schema errors in this document`, sits at the document's
  start. SARIF leaves that line out. `additionalProperties` gives one error per key it does not
  allow, at that key, and each one counts toward the 100.
- **Messages.** A message quotes the value unless that makes it long; then it says `value`. A
  message, with its path, is cut at 200 bytes and ends in `...`. The wording comes from the
  jsonschema crate and may change between releases. The rule id `schema` and the position are
  stable.
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
