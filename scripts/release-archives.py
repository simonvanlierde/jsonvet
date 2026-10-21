# /// script
# requires-python = ">=3.11"
# ///
"""Pack the binaries from each release wheel into an archive for the GitHub release.

Usage: uv run scripts/release-archives.py WHEEL_DIR OUT_DIR

The wheels already hold the binary (maturin bindings = "bin"), so the archives ship the same bytes
that PyPI does. Each archive is named jsonvet-<Rust target>.tar.gz, or .zip on Windows, and holds
every `[[bin]]` of crates/jsonvet-cli (with `.exe` on Windows) and LICENSE at its root. The names
are what `[package.metadata.binstall]` there expects, and cargo-binstall refuses an archive that
lacks one of the crate's binaries. It fails unless every target below gets exactly one archive
holding all of them.
"""

import io
import json
import subprocess
import sys
import tarfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Wheel platform tag (one part of it) and architecture -> Rust target. Musl first: it is the more
# specific match.
TARGETS = [
    ("musllinux", "x86_64", "x86_64-unknown-linux-musl"),
    ("musllinux", "aarch64", "aarch64-unknown-linux-musl"),
    ("manylinux", "x86_64", "x86_64-unknown-linux-gnu"),
    ("manylinux", "aarch64", "aarch64-unknown-linux-gnu"),
    ("macosx", "x86_64", "x86_64-apple-darwin"),
    ("macosx", "arm64", "aarch64-apple-darwin"),
    ("win", "amd64", "x86_64-pc-windows-msvc"),
    ("win", "arm64", "aarch64-pc-windows-msvc"),
]


def target_of(wheel: Path) -> str:
    """The Rust target a wheel was built for, from the platform tag in its file name."""
    platform = wheel.stem.rsplit("-", 1)[1]
    for os_part, arch, target in TARGETS:
        if platform.startswith(os_part) and platform.endswith(arch):
            return target
    msg = f"{wheel.name}: no Rust target for platform tag {platform}"
    raise ValueError(msg)


def bin_names() -> list[str]:
    """The CLI crate's binaries, which every archive must hold.

    Asked of cargo, since `check-json5` is found in src/bin/ and has no `[[bin]]` entry of its own.
    """
    metadata = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"],  # noqa: S607
            cwd=ROOT,
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    )
    package = next(p for p in metadata["packages"] if p["name"] == "jsonvet-cli")
    return sorted(t["name"] for t in package["targets"] if "bin" in t["kind"])


def binaries_of(wheel: Path, names: list[str]) -> list[tuple[str, bytes]]:
    """Each binary's file name and bytes, from the wheel's scripts folder, in the order of `names`."""
    found: dict[str, tuple[str, bytes]] = {}
    with zipfile.ZipFile(wheel) as whl:
        for entry in whl.namelist():
            parts = entry.split("/")
            if len(parts) == 3 and parts[0].endswith(".data") and parts[1] == "scripts":  # noqa: PLR2004
                found[parts[2].removesuffix(".exe")] = (parts[2], whl.read(entry))
    missing = [name for name in names if name not in found]
    if missing:
        msg = f"{wheel.name}: no {', '.join(missing)} in its scripts folder"
        raise ValueError(msg)
    return [found[name] for name in names]


def pack(out: Path, target: str, binaries: list[tuple[str, bytes]], license_text: bytes) -> Path:
    """Write the archive for one target and return its path."""
    if target.endswith("windows-msvc"):
        path = out / f"jsonvet-{target}.zip"
        with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as archive:
            for name, data in binaries:
                archive.writestr(name, data)
            archive.writestr("LICENSE", license_text)
        return path
    path = out / f"jsonvet-{target}.tar.gz"
    files = [(name, data, 0o755) for name, data in binaries] + [("LICENSE", license_text, 0o644)]
    with tarfile.open(path, "w:gz") as archive:
        for name, content, mode in files:
            info = tarfile.TarInfo(name)
            info.size = len(content)
            info.mode = mode
            archive.addfile(info, io.BytesIO(content))
    return path


def main() -> int:
    """Exit 1 unless every target got exactly one archive."""
    wheel_dir, out = Path(sys.argv[1]), Path(sys.argv[2])
    out.mkdir(parents=True, exist_ok=True)
    license_text = (ROOT / "LICENSE").read_bytes()
    names = bin_names()
    done: dict[str, str] = {}
    for wheel in sorted(wheel_dir.rglob("jsonvet-*.whl")):
        target = target_of(wheel)
        if target in done:
            print(f"{wheel.name} and {done[target]} are both for {target}")
            return 1
        done[target] = wheel.name
        print(pack(out, target, binaries_of(wheel, names), license_text))
    missing = {target for _, _, target in TARGETS} - done.keys()
    if missing:
        print(f"no wheel for {', '.join(sorted(missing))}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
