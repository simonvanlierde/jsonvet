# 10. `--schema` limits are deterministic caps, not a timeout

Status: accepted (2026-10-20)

## Context

A pull request can edit the schema file, and the review before 3.1.0 found five crafted schemas
that got past the limits in [design.md](../design.md#--schema): a 1 KB schema on a 15 KB file took
57 s, and others took minutes or ran out of memory.

## Options

- **A wall-clock budget**, exit 2 after N seconds. The verdict would depend on machine speed: a
  file could pass on a fast runner and fail on a busy one. Argued, not measured.
- **Deterministic caps on a cost model:** count `$ref`s followed, `enum` and `const` values and
  schemas `unevaluated*` checks again, stop asking for errors after 100, and keep only each
  error's message and path.
- **Both.** The timeout brings the machine-speed verdict back for whatever the caps miss.

## Decision

Caps only. A file passes or fails the same way on every machine.

## Consequences

On a 12-core Linux machine (2026-10-20), release build:

| Crafted schema | Before | After |
| --- | ---: | ---: |
| 1 KB doubler, 5,000 failing elements | over 60 s, 1.9 GB | 1.5 s, 23 MB |
| 200,000-value `enum` on 1,000 keys | out of memory | 0.13 s, 59 MB |
| 27 nested `unevaluatedProperties` | 28.5 s | refused at once |
| 20,000 `$ref`s by `$id` | 24.4 s | 0.44 s |

The caps are conservative: 20 nested `unevaluatedProperties`, which took 0.16 s, are refused. And
no cap sees what one jsonschema call collects, so an `anyOf` around many failing values can still
take gigabytes ([SECURITY.md](../../SECURITY.md)).

No real schema has hit a cap so far. On 2026-10-21, jsonvet checked SchemaStore's test files
against 456 of their schemas. Every schema it refused had a `$ref` to another file, except one
with a `$ref` that is not a valid URI.

Update, 2026-10-23: jsonschema 0.60.1 shares the values of an `enum` between errors. On a
12-core Linux machine, release build, an `anyOf` around `additionalProperties` with a 200,000-value
`enum` and 1,000 failing keys took 6.27 GB and 2.7 s on 0.59.0, and 40 MB and 0.03 s on 0.60.1. The
caps above stay: they count the cost before jsonschema runs, whatever its version.

Update, 2026-10-28: regexes have caps too, counted before any is compiled. Each pattern may take
2 MB compiled, a schema may hold 1,000 different patterns, and the schemas one run reads 1,000
between them, since each stays in memory until the run ends. SchemaStore's largest pattern takes
715 KB, its most in one schema is 457, and its 43 schemas with the most hold 994 between them. At
the caps a run takes about 7 s and 2.7 GB. Which schema goes past the run cap depends on which
thread reads its file first, so going past it stops the run with exit 2, whichever file named it.
Since a split schema's parts are built only once a document fails, a run at the caps whose files
pass takes 3.5 s and 1.4 GB.

## Confirmation

`errors_past_the_first_100_are_not_looked_for`,
`errors_that_hold_much_of_the_schema_are_dropped_as_they_come`,
`nested_unevaluated_properties_are_a_usage_error`,
`a_ref_resolves_against_its_own_base_not_a_like_named_id_elsewhere`,
`a_schema_whose_regexes_cost_too_much_to_compile_is_a_usage_error` and
`schemas_whose_regexes_together_cost_too_much_are_a_usage_error` in `cli.rs`'s `mod schema`;
`schema_errors_past_the_first_100` and `schema_refs_to_many_ids` in `scaling.rs`; and `schemastore.rs`,
which checks the real schemas from SchemaStore in `tests/schemastore/` with their test files.

## Revisit if

jsonschema can stop after N errors, or a real schema is refused while it checks in well under a
second.
