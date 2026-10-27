# Governance

jsonvet has one maintainer: Simon van Lierde
([@simonvanlierde](https://github.com/simonvanlierde)). This page says who decides what and who
can change the repository and the packages.

## How decisions are made

- The maintainer decides what goes in and what goes on the [roadmap](ROADMAP.md), reviews and
  merges every pull request, triages issues, tags releases, and handles security reports as
  [SECURITY.md](SECURITY.md) says.
- Anyone can propose a change in an issue or a pull request, as
  [CONTRIBUTING.md](CONTRIBUTING.md) describes. The maintainer says yes, asks for changes, or says
  no with a reason, in the open on that issue or pull request.
- Choices that were argued or measured against an alternative are written down in
  [docs/decisions/](docs/decisions/README.md), so the reasoning stays visible.
- The maintainer enforces the [code of conduct](CODE_OF_CONDUCT.md).

## Access

- @simonvanlierde is the only account with access to the repository, as admin.
- `main` takes changes only through pull requests, with the checks in the `protect-main` ruleset
  passing, including the `DCO sign-off` check. Force-pushes and deletion are blocked. The ruleset
  asks for one approving review, which the admin may skip when merging, since there is no second
  reviewer.
- The packages are `jsonvet` and `check-json5` on PyPI and `jsonvet` and `jsonvet-cli` on
  crates.io. @simonvanlierde is the only owner of each. They are published only by `release.yml`,
  through trusted publishing with no stored token. Its `pypi` and `crates-io` environments accept
  only `v*` tags and wait for an approval from @simonvanlierde.
- Release tags are signed with the maintainer's key ([CONTRIBUTING.md](CONTRIBUTING.md#releasing)).
- Renovate opens the dependency pull requests.

## If the maintainer is unavailable

A named person can take over the GitHub repository, and the recovery details for PyPI, crates.io
and Zenodo are in an emergency-access vault.

The code, the release history and the docs are public under the MIT license, so anyone can fork the
project. Installed copies, the pre-commit hook and the GitHub Action keep working without upkeep,
because each release is pinned on PyPI and crates.io.

## Changes to this page

The maintainer changes this page through a normal pull request. A new maintainer is named here
first, with how they are added and how decisions are made among them.
