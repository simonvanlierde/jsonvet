# /// script
# requires-python = ">=3.12"
# ///
"""Time jsonvet against check-json5 1.4.1 with hyperfine, and print a Markdown table.

Usage: uv run scripts/bench.py [--jsonvet PATH] [--runs N]

Each command runs without a shell (`hyperfine -N`) after a warm-up, 20 times by default, and the
table shows the median.

Needs hyperfine and a release build (`cargo build --release -p jsonvet-cli`). The reference is
installed into its own virtual environment and run from there, so uvx's start-up is not timed.
Every case passes in both tools, apart from the `dup` files, which hold a duplicate key.
"""

import argparse
import json
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


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


def main() -> int:
    """Install the reference, write the fixtures, time each case and print the table."""
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--jsonvet", type=Path, default=ROOT / "target" / "release" / "jsonvet")
    parser.add_argument("--runs", type=int, default=20)
    args = parser.parse_args()
    jsonvet = args.jsonvet.resolve()
    release = subprocess.run([jsonvet, "--version"], check=True, capture_output=True, text=True).stdout.split()[-1]  # noqa: S603
    release = ".".join(release.split(".")[:2])
    with tempfile.TemporaryDirectory() as scratch:
        tmp = Path(scratch)
        venv = tmp / "venv"
        setup = [["venv", "--python", "3.13", venv], ["pip", "install", "--python", venv, "check-json5==1.4.1"]]
        for step in setup:
            subprocess.run(["uv", "--quiet", *step], check=True)  # noqa: S603, S607 - fixed commands
        reference = venv / "bin" / "check-json5"
        files = tmp / "files"
        files.mkdir()
        result = tmp / "result.json"
        rows = [f"| Case | check-json5 1.4.1 | jsonvet {release} | Speed-up |", "| --- | ---: | ---: | ---: |"]
        for name, case in fixtures(files):
            listed = " ".join(case)
            # -i: the duplicate-key case exits 1 by design.
            hyperfine = ["hyperfine", "--style", "none", "--warmup", "2", "--runs", str(args.runs), "-N", "-i"]
            hyperfine += ["--export-json", result, "-n", "reference", f"{reference} {listed}"]
            hyperfine += ["-n", "jsonvet", f"{jsonvet} {listed}"]
            subprocess.run(hyperfine, cwd=files, check=True, capture_output=True)  # noqa: S603
            times = {r["command"]: r["median"] for r in json.loads(result.read_text())["results"]}
            ref, new = times["reference"], times["jsonvet"]
            rows.append(f"| {name} | {ref * 1e3:,.0f} ms | {new * 1e3:,.1f} ms | {ref / new:,.0f}x |")
    print("\n".join(rows))
    return 0


if __name__ == "__main__":
    sys.exit(main())
