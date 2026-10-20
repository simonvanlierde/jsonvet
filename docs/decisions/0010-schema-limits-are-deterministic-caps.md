# 10. `--schema` limits are deterministic caps, not a timeout

Status: accepted (2026-10-20)

## Context

A schema file sits in the repository, so a pull request can edit it, and SECURITY.md promises
that a crafted schema cannot make jsonvet take unbounded time or memory. A review before 3.1.0
found five crafted schemas that got past the limits in [design.md](../design.md#--schema): a
1 KB schema on a 15 KB file took 57 s, and others took minutes or ran out of memory.

## Options

- **A wall-clock budget:** exit 2 once a file takes more than N seconds. Argued, not measured:
  the verdict would depend on machine speed, so the same file could pass on a fast runner and
  fail on a slow or busy one.
- **Deterministic caps on a cost model:** count what jsonschema may do for a value (objects and
  arrays with each `$ref` followed, `enum` and `const` values, schemas checked again beside
  `unevaluated*`), stop asking for errors after 100, and keep only each error's message and path.
- **Both:** the caps, with a timeout as a backstop. It brings back the machine-speed verdict for
  whatever the caps miss.

## Decision

Deterministic caps only. A file passes or fails the same way on every machine.

## Consequences

Measured on devbox (2026-10-20), release build, time and peak memory before and after:

- 1 KB doubling schema on 5,000 failing elements: over 60 s, 1.9 GB; now 1.5 s, 23 MB.
- 16-level doubling schema on 40 elements: 11.2 s, 3.2 GB; now 3.0 s, 83 MB.
- `additionalProperties` with a 200,000-value `enum` on 1,000 keys: out of memory at 6 GB; now
  0.13 s, 59 MB.
- 27 nested `unevaluatedProperties` on `{"c": 1}`: 28.5 s; now a usage error at once.
- Two resources with one relative `$id`, 26 levels: out of memory; now a usage error at once.
- 20,000 `$ref`s by `$id`: 24.4 s; now 0.44 s.

The caps are conservative: 20 nested `unevaluatedProperties`, which took 0.16 s, are refused.
A cap cannot see what one jsonschema call collects before handing out the first error, so a
non-split schema with an `anyOf` around many failing values can still take gigabytes.

## Confirmation

In `cli.rs`'s `mod schema`: `errors_past_the_first_100_are_not_looked_for`,
`errors_that_hold_much_of_the_schema_are_dropped_as_they_come`,
`nested_unevaluated_properties_are_a_usage_error` and
`a_ref_resolves_against_its_own_base_not_a_like_named_id_elsewhere`. In `scaling.rs`:
`schema_errors_past_the_first_100` and `schema_refs_to_many_ids`.

## Revisit if

- jsonschema hands out errors one at a time, or can stop after N, so the remaining one-pass case
  can be capped too;
- a real schema is refused by a cap while it checks in well under a second.
