# /// script
# requires-python = ">=3.11"
# ///
"""Copy the SchemaStore schemas and test files that `tests/schemastore/` holds.

Usage: uv run scripts/schemastore-fixtures.py <a SchemaStore clone>

Each schema is copied to `tests/schemastore/<name>/schema.json`, with its files from SchemaStore's
`src/test/<name>/` under `valid/` and from `src/negative_test/<name>/` under `invalid/`. Files
under `extra/` are jsonvet's own and are kept. The commit copied from goes in `SOURCE`.
"""

import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "tests" / "schemastore"

# Chosen for what they exercise: lookaround and back-references (the first five),
# `patternProperties`, `propertyNames`, `if`, `not` and `oneOf`, draft 7 and 2020-12, and files
# many projects keep. Every `$ref` in them points inside the file.
SCHEMAS = [
    "expo-57.0.0",
    "zeropress-build-pages-config-1.0",
    "mongodb-atlas-search-index-definition",
    "web-types",
    "luaurc",
    "dependabot-2.0",
    "github-action",
    "github-issue-forms",
    "codecov",
    "jsontron-0.1",
    "evidence-bundle",
]


def main() -> None:
    """Copy each schema in SCHEMAS and its test files from the clone named on the command line."""
    clone = Path(sys.argv[1]).resolve()
    src = clone / "src"
    for name in SCHEMAS:
        folder = OUT / name
        for part in ("valid", "invalid"):
            shutil.rmtree(folder / part, ignore_errors=True)
        folder.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(src / "schemas" / "json" / f"{name}.json", folder / "schema.json")
        for kind, part in (("test", "valid"), ("negative_test", "invalid")):
            tests = src / kind / name
            for file in sorted(tests.glob("*.json")) if tests.is_dir() else []:
                (folder / part).mkdir(exist_ok=True)
                shutil.copyfile(file, folder / part / file.name)
    shutil.copyfile(clone / "LICENSE", OUT / "LICENSE")
    shutil.copyfile(clone / "NOTICE", OUT / "NOTICE")
    commit = subprocess.run(  # noqa: S603 - our own command line
        ["git", "-C", str(clone), "rev-parse", "HEAD"],  # noqa: S607
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()
    (OUT / "SOURCE").write_text(f"https://github.com/SchemaStore/schemastore/tree/{commit}\n")


if __name__ == "__main__":
    main()
