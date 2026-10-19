# /// script
# requires-python = ">=3.11"
# ///
"""Run every parser in the conformance matrix over the shared inputs.

Usage: uv run conformance/run.py [--only PARSER ...] [--work DIR] [--jsonvet PATH] [--lock]

Third-party parsers install and run only inside throwaway docker containers named tmp-matrix-*,
with the inputs mounted read-only and no network while they parse. jsonvet runs from the local
release build. Results go to conformance/results/<parser>.tsv and results/versions.json, and
conformance/report.py turns them into docs/conformance.md. See conformance/README.md.
"""

import argparse
import datetime
import json
import os
import queue
import re
import subprocess
import sys
import threading
import time
import tomllib
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HERE = ROOT / "conformance"
RESULTS = HERE / "results"
TIMEOUT = 10  # seconds per file
STARTUP = 120  # seconds for a container to start and print "ready"
CPUS = "4"  # per container; containers run one at a time


@dataclass(frozen=True)
class Lang:
    """How to install, lock and run one language's driver in its official image."""

    image: str
    setup: str  # shell; /src is the driver directory (read-only), /work is writable, network on
    lock: str  # shell; refreshes the lock file in /src, mounted writable
    run: str  # the driver command; /work is read-only and there is no network
    env: tuple[str, ...] = ()


LANGS = {
    "node": Lang(
        image="node:22-slim@sha256:c3de60bf2f9dd0ac6370e6117950ff62d6e339527e7472301c9c78a017978392",
        setup="cp /src/driver.mjs /src/package.json /src/package-lock.json /work/ && cd /work"
        " && npm ci --no-audit --no-fund --ignore-scripts --loglevel=error && node --version",
        lock="cd /src && npm install --package-lock-only --no-audit --no-fund --ignore-scripts",
        run="node /work/driver.mjs",
    ),
    "python": Lang(
        image="python:3.13-slim@sha256:70729b46c69b4f1e97c4822c1af3df53a1476cf5ddc6c087c0c10bc3a5678c2f",
        setup="pip install --quiet --no-cache-dir --disable-pip-version-check --only-binary=:all:"
        " --upgrade --target /work/site -r /src/requirements.txt"
        " && cp /src/driver.py /work/ && python -c 'import platform; print(platform.python_version())'",
        lock="true",  # requirements.txt pins every package by hand
        run="python /work/driver.py",
        env=("PYTHONPATH=/work/site",),
    ),
    "go": Lang(
        # The reference hujson needs Go 1.26.
        image="golang:1.26@sha256:d7722066f0b60ceccb6c0643cbed1f5f9e15506ac237146c95333504d7805d89",
        setup="cd /src && go build -trimpath -o /work/driver . && go env GOVERSION",
        lock="cd /src && go mod tidy",
        run="/work/driver",
        env=("GOPATH=/work/go", "GOCACHE=/work/gocache", "GOTOOLCHAIN=local", "GOFLAGS=-mod=readonly"),
    ),
    "rust": Lang(
        image="rust:1@sha256:81015672d6939b0265f09adf96d5de125482bb1330106076956b0518e74edc90",
        setup="cargo build --quiet --release --locked --manifest-path /src/Cargo.toml"
        " && rustc --version | cut -d' ' -f2",
        lock="cargo generate-lockfile --manifest-path /src/Cargo.toml",
        run="/work/target/release/conformance-driver",
        env=("CARGO_HOME=/work/cargo", "CARGO_TARGET_DIR=/work/target"),
    ),
}

