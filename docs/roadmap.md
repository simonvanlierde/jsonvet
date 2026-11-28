# Roadmap

See which install channels and linter registries are planned for jsonvet next.

The list covers the year to October 2027 in rough order, with no release dates. Last reviewed
2026-11-27. Released work is in [CHANGELOG.md](../CHANGELOG.md), and
[GOVERNANCE.md](../GOVERNANCE.md) says who decides what goes here. Ideas and bugs go in
[GitHub issues](https://github.com/simonvanlierde/jsonvet/issues).

## Planned

1. **More ways to install.** A Homebrew tap
   ([#209](https://github.com/simonvanlierde/jsonvet/issues/209)), then conda-forge, winget,
   nixpkgs, AUR and mise ([#210](https://github.com/simonvanlierde/jsonvet/issues/210)). Each
   channel installs the release archives or wheels, so it follows a release.
2. **Listings in linter registries**, such as MegaLinter, Trunk and aqua, once the install
   channels exist.

## Ideas, not planned yet

More machine-readable output, such as streaming JSON Lines diagnostics
([#290](https://github.com/simonvanlierde/jsonvet/issues/290)), and scanner experiments to measure
numeric parsing and UTF-8 validation on the 4.0 baseline
([#211](https://github.com/simonvanlierde/jsonvet/issues/211)).

## Not planned

- Fetching anything from the network by default, schemas included
  ([0006](decisions/0006-offline-json-schema-in-every-build.md)).
- A wall-clock timeout in place of the deterministic caps
  ([0010](decisions/0010-schema-limits-are-deterministic-caps.md)).
- `unsafe` code ([0002](decisions/0002-no-unsafe-code.md)).
- The formats in [design.md](design.md#not-supported): JSON text sequences (RFC 7464) and
  Hjson.
