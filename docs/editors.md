# Editors

Set up jsonvet in VS Code, Neovim, Helix or another editor to see problems as you type and apply
fixes. The language server (`jsonvet lsp`) speaks the Language Server Protocol on standard input
and output.

- **Problems** in each open file, checked as `jsonvet FILE` would check it. The dialect, rules and
  schema come from the nearest `jsonvet.toml` and the file's own `$schema`
  ([configuration](usage.md#configuration-file)), but the config search stops at the workspace
  folder (below). A config file or schema that cannot be used is one error at the start of the
  file.
- **Quick fixes** for the problems that have a safe fix, as `--fix` makes it
  ([fixing](usage.md#fixing)), and one action that applies them all, of the kind
  `source.fixAll.jsonvet`. Duplicate keys get no fix: dropping one changes what the file means.
  Clients that support code-action resolution receive edits when they request an action.
  Other clients receive the edits with the full action list.
- **Formatting** of the whole file, as `jsonvet fmt` does, with the same settings
  ([formatting](format.md#settings)). The editor's own tab size is not used, so `jsonvet fmt
  --check` agrees with the editor.
- **Range formatting**: only the edits inside the selection, so the rest of the file stays byte
  for byte as it was. An edit that would break the file without one outside the selection, such as
  a comma the formatter moves to the other side of a comment, is left out.

The server checks documents with a `file:` URI so it can pick settings by path. This includes a
new file that has a path before its first save. A file with a syntax error is not formatted.
Positions count UTF-16 code units by default or bytes when the editor offers UTF-8. `jsonvet lsp`
accepts `--help`, `--version` and `--stdio`. A file named `lsp` needs `./lsp`, as `fmt` does.

The search for `jsonvet.toml` stops at the workspace folder that holds the file, unless a nearer
`.git` entry stops it first. The server takes the folders from the editor's `initialize` request
(`workspaceFolders`, else `rootUri`) and follows the folders the editor adds and removes later.
For a file outside those folders, including when no folder is open, the search stops at the
file's own folder.

When the editor can watch files for the server, the server asks it to watch `jsonvet.toml`,
`pyproject.toml`, the user's own config file and the local schemas used by open files. It
checks every open file again when one changes. Schema watches follow the files as they open,
change schemas and close. Other clients can send `workspace/didChangeWatchedFiles` for those
files themselves.

The server checks the whole file on each change without waiting for typing to stop. Changes that
arrive while a check runs are checked once, at the latest text.

To limit how many diagnostics arrive per file, send a positive integer `maxProblems` in the
`initialize` request's `initializationOptions`. The server sends syntax and rule findings first,
then schema findings, stopping at the limit. A missing or invalid value, or `0`, sends all
diagnostics. Checking and quick fixes still use the whole file.

## VS Code

The extension in [editors/vscode](../editors/vscode/README.md) is a thin client for `jsonvet lsp`.
Its README says how it shares JSON and JSONC files with VS Code's own JSON validation.

The extension is not on the Marketplace yet. Build it with `just vsix` from the repository root,
then install it:

```bash
code --install-extension editors/vscode/jsonvet-<version>.vsix
```

## Neovim

With Neovim 0.11 or later, in `init.lua`:

```lua
vim.lsp.config("jsonvet", {
  cmd = { "jsonvet", "lsp" },
  filetypes = { "json", "jsonc", "json5", "jsonl" },
  root_markers = { "jsonvet.toml", ".git" },
})
vim.lsp.enable("jsonvet")
```

`vim.lsp.buf.format()` formats the file, or the selection in visual mode.

## Helix

In `languages.toml`:

```toml
[language-server.jsonvet]
command = "jsonvet"
args = ["lsp"]

[[language]]
name = "json"
language-servers = ["jsonvet"]

[[language]]
name = "jsonc"
language-servers = ["jsonvet"]

[[language]]
name = "json5"
language-servers = ["jsonvet"]
```

Listing jsonvet alone replaces Helix's default JSON server, so problems do not show twice. Helix
treats `.jsonl` files as `json`, so they are covered. It has no JWCC language.

## Other editors

Run `jsonvet lsp` as the server command for JSON, JSONC, JSON5 and JSON Lines files, over
standard input and output.
