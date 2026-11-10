# Editors

`jsonvet lsp` is a language server: it speaks the Language Server Protocol on standard input and
output, so any editor that has a client for it can show jsonvet's problems as you type. It offers:

- **Problems** in each open file, checked as `jsonvet FILE` would check it: the dialect, rules and
  schema come from the nearest `jsonvet.toml` and the file's own `$schema`
  ([configuration](usage.md#configuration-file)). A config file or schema that cannot be used is
  one error at the start of the file.
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

Only files on disk are checked (`file:` URIs), since the settings are picked by the file's path. A
file with a syntax error is not formatted. Positions are counted in UTF-16 code units, as the
protocol asks by default, or in bytes when the editor offers UTF-8. `jsonvet lsp` takes no other
options but `--help` and `--version`; `--stdio` is accepted, as some clients pass it. A file named
`lsp` needs `./lsp`, as `fmt` does.

The search for `jsonvet.toml` stops at the workspace folder that holds the file, as a run's stops
at its working directory, unless a `.git` above the file comes first. The server takes the folders
from the editor's `initialize` request (`workspaceFolders`, else `rootUri`) and follows the folders
the editor adds and removes later. With no folder, the search stops at the file's own folder,
unless a `.git` above it comes first.

When the editor can watch files for the server, the server asks it to watch `jsonvet.toml`,
`pyproject.toml` and the user's own config file, and checks every open file again when one
changes. Other clients can send `workspace/didChangeWatchedFiles` for those files themselves.

The server checks the whole file on each change. A 4 MB file took 33 ms (2026-11-06, release
build), so it does not wait for typing to stop. Changes that arrive while a check runs are checked
once, at the latest text.

## VS Code

The extension in [editors/vscode](../editors/vscode/README.md) is a thin client for `jsonvet lsp`.
Its README says how it shares JSON and JSONC files with VS Code's own JSON validation.

## Neovim

With Neovim 0.11 or later, in `init.lua`:

```lua
vim.lsp.config("jsonvet", {
  cmd = { "jsonvet", "lsp" },
  filetypes = { "json", "jsonc", "json5" },
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
```

Listing jsonvet alone replaces Helix's default JSON server, so problems do not show twice.

## Other editors

Run `jsonvet lsp` as the server command for JSON, JSONC, JSON5 and JSON Lines files, over
standard input and output.