# id: (language, package or None for the runtime's built-in parser, what the report uses it for).
# "dupes" marks a variant with duplicate detection turned on, used only in that table.
PARSERS: dict[str, tuple[str, str | None, list[str]]] = {
    "js-json5": ("node", "json5", ["json5"]),
    "js-jsonc-parser": ("node", "jsonc-parser", ["jsonc"]),
    "js-strip-json-comments": ("node", "strip-json-comments", ["jsonc"]),
    "js-json-parse": ("node", None, ["json"]),
    "py-json": ("python", None, ["json"]),
    "py-json5": ("python", "json5", ["json5"]),
    "py-pyjson5": ("python", "pyjson5", ["json5"]),
    "py-jsonlines": ("python", "jsonlines", ["jsonl"]),
    "py-jiter": ("python", "jiter", ["json"]),
    "py-json-dupes": ("python", None, ["dupes"]),
    "py-json5-dupes": ("python", "json5", ["dupes"]),
    "py-jiter-dupes": ("python", "jiter", ["dupes"]),
    "go-hujson": ("go", "github.com/tailscale/hujson", ["jwcc"]),
    "go-json5": ("go", "github.com/titanous/json5", ["json5"]),
    "go-tidwall-jsonc": ("go", "github.com/tidwall/jsonc", ["jsonc"]),
    "go-json-stream": ("go", None, ["jsonl"]),
    "rs-serde-json": ("rust", "serde_json", ["json"]),
    "rs-serde-json-stream": ("rust", "serde_json", ["jsonl"]),
    "rs-json5": ("rust", "json5", ["json5"]),
    "rs-serde-json5": ("rust", "serde_json5", ["json5"]),
    "rs-jsonc-parser": ("rust", "jsonc-parser", ["jsonc"]),
    "jsonvet-json": ("jsonvet", "jsonvet", ["json"]),
    "jsonvet-json5": ("jsonvet", "jsonvet", ["json5"]),
    "jsonvet-jsonc": ("jsonvet", "jsonvet", ["jsonc"]),
    "jsonvet-jwcc": ("jsonvet", "jsonvet", ["jwcc"]),
    "jsonvet-jsonl": ("jsonvet", "jsonvet", ["jsonl"]),
    "jsonvet-dupes": ("jsonvet", "jsonvet", ["dupes"]),
}


def inputs() -> list[str]:
    """Every input file, as a path relative to the repository root."""
    corpus = [p for p in (ROOT / "tests/corpus").rglob("*") if p.is_file()]
    suite = [
        p
        for p in (ROOT / "tests/json5-tests").rglob("*")
        if p.suffix in {".json", ".json5", ".js", ".txt"} and not p.name.startswith(".")
    ]
    extra = [p for p in (HERE / "extra").rglob("*") if p.is_file() and p.name != "expect.tsv"]
    return [p.relative_to(ROOT).as_posix() for group in (corpus, suite, extra) for p in sorted(group)]


def docker(lang: str, work: Path, name: str, *, src_rw: bool = False, run: bool = False) -> list[str]:
    """The docker run prefix for a tmp- container of one language."""
    spec = LANGS[lang]
    args = ["docker", "run", "--rm", "--name", name, "--cpus", CPUS, "--user", f"{os.getuid()}:{os.getgid()}"]
    args += ["-e", "HOME=/work"] + [a for env in spec.env for a in ("-e", env)]
    args += ["-v", f"{HERE / 'drivers' / lang}:/src:{'rw' if src_rw else 'ro'}"]
    args += ["-v", f"{work / lang}:/work:{'ro' if run else 'rw'}"]
    if run:
        args += ["-i", "--network", "none", "-w", "/in", "--memory", "2g", "--pids-limit", "256"]
        args += ["--cap-drop", "ALL", "--security-opt", "no-new-privileges"]
        args += ["-v", f"{ROOT / 'tests'}:/in/tests:ro", "-v", f"{HERE / 'extra'}:/in/conformance/extra:ro"]
    return [*args, spec.image]


def setup(lang: str, work: Path) -> str:
    """Install or build one language's driver; return the runtime version it prints last."""
    (work / lang).mkdir(parents=True, exist_ok=True)
    cmd = [*docker(lang, work, f"tmp-matrix-{lang}-setup"), "sh", "-c", LANGS[lang].setup]
    out = subprocess.run(cmd, check=True, capture_output=True, text=True).stdout  # noqa: S603
    return out.strip().splitlines()[-1]


def lock(lang: str, work: Path) -> None:
    """Refresh one language's lock file from its pinned manifest."""
    (work / lang).mkdir(parents=True, exist_ok=True)
    cmd = [*docker(lang, work, f"tmp-matrix-{lang}-lock", src_rw=True), "sh", "-c", LANGS[lang].lock]
    subprocess.run(cmd, check=True)  # noqa: S603


