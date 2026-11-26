# Roadmap

Last reviewed: 2026-11-20

What is planned for jsonvet over the next year, to October 2027, in rough order. It is a plan, not
a promise, and none of it has a date. Released work is in [CHANGELOG.md](../CHANGELOG.md), and
[GOVERNANCE.md](../GOVERNANCE.md) says who decides what goes here. Ideas and bugs go in
[GitHub issues](https://github.com/simonvanlierde/jsonvet/issues).

## Planned

1. **A redesign of the site.** The playground and the docs pages now share one header, footer and
   theme, and the docs are built with Starlight ([0019](decisions/0019-docs-on-starlight.md)). The
   redesign is one pass over hierarchy, type, colour and the first screen of each surface, with the
   accessibility checks rerun afterwards.
2. **More ways to install.** A Homebrew tap
   ([#209](https://github.com/simonvanlierde/jsonvet/issues/209)), then conda-forge, winget,
   nixpkgs, AUR and mise ([#210](https://github.com/simonvanlierde/jsonvet/issues/210)). Each
   channel installs the release archives or wheels, so it follows a release.
3. **Listings in other linter registries**, such as MegaLinter, Trunk and aqua, once the install
   channels exist.
4. **Where the parsers disagree.** A write-up of the conformance matrix, published after the
   disagreements found so far have been reported to the parsers' maintainers.

## Ideas, not planned yet

A PyO3 Python API ([#287](https://github.com/simonvanlierde/jsonvet/issues/287)),
`jsonc.org` modeline support ([#289](https://github.com/simonvanlierde/jsonvet/issues/289)), more
output formats ([#290](https://github.com/simonvanlierde/jsonvet/issues/290)), Kani proofs of the
pure parser helpers ([#291](https://github.com/simonvanlierde/jsonvet/issues/291)), and more speed
work at the scanner, which needs a new profile that shows a larger share than the attempts in
[#211](https://github.com/simonvanlierde/jsonvet/issues/211), closed in November 2026.

## Not planned

- Fetching anything from the network by default, schemas included
  ([0006](decisions/0006-offline-json-schema-in-every-build.md)).
- A wall-clock timeout in place of the deterministic caps
  ([0010](decisions/0010-schema-limits-are-deterministic-caps.md)).
- `unsafe` code ([0002](decisions/0002-no-unsafe-code.md)).
- The formats in [design.md](design.md#not-supported): JSON text sequences (RFC 7464) and
  Hjson.
