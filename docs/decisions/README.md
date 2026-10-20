# Decision records

Each record here is a design decision where we measured or argued an alternative and turned it
down. It keeps the numbers and the reasons, so nobody has to argue it again without new
evidence. To reopen one, bring the evidence its "Revisit if" asks for, and change the record in
the same pull request as the code.

## Template

```markdown
# N. Title

Status: accepted (YYYY-MM-DD)

## Context

What forced a choice, in a few sentences.

## Options

Each option, with its measured numbers and their setting (date, machine, runs), or "argued, not
measured".

## Decision

What we do.

## Consequences

What it costs, what it gives, and what follows from it.

## Revisit if

The concrete evidence that would reopen it.
```

A record that a later one replaces gets `Status: superseded by NNNN (YYYY-MM-DD)` and stays.

## Index

| # | Decision | Date |
| --- | --- | --- |
| [0001](0001-one-parser-and-the-1.4-contract.md) | One hand-written parser for six dialects, under a contract with check-json5 1.4 | 2026-10-13 |
| [0002](0002-no-unsafe-code.md) | No unsafe code | 2026-10-19 |
| [0003](0003-siphash-for-duplicate-keys.md) | SipHash with OS keys for the duplicate-key index | 2026-10-20 |
| [0004](0004-measure-before-adopting-a-speed-up.md) | Measure a speed-up first, adopt it only past 5% | 2026-10-19 |
| [0005](0005-nearest-config-file-replaces.md) | The nearest config file applies, and replaces the ones above it | 2026-10-20 |
| [0006](0006-offline-json-schema-in-every-build.md) | `--schema` on the jsonschema crate, offline, in every build | 2026-10-18 |
| [0007](0007-hand-written-output-with-one-escaping-rule.md) | SARIF written by hand, with one escaping rule for every output | 2026-10-14 |
| [0008](0008-binary-wheels-behind-a-placeholder.md) | A Rust binary in PyPI wheels, installed through a `setup.py` placeholder | 2026-10-13 |
