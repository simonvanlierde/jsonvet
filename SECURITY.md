# Security

jsonvet reads files it did not write, often in CI on pull requests from strangers. So I treat a
crafted file that does any of these as a security bug:

- crashes jsonvet, hangs it, or makes it use far more memory or time than the file's size explains;
- gets an output line past the escaping, so it can drive a terminal or pass for a GitHub workflow
  command;
- makes the GitHub Action read or write outside the files it was given.

The same goes for a crafted schema passed to `--schema`, with one known exception: jsonschema
collects every error of one validation call before jsonvet sees any, so a schema that puts an
`anyOf` or an `enum` around `additionalProperties` can still take gigabytes on a document with
many failing values. [design.md](docs/design.md#--schema) has the details. Closing it needs a way
in jsonschema to stop collecting errors early. Until then, review schema changes in pull requests
from strangers before CI runs them.

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
