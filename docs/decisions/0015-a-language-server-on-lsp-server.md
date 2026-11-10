# 15. A language server in the CLI, on lsp-server, with messages in serde_json

Status: accepted (2026-11-06)

## Context

Editors should show jsonvet's problems as you type, offer the safe fixes and format a file or a
selection. The ROADMAP has asked for a language server with a thin VS Code client since #188,
rather than an extension that runs the CLI on each change, so any editor that speaks the Language
Server Protocol gets it.

## Options

- **tower-lsp or tower-lsp-server**: async, with typed messages. They pull in tokio and
  lsp-types for a server that answers one request at a time and checks a 4 MB file in 33 ms.
  Argued, not measured.
- **lsp-server with lsp-types**: rust-analyzer's synchronous framing and handshake, plus typed
  messages. lsp-types 0.97.0 was last released in 2024, and brings bitflags 1, serde_repr and a
  second fluent-uri (0.1, beside jsonschema's 0.4) for the dozen fields jsonvet reads and writes.
- **lsp-server alone**, reading and building messages as `serde_json::Value`, which the CLI
  already uses, as it writes SARIF by hand
  ([0007](0007-hand-written-output-with-one-escaping-rule.md)).
- **A cargo feature** to leave the server out of the wheels. The release binary grew from 7.39 to
  7.61 MB (+2.9%, x86-64 Linux, 2026-11-06), and the wasm build does not depend on the CLI crate.

## Decision

`jsonvet lsp` in every build, on lsp-server 0.10 alone. It resolves settings and schemas through
the same code as a run (`check_file` in `crates/jsonvet-cli/src/lib.rs`, `format::setup`), and
fixes through the library's `try_fix_at`, so the editor and the command agree. Range formatting
keeps only the edits of a full format that fall in the range, and only if formatting the result
gives what formatting the file does.

## Consequences

A message field the server reads is a string in the code, not a type: a typo is a bug no compiler
finds, which the in-process tests in `lsp.rs` guard. Each request makes a new config resolver, and
a schema is read again for each check. Where the client allows it, the server has it watch
`jsonvet.toml`, `pyproject.toml`, the user's config file and the local schemas used by open
documents. The schema watches change with the open documents. A change to a watched file checks
every open document again. Changes queued behind a running check are checked once, at their
latest text, so a slow check does not fall behind typing.

The config search stops at the workspace folder holding the document, or a nearer `.git`
directory. The server takes folders from `initialize` and follows later folder changes. A
`.git` directory above the workspace does not widen its search or the schema confinement root
of [0012](0012-schema-paths-stay-in-the-project.md). A file outside all workspace folders,
including when no folder is open, stops at its own folder.

## Confirmation

`cargo nextest run -p jsonvet-cli lsp` runs the server's tests through its message loop, and
`lsp_checks_a_document_over_stdio_and_exits_0_after_shutdown` in `tests/cli.rs` runs the binary.

## Revisit if

The server needs to do work in parallel or cancel it, such as checks that take longer than a
keystroke, or the protocol types it uses grow past what is easy to keep by hand.
