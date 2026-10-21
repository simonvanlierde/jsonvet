# Decision records

Each record is a choice where we measured or argued an alternative and turned it down, with the
numbers, so nobody argues it again without new evidence. A new decision lands in the same pull
request as its code.

A record has Context, Options (with their numbers, date and setting, or "argued, not measured"),
Decision, Consequences, Confirmation (the test or CI job that keeps it true) and Revisit if, as in
[MADR 4](https://adr.github.io/madr/).

We don't rewrite an accepted record. To change a decision, bring what its "Revisit if" asks for
and add a new record; the old one gets `Status: superseded by NNNN (YYYY-MM-DD)`. Fixing a typo,
a link or a renamed test is fine.

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
| [0009](0009-where-the-config-search-stops.md) | Where the config search stops, and what a bad config may show | 2026-10-20 |
| [0010](0010-schema-limits-are-deterministic-caps.md) | `--schema` limits are deterministic caps, not a timeout | 2026-10-20 |
