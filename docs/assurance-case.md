# Assurance case

This page argues why jsonvet meets its security requirements, and names the code, test or
workflow behind each claim. [SECURITY.md](../SECURITY.md) holds the requirements, the known limits
and how to report a problem; the design itself is in [design.md](design.md).

The requirements, from SECURITY.md:

1. A crafted file must not crash or hang jsonvet, or make it use far more time or memory than its
   size explains.
2. No output line may carry text that drives a terminal or passes for a GitHub workflow command.
3. A schema path a project picks must not read or show a file outside the project.
4. The GitHub Action must not read or write outside the files it was given.

The two open cases in SECURITY.md's "Known limits" are exceptions to the first requirement, and
this page does not claim them.

## Threat model

jsonvet often runs in CI on pull requests from strangers, so we assume an attacker who controls:

- the files it checks;
- a config file (`jsonvet.toml`, `pyproject.toml`), an ignore file, and a schema named by
  `--schema`, a config file or a file's own `$schema`;
- the names of files and folders in the checkout, and symlinks, FIFOs or devices among them;
- the text pasted into the browser playground.

We also defend the release against a malicious dependency, a compromised action, or someone who
controls the package index the Action installs from.

We do not defend against someone who can already run code on the machine, change jsonvet's own
binary, or edit the workflow that calls it. A vulnerability in jsonschema, a runner or GitHub
Actions is reported upstream (SECURITY.md, "Scope").

## Trust boundaries

| Boundary | What crosses it | Controls |
| --- | --- | --- |
| Files to check | Bytes from the checkout or stdin | One hand-written parser in safe Rust (`unsafe_code = "forbid"`, [0002](decisions/0002-no-unsafe-code.md)); a nesting cap (`Config::DEFAULT_MAX_DEPTH`, `crates/jsonvet/src/rule.rs`); duplicate keys hashed with SipHash under OS keys, so crafted keys cannot collide ([0003](decisions/0003-siphash-for-duplicate-keys.md), `crates/jsonvet/src/parse.rs`); only regular files are read, so a FIFO cannot hang a run and `/dev/zero` cannot fill memory (`read` in `crates/jsonvet-cli/src/lib.rs`) |
| Config and ignore files | Settings that change what is checked and how | A size cap (`MAX_CONFIG_BYTES`) and the same regular-file rule; the config search stops at the repository root or the working directory ([0009](decisions/0009-where-the-config-search-stops.md)); a symlinked config is followed, but an error in it shows none of its values (`crates/jsonvet-cli/src/settings.rs`) |
| Schemas | A schema chosen by `--schema`, a config file or `$schema` | Fixed caps on schema size and depth, file depth, errors and message length, not a timeout ([0010](decisions/0010-schema-limits-are-deterministic-caps.md), `crates/jsonvet-cli/src/schema/limits.rs`); jsonschema runs with default features off, so nothing is fetched ([0006](decisions/0006-offline-json-schema-in-every-build.md)); a path from a project must stay inside it, and its errors quote nothing from it (`confined` in `settings.rs`, [0012](decisions/0012-schema-paths-stay-in-the-project.md)) |
| Output | File names, paths and messages printed to a terminal, a CI log, JSON or SARIF | One escaping rule for every format: control, bidi and zero-width characters are printed as `\u` escapes, line breaks as `\r` and `\n`, and workflow-command values are percent-encoded (`printable` and `escape` in `crates/jsonvet-cli/src/output.rs`, [0007](decisions/0007-hand-written-output-with-one-escaping-rule.md)); SARIF stops at 25,000 results (`MAX_RESULTS`) |
| The GitHub Action | Its inputs, and the wheel it downloads from PyPI | Inputs reach the script through environment variables, never expanded into the shell (`action.yml`); the wheel is checked with `gh attestation verify` against this repository's release workflow before it is installed (`verify_wheel` in `scripts/action.sh`) |
| The playground | Text pasted into a static page | The library compiled to WebAssembly with no file or network access; a Content-Security-Policy that allows only the page's own script and wasm, and no forms or other origins (`playground/index.html`) |
| Release and CI | Code, dependencies, the built files | Pull requests run under `pull_request` with a read-only token and no secrets, except that CodSpeed's job also gets an OpenID Connect token to upload its results (a pull request from a fork gets none); every checkout drops its credentials; actions are pinned by commit; zizmor checks the workflows; `cargo deny` checks licences, sources and RustSec advisories; the jobs that publish run on `v*` tags only, run no project code, and wait for the maintainer's approval in the `pypi` and `crates-io` environments; every release file gets a build provenance attestation (`.github/workflows/release.yml`, [GOVERNANCE.md](../GOVERNANCE.md#access)) |

## Secure design principles

- **Fail-safe defaults.** A file jsonvet cannot read fails the run (`unreadable`), whatever the
  options. A schema path that leaves the project is an error even if nothing is there. The Action
  stops if a wheel's attestation fails, and only an explicit `verify: false` skips the check.
- **Least privilege.** The library has no file or network access; the CLI reads only what it is
  given or finds by the config search, and never writes. Workflows start from `contents: read`.
  `id-token: write` goes only to the release jobs that attest or publish, the Pages `deploy` job,
  and the CodSpeed and Scorecard jobs, which upload their results with it.
- **Economy of mechanism.** One parser serves all six dialects
  ([0001](decisions/0001-one-parser-and-the-1.4-contract.md)), and one escaping rule serves every
  output format.
- **Complete mediation.** Every file, config, ignore file and schema is read only if it is a
  regular file, all but the checked files under a size cap, and every printed string goes
  through `printable`.
- **Deterministic limits.** Caps count work rather than time, so a file gets the same verdict on
  every machine and a slow runner cannot be pushed into a different result.
- **Open design.** Nothing relies on secret code: the source, the caps and the known limits are
  public.

## Countering common weaknesses

- **Memory safety (CWE-787, CWE-125).** No `unsafe` code in the crates; the fuzz targets run with
  AddressSanitizer anyway (`.github/workflows/fuzz.yml`).
- **Resource exhaustion (CWE-400, CWE-407).** The caps above, and the `scaling` CI job, which
  fails if a path that was once quadratic grows faster than linear, by instruction counts
  (`crates/jsonvet-cli/tests/scaling.rs`).
- **Output injection (CWE-150, CWE-117).** The escaping rule above, with tests for escape
  sequences, bidi controls and workflow commands in `output.rs`.
- **Path traversal and file disclosure (CWE-22, CWE-59).** Schema paths confined to the project,
  symlinked configs followed but their values not shown, and devices and FIFOs refused.
- **Hash flooding (CWE-407).** SipHash with OS keys for the duplicate-key index.
- **Supply chain (CWE-494, CWE-1357).** Trusted publishing with no stored token, attestations on
  every release file, signed release tags, Renovate with a seven-day wait before a new version (a
  `Cargo.lock` refresh skips the wait and waits for review instead), and `cargo deny` on every pull
  request and every Monday.
- **Bugs not caught by review.** Four cargo-fuzz targets on every pull request that touches the
  library and for 20 minutes each every week, weekly mutation testing, a differential test
  against check-json5 1.4.1 on every pull request, and clippy's pedantic lints as errors.
