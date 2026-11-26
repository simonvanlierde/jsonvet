# /// script
# requires-python = ">=3.11"
# ///
"""Pack each release wheel's binary into an archive for the GitHub release.

Usage: uv run scripts/release-archives.py WHEEL_DIR OUT_DIR

The wheels already hold the binary (maturin bindings = "bin"), so the archives ship the same bytes
that PyPI does. Each archive is named jsonvet-<Rust target>.tar.gz, or .zip on Windows, and holds
the binary and both license files at its root. The archive and binary names match
`[package.metadata.binstall]` in crates/jsonvet-cli/Cargo.toml.
"""

import io
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


def binary_of(wheel: Path, name: str) -> bytes:
    """Read the binary from the wheel's scripts folder."""
    with zipfile.ZipFile(wheel) as whl:
        for entry in whl.namelist():
            root, _, path = entry.partition("/")
            if root.endswith(".data") and path == f"scripts/{name}":
                return whl.read(entry)
    msg = f"{wheel.name}: no {name} in its scripts folder"
    raise ValueError(msg)


def pack(out: Path, target: str, wheel: Path, licenses: dict[str, bytes]) -> Path:
    """Write the archive for one target and return its path."""
    windows = target.endswith("windows-msvc")
    name = "jsonvet.exe" if windows else "jsonvet"
    binary = binary_of(wheel, name)
    if windows:
        path = out / f"jsonvet-{target}.zip"
        with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as archive:
            archive.writestr(name, binary)
            for name, data in licenses.items():
                archive.writestr(name, data)
        return path
    path = out / f"jsonvet-{target}.tar.gz"
    files = [(name, binary, 0o755), *((n, d, 0o644) for n, d in licenses.items())]
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
    licenses = {n: (ROOT / n).read_bytes() for n in ("LICENSE-MIT", "LICENSE-APACHE")}
    done: dict[str, str] = {}
    for wheel in sorted(wheel_dir.rglob("jsonvet-*.whl")):
        target = target_of(wheel)
        if target in done:
            print(f"{wheel.name} and {done[target]} are both for {target}")
            return 1
        done[target] = wheel.name
        print(pack(out, target, wheel, licenses))
    missing = {target for _, _, target in TARGETS} - done.keys()
    if missing:
        print(f"no wheel for {', '.join(sorted(missing))}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
