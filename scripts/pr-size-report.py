"""Report the pr-size trial: merged pull requests since 2026-11-06, size against the 500-line cap.

Usage: scripts/pr-size-report.py [SINCE_DATE]

Changed lines are additions plus deletions over the files the PR files API lists, minus the paths
the checkout's .gitattributes marks linguist-generated, so run it in a checkout. Review rounds are
the commits pushed after the first review or comment by someone other than the author.
Needs `gh`, logged in.
"""

import json
import subprocess
import sys
from typing import Any

CAP = 500


def gh(*args: str) -> Any:  # noqa: ANN401 - gh's JSON has a different shape per command
    """Run `gh` and parse its JSON output."""
    out = subprocess.run(  # noqa: S603 - our own command line
        ["gh", *args],  # noqa: S607
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return json.loads(out)


def generated(paths: list[str]) -> set[str]:
    """Return the paths among `paths` that .gitattributes marks linguist-generated."""
    out = subprocess.run(
        ["git", "check-attr", "-z", "--stdin", "linguist-generated"],  # noqa: S607
        input="\0".join(paths) + "\0",
        check=True,
        capture_output=True,
        text=True,
    ).stdout.split("\0")
    # Triples of path, attribute, value.
    return {out[i] for i in range(0, len(out) - 2, 3) if out[i + 2] == "true"}


def main() -> None:
    """Print one line per merged pull request, then the share over the cap."""
    since = sys.argv[1] if len(sys.argv) > 1 else "2026-11-06"
    prs = gh("pr", "list", "--state", "merged", "--search", f"merged:>={since}",
             "--limit", "500", "--json", "number,author")  # fmt: skip
    over = 0
    for pr in prs:
        n = pr["number"]
        pages = gh("api", "--paginate", "--slurp", f"repos/{{owner}}/{{repo}}/pulls/{n}/files")
        files = [f for page in pages for f in page]
        skip = generated([f["filename"] for f in files])
        lines = sum(f["additions"] + f["deletions"] for f in files if f["filename"] not in skip)
        view = gh("pr", "view", str(n), "--json", "reviews,comments,commits")
        talk = [
            x.get("submittedAt") or x["createdAt"]
            for x in view["reviews"] + view["comments"]
            if x["author"]["login"] != pr["author"]["login"]
        ]
        first = min(talk, default=None)
        rounds = sum(1 for c in view["commits"] if first and c["committedDate"] > first)
        over += lines > CAP
        print(f"#{n}\t{lines}\t{'over' if lines > CAP else 'under'}\t{rounds} rounds")
    print(f"{over} of {len(prs)} merged pull requests over {CAP} lines")


if __name__ == "__main__":
    main()
