# /// script
# requires-python = ">=3.12"
# ///
"""Compare jsonvet with check-json5 1.4.1, file by file, in the two dialects 1.4 knows.

Usage: uv run scripts/differential.py [--cases N] [--seed S] [--jsonvet PATH]

It writes random JSON-ish files, the json5-tests suite and tests/corpus to a temporary
directory, runs both tools over it in JSON5 mode and with --strict, and compares the verdict
for every file and, where both find no syntax error, the duplicate-key reports. It sorts every
difference into a category. The design accepts some of them, and docs/design.md lists those. The
exit status is 1 when a difference falls in no accepted category.
"""

import argparse
import json
import random
import re
import shutil
import subprocess
import sys
import tempfile
from collections import Counter, defaultdict
from pathlib import Path
from typing import NamedTuple

ROOT = Path(__file__).resolve().parent.parent
REFERENCE = ["uvx", "--quiet", "--python", "3.13", "--from", "check-json5==1.4.1", "check-json5"]
BATCH = 500  # files per run, to keep the command line short
EXAMPLES = 5  # differences shown per category

TOKENS = [
    "{", "}", "[", "]", ":", ",", ",", '"a"', '"b"', "'a'", "a", "$k", "1", "-2.5e3", "0x1F", ".5", "5.",
    "+1", "01", "NaN", "Infinity", "true", "null", "nul", "// c\n", "/* c */", "/*", "*/", "\n", "\r\n",
    " ", "\t", '"//"', '",]"', "\\", '"\\u0061"', '"\\q"', '"\\x41"', '"a\\\nb"', "é", "\u00a0", '"',
    "\u2028", "\u2029", "\r", "\ufeff",
]  # fmt: skip

# A JSON-shaped skeleton with a repeated key, so duplicates get compared on valid documents.
OBJECTS = [
    '{{"{a}": 1, "{b}": [{{"{a}": 2, "{a}": 3}}], "{a}": 4}}',
    "{{{a}: 1, // c\n  '{b}': {{x: 1, x: 2}},\n  {a}: 5,\n}}",
    '[{{"{a}": 1}}, {{"{b}": 2, "{b}": 3}}]',
]
KEYS = ["a", "b c", "ünï", "$x", "1st", 'q\\"', "\\u0061", "\\ud800", "\\udbff", "a\\u093e"]

# Corpus files that meet a deliberate difference listed in docs/design.md.
KNOWN = {
    "corpus-json5-depth-10000.json": "accepted: 1.4 stops at 9,998 levels, jsonvet at 10,000",
    "corpus-json5-combining-mark-starts-key-escaped.json": (
        "accepted: a \\u escape in a key must stand for a character the key could hold"
    ),
}

ESCAPE = re.compile(r"(\\u[0-9a-f]{4})+")

LINE = re.compile(r"^(?P<file>[^:]+?):(?:(?P<line>\d+):(?P<col>\d+):)? ?(?P<rest>.*)$")


class Problem(NamedTuple):
    """One reported problem. `kind` is `duplicate` for a duplicate key and `other` otherwise."""

    line: str | None
    col: str | None
    kind: str
    severity: str
    text: str


def corpus(directory: Path, cases: int, seed: int) -> list[Path]:
    """Write `cases` generated files and the json5-tests suite to `directory`."""
    rng = random.Random(seed)  # noqa: S311 - test data, not security
    files = []
    for i in range(cases):
        if i % 4 == 0:
            text = rng.choice(OBJECTS).format(a=rng.choice(KEYS), b=rng.choice(KEYS))
        else:
            text = "".join(rng.choice(TOKENS) for _ in range(rng.randint(1, 30)))
        path = directory / f"case-{i:05}.json"
        path.write_text(text, encoding="utf-8", newline="")
        files.append(path)
    suite = ROOT / "tests" / "json5-tests"
    for i, case in enumerate(sorted(suite.rglob("*"))):
        if case.suffix in {".json", ".json5", ".js", ".txt"}:
            path = directory / f"suite-{i:03}.json"
            shutil.copyfile(case, path)
            files.append(path)
    for dialect in ("json5", "json"):  # the contract corpus, in the two dialects 1.4 knows
        for case in sorted((ROOT / "tests" / "corpus" / dialect).iterdir()):
            path = directory / f"corpus-{dialect}-{case.name}"
            shutil.copyfile(case, path)
            files.append(path)
    return files