def stream(proc: subprocess.Popen[str], lines: queue.Queue[str | None]) -> None:
    """Copy the driver's output lines into a queue, then None at the end."""
    assert proc.stdout is not None  # noqa: S101 - always set with stdout=PIPE
    for line in proc.stdout:
        lines.put(line.rstrip("\n"))
    lines.put(None)


def run_container(parser: str, work: Path, files: list[str]) -> dict[str, str]:
    """Run one parser over the files, restarting its container after a crash or a timeout.

    A file whose line never comes is a timeout after TIMEOUT seconds, and a crash when the
    driver exits first. Either way the next container starts after that file.
    """
    lang = PARSERS[parser][0]
    verdicts: dict[str, str] = {}
    todo = list(files)
    attempt = 0
    while todo:
        attempt += 1
        name = f"tmp-matrix-{parser}-{attempt}"
        cmd = [*docker(lang, work, name, run=True), "sh", "-c", f'exec {LANGS[lang].run} "$0"', parser]
        proc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)  # noqa: S603
        lines: queue.Queue[str | None] = queue.Queue()
        threading.Thread(target=stream, args=(proc, lines), daemon=True).start()
        assert proc.stdin is not None  # noqa: S101 - always set with stdin=PIPE
        proc.stdin.write("".join(f"{path}\n" for path in todo))
        proc.stdin.close()
        try:
            ready = lines.get(timeout=STARTUP)
        except queue.Empty:
            ready = None
        if ready != "ready":
            sys.exit(f"{parser}: the driver did not start")
        done = 0
        for path in todo:
            try:
                line = lines.get(timeout=TIMEOUT)
            except queue.Empty:
                verdicts[path] = "timeout"
                subprocess.run(["docker", "kill", name], check=False, capture_output=True)  # noqa: S603, S607
                break
            if line is None:
                verdicts[path] = "crash"
                break
            got, verdict = line.split("\t")
            if got != path:
                sys.exit(f"{parser}: expected a line for {path}, got {line!r}")
            verdicts[path] = verdict
            done += 1
        proc.wait()
        todo = todo[done + 1 :]
    return verdicts


def run_jsonvet(parser: str, jsonvet: Path, files: list[str]) -> dict[str, str]:
    """Run jsonvet once per file in the parser's dialect."""
    verdicts = {}
    for path in files:
        if parser == "jsonvet-dupes":  # every rule on, in the corpus folder's dialect
            folder = path.split("/")[2] if path.startswith("tests/corpus/") else "json5"
            args = ["--dialect", folder]
        else:
            args = ["--dialect", parser.removeprefix("jsonvet-"), "--ignore", "duplicate-key"]
        try:
            code = subprocess.run(  # noqa: S603 - our own command line
                [jsonvet, "--no-config", *args, path], cwd=ROOT, capture_output=True, timeout=TIMEOUT, check=False
            ).returncode
        except subprocess.TimeoutExpired:
            verdicts[path] = "timeout"
            continue
        verdicts[path] = {0: "accept", 1: "reject"}.get(code, "crash")
    return verdicts


def package_versions() -> dict[str, dict[str, str]]:
    """The pinned version of every package, read from each language's lock file."""
    drivers = HERE / "drivers"
    lock = json.loads((drivers / "node/package-lock.json").read_text())["packages"]
    node = {key.removeprefix("node_modules/"): value["version"] for key, value in lock.items() if key}
    python = dict(
        line.split("==") for line in (drivers / "python/requirements.txt").read_text().splitlines() if "==" in line
    )
    go = dict(re.findall(r"^\s+(\S+) (v\S+)$", (drivers / "go/go.mod").read_text(), re.MULTILINE))
    rust = {p["name"]: p["version"] for p in tomllib.loads((drivers / "rust/Cargo.lock").read_text())["package"]}
    return {"node": node, "python": python, "go": go, "rust": rust}


