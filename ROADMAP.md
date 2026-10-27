# Roadmap

What is planned for jsonvet over the next year, to October 2027, in rough order. It is a plan, not
a promise, and none of it has a date. Released work is in [CHANGELOG.md](CHANGELOG.md), and
[GOVERNANCE.md](GOVERNANCE.md) says who decides what goes here. Ideas and bugs go in
[GitHub issues](https://github.com/simonvanlierde/jsonvet/issues).

## Planned

- **`--fix` for safe fixes:** a byte order mark, trailing commas in JSON and JSONC, a missing final
  newline in JSON Lines, CRLF line ends. Duplicate keys only behind an explicit unsafe option.
- **A formatter that keeps comments** in JSONC and JSON5, with a `jsonvet-format` hook.
- **4.0: drop the check-json5 compatibility.** The `check-json5` binary and package, `--strict`
  and the old hook id go away. The 3.x releases keep them.
- **Less memory for `--schema`:** check the document without copying it into a `serde_json`
  value, which also gives byte positions for the values that fail.
- **Schemas from a catalog, opt-in only:** look up a schema for a file in a SchemaStore-style
  catalog, with a local cache. jsonvet stays offline unless you turn this on.

## Not planned

- Fetching anything from the network by default, schemas included
  ([0006](docs/decisions/0006-offline-json-schema-in-every-build.md)).
- A wall-clock timeout in place of the deterministic caps
  ([0010](docs/decisions/0010-schema-limits-are-deterministic-caps.md)).
- `unsafe` code ([0002](docs/decisions/0002-no-unsafe-code.md)).
- The formats in [design.md](docs/design.md#not-supported): JSON text sequences (RFC 7464) and
  Hjson.
