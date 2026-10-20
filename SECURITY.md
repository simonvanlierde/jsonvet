# Security

jsonvet reads files it did not write, often in CI on pull requests from strangers. So I treat a
crafted file that does any of these as a security bug:

- crashes jsonvet, hangs it, or makes it use far more memory or time than the file's size explains;
- gets an output line past the escaping, so it can drive a terminal or pass for a GitHub workflow
  command;
- makes the GitHub Action read or write outside the files it was given.

The same goes for a crafted schema passed to `--schema`, and for a crafted config file
(`jsonvet.toml`, `pyproject.toml`) or ignore file (`.gitignore`, `.ignore`) in the repository.

Output grows linearly with the input, but can be many times its size: the worst case I could build,
a JSON5 file of 200,000 duplicate keys of 4 bytes each under a path cut at 256 bytes, printed 82
times its size as text, 119 times with `--format json`, and 172 times in GitHub Actions, where
each problem also prints as an annotation. Each problem also repeats the file's name. Duplicate-key
reports stay complete, as 1.4's are, so a CI log can be large.

## Reporting

Please report it privately through
[GitHub's vulnerability reporting](https://github.com/simonvanlierde/jsonvet/security/advisories/new),
not in a public issue. Include the file, or the smallest part of it that still shows the problem,
the command you ran, and the jsonvet version (`jsonvet --version`).

I'll reply within a week. Once a fix is out, I'll publish an advisory and credit you, unless you'd
rather not be named.

## Supported versions

Fixes go into the newest release only, currently 3.x. The `check-json5` package on PyPI installs
the same code, so it gets the same fixes.

## Verifying a release

Every file a release builds has a build provenance attestation: the PyPI wheels and sdist, the
binary archives on the GitHub release, and its SBOM. It records that the file came from this
repository's `release.yml`, for that tag. With the [GitHub CLI](https://cli.github.com/):

```bash
gh attestation verify jsonvet-x86_64-unknown-linux-gnu.tar.gz --repo simonvanlierde/jsonvet
```

The same works on a wheel downloaded from PyPI (`pip download jsonvet --no-deps`). PyPI also
shows the wheels' own PEP 740 attestations on each file's page.
`sha256sum --check --ignore-missing SHA256SUMS` checks downloaded archives against the release's
checksums. Releases up to 3.0.0 have no attestations.

The GitHub Action checks the wheel it downloads this way before installing it, from 3.1.0 on, and
stops if the check fails. `verify: false` turns that off, for GitHub Enterprise Server or a runner
without `gh`. The Action cannot pin a hash instead: the wheels are built after the tag it is
pinned to.
