# Security

jsonvet reads files it did not write, often in CI on pull requests from strangers. So I treat a
crafted file that does any of these as a security bug:

- crashes jsonvet, hangs it, or makes it use far more memory or time than the file's size explains;
- gets an output line past the escaping, so it can drive a terminal or pass for a GitHub workflow
  command;
- makes the GitHub Action read or write outside the files it was given.

That holds for every file jsonvet reads: the files it checks, config and ignore files, and
schemas. A way to make jsonvet read or show a file outside the project, through a schema path the
project picks, is a security bug too
([0012](docs/decisions/0012-schema-paths-stay-in-the-project.md)).
[docs/assurance-case.md](docs/assurance-case.md) argues how jsonvet meets these requirements.

## Known limits

Three cases are known and still open. Until they close, review schema changes in pull requests
from strangers before CI runs them. The first two need a schema that jsonvet cannot check one part
at a time ([design.md](docs/design.md#--schema)).

- jsonschema collects every error of one validation call before jsonvet sees any. So a schema
  that puts an `anyOf` around `additionalProperties` can take gigabytes on a document with many
  failing values ([sizes](docs/usage.md#json-schema-limits)). Closing it needs a way in
  jsonschema to stop early.
- A string too costly to match against a pattern under an `allOf`, beside many strings that fail
  that pattern, costs more than the input explains, and the cost grows faster than the input.
  One beside 1,600,000 failing strings (a 9.6 MB file) took 62 s and 7.5 GB, against 6.3 s and
  1.35 GB without it.
- Compiling a schema's patterns is capped per pattern, per schema and per run
  ([caps](docs/usage.md#json-schema-limits)), but what the caps allow is still far more than a
  schema's size explains. 1,000 patterns just under 2 MB compiled, in a 32 KB schema, take 3.5 s
  and 1.4 GB on a file that passes. A failing document, or one lookahead among the patterns, makes
  jsonvet compile them a second time to check it a member at a time: 7.5 s and 2.7 GB, or 11 s
  with the lookahead. That is the most for one run, however many schemas it reads.

Output grows linearly with the input, but can be many times its size: up to 172 times in GitHub
Actions, for a file of 200,000 duplicate keys of 4 bytes each. Duplicate-key reports stay
complete, as 1.4's are, so a CI log can be large.

## Report a vulnerability

Please report it privately through
[GitHub's vulnerability reporting](https://github.com/simonvanlierde/jsonvet/security/advisories/new),
not in a public issue or pull request. Include the file, or the smallest part of it that still
shows the problem, the command you ran, the jsonvet version (`jsonvet --version`) and the impact
you expect. I'll reply within a week, to say whether I can reproduce it and when I expect a fix.

## Fixes and disclosure

A fix ships in a new release on GitHub, PyPI and crates.io. Then I publish the GitHub security
advisory, with a CVE where one applies, and link it from the CHANGELOG. It credits you unless
you'd rather not be named.

## Scope

In scope:

- Anything in the list at the top of this page, for every file jsonvet reads.
- The GitHub Action (`action.yml` and `scripts/action.sh`), including its release check.
- The release workflow and the files it publishes.
- The playground: `crates/jsonvet-wasm`, `playground/` and `.github/workflows/pages.yml`, which
  deploys it to GitHub Pages.

Out of scope:

- The open cases under "Known limits". A new way to reach them, or a fix, is in scope.
- A vulnerability in a tool jsonvet builds on or runs, such as jsonschema, a runner or GitHub
  Actions. Report it upstream. A pin or setting that jsonvet should change is in scope.
- A check that jsonvet reports wrongly, or a file it accepts or rejects against its dialect.
  Open an issue for those.

## Supported versions

Fixes go into the newest release only, currently 3.x. The `check-json5` package installs the
same code.

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

The GitHub Action runs this check on the wheel it downloads, and stops if it fails. It cannot pin
a hash instead, because the wheels are built after the tag it is pinned to. Without `gh` on the
runner it installs the wheel unchecked ([reference](docs/usage.md#github-action)), but someone who
controls the package index cannot bring that about: only the runner's owner can remove `gh`.
