# /// script
# requires-python = ">=3.11"
# ///
"""Render conformance/results/ into docs/conformance.md.

Usage: uv run conformance/report.py [--check]

--check only verifies that README.md names the right number of parsers.

Each input file gets an expected verdict: from the spec where the spec decides it (SPEC below),
otherwise from the json5-tests file extension, conformance/extra/*/expect.tsv, or jsonvet's
reviewed corpus snapshot. A parser's cell says whether its verdict agrees. See README.md.
"""

import json
import re
import sys
from collections import Counter
from pathlib import Path

from run import TIMEOUT

ROOT = Path(__file__).resolve().parent.parent
HERE = ROOT / "conformance"
RESULTS = HERE / "results"
OUT = ROOT / "docs/conformance.md"
README = ROOT / "README.md"
SNAPSHOT = ROOT / "crates/jsonvet-cli/tests/snapshots/corpus__corpus.snap"
CORPUS = "tests/corpus/"
SUITE = "tests/json5-tests/"
EXTRA = "conformance/extra/"

# json5-tests cases labelled wrongly; json5/json5-tests#4. The list is shared with
# crates/jsonvet/tests/conformance.rs, which holds the same extension-to-verdict rule as
# `expectation` below: change both together.
MISLABELED = {
    f"{SUITE}{case}": why
    for case, why in (
        line.split("\t", 1)
        for line in (ROOT / "tests/json5-tests-mislabeled.tsv").read_text().splitlines()
        if line and not line.startswith("#")
    )
}

# (title, dialect, spec link, input folders)
TABLES = [
    ("JSON5", "json5", "https://spec.json5.org/", [f"{CORPUS}json5/", SUITE]),
    ("JSONC", "jsonc", "https://jsonc.org/", [f"{CORPUS}jsonc/", f"{EXTRA}jsonc/"]),
    (
        "JWCC (HuJSON)",
        "jwcc",
        "https://nigeltao.github.io/blog/2021/json-with-commas-comments.html",
        [f"{CORPUS}jwcc/", f"{EXTRA}jwcc/"],
    ),
    ("JSON Lines", "jsonl", "https://jsonlines.org/", [f"{CORPUS}jsonl/", f"{EXTRA}jsonl/"]),
    (
        "Strict JSON (baseline)",
        "json",
        "https://www.rfc-editor.org/rfc/rfc8259",
        [f"{CORPUS}json/", f"{CORPUS}i-json/", SUITE],
    ),
]

