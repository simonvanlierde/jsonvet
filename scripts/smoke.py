# /// script
# requires-python = ">=3.11"
# ///
"""Install a built jsonvet package and run it, as CI and the release do before anything uploads.

Usage: uv run scripts/smoke.py JSONVET_WHEEL_OR_SDIST [CHECK_JSON5_WHEEL]

Both commands, jsonvet and check-json5, must reject a duplicate key, and `jsonvet --version` must
run. `jsonvet --schema` must find a schema error, so the wheels carry it. With a
check-json5 compatibility wheel, that package must install with its jsonvet pin met by
the first package, which may not be on PyPI yet.

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
    package, *compat = sys.argv[1:]
    with tempfile.TemporaryDirectory() as tmp:
        dup = Path(tmp) / "dup.json"
        dup.write_text('{"a": 1, "a": 2}\n')
        for command in ("jsonvet", "check-json5"):
            # 1 is a problem found; 2 would be a failed install or an unreadable file.
            if uv_run("--with", package, command, str(dup)) != 1:
                print(f"{command} did not report the duplicate key", file=sys.stderr)
                return 1
        # A file with no other problem, so exit 1 can only come from the schema.
        clean = Path(tmp) / "clean.json"
        clean.write_text('{"a": 1}\n')
        schema = Path(tmp) / "schema.json"
        schema.write_text('{"required": ["b"]}\n')
        if uv_run("--with", package, "jsonvet", "--schema", str(schema), str(clean)) != PROBLEM_FOUND:
            print("jsonvet --schema did not report the missing key", file=sys.stderr)
            return 1
    if uv_run("--with", package, "jsonvet", "--version"):
        return 1
    if compat and uv_run("--find-links", str(Path(package).parent), "--with", compat[0], "check-json5", "--version"):
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
