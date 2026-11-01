# Formatting

`jsonvet fmt` rewrites JSON, JSONC, JWCC (HuJSON), JSON5 and JSON Lines files and keeps their
comments. Why it works this way is in [design.md](design.md).

```bash
jsonvet fmt config/ package.json   # rewrite the files in place
jsonvet fmt --check .              # list the files that would change; exit 1 if any
jsonvet fmt - < in.json            # standard input to standard output
```

A file literally named `fmt` needs `./fmt`, because `fmt` as the first argument is the
subcommand. Plain `jsonvet FILE...` still checks.

The pre-commit hook is `jsonvet-format`:

```yaml
- repo: https://github.com/simonvanlierde/jsonvet
  rev: v3.2.0
  hooks:
    - id: jsonvet-format
```

## What it does

- A container goes on one line if it fits in the line width. If not, each member gets its own
  line. An empty container is `{}` or `[]`. A one-line object has a space inside its braces,
  `{ "a": [1, 2] }`, and a one-line array has none. The spaces count toward the width.
- An object stays expanded if you put a line break between its `{` and its first key, even when
  it would fit on one line, and so do the containers around it. Arrays follow the width only.
  JSON Lines records are always one line.
- A comment inside a container puts that container, and the ones around it, on separate lines.
- A line comment stays at the end of its line. A block comment stays where it was next to its
  tokens. A comment on its own line stays on its own line.
- Blank lines between members stay, but at most one. Blank lines at the start and end of a
  container go.
- A block comment over several lines moves its later lines by as many columns as the comment
  itself moved. A line follows only if it has no indent or its indent starts with the formatter's
  indent character (a space, or a tab with `indent = "tab"`), and the move is a whole number of them; a line
  that cannot follow, a blank line, and a move that is not a whole number of tabs leave the
  lines as written. Line comments and one-line block comments are not touched.
- Strings and numbers keep their spelling, apart from line endings. No escape is added or removed.
- The file ends with exactly one newline. Line endings follow the first line ending in the file:
  `\r\n`, `\n` or a lone `\r`, as the [mixed-line-endings](usage.md#mixed-line-endings) rule sees
  it. JWCC uses `\n` in place of a lone `\r`, because a JWCC line comment ends only at `\n`.
  Line endings inside block comments and JSON5 line continuations follow the same one. A byte
  order mark stays.
- A second run changes nothing.

By dialect:

- JSON and I-JSON: the layout above, and no trailing commas.
- JSONC and JWCC: comments stay. jsonvet adds no trailing comma. In JWCC, a trailing comma stays
  where the container has several lines, and goes where it fits on one.
- JSON5: written as it was (quotes, unquoted keys, hex, `+1`, `.5`, `Infinity`, `NaN`), unless
  `normalize-json5` is on. A trailing comma stays where the container has several lines, and goes
  where it fits on one.
- JSON Lines: each record on one line as `{"a": 1, "b": [1, 2]}`, with no space inside the
  braces. The width does not apply.

A comment between a key and its colon stays there if it is a block comment. A line comment
there moves to after the colon.

A file with a syntax error is left as it is. jsonvet prints the error and exits 1.

## Settings

The defaults are 2 spaces, 80 columns and spaces inside braces. Each setting comes from the first
of these that sets it:

1. The `[format]` table of the project's `jsonvet.toml` (or `[tool.jsonvet.format]` in
   `pyproject.toml`), or of the file `--config` names.
2. `.editorconfig`.
3. The `[format]` table of your own `jsonvet/jsonvet.toml`, which jsonvet reads only when the
   search finds no project config file.
4. The default.

So the project decides, and `jsonvet fmt --check` gives the same answer on every machine. Your own
config fills in only what the project and its `.editorconfig` leave open.

```toml
[format]
indent = 4              # spaces from 0 to 16, or "tab"
width = 100             # columns, 1 or more
normalize-json5 = true  # JSON5 files only
bracket-spacing = false # {"a": 1} instead of { "a": 1 }
```

`.editorconfig` is read in part: `indent_style`, `indent_size` and `max_line_length`, in sections
with the glob `*`, `*.json` or `*.{json,jsonc}`, and `root = true`. Sections with a path in the
glob are skipped. A tab counts as four columns when jsonvet measures a line.

Each kind of file is looked up on its own:

- `jsonvet.toml` is found as for a check: the nearest one in the file's folder or a parent,
  up to the repository root. It replaces any further up, and nothing is merged.
- `.editorconfig` files are merged, nearer ones over farther ones, up to the repository root or
  a file with `root = true`.

So a `jsonvet.toml` setting wins over `.editorconfig` even when the `.editorconfig` is nearer.
A setting the nearest `jsonvet.toml` leaves out falls back to `.editorconfig`, then your own
config, then the default.

The dialect of a file is picked as for a check, by `--dialect`, `dialects` in `jsonvet.toml`,
or the extension. A `.json` file is JSON5 by default, so `normalize-json5` also applies to it.
To keep `.json` files as strict JSON, set their dialect:

```toml
dialects = [{ files = "*.json", dialect = "json" }]
```

## normalize-json5

With `normalize-json5 = true`, a JSON5 file gets:

- double quotes on strings, with `\'` written as `'` and a bare `"` written as `\"`;
- keys quoted only where an identifier will not do: `{'a': 1, "b-c": 2}` becomes
  `{a: 1, "b-c": 2}`. Keys with an escape stay quoted;
- a trailing comma in each container that has several lines, and none in one-line containers.

Number forms stay as written. The setting does nothing for the other dialects.

## Options

`jsonvet fmt` takes `--check`, `--dialect`, `--strict`, `--config`, `--no-config` and
`--stdin-filename`, which work as for a check. The options for checking (`--format`, `--color`,
`--schema`, `--max-warnings`, `--error`, `--warn`, `--ignore`, `--fix` and `--unsafe-fixes`) are a
usage error under `fmt`.
With no files, `jsonvet fmt` does nothing, as a bare `jsonvet` does. With `--check` nothing is
written. Each file that would change is printed, and the exit status is 1 if there is one.
Standard input goes to standard output; with `--check` it prints only its name.

Exit status: 0 when every file is formatted or was rewritten, 1 when a file has a syntax error,
cannot be read or written, or with `--check` would change, and 2 for a usage error.

A file is replaced through a temporary file in its folder and a rename. A symlink stays a
symlink, and the file keeps its permissions. The rename replaces the file, so another hard link to
it keeps the old content. A read-only file is not written: jsonvet says "the file is read-only"
and exits 1.

## Size limits

jsonvet keeps a tree of the file while it lays it out, which takes a few bytes of memory for each
byte of the file, plus the text and the output. A file of 88 MB takes about 300 MB.

Deep nesting makes the output much larger than the file, because each level adds its indentation
to every line below it. If the output would pass 16 times the size of the file, or 64 MiB if that
is more, jsonvet works out its size without building it, prints
`not formatted: the output would be 4.0 GB`, leaves the file as it is, and exits 1. A file over
512 MiB is refused the same way.

## Not done yet

- Range formatting and sorting keys.
- Filling a number array across lines, as Prettier does.
