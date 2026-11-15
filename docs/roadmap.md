# Roadmap

Last reviewed: 2026-11-15

What is planned for jsonvet over the next year, to October 2027, in rough order. It is a plan, not
a promise, and none of it has a date. Released work is in [CHANGELOG.md](../CHANGELOG.md), and
[GOVERNANCE.md](../GOVERNANCE.md) says who decides what goes here. Ideas and bugs go in
[GitHub issues](https://github.com/simonvanlierde/jsonvet/issues).

## Planned

- **4.0: drop the check-json5 compatibility.** The `check-json5` binary and package, `--strict`
  and the old hook id go away. The 3.x releases keep them
  ([#208](https://github.com/simonvanlierde/jsonvet/issues/208)).

## Not planned

- Fetching anything from the network by default, schemas included
  ([0006](decisions/0006-offline-json-schema-in-every-build.md)).
- A wall-clock timeout in place of the deterministic caps
  ([0010](decisions/0010-schema-limits-are-deterministic-caps.md)).
- `unsafe` code ([0002](decisions/0002-no-unsafe-code.md)).
- The formats in [design.md](design.md#not-supported): JSON text sequences (RFC 7464) and
  Hjson.
