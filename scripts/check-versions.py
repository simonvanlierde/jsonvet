# /// script
# requires-python = ">=3.11"
# ///
"""Check that every place naming jsonvet's release version agrees.

The crates' version (Cargo.toml), the `=x.y.z` pin of the library in the CLI crate, the wheel pin
in the hook placeholder (setup.py), the check-json5 compatibility package's version and pin
(compat/check-json5/pyproject.toml), the `rev:` in the README's hook example and a heading in the
CHANGELOG must name the same release, or a hook install would pull another version than the tag.

The speed table in docs/design.md, which `scripts/bench.py --write` fills in, must name the
release's minor version (`jsonvet 3.2` for 3.2.x), so each minor release measures again. The
README's "up to N times faster than check-json5 1.4.1" must name the table's largest speed-up over
1.4.1, since the README is also the crate's page on crates.io.

The minimum supported Rust has three places too: `rust-version` in Cargo.toml, the toolchain of
ci.yml's msrv job and `cargo +X` in the justfile's msrv recipe.

The CHANGELOG rule holds on main and on a release branch alike. Main keeps the last release's
version until a release commit bumps it, so `## [X.Y.Z]` exists there already; the release commit
moves the Unreleased entries under a new `## [X.Y.Z]` heading in the same change that bumps the
version.
"""

import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PINS = 2  # setup.py and the compatibility package each pin jsonvet once


def pep440(cargo: str) -> str:
    """Cargo's `2.0.0-alpha.1` as Python's `2.0.0a1`."""
    for long, short in (("-alpha.", "a"), ("-beta.", "b"), ("-rc.", "rc")):
        cargo = cargo.replace(long, short)
    return cargo


def first(pattern: str, path: str) -> str | None:
    """The first group of the pattern's first match in the file, or None."""
    match = re.search(pattern, (ROOT / path).read_text(), re.MULTILINE | re.DOTALL)
    return match[1] if match else None


def main() -> int:
    """Print each version and exit 1 when they differ or the CHANGELOG has no heading for it."""
    package = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]
    cargo = package["version"]
    compat = tomllib.loads((ROOT / "compat" / "check-json5" / "pyproject.toml").read_text())["project"]
    pins = re.findall(r"jsonvet==([^\"']+)", (ROOT / "setup.py").read_text()) + [
        dep.split("==")[1] for dep in compat["dependencies"] if dep.startswith("jsonvet==")
    ]
    cli = tomllib.loads((ROOT / "crates" / "jsonvet-cli" / "Cargo.toml").read_text())
    cli_pin = cli["dependencies"]["jsonvet"]["version"].removeprefix("=")
    revs = re.findall(r"^\s*rev: v(\S+)\s*$", (ROOT / "README.md").read_text(), re.MULTILINE)
    found = {"Cargo.toml": pep440(cargo), "compat version": compat["version"]}
    found.update({f"pin {i + 1}": pin for i, pin in enumerate(pins)})
    found["cli crate pin"] = pep440(cli_pin)
    found.update({f"README rev {i + 1}": pep440(rev) for i, rev in enumerate(revs)})
    for where, version in found.items():
        print(f"{where:16} {version}")
    heading = f"## [{cargo}]"
    has_heading = any(line.startswith(heading) for line in (ROOT / "CHANGELOG.md").read_text().splitlines())
    print(f"{'CHANGELOG':16} {'has' if has_heading else 'lacks'} {heading}")
    msrvs = {
        "rust-version": package["rust-version"],
        "ci.yml msrv": first(r'^  msrv:$.*?toolchain: "([^"]+)"', ".github/workflows/ci.yml"),
        "just msrv": first(r"^msrv:\n\s+cargo \+(\S+)", "justfile"),
    }
    for where, version in msrvs.items():
        print(f"{where:16} {version}")
    minor = ".".join(cargo.split("-")[0].split(".")[:2])
    speed = first(r"<!-- speed:start -->.*?\| jsonvet (\S+) +\|", "docs/design.md")
    print(f"{'design.md speed':16} {speed}")
    table = first(r"<!-- speed:start -->(.*?)<!-- speed:end -->", "docs/design.md") or ""
    rows = [row.split("|") for row in table.splitlines() if row.startswith("|")][2:]
    largest = max((row[5].strip() for row in rows), key=lambda up: int(up.rstrip("x").replace(",", "")), default=None)
    claim = first(r"up to ([\d,]+)\s+times faster than check-json5 1\.4\.1", "README.md")
    print(f"{'README speed-up':16} {claim}x, table {largest}")
    if (
        len(pins) != PINS
        or len(revs) != 1
        or len(set(found.values())) != 1
        or not has_heading
        or len(set(msrvs.values())) != 1
        or speed != minor
        or f"{claim}x" != largest
    ):
        print("versions differ", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
