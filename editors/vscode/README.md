# jsonvet for VS Code

Shows jsonvet's problems in JSON, JSONC, JWCC, JSON5 and JSON Lines files as you type, offers the
safe fixes as quick fixes, and formats a file or a selection as `jsonvet fmt` does. It is a thin
client for `jsonvet lsp`, the language server in the jsonvet command, so it needs jsonvet 3.4 or
later installed:

```bash
uv tool install jsonvet     # or: pipx install jsonvet, cargo binstall jsonvet-cli
```

The extension runs `jsonvet` from your `PATH`, or the program the `jsonvet.path` setting names. It
checks each file with the dialect, rules and schema a `jsonvet` run would use for that path, from
the nearest `jsonvet.toml` ([configuration](https://github.com/simonvanlierde/jsonvet#configuration)).
Only files on disk are checked: an untitled file has no path to pick its settings by.

A change to a `jsonvet.toml` or `pyproject.toml` checks every open file again.

## VS Code's own JSON validation

VS Code's own JSON validation also reports syntax errors and duplicate keys in JSON and JSONC
files. While it is on (`json.validate.enable`, on by default), the extension leaves those two to
it and shows jsonvet's other problems beside it, so none shows twice. VS Code also checks the JSON
Schemas it knows, such as those it downloads from SchemaStore for `package.json` and
`tsconfig.json`; jsonvet checks only the local schemas a file's `$schema` or `jsonvet.toml` names,
and fetches nothing. JSON5 and JSON Lines files get all of jsonvet's problems.

To see jsonvet's syntax errors and duplicate keys instead, turn VS Code's validation off, which
also turns off its schema checks:

```json
{ "json.validate.enable": false }
```

The first time the extension starts in a workspace folder, it offers to set that in
`.vscode/settings.json`, and asks again next time if you close the message without a choice. The
change shows on each file at its next edit.

## Formatting

VS Code also has a JSON formatter. To format with jsonvet, pick it as the default:

```json
{
  "[json]": { "editor.defaultFormatter": "placeholder-publisher-id.jsonvet" },
  "[jsonc]": { "editor.defaultFormatter": "placeholder-publisher-id.jsonvet" }
}
```

Format Document formats the whole file and Format Selection only the lines you selected: the
rest of the file stays byte for byte as it was. The layout comes from `jsonvet.toml` and
`.editorconfig`, as for `jsonvet fmt`, not from the editor's tab size, so `jsonvet fmt --check`
agrees with the editor.

To apply every safe fix when you save:

```json
{ "editor.codeActionsOnSave": { "source.fixAll.jsonvet": "explicit" } }
```

## Settings

- `jsonvet.path`: the `jsonvet` program to run. Empty runs `jsonvet` from `PATH`. A user setting
  only, so a folder you open cannot point it at a program of its own. Reload the window after you
  change it.

The extension does not run in Restricted Mode: on Windows, `jsonvet` from `PATH` could be a
`jsonvet.exe` in the folder you opened.

## Building

```bash
just vsix    # from the repository root: editors/vscode/jsonvet-<version>.vsix
```
