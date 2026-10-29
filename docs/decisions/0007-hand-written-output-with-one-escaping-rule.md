# 7. SARIF written by hand, with one escaping rule for every output

Status: accepted (2026-10-14)

## Context

File names and messages come from strangers' files. In a terminal, control and bidirectional
characters can move the cursor or hide text; in GitHub Actions, a line starting with `::` or `##[`
is a workflow command. Injection got through twice before we fixed it (#27, #52).

## Options

- **serde_json for SARIF.** It escapes only quotes, backslashes and control characters, so
  bidirectional and invisible ones need a second pass.
- **Hand-written output with one escaping function** shared by every format.

Argued, not measured.

## Decision

Write SARIF by hand (#38), and pass every string through the one escaping rule in
[design.md](../design.md#output). `--format json` (#74) uses it too.

## Consequences

One function to test for every output. SARIF keeps the first 25,000 results, since GitHub keeps
only 5,000 of a run past that and rejects one past 1,000,000. SARIF results are written as they
come, as JSON diagnostics are.

## Confirmation

`each_output_path_escapes_names_and_messages`, `usage_errors_show_control_characters_escaped` and
`sarif_keeps_other_brackets` in `crates/jsonvet-cli/tests/cli.rs`, and the unit tests in
`output.rs`.

## Revisit if

A serde_json-based crate offers escaping that covers the same characters.