# Corpus files whose verdict the spec itself decides: (expected, the section that decides it).
# "either" means the spec allows both verdicts, so the row is shown but not scored.
J5 = "JSON5 1.0.0"
ES = "ECMAScript 5.1 §7.6"
SPEC = {
    "json5/bom.json": ("valid", f"{J5} §8: U+FEFF is white space"),
    "json5/combining-mark-in-key.json": ("valid", f"{J5} §3 and {ES}: a combining mark may continue a key"),
    "json5/combining-mark-starts-key.json": ("invalid", f"{J5} §3 and {ES}: a combining mark cannot start a key"),
    "json5/combining-mark-starts-key-escaped.json": (
        "invalid",
        f"{ES}: an escape in a key must still be a character allowed there",
    ),
    "json5/letter-u0e33-starts-key.json": ("valid", f"{ES}: U+0E33 is a letter in every Unicode version"),
    "json5/mark-u1885-starts-key.json": (
        "either",
        f"{ES} allows any Unicode version from 3.0: U+1885 was a letter until Unicode 9.0, a mark since",
    ),
    "json5/punctuation-u00b7-in-key.json": (
        "invalid",
        f"{ES}: U+00B7 is punctuation, not allowed in a key (ECMAScript 2015 later allowed it)",
    ),
    "json5/key-with-dash.json": ("invalid", f"{J5} §3: an unquoted key must be an IdentifierName"),
    "json5/number-led-key.json": ("invalid", f"{J5} §3: an unquoted key must be an IdentifierName"),
    "json5/escape-letter.json": ("valid", f"{J5} §5.1: an unknown escape keeps the letter and drops the backslash"),
    "json5/escape-nine.json": ("invalid", f"{J5} §9.1: escapes of the digits 1 to 9 are not in the grammar"),
    "json5/escaped-newline-in-string.json": ("valid", f"{J5} §5.1: a backslash before a line break continues it"),
    "json5/raw-newline-in-string.json": ("invalid", f"{J5} §5: a string cannot hold a raw line break"),
    "json5/raw-u2028-in-string.json": (
        "valid",
        f"{J5} §5.2: U+2028 may appear raw in a string (the grammar in §9.1 says otherwise)",
    ),
    "json5/line-comment-ends-at-u2028-clean.json": (
        "valid",
        f"{J5} §7: a line comment ends at a LineTerminator, and U+2028 is one",
    ),
    "json5/line-comment-ends-at-u2028.json": ("invalid", f"{J5} §7: the comment ends at U+2028, so x is left over"),
    "json5/hex-and-friends.json": ("valid", f"{J5} §6: hex, bare decimal points, plus signs, Infinity, NaN"),
    "json5/negative-hex.json": ("valid", f"{J5} §6: hex numbers may be signed"),
    "json5/top-level-scalar.json": ("valid", f"{J5} §2: a JSON5 text is any value"),
    "json5/empty.json": ("invalid", f"{J5} §9.2: a JSON5 text is exactly one value"),
    "json5/whitespace-only.json": ("invalid", f"{J5} §9.2: a JSON5 text is exactly one value"),
    "json5/double-comma-array.json": ("invalid", f"{J5} §4: a single comma may follow the final element"),
    "json5/lone-comma-array.json": ("invalid", f"{J5} §4: a single comma may follow the final element"),
    "json5/lone-comma-object.json": ("invalid", f"{J5} §3: a single comma may follow the final member"),
    "json5/not-utf8.json": ("either", f"{J5} §9.1 works on Unicode characters and names no byte encoding"),
    "json5/depth-10000.json": ("either", f"{J5} §10: a parser may limit the depth of nesting"),
    "json5/depth-10001.json": ("either", f"{J5} §10: a parser may limit the depth of nesting"),
    "json5/integer-5000-digits.json": ("either", f"{J5} §10: a parser may limit the range of numbers"),
    "json5/unclosed-comment.json": ("invalid", f"{J5} §7: a block comment ends with */"),
    "json5/unclosed-comment-own-star-slash.json": ("invalid", f"{J5} §7: the */ cannot reuse the opening *"),
    "json/bom.json": ("either", "RFC 8259 §8.1: a parser MAY ignore a byte order mark"),
    "i-json/bom.json": ("either", "RFC 8259 §8.1: a parser MAY ignore a byte order mark"),
    "json/duplicate-key.json": ("valid", "RFC 8259 §4: names SHOULD be unique, but repeats are valid syntax"),
    "i-json/duplicate-key.json": ("valid", "RFC 8259 §4: names SHOULD be unique, but repeats are valid syntax"),
    "i-json/lone-surrogate.json": ("valid", "RFC 8259 §8.2: the grammar allows a lone surrogate escape"),
    "json/lone-surrogates-are-distinct-keys.json": ("valid", "RFC 8259 §8.2: the grammar allows lone surrogates"),
    "i-json/unsafe-integer.json": ("valid", "RFC 8259 §6: the grammar has no range limit (I-JSON has one)"),
    "json/leading-zero.json": ("invalid", "RFC 8259 §6: a number cannot have a leading zero"),
    "jsonc/trailing-comma.json": ("either", "JSONC, Trailing commas: parsers MAY support them"),
    "jsonc/line-comment-not-ended-by-u2028.json": (
        "invalid",
        "JSONC, Conventions: a line ending is LF, CRLF or CR, so U+2028 does not end the comment",
    ),
    "jwcc/single-quotes.json": ("invalid", "JWCC adds only comments and trailing commas; no single quotes"),
    "jwcc/trailing-comma-and-comment.json": ("valid", "JWCC: one comma after the final element, and comments"),
    "jsonl/blank-line.jsonl": ("invalid", "JSON Lines rule 2: a blank line is not a value"),
    "jsonl/crlf-duplicate.jsonl": ("valid", "JSON Lines rule 3: \\r\\n works, as white space around a value"),
    "jsonl/no-final-newline.jsonl": ("valid", "JSON Lines rule 3: a final line terminator is not required"),
    "jsonl/two-values-on-a-line.jsonl": ("invalid", "JSON Lines rule 2: each line is one JSON value"),
    "jsonl/value-across-lines.jsonl": ("invalid", "JSON Lines rule 2: each line is one JSON value"),
}