def check() -> int:
    """Compare the pinned packages and images with results/versions.json; 1 if any differ."""
    recorded = json.loads((RESULTS / "versions.json").read_text())
    packages = package_versions()
    stale = [
        f"{name}: pinned {packages[lang].get(package)}, results from {recorded['parsers'][name]['version']}"
        for name, (lang, package, _) in PARSERS.items()
        if lang in LANGS and package and packages[lang].get(package) != recorded["parsers"][name]["version"]
    ]
    stale += [
        f"{lang} image: pinned {spec.image}, results from {recorded['images'].get(lang)}"
        for lang, spec in LANGS.items()
        if recorded["images"].get(lang) != spec.image
    ]
    for line in stale:
        print(line)
    if stale:
        print("re-run the matrix: uv run conformance/run.py && uv run conformance/report.py")
    return 1 if stale else 0


def jsonvet_version(jsonvet: Path) -> str:
    """The jsonvet version and the commit it was built from."""
    version = subprocess.run([jsonvet, "--version"], capture_output=True, text=True, check=True)  # noqa: S603
    commit = subprocess.run(
        ["git", "rev-parse", "--short", "HEAD"],  # noqa: S607
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    return f"{version.stdout.split()[-1]} ({commit.stdout.strip()})"


def parser_versions(chosen: list[str], old: dict, runtimes: dict[str, str]) -> dict[str, dict[str, str]]:
    """Each parser's package and version; a parser not re-run keeps the entry it was run with."""
    packages = package_versions()
    parsers = {}
    for name, (lang, package, uses) in PARSERS.items():
        if name not in chosen and name in old:
            parsers[name] = old[name]
            continue
        if package is None:
            version = f"built in ({lang} {runtimes.get(lang, '?')})"
        elif lang == "jsonvet":
            version = runtimes["jsonvet"]
        else:
            version = packages[lang].get(package, "?")
        parsers[name] = {"language": lang, "package": package or "", "version": version, "uses": uses}
    return parsers


def main() -> None:
    """Set up the drivers, run the parsers and write the results."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--only", nargs="+", choices=PARSERS, help="run only these parsers")
    parser.add_argument("--work", type=Path, default=HERE / ".work", help="build and cache directory")
    parser.add_argument("--jsonvet", type=Path, default=ROOT / "target/release/jsonvet")
    parser.add_argument("--lock", action="store_true", help="refresh the lock files, then stop")
    parser.add_argument("--check", action="store_true", help="fail if the pins differ from the last run")
    args = parser.parse_args()
    if args.check:
        sys.exit(check())
    work = args.work.resolve()
    if args.lock:
        for lang in LANGS:
            lock(lang, work)
        return

    started = time.monotonic()
    chosen = args.only or list(PARSERS)
    files = inputs()
    versions_file = RESULTS / "versions.json"
    old = json.loads(versions_file.read_text()) if args.only and versions_file.exists() else {}
    runtimes = old.get("runtimes", {})
    for lang in dict.fromkeys(PARSERS[p][0] for p in chosen):
        if lang in LANGS:
            print(f"setting up {lang}", flush=True)
            runtimes[lang] = setup(lang, work)
    if any(PARSERS[p][0] == "jsonvet" for p in chosen):
        runtimes["jsonvet"] = jsonvet_version(args.jsonvet)

    RESULTS.mkdir(exist_ok=True)
    for name in chosen:
        print(f"running {name} on {len(files)} files", flush=True)
        if PARSERS[name][0] == "jsonvet":
            verdicts = run_jsonvet(name, args.jsonvet, files)
        else:
            verdicts = run_container(name, work, files)
        (RESULTS / f"{name}.tsv").write_text("".join(f"{path}\t{verdicts[path]}\n" for path in files))

    parsers = parser_versions(chosen, old.get("parsers", {}), runtimes)
    seconds = round(time.monotonic() - started)
    summary = {
        "date": datetime.datetime.now(datetime.UTC).date().isoformat(),
        "seconds": seconds if not args.only else old.get("seconds"),
        "images": {lang: spec.image for lang, spec in LANGS.items()},
        "runtimes": runtimes,
        "parsers": parsers,
    }
    versions_file.write_text(json.dumps(summary, indent=2) + "\n")
    print(f"done in {seconds} s; now run: uv run conformance/report.py")


if __name__ == "__main__":
    main()
