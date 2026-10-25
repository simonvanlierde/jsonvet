# Security

jsonvet reads files it did not write, often in CI on pull requests from strangers. So I treat a
crafted file that does any of these as a security bug:

- crashes jsonvet, hangs it, or makes it use far more memory or time than the file's size explains;
- gets an output line past the escaping, so it can drive a terminal or pass for a GitHub workflow
  command;
- makes the GitHub Action read or write outside the files it was given.

That holds for every file jsonvet reads: the files it checks, a config file (`jsonvet.toml`,
`pyproject.toml`), an ignore file (`.gitignore`, `.ignore`) and a schema named by `--schema`, a
config file or a file's `$schema`. A schema path from a `$schema`, or from a config file the
search finds rather than one named by `--config`, must stay inside the project, and an error in
such a schema quotes nothing from it
([0012](docs/decisions/0012-schema-paths-stay-in-the-project.md)). A way to read or show a file
outside the project through one is a security bug too.

## Known limits

Two cases are known and still open, both in a schema jsonvet cannot check a part at a time
([design.md](docs/design.md#--schema)). Until they close, review schema changes in pull requests
from strangers before CI runs them.

- jsonschema collects every error of one validation call before jsonvet sees any. So a schema
  that puts an `anyOf` around `additionalProperties` can still take gigabytes on a document with
  many failing values. Each takes about 420 bytes
  ([usage.md](docs/usage.md#json-schema-limits)), on top of the file's own values, which the Memory
  bullet there sizes. Closing it needs a way in jsonschema to stop early.
- A pattern too costly to match, under an `allOf`, is much cheaper than before but not free.
  5,000 such strings took 26.3 s on 0.59 and take 0.05 s now, and one beside 100,000 strings that
  fail the pattern went from over 120 s to 2.55 s. But one beside 1,600,000 failing strings (a
  9.6 MB file) took 62 s and 7.5 GB, against 6.3 s and 1.35 GB for the same strings without it:
  about 10 times the time and 5.6 times the memory, growing faster than the input.

Output grows linearly with the input, but can be many times its size. The worst case I could
build is a JSON5 file of 200,000 duplicate keys of 4 bytes each, under a path cut at 256 bytes.
It printed 82 times its size as text, 119 times with `--format json`, and 172 times in GitHub
Actions, where each problem also prints as an annotation. Each problem also repeats the file's
name. Duplicate-key reports stay complete, as 1.4's are, so a CI log can be large.

## Report a vulnerability

Please report it privately through
[GitHub's vulnerability reporting](https://github.com/simonvanlierde/jsonvet/security/advisories/new).
Don't open a public issue or pull request for it: anyone can read those before there is a fix.

Please include:

- The file, or the smallest part of it that still shows the problem.
- The command you ran, and the jsonvet version (`jsonvet --version`).
- The impact you expect.

I'll reply within a week, to say whether I can reproduce it and when I expect a fix.

## Fixes and disclosure

A fix ships in a new release, on the GitHub release, PyPI (`jsonvet` and `check-json5`) and
crates.io. When the release is out, I publish the GitHub security advisory for it, with a CVE
where one applies, and credit you unless you'd rather not be named. The CHANGELOG entry for that
release links the advisory.

## Scope

In scope:

- Anything in the list at the top of this page, for every file jsonvet reads.
- The GitHub Action (`action.yml` and `scripts/action.sh`), including its release check.
- The release workflow and the files it publishes.
- The playground: `crates/jsonvet-wasm`, `playground/` and `.github/workflows/pages.yml`, which
  deploys it to GitHub Pages.

Out of scope:

- The open cases under "Known limits", which are already known. A new way to reach them, or a
  fix, is in scope: report it here.
- A vulnerability in a tool jsonvet builds on or runs, such as jsonschema, a runner or GitHub
  Actions. Report it upstream; a pin or setting jsonvet should change is in scope.
- A check that jsonvet reports wrongly, or a file it accepts or rejects against its dialect.
  Open an issue for those.

## Supported versions

Fixes go into the newest release only, currently 3.x. The `check-json5` package on PyPI installs
the same code, so it gets the same fixes.

## Verifying a release

Every file a release builds has a build provenance attestation: the PyPI wheels and sdist, the
binary archives on the GitHub release, and its SBOM. It records that the file came from this
repository's `release.yml`, for that tag. Releases up to 3.0.0 have no attestations. To check a
file, use the [GitHub CLI](https://cli.github.com/):

```bash
gh attestation verify jsonvet-x86_64-unknown-linux-gnu.tar.gz --repo simonvanlierde/jsonvet
```

The same works on a wheel downloaded from PyPI (`pip download jsonvet --no-deps`). PyPI also
shows the wheels' own PEP 740 attestations on each file's page. To check downloaded archives
against the release's checksums, run `sha256sum --check --ignore-missing SHA256SUMS`.

The GitHub Action runs this check on the wheel it downloads, from 3.1.0 on, and stops if it fails.
It cannot pin a hash instead, because the wheels are built after the tag it is pinned to. On a
runner without `gh`, it installs the wheel unchecked, with a notice. Someone who controls the
package index cannot use that, because only the runner's owner can remove `gh`. The `verify` input
turns the check off, for GitHub Enterprise Server
([reference](docs/usage.md#github-action)).
