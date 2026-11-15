# Roadmap

Last reviewed: 2026-11-16

What is planned for jsonvet over the next year, to October 2027, in rough order. It is a plan, not
a promise, and none of it has a date. Released work is in [CHANGELOG.md](../CHANGELOG.md), and
[GOVERNANCE.md](../GOVERNANCE.md) says who decides what goes here. Ideas and bugs go in
[GitHub issues](https://github.com/simonvanlierde/jsonvet/issues).

## Planned

1. **A redesign of the site.** The playground, the docs pages and the conformance matrix page get one
   design source and one pass over hierarchy, type, colour and the first screen of each, with the
   accessibility checks rerun afterwards. Small fixes and the docs search come first, in 3.5.x.
2. **More ways to install.** A Homebrew tap
   ([#209](https://github.com/simonvanlierde/jsonvet/issues/209)), then conda-forge, winget,
   nixpkgs, AUR and mise ([#210](https://github.com/simonvanlierde/jsonvet/issues/210)). Each
   channel installs the release archives or wheels, so it follows a release.
3. **Listings in other linter registries**, such as MegaLinter, Trunk and aqua, once the install
   channels exist.
4. **Where the parsers disagree.** A write-up of the conformance matrix, published after the
   disagreements found so far have been reported to the parsers' maintainers.
5. **4.0: drop the check-json5 compatibility.** The `check-json5` binary and package, `--strict`
   and the old hook id go away. The 3.x releases keep them
   ([#208](https://github.com/simonvanlierde/jsonvet/issues/208)).

## Ideas, not planned yet

A Python API, JSONPath (RFC 9535) queries, proofs of the pure parser helpers, and more speed work
at the scanner, which needs a new profile that shows a larger share than the attempts in
[#211](https://github.com/simonvanlierde/jsonvet/issues/211), closed in November 2026. They are in
[#212](https://github.com/simonvanlierde/jsonvet/issues/212).

## Not planned

- Fetching anything from the network by default, schemas included
  ([0006](decisions/0006-offline-json-schema-in-every-build.md)).
- A wall-clock timeout in place of the deterministic caps
  ([0010](decisions/0010-schema-limits-are-deterministic-caps.md)).
- `unsafe` code ([0002](decisions/0002-no-unsafe-code.md)).
- The formats in [design.md](design.md#not-supported): JSON text sequences (RFC 7464) and
  Hjson.