# Plain-words notes on rows where parsers disagree with the expectation. A note shows only while
# its row still has a disagreement, so a fixed parser drops it from the page.
C5 = "tests/corpus/json5/"
NOTES = {
    f"{C5}unclosed-comment.json": "Go `titanous/json5` accepts a block comment that never closes at the end"
    " of the file (also unclosed-comment-own-star-slash.json and json5-tests unterminated-block-comment.txt).",
    f"{C5}escape-nine.json": "The Rust `json5` crate accepts the escape `\\9`, which the grammar does not have.",
    f"{C5}escape-letter.json": "Go `titanous/json5` rejects `\\a`, which §5.1 says is a plain `a`.",
    f"{C5}hex-and-friends.json": "Go `titanous/json5` rejects `-NaN`, and Rust `serde_json5` rejects"
    " `+Infinity`; §6 allows a sign on both.",
    f"{C5}negative-hex.json": "Rust `serde_json5` rejects a sign on a hex number, `-0x10` and `+0x1F` alike.",
    f"{C5}raw-u2028-in-string.json": "Rust `serde_json5` rejects a raw U+2028 in a string. §5.2 allows it but"
    " the grammar in §9.1 does not, so the spec shares the blame.",
    f"{C5}bom.json": "Go `titanous/json5` rejects a leading byte order mark, which §8 lists as white space.",
    f"{C5}letter-u0e33-starts-key.json": "Go `titanous/json5` takes only ASCII letters, digits, `_` and `$` in"
    " unquoted keys, with no escapes, so it also rejects the keys in other rows that hold accents, combining"
    " marks or `\\u` escapes.",
    f"{C5}combining-mark-starts-key-escaped.json": "Python `json5`, `pyjson5` and Rust `serde_json5` accept an"
    " escaped combining mark at the start of a key; ECMAScript 5.1 §7.6 does not allow one there.",
    f"{C5}punctuation-u00b7-in-key.json": "Rust `serde_json5` accepts a middle dot (U+00B7) in a key. ECMAScript"
    " 2015 allows it; ECMAScript 5.1, which JSON5 cites, does not.",
    f"{C5}line-comment-ends-at-u2028.json": "Go `titanous/json5` does not end a line comment at U+2028, so the"
    " stray `x` after it disappears into the comment.",
    f"{C5}depth-10000.json": "Python `json5` raises RecursionError. Rust `serde_json5` overflowed its stack"
    " in the 2026-10-14 run and parsed the file in the 2026-10-19 run, with the same version, compiler and"
    " 8 MB stack, so its verdict here is not stable. `pyjson5` rejects the file at its own nesting limit,"
    " which §10 allows.",
    "conformance/extra/jsonc/lone-cr-ends-comment.jsonc": "`strip-json-comments` and Go `tidwall/jsonc` end a"
    " line comment only at LF, so a comment ended by a lone CR swallows the rest of the file.",
    "conformance/extra/jsonc/comment-only.jsonc": "The Rust `jsonc-parser` returns no value and no error for a"
    " file holding only a comment; the driver counts that as an accept.",
    "conformance/extra/jsonc/json5-syntax.jsonc": "The Rust `jsonc-parser` accepts unquoted keys, single quotes"
    " and hex by default; it is lenient by design.",
    "conformance/extra/jwcc/line-comment-at-end.json": "`hujson`, the reference implementation, rejects a line"
    " comment that ends the file without a newline. The spec allows that since its 2021-07-20 update"
    " (reported as tailscale/hujson#56).",
    "conformance/extra/jsonl/scalars.jsonl": "Python `jsonlines` rejects a `null` line unless you pass"
    " `allow_none=True`; the spec permits any JSON value.",
    "conformance/extra/jsonl/bom.jsonl": "Python `jsonlines` accepts a leading byte order mark, which the spec"
    " forbids.",
    "tests/corpus/jsonl/blank-line.jsonl": "Go's `json.Decoder` and serde_json's `StreamDeserializer` read values"
    " split by any white space, not by lines. They are how those languages usually read JSON Lines, so blank"
    " lines, two values on a line and a value across lines all pass.",
    "tests/corpus/json/nan-and-infinity.json": "Python's `json` accepts NaN, Infinity and -Infinity by default;"
    " RFC 8259 §6 has no such numbers.",
    "tests/corpus/i-json/lone-surrogate.json": "serde_json rejects an escaped lone surrogate when it builds the"
    " string, though the RFC 8259 grammar allows one (§8.2).",
}

