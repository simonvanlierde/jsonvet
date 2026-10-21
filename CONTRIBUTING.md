# Contributing

## Set up

Clone with `--recurse-submodules` and install the tools in `mise.toml` (`mise install`), plus
Rust from rustup. Then run `just setup` once: it fetches the json5-tests submodule and installs
the git hooks with [prek](https://github.com/j178/prek). `just --list` shows every task.

## Checks

Run `just check` before you open a pull request. It runs CI's `lint` and `test` jobs on your OS:
formatting, clippy, rustdoc, cargo deny, ruff, zizmor, the version check, the conformance page
diff and pin check, and the tests with doctests. The page diff fails while `README.md` or
`docs/conformance.md` has unstaged edits, so stage them first.

`just check` leaves out these jobs in `ci.yml`:

- `test` on the other two OSes, and on macOS `scripts/test-action.sh`, the Action script's tests
  under bash 3.2. Run `bash scripts/test-action.sh` yourself when you change `scripts/action.sh`.
- `msrv`, `scaling`, `bench` and `differential`. `just msrv`, `just scaling`, `just counts` and
  `just differential` run them locally.
- `hook`, `action` and `packaging`, which build the wheels and run the hook and the Action as
  users do.

Other workflows run besides `ci.yml`. On a pull request, `semver.yml` runs `cargo semver-checks`,
`fuzz.yml` fuzzes each target for 30 seconds, and `coverage.yml` reports coverage to Codecov. Every
week, `fuzz.yml` and `mutants.yml` run in full. Every Monday, `advisories.yml` runs
`cargo deny check advisories`, so a new RustSec advisory against `Cargo.lock` fails a run without
waiting for a pull request. `scorecard.yml` rates the repository for the OpenSSF Scorecard badge.
`codspeed.yml` runs the divan benchmarks on every pull request and push to main, and CodSpeed
comments with the change against main.

User-facing docs live in three places: the README for getting started, `docs/usage.md` for the
reference, and `docs/design.md` for the design as it is now. The choices behind it, with their
numbers, are in [docs/decisions/](docs/decisions/README.md). Put a new detail in one place and
link to it from the others.

## Pull requests

`main` is protected, and the `no-commit-to-branch` hook stops a commit on it. Work on a branch and
open a pull request. Write commit subjects as [Conventional Commits](https://www.conventionalcommits.org/),
such as `fix(parser): reject a lone CR in a JWCC comment`. The pull request number is added on merge.
A change that reopens a choice in [docs/decisions/](docs/decisions/README.md) brings the evidence
its record asks for under "Revisit if".
A change in behaviour comes with a test that fails without it, and a bug fix with a test that
reproduces the bug.

Commit hooks run `cargo fmt --check`, ruff, zizmor and the version check. `git push` runs clippy
and cargo deny.

## Tests

- **Snapshots.** CLI output is tested with [insta](https://insta.rs). A change in behaviour shows
  up as a `.snap` diff. Review it with `cargo insta review`, or accept everything with
  `INSTA_UPDATE=always cargo test`, then read the diff and commit the `.snap` files.
- **Corpus.** `tests/corpus/<dialect>/` has one file per case. Its bytes are part of the test:
  missing final newlines, CRs and trailing spaces stay. `.gitattributes` sets `-text` on it and
  the hooks skip it, so do not let an editor reformat those files.
- **SchemaStore.** `tests/schemastore/` holds real schemas with the files SchemaStore's own tests
  accept and reject, and `crates/jsonvet-cli/tests/schemastore.rs` checks that jsonvet accepts
  and rejects the same ones. Refresh them with
  `uv run scripts/schemastore-fixtures.py <a SchemaStore clone>`, which lists the schemas, and
  add jsonvet's own cases under a schema's `extra/`.
- **`\uXXXX` escapes.** Tests and snapshots contain them. Edit those files with a script, not an
  editor that unescapes them, and check the diff.
- **File lists.** The hooks in `.pre-commit-hooks.yaml` and the Action's `scripts/action.sh`
  pick files by their own lists of extensions and names. A new default extension or file name in
  the library (`Dialect::EXTENSIONS`, `Dialect::FILE_NAMES`) goes into both hooks,
  `scripts/action.sh` and the README's list, or `crates/jsonvet-cli/tests/file_lists.rs` fails.
  Add it to the README's dialect table too.
- **New rules and dialects.** A rule goes in `Rule::ALL` (`crates/jsonvet/src/rule.rs`). Give it
  an explanation in `crates/jsonvet-cli/src/rules.rs` and a `### <id>` section in `docs/usage.md`
  with the same words, or `every_rule_has_help_that_its_docs_section_holds` fails. The `--help`
  text must name it, or `help_names_every_rule_and_dialect` fails. Add it to the README's rules
  table. A dialect goes in `Dialect::ALL` (`crates/jsonvet/src/dialect.rs`), the `--help` text and
  the README's dialect table, and its extensions as the file-lists bullet says.
- **Conformance matrix.** `just matrix` runs one set of inputs through 18 other parsers in docker
  containers (never on the host), then renders `docs/conformance.md` and the block in the README.
  Commit `conformance/results/` and both rendered files. CI fails when the report is stale. It also
  fails when `conformance/results/versions.json` disagrees with the pinned parsers
  (`run.py --check`), as it does after a pin bump until someone re-runs the matrix. After a pin
  change, run `just matrix-lock` first. Details: [conformance/README.md](conformance/README.md).
- **Fuzzing.** `just fuzz check` (or `diff_serde_json`, `diff_json5`, `parse`) runs a target for 20
  minutes. It needs a nightly toolchain and cargo-fuzz. CI runs each target for 30 seconds on a
  pull request that touches the library or `fuzz/`, and for 20 minutes once a week. Coverage
  stops growing within about two hours per target. A new json5 version in `fuzz/` can need new
  skips in `diff_json5`. Read its skip list before you call a divergence a
  jsonvet bug.
- **Mutation testing.** `just mutants` runs cargo-mutants over both crates in about 25 minutes, and
  `mutants.yml` runs it weekly. It fails when a mutant survives that `.cargo/mutants.toml` does
  not list. List one there only when it changes no behaviour, and say why.
- **Differential.** `just differential` compares jsonvet with check-json5 1.4.1 on generated
  files; CI runs 5,000 cases with a new seed each run.
- **Public API.** `semver.yml` runs `cargo semver-checks` on a pull request that touches the
  library, and reports API changes. It does not block the merge.
- **Speed claims.** Measure with `just bench`: hyperfine without a shell, warm, 20 runs. Take the
  numbers on a quiet machine, not a loaded one, and report the median. `scripts/bench.py` also
  times the check-json5 reference in every case, so a `--runs 40` pass takes about 8 minutes. To
  compare two jsonvet builds, write the fixtures with `bench.fixtures(dir)` and run
  `hyperfine -N -r 40` on the two binaries per case, both built the same way. A change that
  touches every file also gets timed on a 10,000-small-file walk: the 1,100-file config case runs
  in 4-7 ms, so 0.5 µs a file of overhead shows there as 10-25%.
- **Instruction counts.** `just counts` runs `crates/jsonvet/benches/check.rs`, the parser, and
  `crates/jsonvet-cli/benches/schema.rs`, whole `--schema` runs of the binary, under callgrind
  (gungraun) and prints each case's change since the last run. Counts move by under 0.3% between
  runs, because hash keys are random, so a smaller change is noise. On a pull request, CI counts
  the base branch and then the pull request, and fails when a case rises by more than 1%. To
  compare two commits locally, run `just counts --save-baseline=base` on one and
  `just counts --baseline=base` on the other. In a Docker container, set
  `GUNGRAUN_ALLOW_ASLR=yes`: the default seccomp profile refuses the `setarch -R` gungraun runs
  valgrind under. `gungraun-runner` must be the gungraun version in `Cargo.lock`; on a mismatch
  the benchmarks stop and name the `cargo install` command that fixes it.
- **CodSpeed.** `crates/jsonvet/benches/codspeed.rs` holds divan benchmarks of the library:
  `check` in every dialect, `parse` with its events, the JSON Lines split, and locations.
  `crates/jsonvet-cli/benches/codspeed.rs` runs `--schema` in-process on the cases
  `benches/schema.rs` counts. `codspeed.yml` runs both under CodSpeed's CPU simulation and its
  memory instrument (peak memory and allocations), which report each pull request's change
  against main in a comment and a dashboard with flamegraphs. Nothing fails on them yet: the
  `bench` job is the gate. Locally, `cargo bench -p jsonvet -p jsonvet-cli --bench codspeed`
  times them with divan; `cargo codspeed build -m simulation,memory -p jsonvet -p jsonvet-cli
  --bench codspeed` and `codspeed run -m simulation -- cargo codspeed run -p jsonvet -p
  jsonvet-cli` measure them as CI does.
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
6. in `CHANGELOG.md`, the Unreleased entries moved under a new `## [X.Y.Z] - date` heading, and
   the compare links at the bottom

[cargo-release](https://github.com/crate-ci/cargo-release) makes all of them
(`cargo install --locked cargo-release` or `cargo binstall cargo-release`). `release.toml` holds
the workspace settings and `crates/jsonvet-cli/release.toml` the replacements. It only bumps and
commits: it never tags, pushes or publishes, and it refuses to run on `main`
([0011](docs/decisions/0011-release-tooling.md)). Write the Unreleased entries first; the
release commit only moves them.

```sh
git fetch && git switch -c release/X.Y.Z origin/main
cargo release X.Y.Z                       # dry run: shows every change
cargo release X.Y.Z --execute             # commits "chore: release X.Y.Z"
uv run scripts/check-versions.py
git push -u origin release/X.Y.Z          # then open the pull request and squash-merge it
git switch main && git pull --ff-only
git log -1 --oneline                      # must be "chore: release X.Y.Z (#NN)"
git tag vX.Y.Z && git push origin vX.Y.Z
```

The "Publishing jsonvet, jsonvet-cli" line cargo-release prints publishes nothing.
`uv run scripts/check-versions.py` fails when any place disagrees, or when the CHANGELOG has no
heading for the version. Renovate owns the dependency pins, so do not bump those by hand.

Do not create the GitHub release by hand. The tag runs `release.yml`. It checks the tag against
the versions and builds the wheels, the binary archives and the SBOM, attests them, and drafts the
GitHub release with the archives attached and the version's CHANGELOG section as notes. Then it
publishes the wheels to PyPI, the `check-json5` package, and the crates to crates.io, all by
trusted publishing, and publishes the release last. Each publish job waits for an approval: the
`pypi` environment twice (the wheels, then `check-json5`) and `crates-io` once. With immutable
releases turned on in the repository settings, the release cannot take files after that. If a
publish job fails, rerun it: it skips the files PyPI already has and the crate versions crates.io
already has.

A pre-release has a `-` in its tag (`v3.1.0-rc.1`). cargo-release bumps only the crates for one
(`cargo release 3.2.0-rc.1 --execute`); edit the other places by hand, with the PEP 440 form
(`3.2.0rc1`) in `setup.py` and the compatibility package.

Once the release is published, move the major tag (`v3`), which `uses: simonvanlierde/jsonvet@v3`
follows: `git push --force origin vX.Y.Z^{commit}:refs/tags/v3`. Do it only when the new tag is
the newest `vX.*.*`, not for a pre-release or a patch to an older minor. It is a manual step
because the workflow's token cannot move a tag past changes to `.github/workflows/`, and a token
that can would give CI write access to the workflows.
