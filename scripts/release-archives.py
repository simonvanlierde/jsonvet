# /// script
# requires-python = ">=3.11"
# ///
"""Pack the jsonvet binary from each release wheel into an archive for the GitHub release.

Usage: uv run scripts/release-archives.py WHEEL_DIR OUT_DIR

The wheels already hold the binary (maturin bindings = "bin"), so the archives ship the same bytes
that PyPI does. Each archive is named jsonvet-<Rust target>.tar.gz, or .zip on Windows, and holds
`jsonvet` (or `jsonvet.exe`) and LICENSE at its root. The names are what
`[package.metadata.binstall]` in crates/jsonvet-cli/Cargo.toml expects. It fails unless every
target below gets exactly one archive.
"""

import io
import sys
import tarfile
import zipfile
from pathlib import Path

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


def binary_of(wheel: Path) -> tuple[str, bytes]:
    """The jsonvet binary's file name and bytes, from the wheel's scripts folder."""
    with zipfile.ZipFile(wheel) as whl:
        for name in whl.namelist():
            parts = name.split("/")
            if (
                len(parts) == 3  # noqa: PLR2004
                and parts[0].endswith(".data")
                and parts[1] == "scripts"
                and parts[2] in {"jsonvet", "jsonvet.exe"}
            ):
                return parts[2], whl.read(name)
    msg = f"{wheel.name}: no jsonvet binary in its scripts folder"
    raise ValueError(msg)


def pack(out: Path, target: str, binary: str, data: bytes, license_text: bytes) -> Path:
    """Write the archive for one target and return its path."""
    if target.endswith("windows-msvc"):
        path = out / f"jsonvet-{target}.zip"
        with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as archive:
            archive.writestr(binary, data)
            archive.writestr("LICENSE", license_text)
        return path
    path = out / f"jsonvet-{target}.tar.gz"
    with tarfile.open(path, "w:gz") as archive:
        for name, content, mode in [(binary, data, 0o755), ("LICENSE", license_text, 0o644)]:
            info = tarfile.TarInfo(name)
            info.size = len(content)
            info.mode = mode
            archive.addfile(info, io.BytesIO(content))
    return path


def main() -> int:
    """Exit 1 unless every target got exactly one archive."""
    wheel_dir, out = Path(sys.argv[1]), Path(sys.argv[2])
    out.mkdir(parents=True, exist_ok=True)
    license_text = (Path(__file__).parent.parent / "LICENSE").read_bytes()
    done: dict[str, str] = {}
    for wheel in sorted(wheel_dir.rglob("jsonvet-*.whl")):
        target = target_of(wheel)
        if target in done:
            print(f"{wheel.name} and {done[target]} are both for {target}")
            return 1
        done[target] = wheel.name
        print(pack(out, target, *binary_of(wheel), license_text))
    missing = {target for _, _, target in TARGETS} - done.keys()
    if missing:
        print(f"no wheel for {', '.join(sorted(missing))}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
