# Contributing

To report a bug or ask for a feature, open an
[issue](https://github.com/simonvanlierde/jsonvet/issues). Say what you did, what you expected,
what happened, and the jsonvet version (`jsonvet --version`). Report a vulnerability as
[SECURITY.md](SECURITY.md) says, not in an issue.

## Set up

Clone with `--recurse-submodules` and install the tools in `mise.toml` (`mise install`), plus
Rust from rustup. Then run `just setup` once. It fetches the json5-tests submodule and installs
the git hooks with [prek](https://github.com/j178/prek), including a commit-msg hook that checks
each commit's sign-off, so commit with `git commit -s`
([why](#sign-off-your-commits)). `just --list` shows every task.

Some checks need tools that `mise.toml` does not install:

| Tool | For |
| --- | --- |
| valgrind and gungraun-runner | `just counts` and `just scaling` (Linux only) |
| wasm-bindgen-cli and wasm-opt | `just playground` |
| docker | `just matrix` |
| cargo-release | releasing |

## Checks

Run `just check` before you open a pull request. It runs CI's `lint` and `test` jobs on your OS:
formatting, clippy, rustdoc, cargo deny, ruff, zizmor, the version check, the conformance page
check, and the tests with doctests. The page check fails while `README.md` or
`docs/conformance.md` has unstaged edits, so stage them first. zizmor runs its online audits only
when `GH_TOKEN` is set, so without it `just lint` can pass where CI fails.

`just check` leaves out these jobs in `ci.yml`:

- `test` on the other two OSes. On macOS it also runs `scripts/test-action.sh`, the Action
  script's tests, under bash 3.2. Run `bash scripts/test-action.sh` yourself when you change
  `scripts/action.sh`.
- `msrv`, `scaling`, `bench` and `differential`: `just msrv`, `just scaling`, `just counts` and
  `just differential`.
- `hook`, `action` and `packaging`, which build the wheels and run the hook and the Action as
  users do.

Other workflows:

- On a pull request: `semver.yml` (`cargo semver-checks`), `fuzz.yml` (30 seconds a target),
  `coverage.yml` (Codecov) and `codspeed.yml` (benchmarks).
- Every week: `fuzz.yml` and `mutants.yml` in full. Every Monday: `advisories.yml`, so a new
  RustSec advisory against `Cargo.lock` fails a run without waiting for a pull request.
- `pages.yml` builds and tests the playground on a pull request that touches it, and deploys it
  from main. `just playground` does the same build and tests, and serves it at
  <http://127.0.0.1:8000>.
- `scorecard.yml` rates the repository for the OpenSSF Scorecard badge.

Put each detail of the docs in one place and link to it: the README to get started,
`docs/usage.md` for the reference, `docs/design.md` for why, and
[docs/decisions/](docs/decisions/README.md) for the choices behind it.

## Pull requests

`main` takes changes only through pull requests. Write commit subjects as
[Conventional Commits](https://www.conventionalcommits.org/), such as
`fix(parser): reject a lone CR in a JWCC comment`. A change in behaviour comes with a test that
fails without it, and a bug fix with a test that reproduces the bug. A change that reopens a
choice in [docs/decisions/](docs/decisions/README.md) brings the evidence its "Revisit if" asks for.

## Choosing dependencies

Prefer the standard library or a crate the workspace already uses. A new dependency in `jsonvet`
or `jsonvet-cli` needs a reason in the pull request: what it saves, that it is maintained, and for
a speed-up, the measurement [0004](docs/decisions/0004-measure-before-adopting-a-speed-up.md) asks
for. `cargo deny` checks that every crate in `Cargo.lock` comes from crates.io, has a permissive
licence and has no RustSec advisory or unmaintained notice. Renovate bumps the pins
([schedule](docs/design.md#dependencies-and-pins)); do not bump a pin by hand.

## Tests

- **Snapshots.** CLI output is tested with [insta](https://insta.rs). Review a `.snap` diff with
  `cargo insta review`, or accept everything with `INSTA_UPDATE=always cargo test`, then read the
  diff before you commit.
- **Corpus.** `tests/corpus/<dialect>/` has one file per case. Its bytes are part of the test, so
  `.gitattributes` and the hooks leave it alone. Do not let an editor reformat those files, or
  those with `\uXXXX` escapes in tests and snapshots.
- **SchemaStore.** `tests/schemastore/` holds real schemas with the files SchemaStore's own tests
  accept and reject, and `crates/jsonvet-cli/tests/schemastore.rs` checks that jsonvet agrees.
  Refresh them with `uv run scripts/schemastore-fixtures.py <a SchemaStore clone>`, and add
  jsonvet's own cases under a schema's `extra/`.
- **File lists.** A new default extension or file name in the library (`Dialect::EXTENSIONS`,
  `Dialect::FILE_NAMES`) goes into both hooks in `.pre-commit-hooks.yaml`, `scripts/action.sh`,
  and the README's file list and dialect table, or `crates/jsonvet-cli/tests/file_lists.rs` fails.
- **New rules and dialects.** A rule goes in `Rule::ALL` (`crates/jsonvet/src/rule.rs`), with an
  explanation in `crates/jsonvet-cli/src/rules.rs` and the same words in a `### <id>` section of
  `docs/usage.md`, or `every_rule_has_help_that_its_docs_section_holds` fails. The `--help` text
  must name it, or `help_names_every_rule_and_dialect` fails. A dialect goes in `Dialect::ALL`
  (`crates/jsonvet/src/dialect.rs`) and the `--help` text. Add either to the README's table.
- **Conformance matrix.** `just matrix` runs one set of inputs through 18 other parsers in docker
  containers, then renders `docs/conformance.md` and the README block. Commit
  `conformance/results/` and both rendered files. CI fails when the report is stale, or when
  `conformance/results/versions.json` disagrees with the pinned parsers. After a pin change, run
  `just matrix-lock` first ([details](conformance/README.md)).
- **Fuzzing.** `just fuzz check` (or `diff_serde_json`, `diff_json5`, `parse`) runs a target for 20
  minutes on a nightly toolchain. A new json5 version in `fuzz/` can need new skips in
  `diff_json5`: read its skip list before you call a divergence a jsonvet bug.
- **Mutation testing.** `just mutants` takes over an hour on one CI runner, so `mutants.yml` runs
  it weekly in ten parallel shards. It fails when a mutant survives that `.cargo/mutants.toml`
  does not list. List one there only when it changes no behaviour, and say why.
- **Differential.** `just differential` compares jsonvet with check-json5 1.4.1 on generated
  files. CI runs 5,000 cases with a new seed each run.
- **Speed claims.** Measure with `just bench` on a quiet machine, and report the median. To
  compare two builds, write the fixtures with `bench.fixtures(dir)` and run `hyperfine -N -r 40`
  on both. Time a change that touches every file on the 10,000-small-file walk too.
- **Instruction counts.** `just counts` counts the instructions of the parser
  (`crates/jsonvet/benches/check.rs`) and of whole `--schema` runs
  (`crates/jsonvet-cli/benches/schema.rs`) under callgrind, and prints each case's change since
  the last run. A change under 0.3% is noise. CI fails a pull request when a case rises by more
  than 1% over the base branch. Compare two commits with `just counts --save-baseline=base` on one
  and `just counts --baseline=base` on the other. In a Docker container, set
  `GUNGRAUN_ALLOW_ASLR=yes`. `gungraun-runner` must be the gungraun version in `Cargo.lock`.
- **CodSpeed.** CodSpeed comments each pull request's change in time, peak memory and
  allocations on the divan benchmarks in each crate's `benches/codspeed.rs`. Nothing fails on it:
  the `bench` job is the gate.
- **Scaling.** `just scaling` runs `crates/jsonvet-cli/tests/scaling.rs` under valgrind, on
  Linux only. Each test counts the instructions on an input of size n and 10n. Most fail when the
  ratio reaches 15 (linear gives about 10): duplicate keys on many lines and on one long line,
  `--schema` on keys with lone surrogates and on many `$ref`s by `$id`, json5's hex numbers in the
  `diff_json5` target, and a walk with many `exclude` and `dialects` patterns. Two fail unless the
  count stays within twice: schema errors past the first 100, and keys too costly for a pattern
  past the first 100.

## Releasing

A release changes the version in these places, in one commit on a branch:

1. `version` in the root `Cargo.toml`, and `Cargo.lock` with it
2. the `=x.y.z` pin on `jsonvet` in `crates/jsonvet-cli/Cargo.toml`
3. the pin in `setup.py`
4. `version` and the `jsonvet==` pin in `compat/check-json5/pyproject.toml`
5. `rev: vX.Y.Z` in the README's hook example
6. in `CHANGELOG.md`, the Unreleased entries moved under a new `## [X.Y.Z] - date` heading, and
   the compare links at the bottom

[cargo-release](https://github.com/crate-ci/cargo-release) makes all of them. It only bumps and
commits, and refuses to run on `main` ([0011](docs/decisions/0011-release-tooling.md)). Write the
Unreleased entries first; the release commit only moves them. Every figure in a release's
CHANGELOG section compares with the last release.

```sh
git fetch && git switch -c release/X.Y.Z origin/main
cargo release X.Y.Z                       # dry run: shows every change
SKIP=dco-signoff,check-versions cargo release X.Y.Z --execute   # commits "chore: release X.Y.Z"
git commit --amend -s --no-edit           # cargo-release cannot sign off
just bench --runs 40 --write              # a new minor only, on a quiet machine
git commit -s -m "docs: speed figures for X.Y" -- docs/design.md README.md
just matrix                               # records the new version in the matrix
git commit -s -m "docs: re-run the conformance matrix for X.Y.Z" -- docs/conformance.md README.md conformance/results
uv run scripts/check-versions.py
git push -u origin release/X.Y.Z          # then open the pull request and squash-merge it
git switch main && git pull --ff-only
git log -1 --oneline                      # must be "chore: release X.Y.Z (#NN)"
git tag -s vX.Y.Z -m vX.Y.Z               # signed with your git signing key
git verify-tag vX.Y.Z && git push origin vX.Y.Z
```

The tag is signed, so GitHub shows it as verified and anyone can check it with `git verify-tag`.
cargo-release prints a "Publishing" line, but it publishes nothing.

Do not create the GitHub release by hand. The tag runs `release.yml`, which builds, attests and
publishes everything ([design.md](docs/design.md#publishing)). Each publish job waits for an
approval: the `pypi` environment twice (the wheels, then `check-json5`) and `crates-io` once. If a
publish job fails, use "Re-run failed jobs", never "Re-run all jobs". A full re-run builds the
wheels again, so PyPI and the GitHub release could hold different builds.

A pre-release has a `-` in its tag (`v3.1.0-rc.1`). cargo-release bumps only the crates for one.
Edit the other places by hand, with the PEP 440 form (`3.2.0rc1`) in `setup.py` and the
compatibility package.

Once the release is published, move the major tag (`v3`), which `uses: simonvanlierde/jsonvet@v3`
follows: `git push --force origin vX.Y.Z^{commit}:refs/tags/v3`. Do it only when the new tag is
the newest `vX.*.*`, not for a pre-release or a patch to an older minor. The workflow's token
cannot move a tag past changes to `.github/workflows/`, and one that could would give CI write
access to the workflows.

## Sign off your commits

Every commit in a pull request carries a `Signed-off-by:` line, which says you wrote the change
or have the right to submit it under the MIT license: the
[Developer Certificate of Origin 1.1](https://developercertificate.org/). `git commit -s` adds
it. `git rebase --signoff upstream/main` adds it to commits you already made. The `DCO sign-off`
check fails any pull request with a commit that lacks it, the maintainer's and Renovate's
included.

## Who maintains this

[GOVERNANCE.md](GOVERNANCE.md) names the maintainer, who can publish each package, and what
happens if the maintainer is unavailable. [ROADMAP.md](ROADMAP.md) says what is planned.
