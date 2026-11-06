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

## Problems shown twice

VS Code's own JSON validation also reports syntax errors and duplicate keys in JSON and JSONC
files, so those would show twice. The first time the extension starts in a workspace folder, it
asks whether to turn that validation off there, which sets this in `.vscode/settings.json`:

```json
{ "json.validate.enable": false }
```

That also turns off VS Code's JSON Schema checks, including the schemas it downloads from
SchemaStore for files such as `package.json` and `tsconfig.json`. jsonvet checks only the local
schemas a file's `$schema` or `jsonvet.toml` names, and fetches nothing. If you want SchemaStore's
schemas, keep both and accept that some problems show twice.

The extension does not turn the setting off on its own, and does not claim only some files: which
files VS Code validates depends on settings the extension cannot see, such as `files.associations`.

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
