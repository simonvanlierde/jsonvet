# /// script
# requires-python = ">=3.12"
# ///
"""Time jsonvet against check-json5 1.4.1 and 1.0.1 with hyperfine, and print a Markdown table.

Usage: uv run scripts/bench.py [--jsonvet PATH] [--runs N] [--ref-runs N] [--old-runs N] [--write]
       uv run scripts/bench.py --files DIR

Each command runs without a shell (`hyperfine -N`) and the table shows the median. jsonvet runs
20 times after a warm-up by default. check-json5 1.4.1 is far slower, so run-to-run noise moves
its median less, and it runs 10 times; the JSON5 case alone takes seconds a run. check-json5
1.0.1, the last release of the original hook, reads every file with the pure-Python json5
package and takes seconds a file, so it runs 3 times with no warm-up by default. `--write` also
puts the table, with the machine and date, between the `speed` markers in docs/design.md;
check-versions.py fails when that table names an older minor release than Cargo.toml.

Needs hyperfine and a release build (`cargo build --release -p jsonvet-cli`). Each check-json5 is
installed into its own virtual environment and run from there, so uvx's start-up is not timed.
1.0.1 was never on PyPI, so it comes from the commit its v1.0.1 tag names.
Every case passes in both tools, apart from the `dup` files, which hold a duplicate key.

`--files DIR` only writes the files, plus two large ones for `just profile` and `just counters`,
and times nothing.
"""

import argparse
import datetime as dt
import json
import os
import platform
import re
import subprocess
import sys
import tempfile
import textwrap
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DESIGN = ROOT / "docs" / "design.md"
README = ROOT / "README.md"
# 1.0.1 by its commit, the v1.0.1 tag: a tag can be moved.
REFERENCES = {
    "1.4.1": "check-json5==1.4.1",
    "1.0.1": "check-json5 @ git+https://github.com/maresb/check-json5@82381b1133603e4ff6eb3d866ec4501d121b669b",
}
MARKERS = ("<!-- speed:start -->\n", "<!-- speed:end -->")


def doc(n: int) -> dict:
    """An object of n rows, each a small array of every JSON type."""
    return {f"k{i}": [1, 2.5, "text", True, None, {"x": i, "y": [i, "s"]}] for i in range(n)}


def json5_doc(n: int) -> str:
    """JSON5 that strict JSON rejects: unquoted keys, single quotes, hex, comments, trailing commas."""
    rows = (f"  k{i}: [1, 0x1F, .5, 'text', true, null, {{x: {i}, y: [{i}, 's',],}},], // row {i}" for i in range(n))
    return "{\n" + "\n".join(rows) + "\n}\n"


def fixtures(out: Path) -> list[tuple[str, list[str]]]:
    """Write the files and return (case name, file names) pairs."""
    small = json.dumps({"compilerOptions": {f"opt{i}": True for i in range(30)}, "include": ["src"]}, indent=2)
    names = []
    for i in range(1000):
        (out / f"s{i}.json").write_text(small)
        names.append(f"s{i}.json")
    for i in range(100):
        (out / f"c{i}.jsonc").write_text("// c\n" + small)
        names.append(f"c{i}.jsonc")
    big = json.dumps(doc(20000), indent=2)
    (out / "big.json").write_text(big)
    (out / "big_dup.json").write_text(big[:-2] + ',\n  "k5": 0\n}')
    (out / "mid.jsonc").write_text("// c\n" + json.dumps(doc(1000), indent=2))
    (out / "big.json5").write_text(json5_doc(20000))
    return [
        ("1,100 config files", names),
        ("2.7 MB JSON", ["big.json"]),
        ("2.7 MB JSON, duplicate key", ["big_dup.json"]),
        ("120 KB JSONC", ["mid.jsonc"]),
        (f"{(out / 'big.json5').stat().st_size / 1e6:.1f} MB JSON5", ["big.json5"]),
    ]


def profile_files(out: Path) -> list[str]:
    """Write doc(600000) pretty (88.5 MB) and minified (38.7 MB), the scanner profile's inputs."""
    big = doc(600_000)
    (out / "doc-pretty.json").write_text(json.dumps(big, indent=2))
    (out / "doc-min.json").write_text(json.dumps(big, separators=(",", ":")))
    return ["doc-pretty.json", "doc-min.json"]


def aligned(rows: list[list[str]]) -> list[str]:
    """Markdown table lines with padded columns, the first left-aligned and the rest right-aligned."""
    widths = [max(len(row[i]) for row in rows) for i in range(len(rows[0]))]

    def line(row: list[str]) -> str:
        cells = [
            cell.ljust(w) if i == 0 else cell.rjust(w) for i, (cell, w) in enumerate(zip(row, widths, strict=True))
        ]
        return "| " + " | ".join(cells) + " |"

    rule = "| " + " | ".join(["-" * widths[0]] + ["-" * (w - 1) + ":" for w in widths[1:]]) + " |"
    return [line(rows[0]), rule, *map(line, rows[1:])]


def verdict(tool: Path, case: list[str], cwd: Path) -> int:
    """The exit code of one tool on one case."""
    return subprocess.run([tool, *case], check=False, cwd=cwd, capture_output=True).returncode  # noqa: S603


