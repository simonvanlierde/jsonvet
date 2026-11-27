# /// script
# requires-python = ">=3.11"
# ///
"""Find public repositories that may use jsonvet. Requires authenticated gh."""

import json
import subprocess
from collections import defaultdict
from urllib.parse import urlencode

QUERIES = (
    '"simonvanlierde/jsonvet"',
    '"uvx jsonvet"',
    '"uv tool install jsonvet"',
    '"pipx install jsonvet"',
    '"cargo install jsonvet-cli"',
    '"cargo binstall jsonvet-cli"',
    '"package: jsonvet"',
    '"jsonvet" filename:.pre-commit-config.yaml',
    '"jsonvet" path:.github/workflows',
    '"jsonvet" filename:Cargo.toml',
)
OWNER = "simonvanlierde"
SEARCH_LIMIT = 1000


def main() -> None:
    """Print candidate repositories and the files to verify for each one."""
    matches: dict[str, dict[str, str]] = defaultdict(dict)
    incomplete = False
    for term in QUERIES:
        query = f"{term} -user:{OWNER}"
        endpoint = "search/code?" + urlencode({"q": query, "per_page": 100})
        result = subprocess.check_output(  # noqa: S603
            ["gh", "api", "--paginate", "--slurp", endpoint],  # noqa: S607
            text=True,
        )
        for page in json.loads(result):
            incomplete |= page["incomplete_results"] or page["total_count"] > SEARCH_LIMIT
            for item in page["items"]:
                repo = item["repository"]
                owner, _, _ = repo["full_name"].partition("/")
                if repo["private"] or repo["fork"] or owner.casefold() == OWNER:
                    continue
                matches[repo["full_name"]][item["path"]] = item["html_url"]

    print(f"{len(matches)} public candidate repositories. Verify the matching files for actual use.")
    for repo, files in sorted(matches.items(), key=lambda item: item[0].casefold()):
        print(repo)
        for path, url in sorted(files.items()):
            print(f"  {path}: {url}")
    if incomplete:
        print("GitHub reported incomplete results or exceeded its 1,000-file search limit.")


if __name__ == "__main__":
    main()