MARK = {True: "✓", False: "✗"}
CRASH = "💥"


def snapshot() -> dict[str, tuple[bool, bool]]:
    """Corpus path -> (valid apart from duplicate keys, the only errors are duplicate keys)."""
    out: dict[str, tuple[bool, bool]] = {}
    for entry in re.split(r"\n(?=\S)", SNAPSHOT.read_text().split("---\n", 2)[2]):
        head, *details = entry.strip().splitlines()
        path, code = head.split(" -> ")
        errors = [line for line in details if " error[" in line or ": error:" in line]
        dupes_only = bool(errors) and all("error[duplicate-key]" in line for line in errors)
        out[CORPUS + path] = (code == "0" or dupes_only, dupes_only)
    return out


def extras() -> dict[str, tuple[str, str]]:
    """conformance/extra/<dialect>/expect.tsv, as path -> (expected, why)."""
    out = {}
    for tsv in sorted((HERE / "extra").glob("*/expect.tsv")):
        for line in tsv.read_text().splitlines():
            if line and not line.startswith("#"):
                name, expected, why = line.split("\t")
                out[f"{EXTRA}{tsv.parent.name}/{name}"] = (expected, why)
    return out


def expectation(path: str, dialect: str, snap: dict, extra: dict) -> tuple[str, str]:
    """The expected verdict for a file in a dialect's table, and the spec note if any."""
    if path.startswith(SUITE):
        ext = path.rsplit(".", 1)[1]
        if dialect == "json5":
            return ("valid" if ext in {"json", "json5"} else "invalid"), ""
        if path in MISLABELED:
            return "invalid", f"json5/json5-tests#4: named .json but {MISLABELED[path]}"
        return ("valid" if ext == "json" else "invalid"), ""
    if path.startswith(EXTRA):
        return extra[path]
    short = path.removeprefix(CORPUS)
    if short in SPEC:
        return SPEC[short]
    return ("valid" if snap[path][0] else "invalid"), ""


def cell(verdict: str, expected: str) -> str:
    """One grid cell."""
    if verdict in {"crash", "timeout"}:
        return CRASH
    if expected == "either":
        return verdict[0].upper()  # A or R; both conform
    return MARK[(verdict == "accept") == (expected == "valid")]


def display(path: str) -> str:
    """A short link to an input file, relative to docs/."""
    name = path.removeprefix("tests/").removeprefix("conformance/")
    return f"[{name}](../{path})"


def grid(columns: list[str], rows: list[tuple[str, str, str]], verdicts: dict) -> list[str]:
    """A markdown table of rows (path, expected, note) by parser columns."""
    lines = ["| file | expected | " + " | ".join(f"`{c}`" for c in columns) + " |"]
    lines.append("|---|---|" + "---|" * len(columns))
    for path, expected, note in rows:
        cells = [cell(verdicts[c][path], expected) for c in columns]
        lines.append(f"| {display(path)} | {expected}{' †' if note else ''} | " + " | ".join(cells) + " |")
    return lines


