# 7. SARIF written by hand, with one escaping rule for every output

Status: accepted (2026-10-14)

## Context

File names and messages come from the checked files, which in CI can come from a stranger's pull
request. In a terminal, control and bidirectional characters can move the cursor or hide text.
In GitHub Actions, a line starting with `::` or `##[` is a workflow command. SARIF carries the
same text into code scanning alerts. Workflow-command injection got through twice: a newline in
a file name before 2.0 (#27), and a file named `--format=` plus a newline before 3.0 (#52).

## Options

- **serde_json for SARIF.** It escapes only quotes, backslashes and control characters, so the
  bidirectional and invisible ones need a second pass.
- **Hand-written output with one escaping function** shared by text, annotations and SARIF.

Argued, not measured.

## Decision

Write SARIF by hand (#38) and pass every string through the one escaping rule in
[design.md](../design.md#output): control characters as `\u001b` (CR and LF as `\r` and `\n`),
bidirectional and invisible ones as `\u202e` or a UTF-16 pair, a name starting with `::` as
`./::…`, and a `[` after `##` as `\u005b`. An annotation's `file=` holds that escaped name,
percent-encoded as GitHub requires.

## Consequences

- One function to test for every output; #38 checked the SARIF against the 2.1.0 schema.
- SARIF keeps the first 25,000 results and counts the rest: GitHub keeps only 5,000 of a run past
  25,000 and rejects one past 1,000,000, which one 4 MB JSON Lines file reached.
- Results are held until the end, 1.0 GB at 1,000,000 before the cap. Streaming waits for a
  memory complaint.
- serde_json has been in the CLI since `--schema` (#50); the escaping argument stands.
- `--format json` (#74) is written the same way, with the same escaping.

## Confirmation

`each_output_path_escapes_names_and_messages`, `usage_errors_show_control_characters_escaped` and
`sarif_keeps_other_brackets` in `crates/jsonvet-cli/tests/cli.rs`, and the unit tests in
`output.rs`.

## Revisit if

- a crate on top of serde_json offers escaping that covers the same characters.
