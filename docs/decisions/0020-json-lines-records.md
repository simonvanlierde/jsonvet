# 20. JSON Lines output: the JSON diagnostic, a record per file, a summary last

Status: accepted (2026-11-28)

## Context

`--format json` is one document, so a run that is killed or ends on an error leaves output that
does not parse, and a clean file leaves no trace. A reader that wants partial reports, or a final
status record it can trust, needs one line per finished item (#290).

## Options

- **One `diagnostic` record per problem, nothing else.** Smallest, but a clean file is
  invisible, so a partial report cannot tell a clean file from one never reached, and there is
  no status record.
- **One `file` record per file, holding its diagnostics.** One line per file, but a new shape
  for the problems, so a reader needs a second model, and a file with 100,000 problems is one
  line.
- **`diagnostic` records as in JSON, a `file` record after each file, and a `summary` last.**
  The problem shape stays the JSON report's; a file record marks each finished file.

Argued, not measured.

## Decision

The third option, as `--format jsonl`. The dialect `jsonl` names JSON Lines input under
`--dialect`; the same word under `--format` names JSON Lines output, so the two never meet in one
flag.

- Every record has `type` (`diagnostic`, `file` or `summary`) and the JSON report's
  `schema_version`; both formats share one version, so a bump covers both.
- A `diagnostic` record is the JSON diagnostic with those two members in front.
- A `file` record follows each file's diagnostics with `error_count` and `warning_count`.
  jsonvet flushes standard output after it.
- The `summary` has `files`, `files_with_problems`, `error_count`, `warning_count` and
  `exit_code`, and `notes` as in JSON. `files_with_problems` counts a file with only a warning,
  on a run that can pass, so it is not called "failed". The counts are not named `errors`, which
  keeps JSON's meaning: a usage error, a bad schema or an error later in the run is a `summary`
  with `exit_code` 2 and `errors`, and no counts. Unlike JSON, that record also comes after other
  output, since another line still parses.
- Strings go through the one escaping rule ([0007](0007-hand-written-output-with-one-escaping-rule.md)).

## Consequences

One flush per file, which costs a write call each; not measured on a large walk. Records come in
the order files are given, a chunk at a time, not as each core finishes. The summary is written
once the exit code is known, after the fixed text and the fix count, so a run has one summary
and its `exit_code` is the process's; a failed write to standard output can leave it out.

## Confirmation

`json_lines_stream_records_and_end_with_a_summary` in `crates/jsonvet-cli/tests/cli.rs`, which
also checks each line against `$defs/record` in `docs/json-output.schema.json`, and the unit tests
in `json.rs`.

## Revisit if

A reader needs records as each core finishes rather than in order, or the per-file flush shows up
in a measured walk.