def dialect_section(
    table: tuple[str, str, str, list[str]], data: tuple[dict, dict, dict, dict], scores: dict
) -> list[str]:
    """One dialect's heading, summary, grid and notes; adds each parser's agreement to scores."""
    title, dialect, link, folders = table
    verdicts, parsers, snap, extra = data
    columns = [p for p, info in parsers.items() if dialect in info["uses"]]
    files = [p for p in next(iter(verdicts.values())) if any(p.startswith(f) for f in folders)]
    order = {"valid": 0, "invalid": 1, "either": 2}
    rows = sorted(((p, *expectation(p, dialect, snap, extra)) for p in files), key=lambda r: (order[r[1]], r[0]))
    scored = [r for r in rows if r[1] != "either"]
    out = [f"## {title}", "", f"Spec: <{link}>. {len(rows)} files, {len(scored)} scored.", ""]

    out += ["| parser | agrees | disagrees | crash or timeout |", "|---|---|---|---|"]
    for c in columns:
        marks = Counter(cell(verdicts[c][p], e) for p, e, _ in scored)
        crashes = sum(verdicts[c][p] in {"crash", "timeout"} for p, _, _ in rows)
        pct = 100 * marks[MARK[True]] / len(scored)
        out.append(f"| `{c}` | {marks[MARK[True]]} ({pct:.0f}%) | {marks[MARK[False]]} | {crashes} |")
        scores.setdefault((title, len(scored)), []).append((c, pct))
    out.append("")

    def clean(row: tuple[str, str, str]) -> bool:
        return all(cell(verdicts[c][row[0]], row[1]) not in {MARK[False], CRASH} for c in columns)

    disputed = [r for r in rows if not clean(r)]
    agreed = [r for r in rows if clean(r)]
    if disputed:
        out += [f"Files where at least one parser disagrees ({len(disputed)}):", ""]
        out += [*grid(columns, disputed, verdicts), ""]
    if agreed:
        out += [f"<details><summary>Files where every parser agrees ({len(agreed)})</summary>", ""]
        out += [*grid(columns, agreed, verdicts), "", "</details>", ""]

    noted = [r for r in rows if r[2]]
    if noted:
        out += ["Spec notes (†):", ""] + [f"- {display(p)}: {note}." for p, _, note in noted] + [""]
    notes = [r for r in disputed if r[0] in NOTES]
    if notes:
        out += ["Notable disagreements:", ""] + [f"- {display(p)}: {NOTES[p]}" for p, _, _ in notes] + [""]
    return out


def duplicates_section(verdicts: dict, snap: dict) -> list[str]:
    """Which parsers can report duplicate keys at all, on the corpus files whose only flaw is one."""
    files = [p for p, (_, dupes) in snap.items() if dupes]
    # Each detecting variant, and the parser run with the same syntax but without detection.
    variants = {
        "py-json-dupes": lambda _: "py-json",
        "py-json5-dupes": lambda _: "py-json5",
        "py-jiter-dupes": lambda _: "py-jiter",
    }
    variants["jsonvet-dupes"] = lambda p: "jsonvet-" + ("json" if "/i-json/" in p else p.split("/")[2])
    out = [
        "## Duplicate keys",
        "",
        "Every spec here lets a parser keep one of the values, and JSON5 §3 also lets it fail. Only four",
        "parsers in the matrix can report duplicates: Python's `json` through an `object_pairs_hook`",
        "that raises, Python `json5` with `allow_duplicate_keys=False`, jiter with",
        "`catch_duplicate_keys=True`, and jsonvet by default. The",
        "others have no switch for it in their sources, and accepted these files wherever their syntax",
        "allowed. A dash means the parser rejects the file's syntax, so the test says nothing.",
        "",
        "| file | " + " | ".join(f"`{v}`" for v in variants) + " |",
        "|---|" + "---|" * len(variants),
    ]
    for path in files:
        cells = []
        for variant, base in variants.items():
            got = verdicts[variant][path]
            if got in {"crash", "timeout"}:
                cells.append(CRASH)
            elif verdicts[base(path)][path] != "accept":
                cells.append("-")
            else:
                cells.append("detected" if got == "reject" else "missed")
        out.append(f"| {display(path)} | " + " | ".join(cells) + " |")
    return [*out, ""]


