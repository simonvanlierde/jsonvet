# 6. JSON Schema with offline references and an opt-in catalog

Status: accepted (2026-10-18)

## Context

check-jsonschema reports no line numbers (its #359), and jsonvet already knows every value's
offset, so it can put schema errors at their line and column.

## Options

- **Leave schemas to other tools.**
- **jsonschema with remote `$ref` and a SchemaStore catalog.** Needs network access in hooks and
  CI, and a cache. Waits for a network policy.
- **jsonschema offline**, with each error's JSON Pointer mapped back to an offset.

## Decision

Offline jsonschema, so every `$ref` stays inside the schema file, in every build since 3.0.0
(#54). A cargo feature first kept it out of the wheels, but nearly every install comes from PyPI:
about 900 a week, against 11 crates.io downloads in total (2026-10-18).

Updated 2026-11-10: `--schema-catalog` explicitly enables HTTPS SchemaStore downloads by file
name, after explicit, configured and local document schemas. Configuration files cannot
enable networking. Default runs and the language server stay offline. Document `$schema`
URLs and external `$ref`s remain offline in every mode.

Downloads use a blocking HTTPS client with certificate validation. Only the three exact
SchemaStore origins are supported, including redirects; matched third-party entries fail
clearly. This avoids arbitrary network destinations without adding DNS filtering or a second
URL library. Catalogs and schemas are cached for seven days. A failed refresh uses a usable
stale copy with a warning; an uncached failure is a usage error. Cache entries record their
URL and replace atomically only after validation. Size, time and redirect limits are in
[usage.md](../usage.md#schemastore-catalog).

## Consequences

The x86-64 wheel grew from 0.67 to 4.33 MB, still under prek and ruff, and a plain check starts in
1.1 ms either way. About 70 more crates come along, one of them (`num-cmp`) unmaintained since
2017. A crafted schema can cost far more than its size, so `--schema` has limits, listed in
[design.md](../design.md#--schema).

## Confirmation

The limit tests in `cli.rs`'s `mod schema`, and the `packaging` CI job, which builds the wheels.

## Revisit if

Users need catalog schemas on other origins, jsonschema's `Json` trait settles so it can read our
events directly, or the wheel size draws complaints.
