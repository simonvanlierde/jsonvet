# /// script
# requires-python = ">=3.11"
# ///
"""Check that every binary in the given wheels carries its dependency list.

Usage: python3 scripts/check-auditable.py WHEEL...

`cargo auditable` puts the list in a `.dep-v0` section of the binary, which `cargo audit bin` and
other scanners read. The wheel build routes cargo through scripts/cargo-auditable, and a broken
route still builds a working wheel, so this fails the build instead. The name is in the section
table of an ELF file and of a Mach-O one alike.
"""

import sys
import zipfile
from pathlib import Path

SECTION = b".dep-v0"


def main() -> int:
    """Exit 1 when a wheel has no binary, or one without the section."""
    status = 0
    for wheel in map(Path, sys.argv[1:]):
        with zipfile.ZipFile(wheel) as archive:
            binaries = [name for name in archive.namelist() if ".data/scripts/" in name]
            if not binaries:
                print(f"{wheel.name}: no binaries", file=sys.stderr)
                status = 1
            for name in binaries:
                if SECTION not in archive.read(name):
                    print(f"{wheel.name}: {name} has no {SECTION.decode()} section", file=sys.stderr)
                    status = 1
    return status


if __name__ == "__main__":
    sys.exit(main())