def check_readme_count(parsers: dict) -> int:
    """1 if README.md's "against N other parsers" disagrees with the matrix, else 0."""
    # The duplicate-key variants run a parser already counted.
    others = sum(info["language"] != "jsonvet" and "dupes" not in info["uses"] for info in parsers.values())
    found = re.search(r"against (\d+) other parsers", README.read_text())
    if found and int(found.group(1)) == others:
        return 0
    print(f'README.md should say "against {others} other parsers" (found {found and found.group(0)!r})')
    return 1


def main() -> None:
    """Write docs/conformance.md and the summary table in README.md."""
    summary = json.loads((RESULTS / "versions.json").read_text())
    parsers = summary["parsers"]
    if sys.argv[1:] == ["--check"]:
        sys.exit(check_readme_count(parsers))
    verdicts = {
        name: dict(line.split("\t") for line in (RESULTS / f"{name}.tsv").read_text().splitlines()) for name in parsers
    }
    data = (verdicts, parsers, snapshot(), extras())
    out = [
        "# Conformance matrix",
        "",
        "<!-- Generated by conformance/report.py from conformance/results/. Do not edit. -->",
        "",
        f"How {len(parsers)} parser runs treat the same files in JSON5, JSONC, JWCC (HuJSON), JSON",
        "Lines and strict JSON. Each dialect's table lists the parsers that claim that dialect.",
        f"Run on {summary['date']}; a full run took {summary['seconds']} s. How it works and how to",
        "rerun it: [conformance/README.md](../conformance/README.md).",
        "",
        "Cells: ✓ the parser agrees with the expected verdict, ✗ it disagrees, 💥 it crashed or",
        f"took longer than {TIMEOUT} s. Where the spec allows both verdicts (expected: either), the cell",
        "shows what the parser did: A accepted, R rejected. Duplicate keys count as valid in the",
        "syntax tables (jsonvet runs with `--ignore duplicate-key`); they have their own table.",
        "",
        "Expected verdicts come from the spec where it decides the case (marked †, with the section",
        "under the table), from the json5-tests file extension, from conformance/extra/, or else",
        "from jsonvet's reviewed corpus snapshot. Where jsonvet disagrees with a spec, its column",
        "shows ✗ like any other parser.",
        "",
    ]
    scores: dict[tuple[str, int], list[tuple[str, float]]] = {}
    for table in TABLES:
        out += dialect_section(table, data, scores)
    out += duplicates_section(verdicts, data[2])
    out += ["## Versions", "", "| parser | language | package | version |", "|---|---|---|---|"]
    for name, info in parsers.items():
        out.append(f"| `{name}` | {info['language']} | {info['package'] or '-'} | {info['version']} |")
    images = ", ".join(f"`{image}`" for image in summary["images"].values())
    out += ["", f"Images: {images}. Runtimes: " + "; ".join(f"{k} {v}" for k, v in summary["runtimes"].items()) + "."]
    OUT.write_text("\n".join(out) + "\n")
    write_readme_summary(scores, summary["date"])
    print(f"wrote {OUT.relative_to(ROOT)} and the summary in README.md")
    sys.exit(check_readme_count(parsers))


def write_readme_summary(scores: dict, date: str) -> None:
    """Replace the table between README.md's conformance markers with each parser's agreement."""
    start, end = "<!-- conformance:start -->", "<!-- conformance:end -->"
    lines = [
        start,
        "",
        "| dialect | files | share of files where each parser gives the expected verdict |",
        "|---|---|---|",
    ]
    for (title, files), row in scores.items():
        ranked = sorted(row, key=lambda r: (-r[1], r[0]))
        lines.append(f"| {title} | {files} | " + ", ".join(f"`{c}` {pct:.0f}%" for c, pct in ranked) + " |")
    grid = "[docs/conformance.md](https://github.com/simonvanlierde/jsonvet/blob/main/docs/conformance.md)"
    lines += ["", f"Run on {date}. The full grid, and where each expected verdict comes from: {grid}.", "", end]
    text = README.read_text()
    before, rest = text.split(start, 1)
    README.write_text(before + "\n".join(lines) + rest.split(end, 1)[1])


if __name__ == "__main__":
    main()