def machine() -> str:
    """The CPU model and core count, such as `12 cores of a Ryzen 7 H 255 under Linux`."""
    model = platform.machine()
    cpuinfo = Path("/proc/cpuinfo")
    if cpuinfo.exists() and (found := re.search(r"^model name\s*: (.+)$", cpuinfo.read_text(), re.MULTILINE)):
        model = re.sub(r"^(AMD|Intel\(R\)) |\(\w+\)| w/ .*| CPU .*|\s+\d+-Core Processor", "", found[1]).strip()
    return f"{os.cpu_count()} cores of a {model} under {platform.system()}"


def write(table: list[str], runs: int, ref_runs: int, old_runs: int, largest: str) -> None:
    """Replace the text between the speed markers in docs/design.md, and the README's top speed-up."""
    text = DESIGN.read_text()
    start, end = text.index(MARKERS[0]) + len(MARKERS[0]), text.index(MARKERS[1])
    intro = (
        f"Median of {runs} runs of jsonvet from `uv run scripts/bench.py`, on {machine()} "
        f"({dt.datetime.now(dt.UTC).date().isoformat()}), of {ref_runs} for 1.4.1 and of {old_runs} for 1.0.1. "
        "Every tool gives the same verdict in every case:"
    )
    DESIGN.write_text(text[:start] + "\n".join([textwrap.fill(intro, 100), "", *table]) + "\n" + text[end:])
    claim = r"(up to )[\d,]+(\s+times faster than check-json5 1\.4\.1)"
    README.write_text(re.sub(claim, rf"\g<1>{largest}\g<2>", README.read_text(), count=1))


def main() -> int:
    """Install the reference, write the fixtures, time each case and print the table."""
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--jsonvet", type=Path, default=ROOT / "target" / "release" / "jsonvet")
    parser.add_argument("--runs", type=int, default=20)
    parser.add_argument("--ref-runs", type=int, default=10, help="runs of check-json5 1.4.1")
    parser.add_argument("--old-runs", type=int, default=3, help="runs of check-json5 1.0.1")
    parser.add_argument("--write", action="store_true", help="put the table in docs/design.md")
    parser.add_argument("--files", type=Path, help="write the files here for profiling, and time nothing")
    args = parser.parse_args()
    if args.files:
        args.files.mkdir(parents=True, exist_ok=True)
        fixtures(args.files)
        print(f"{args.files}: {', '.join(profile_files(args.files))} and the timed cases")
        return 0
    jsonvet = args.jsonvet.resolve()
    release = subprocess.run([jsonvet, "--version"], check=True, capture_output=True, text=True).stdout.split()[-1]  # noqa: S603
    release = ".".join(release.split(".")[:2])
    with tempfile.TemporaryDirectory() as scratch:
        tmp = Path(scratch)
        tools = {}
        for version, requirement in REFERENCES.items():
            venv = tmp / f"venv-{version}"
            for step in (["venv", "--python", "3.13", venv], ["pip", "install", "--python", venv, requirement]):
                subprocess.run(["uv", "--quiet", *step], check=True)  # noqa: S603, S607 - fixed commands
            tools[version] = venv / "bin" / "check-json5"
        tools["jsonvet"] = jsonvet
        files = tmp / "files"
        files.mkdir()
        result = tmp / "result.json"

        def median(names: list[str], listed: str, runs: int, warmup: int) -> dict[str, float]:
            # -i: the duplicate-key case exits 1 by design.
            hyperfine = ["hyperfine", "--style", "none", "--warmup", str(warmup), "--runs", str(runs), "-N", "-i"]
            hyperfine += ["--export-json", result]
            for name in names:
                hyperfine += ["-n", name, f"{tools[name]} {listed}"]
            subprocess.run(hyperfine, cwd=files, check=True, capture_output=True)  # noqa: S603
            return {r["command"]: r["median"] for r in json.loads(result.read_text())["results"]}

        header = ["Case", "check-json5 1.0.1", "check-json5 1.4.1", f"jsonvet {release}", "Speed-up over 1.4.1"]
        rows = [[*header, "over 1.0.1"]]
        ups = []  # speed-ups over 1.4.1, for the README
        for name, case in fixtures(files):
            exits = {verdict(tool, case, files) for tool in tools.values()}
            if len(exits) != 1:
                print(f"{name}: the tools disagree", file=sys.stderr)
                return 1
            listed = " ".join(case)
            times = (
                median(["jsonvet"], listed, args.runs, 2)
                | median(["1.4.1"], listed, args.ref_runs, 2)
                | median(["1.0.1"], listed, args.old_runs, 0)
            )
            old, ref, new = times["1.0.1"], times["1.4.1"], times["jsonvet"]
            ups.append(ref / new)
            rows.append(
                [
                    name,
                    f"{old * 1e3:,.0f} ms",
                    f"{ref * 1e3:,.0f} ms",
                    f"{new * 1e3:,.1f} ms",
                    f"{ref / new:,.0f}x",
                    f"{old / new:,.0f}x",
                ]
            )
    table = aligned(rows)
    print("\n".join(table))
    if args.write:
        write(table, args.runs, args.ref_runs, args.old_runs, f"{max(ups):,.0f}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
