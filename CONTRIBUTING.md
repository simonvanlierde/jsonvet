# Contributing

## Set up

Clone with `--recurse-submodules` and install the tools in `mise.toml` (`mise install`), plus
Rust from rustup. Then run `just setup` once: it fetches the json5-tests submodule and installs
the git hooks with [prek](https://github.com/j178/prek). `just --list` shows every task.

Run `just check` before you open a pull request. It runs CI's Linux `lint` and `test` jobs:
formatting, clippy, rustdoc, cargo deny, ruff, zizmor, the tests with doctests, the version check
and the conformance page diff.

The page diff fails while `README.md` or `docs/conformance.md` has unstaged edits, so stage them
first.

CI also runs the msrv, scaling, bench, differential, hook, packaging, action-script, action and
semver jobs. `just msrv`, `just scaling`, `just counts` and `just differential` run four of them
locally.
Outside pull requests, `advisories.yml` runs `cargo deny check advisories` every Monday, so a new
RustSec advisory against `Cargo.lock` fails a run without waiting for a pull request, and
`scorecard.yml` rates the repository for the OpenSSF Scorecard badge.

User-facing docs live in three places: the README for getting started, `docs/usage.md` for the
reference, and `docs/design.md` for the reasons. Put a new detail in one of them and link to it
from the others.

## Pull requests

`main` is protected, and the `no-commit-to-branch` hook stops a commit on it. Work on a branch and
open a pull request. Write commit subjects as [Conventional Commits](https://www.conventionalcommits.org/),
such as `fix(parser): reject a lone CR in a JWCC comment`. The pull request number is added on merge.
A change that reopens a choice in [docs/decisions/](docs/decisions/README.md) brings the evidence
its record asks for under "Revisit if".

Commit hooks run `cargo fmt --check`, ruff and the version check; `git push` runs clippy and
cargo deny.

## Tests

- **Snapshots.** CLI output is tested with [insta](https://insta.rs). A change in behaviour shows
  up as a `.snap` diff. Review it with `cargo insta review`, or accept everything with
  `INSTA_UPDATE=always cargo test`, then read the diff and commit the `.snap` files.
- **Corpus.** `tests/corpus/<dialect>/` has one file per case. Its bytes are part of the test:
  missing final newlines, CRs and trailing spaces stay. `.gitattributes` sets `-text` on it and
  the hooks skip it, so do not let an editor reformat those files.
- **`\uXXXX` escapes.** Tests and snapshots contain them. Edit those files with a script, not an
  editor that unescapes them, and check the diff.
- **File lists.** The hooks in `.pre-commit-hooks.yaml` and the Action's `scripts/action.sh`
  pick files by their own lists of extensions and names. A new default extension or file name in
  the library (`Dialect::EXTENSIONS`, `Dialect::FILE_NAMES`) goes into both hooks,
  `scripts/action.sh` and the README's list, or `crates/jsonvet-cli/tests/file_lists.rs` fails.
  Add it to the README's dialect table too.
- **Conformance matrix.** `just matrix` runs one set of inputs through 17 other parsers in docker
  containers (never on the host), then renders `docs/conformance.md` and the block in the README.
  Commit `conformance/results/` and both rendered files. CI fails when the report is stale. It also
  fails when `conformance/results/versions.json` disagrees with the pinned parsers
  (`run.py --check`), as it does after a pin bump until someone re-runs the matrix. After a pin
  change, run `just matrix-lock` first. Details: [conformance/README.md](conformance/README.md).
- **Fuzzing.** `just fuzz check` (or `diff_serde_json`, `diff_json5`, `parse`) runs a target for 20
  minutes. CI runs each target for 30 seconds on a pull request that touches the library, and for
  20 minutes after a parser change on `main` and once a week. It needs a nightly toolchain and
  cargo-fuzz. Coverage stops growing within about two hours per target. A new json5 version in
  `fuzz/` can need new skips in `diff_json5`. Read its skip list before you call a divergence a
  jsonvet bug.
- **Mutation testing.** `just mutants` runs cargo-mutants over both crates in about 25 minutes, and
  `mutants.yml` runs it weekly. It fails when a mutant survives that `.cargo/mutants.toml` does
  not list. List one there only when it changes no behaviour, and say why.
- **Differential.** `just differential` compares jsonvet with check-json5 1.4.1 on generated
  files; CI runs 5,000 cases with a new seed each run.
- **Public API.** `cargo semver-checks` reports library API changes on a pull request. It does not
  block the merge.
- **Speed claims.** Measure with `just bench`: hyperfine without a shell, warm, 20 runs. Take the
  numbers on a quiet machine, not a loaded one, and report the median.
- **Instruction counts.** `just counts` runs `crates/jsonvet/benches/check.rs` under callgrind
  (gungraun) and prints each case's change since the last run. Counts move by under 0.3% between
  runs, because hash keys are random, so a smaller change is noise. On a pull request, CI counts
  the base branch and then the pull request, and fails when a case rises by more than 1%. To
  compare two commits locally, run `just counts --save-baseline=base` on one and
  `just counts --baseline=base` on the other. In a Docker container, set
  `GUNGRAUN_ALLOW_ASLR=yes`: the default seccomp profile refuses the `setarch -R` gungraun runs
  valgrind under. `gungraun-runner` must be the gungraun version in `Cargo.lock`; on a mismatch
  the benchmarks stop and name the `cargo install` command that fixes it.
- **Scaling.** `just scaling` runs `crates/jsonvet-cli/tests/scaling.rs` under valgrind: each test
  counts the instructions on an input of size n and 10n and fails when the ratio reaches 15
  (linear gives about 10, quadratic about 100). It guards paths that were once quadratic: the
  location of many duplicate keys, `--schema` on many keys with lone surrogates, and json5's hex
  numbers in the `diff_json5` fuzz target. Linux only; ignored by plain `cargo test`.

## Releasing

A release changes the version in these places, in one commit on a branch:

1. `version` in the root `Cargo.toml`, and `Cargo.lock` with it
2. the `=x.y.z` pin on `jsonvet` in `crates/jsonvet-cli/Cargo.toml`
3. the pin in `setup.py`
4. `version` and the `jsonvet==` pin in `compat/check-json5/pyproject.toml`
5. `rev: vX.Y.Z` in the README's hook example
6. in `CHANGELOG.md`, the Unreleased entries moved under a new `## [X.Y.Z] - date` heading

`uv run scripts/check-versions.py` fails when any of them disagrees, or when the CHANGELOG has no
heading for the version. Renovate owns the dependency pins, so do not bump those by hand.

After the pull request merges, tag the merge commit and push the tag:
`git tag vX.Y.Z <sha> && git push origin vX.Y.Z`. Do not create the GitHub release by hand. The
tag runs `release.yml`. It checks the tag against the versions and builds the wheels, the binary
archives and the SBOM, attests them, and drafts the GitHub release with the archives attached and
the version's CHANGELOG section as notes. Then it publishes the wheels to PyPI, the `check-json5`
package, and the crates to crates.io, all by trusted publishing, and publishes the release last.
The release is immutable, so it cannot take files after that. If a publish job fails, rerun it:
it skips the files PyPI already has and the crate versions crates.io already has. A tag with a `-` (`v3.1.0-rc.1`)
makes a pre-release. Once the wheels are on PyPI, the `major-tag` job moves the `v<major>` tag
(`v3`), which `uses: simonvanlierde/jsonvet@v3` follows. It moves the tag only when the new tag is
the newest `vX.*.*`, so a patch to an older minor leaves it alone. It skips pre-releases.
