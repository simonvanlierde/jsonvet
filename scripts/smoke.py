# /// script
# requires-python = ">=3.11"
# ///
"""Install a built jsonvet package and run it, as CI and the release do before anything uploads.

Usage: uv run scripts/smoke.py JSONVET_WHEEL_OR_SDIST

The installed command must reject a duplicate key, report a schema error and print its version.

Python, not bash: the Windows checkout converts line endings, which bash scripts do not survive.
"""

import subprocess
import sys
import tempfile
from pathlib import Path

PROBLEM_FOUND = 1  # jsonvet's exit status when it reports a problem


def uv_run(*args: str) -> int:
    """Run a command in a fresh environment, outside any project."""
    return subprocess.run(["uv", "run", "--isolated", "--no-project", *args], check=False).returncode  # noqa: S603, S607


def main() -> int:
    """Exit 1 when a check fails."""
    (package,) = sys.argv[1:]
    with tempfile.TemporaryDirectory() as tmp:
        dup = Path(tmp) / "dup.json"
        dup.write_text('{"a": 1, "a": 2}\n')
        if uv_run("--with", package, "jsonvet", str(dup)) != PROBLEM_FOUND:
            print("jsonvet did not report the duplicate key", file=sys.stderr)
            return 1
        # A file with no other problem, so exit 1 can only come from the schema.
        clean = Path(tmp) / "clean.json"
        clean.write_text('{"a": 1}\n')
        schema = Path(tmp) / "schema.json"
        schema.write_text('{"required": ["b"]}\n')
        if uv_run("--with", package, "jsonvet", "--schema", str(schema), str(clean)) != PROBLEM_FOUND:
            print("jsonvet --schema did not report the missing key", file=sys.stderr)
            return 1
    return int(uv_run("--with", package, "jsonvet", "--version") != 0)


if __name__ == "__main__":
    sys.exit(main())
