# 7. SARIF written by hand, with one escaping rule for every output

Status: accepted (2026-10-14)

## Context

File names and messages come from the files jsonvet checks, and in CI those can come from a
stranger's pull request. Each output has its own attack:

- in a terminal, control and bidirectional characters can move the cursor or hide text;
- in GitHub Actions, a line that starts with `::` or `##[` is a workflow command;
- in SARIF, the same text ends up in code scanning alerts.

Workflow-command injection has happened twice. Before 2.0, a newline in a file name could print
a workflow command (fixed in #27). Before 3.0, a file named `--format=` plus a newline did the
same through a usage error (fixed in #52).

## Options

- **serde_json for SARIF.** Correct JSON, but serde_json escapes only quotes, backslashes and
  control characters. The bidirectional and invisible characters would need a second pass, and
  at the time serde_json was not a dependency of the CLI.
- **Hand-written output, with one escaping function** shared by text lines, annotations and
  SARIF.

Neither was measured. The choice rests on the argument above.

## Decision

Write SARIF by hand (#38, merged 2026-10-14). Every string goes through the same escaping, in
[design.md](../design.md#output): control characters as `\u001b` (CR and LF as `\r` and `\n`),
bidirectional and invisible characters as `\u202e` or a UTF-16 pair, a name that starts with
`::` as `./::…`, and a `[` after `##` as `\u005b`. An annotation's `file=` value holds the real
name, percent-encoded as GitHub requires.

## Consequences

- One escaping function to test for all three outputs. #38 checked the SARIF against the 2.1.0
  schema.
- SARIF keeps the first 25,000 results and counts the rest. GitHub keeps only 5,000 results of a
  run past 25,000 and rejects one past 1,000,000, and one 4 MB JSON Lines file reached 1,000,000.
- Results are buffered until the end: 1.0 GB peak at 1,000,000 results before the cap. Streaming
  waits for a memory complaint.
- serde_json has been in the CLI since `--schema` (#50), so the dependency half of the argument
  is gone. The escaping half stands.

## Revisit if

- a new output format needs a full JSON writer we would otherwise rebuild;
- serde_json, or a crate on top of it, offers escaping we can configure to cover the same
  characters.
