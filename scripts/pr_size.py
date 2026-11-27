"""Count changed lines and write the pull request size warning."""

import os
import subprocess
import sys
from collections.abc import Iterable
from pathlib import Path

CAP = 500


def changed_lines(files: Iterable[tuple[str, int, int]]) -> int:
    """Sum additions and deletions for files outside linguist-generated paths."""
    rows = list(files)
    out = subprocess.run(
        ["git", "check-attr", "-z", "--stdin", "linguist-generated"],  # noqa: S607
        input=b"".join(os.fsencode(path) + b"\0" for path, _, _ in rows),
        check=True,
        capture_output=True,
    ).stdout.split(b"\0")
    generated = {
        os.fsdecode(path) for path, _, value in zip(out[::3], out[1::3], out[2::3], strict=False) if value == b"true"
    }
    return sum(added + deleted for path, added, deleted in rows if path not in generated)


def main() -> None:
    """Summarize the diff and warn if it exceeds the trial cap."""
    numstat = subprocess.check_output(  # noqa: S603
        ["git", "diff", "--numstat", "--no-renames", "-z", sys.argv[1], "--"],  # noqa: S607
    )
    files = []
    for entry in numstat.split(b"\0"):
        if entry:
            added, deleted, path = entry.split(b"\t", 2)
            if added != b"-":
                files.append((os.fsdecode(path), int(added), int(deleted)))
    total = changed_lines(files)
    summary = f"Changed lines, generated files excluded: {total} (cap {CAP})"
    if path := os.environ.get("GITHUB_STEP_SUMMARY"):
        with Path(path).open("a", encoding="utf-8") as file:
            file.write(summary + "\n")
    else:
        print(summary)
    if total > CAP:
        print(
            f"::warning title=Large pull request::This pull request changes {total} lines "
            f"(cap {CAP}, generated files excluded). Split refactors into their own pull request."
        )


if __name__ == "__main__":
    main()
