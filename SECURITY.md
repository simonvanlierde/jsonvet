# Security

jsonvet reads files it did not write, often in CI on pull requests from strangers. So I treat a
crafted file that does any of these as a security bug:

- crashes jsonvet, hangs it, or makes it use far more memory or time than the file's size explains;
- gets an output line past the escaping, so it can drive a terminal or pass for a GitHub workflow
  command;
- makes the GitHub Action read or write outside the files it was given.

The same goes for a crafted schema passed to `--schema`.

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
