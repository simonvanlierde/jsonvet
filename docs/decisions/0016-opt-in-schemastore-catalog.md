# 16. An opt-in SchemaStore catalog over HTTPS

Status: accepted (2026-11-10)

Supersedes in part [0006](0006-offline-json-schema-in-every-build.md): the catalog it left for
later is now an option. Default runs stay offline.

## Context

0006 asked to revisit when users want schemas picked by file name. A run that names no schema
sees no SchemaStore schema for `package.json` or a workflow file, and a hook or CI job cannot
fetch it by hand.

## Options

- **Stay offline.** No new code, no network in hooks. Users bring the schema file themselves.
- **Fetch by default.** Surprising in a hook and in CI, and a network failure becomes a failed
  check.
- **An opt-in flag**, `--schema-catalog`, with a cache. Network use is explicit, and a default
  run is the same as before.

## Decision

`--schema-catalog` enables HTTPS SchemaStore downloads by file name, after explicit, configured and
local document schemas. Configuration files cannot enable networking. Default runs and the
language server stay offline. Document `$schema` URLs and external `$ref`s remain offline in every
mode.

Downloads use a blocking HTTPS client (ureq, with rustls) with certificate validation. Only the
three exact SchemaStore origins are supported, including redirects; matched third-party entries
fail clearly. This avoids arbitrary network destinations without adding DNS filtering or a second
URL library. Catalogs and schemas are cached for seven days. A failed refresh uses a usable stale
copy with a warning; an uncached failure is a usage error. Cache entries record their URL and
replace atomically only after validation. Size, time and redirect limits are in
[usage.md](../usage.md#schemastore-catalog).

## Consequences

`Cargo.lock` went from 171 packages at v3.3.1 to 199 at 3.4.0. The new HTTP crates are ureq,
ureq-proto, http, httparse, rustls, rustls-webpki, rustls-pki-types, webpki-roots and ring. The
release also adds the language server ([0015](0015-a-language-server-on-lsp-server.md)), so the
28 are not all the catalog's. Binary and wheel size were not measured for the catalog alone. A
cached catalog also means a run can differ from one day to the next, until the cache expires.

## Confirmation

The tests in `crates/jsonvet-cli/tests/catalog.rs`, and the `packaging` CI job, which builds the
wheels.

## Revisit if

Users need catalog schemas on other origins, the HTTP dependencies draw advisories, or the wheel
size draws complaints.
