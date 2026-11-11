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
Only files on disk are checked: an untitled file has no path to pick its settings by. VS Code
knows no JSON5 or JWCC language, so the extension adds them for `.json5`, `.jwcc` and `.hujson`
files, without syntax highlighting.

A change to a `jsonvet.toml` or `pyproject.toml` checks every open file again.

## VS Code's own JSON validation

VS Code's own JSON validation also reports syntax errors and duplicate keys in JSON and JSONC
files. While it is on (`json.validate.enable`, on by default), the extension leaves those two to
it and shows jsonvet's other problems beside it, so none shows twice. VS Code also checks the JSON
Schemas it knows, such as those it downloads from SchemaStore for `package.json` and
`tsconfig.json`; jsonvet checks only the local schemas a file's `$schema` or `jsonvet.toml` names,
and fetches nothing. JSON5 and JSON Lines files get all of jsonvet's problems.

The two do not agree on trailing commas in JSONC. jsonvet's `jsonc` rejects them, VS Code only
warns, and it allows them without a warning in files whose schema permits them, such as
`*.code-workspace`. There, `jsonvet` on the command line fails where the editor shows nothing.
Check such files as `jwcc` (a `[[dialects]]` entry in `jsonvet.toml`), or turn VS Code's
validation off.

To see jsonvet's syntax errors and duplicate keys instead, turn VS Code's validation off, which
also turns off its schema checks:

```json
{ "json.validate.enable": false }
```

The first time the extension starts in a workspace folder, it offers to set that in
`.vscode/settings.json`, and asks again next time if you close the message without a choice. The
change restarts the server to refresh diagnostics.

## Formatting

VS Code also has a JSON formatter. To format with jsonvet, pick it as the default:

```json
{
  "[json]": { "editor.defaultFormatter": "simonvanlierde.jsonvet" },
  "[jsonc]": { "editor.defaultFormatter": "simonvanlierde.jsonvet" }
}
```

Format Document formats the whole file and Format Selection changes only the text you selected:
the rest of the file stays byte for byte as it was. The layout comes from `jsonvet.toml` and
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

The extension does not run in Restricted Mode: a `jsonvet` found on `PATH` could be one that the
folder you opened holds.

## Building

The extension is not on the Marketplace yet. Build it from the repository root, then install it:

```bash
just vsix    # writes editors/vscode/jsonvet-<version>.vsix
code --install-extension editors/vscode/jsonvet-<version>.vsix
```