def run(command: list[str], files: list[Path], cwd: Path) -> dict[str, list[Problem]]:
    """Problems per file name.

    A tool that cannot start also exits 1, but prints only to stderr; that stops the run.
    """
    found: dict[str, list[Problem]] = defaultdict(list)
    names = [f.name for f in files]
    for start in range(0, len(names), BATCH):
        out = subprocess.run(  # noqa: S603 - our own command line
            [*command, *names[start : start + BATCH]], cwd=cwd, capture_output=True, text=True, check=False
        )
        if out.returncode not in {0, 1} or (out.returncode == 1 and not out.stdout and out.stderr):
            sys.exit(f"{' '.join(command[:6])} failed with {out.returncode}: {out.stderr}")
        for line in out.stdout.splitlines():
            m = LINE.match(line)
            if not m or line.startswith("::"):
                continue
            rest = m["rest"]
            severity = "error"
            if (s := re.match(r"(error|warning)\[([\w-]+)\] (.*)", rest)) is not None:
                severity, rest = s[1], s[3]
            kind = "duplicate" if rest.startswith("duplicate key ") else "other"
            if kind == "duplicate":
                rest = rest.split(" (first at ")[0]
            found[m["file"]].append(Problem(m["line"], m["col"], kind, severity, rest))
    return found


def failed(problems: list[Problem]) -> bool:
    """Whether any problem is an error."""
    return any(p.severity == "error" for p in problems)


def has_syntax_error(problems: list[Problem]) -> bool:
    """Whether the tool found a syntax error, which is the only thing a syntax verdict is about."""
    return any(p.kind == "other" and p.severity == "error" for p in problems)


def duplicates(problems: list[Problem]) -> list[tuple]:
    """The duplicate-key reports without their severity, sorted, as (line, col, kind, text)."""
    return sorted((p.line, p.col, p.kind, p.text) for p in problems if p.kind == "duplicate")


def syntax_only_in_ours(ours: list[Problem], theirs: list[Problem]) -> str | None:
    """The accepted category for a syntax error that only jsonvet reports, if there is one."""
    if not has_syntax_error(ours) or has_syntax_error(theirs):
        return None
    if any(p.text.startswith("expected a key, found `\\`") for p in ours):
        # An unquoted key that starts with a \u escape: 1.4 passes any escape, jsonvet only one
        # that stands for a character the key could hold.
        return "accepted: a \\u escape in a key must stand for a character the key could hold"
    if failed(theirs) and duplicates(ours) == duplicates(theirs):
        # 1.4 prints the duplicate keys and stops; jsonvet goes on to the syntax error after them.
        return "accepted: jsonvet also reports a syntax error that follows a duplicate key"
    return None


def as_written(path: str) -> str:
    r"""A 1.4 path with its `\u` escapes undone where jsonvet writes the character itself.

    jsonvet keeps escaping control and invisible characters, `…` and lone surrogates.
    """

    def undo(m: re.Match) -> str:
        text = json.loads(f'"{m[0]}"')
        return "".join(c if c >= "\x80" and c.isprintable() and c != "…" else json.dumps(c)[1:-1] for c in text)

    return ESCAPE.sub(undo, path)


def classify(name: str, text: str, ours: list[Problem], theirs: list[Problem]) -> str:  # noqa: PLR0911 - a return per category
    """Why the two tools disagree, as an accepted category or `unexplained`."""
    if name in KNOWN:
        return KNOWN[name]
    written = [p._replace(text=as_written(p.text)) if p.kind == "duplicate" else p for p in theirs]
    if written != theirs:
        if failed(ours) == failed(theirs) and duplicates(ours) == duplicates(written):
            return "accepted: non-ASCII keys in paths print as written, not as \\u escapes"
        theirs = written
    flipped = failed(ours) != failed(theirs)
    syntax_differs = has_syntax_error(ours) != has_syntax_error(theirs)
    # The same duplicate keys with the same paths, at other positions.
    moved_only = (
        not flipped and not syntax_differs and [d[2:] for d in duplicates(ours)] == [d[2:] for d in duplicates(theirs)]
    )
    if "\u2028" in text or "\u2029" in text:
        # Both are allowed in strings, and end a `//` comment, in jsonvet only. So jsonvet may
        # accept a string 1.4 rejects, and may reject a file only where a `//` comment ends early.
        string_accepted = not has_syntax_error(ours) and has_syntax_error(theirs)
        comment_ended = has_syntax_error(ours) and "//" in text
        if moved_only or string_accepted or comment_ended:
            return "accepted: U+2028/U+2029 (line terminators in jsonvet, and allowed in strings)"
    elif "\r" in text.replace("\r\n", "") and moved_only:
        # A lone CR ends a line in jsonvet, which moves positions but never changes a verdict.
        return "accepted: lone CR ends a line in jsonvet"
    if reason := syntax_only_in_ours(ours, theirs):
        return reason
    if flipped:
        return "unexplained: verdict"
    return "unexplained: syntax verdict" if syntax_differs else "unexplained: duplicate reports"


def compare(mode: str, files: list[Path], cwd: Path, jsonvet: str, extra: list[str]) -> Counter:
    """Run both tools on `files` and print how their reports differ, by category."""
    ours = run([jsonvet, "--no-config", *extra], files, cwd)
    theirs = run([*REFERENCE, *extra], files, cwd)
    tally: Counter = Counter()
    examples: dict[str, list[str]] = defaultdict(list)
    for path in files:
        a, b = ours.get(path.name, []), theirs.get(path.name, [])
        # The syntax verdicts are compared on their own: a syntax error on one side and only a
        # duplicate key on the other would otherwise both read as a failure.
        same_verdicts = failed(a) == failed(b) and has_syntax_error(a) == has_syntax_error(b)
        if same_verdicts and (has_syntax_error(a) or duplicates(a) == duplicates(b)):
            tally["same"] += 1
            continue
        # newline="": a lone CR must stay one, as the tools read it. Path.read_text takes
        # newline only from Python 3.13 on, and this script runs on 3.12.
        with path.open(encoding="utf-8", newline="") as f:
            text = f.read()
        category = classify(path.name, text, a, b)
        tally[category] += 1
        if len(examples[category]) < EXAMPLES:
            examples[category].append(f"{path.name}: {text[:80]!r}\n      jsonvet: {a}\n      1.4:     {b}")
    print(f"\n== {mode}: {len(files)} files")
    for category, n in tally.most_common():
        print(f"{n:6}  {category}")
    for category, items in examples.items():
        shown = items if category.startswith("unexplained") else items[:2]
        print(f"\n-- {category}")
        print("\n".join(shown))
    return tally


def check_reference(cwd: Path) -> None:
    """Fetch the reference fresh, and check it passes a valid file and fails an invalid one.

    Otherwise a broken install would read as a file with no problems.
    """
    (cwd / "probe-ok.json").write_text("{}")
    (cwd / "probe-bad.json").write_text("[1,]")
    refresh = [REFERENCE[0], "--refresh-package", "check-json5", *REFERENCE[1:]]
    ok = subprocess.run(  # noqa: S603 - our own command line
        [*refresh, "--strict", "*", "probe-ok.json"], cwd=cwd, capture_output=True, text=True, check=False
    )
    bad = subprocess.run(  # noqa: S603 - our own command line
        [*REFERENCE, "--strict", "*", "probe-bad.json"], cwd=cwd, capture_output=True, text=True, check=False
    )
    if ok.returncode != 0 or bad.returncode != 1 or "probe-bad.json" not in bad.stdout:
        sys.exit(f"the reference does not work: {ok.stderr or bad.stderr or bad.stdout}")
    for probe in ("probe-ok.json", "probe-bad.json"):
        (cwd / probe).unlink()


def main() -> int:
    """Run the comparison in both modes; 1 when a difference is unexplained."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--cases", type=int, default=3000)
    parser.add_argument("--seed", type=int, default=7)
    parser.add_argument("--jsonvet", default=str(ROOT / "target" / "release" / "jsonvet"))
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="jsonvet-differential-") as tmp:
        cwd = Path(tmp)
        check_reference(cwd)
        files = corpus(cwd, args.cases, args.seed)
        totals = compare("json5", files, cwd, args.jsonvet, [])
        totals += compare("strict", files, cwd, args.jsonvet, ["--strict", "*"])
    unexplained = sum(n for c, n in totals.items() if c.startswith("unexplained"))
    print(f"\nunexplained differences: {unexplained}")
    return 1 if unexplained else 0


if __name__ == "__main__":
    sys.exit(main())
