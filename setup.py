"""Placeholder that pre-commit installs for the hook: it pins the prebuilt jsonvet wheel.

pre-commit runs `pip install .` on a hook repository. Without this file that would build the
Rust project from source. A release commit sets the pinned version by hand, and the release
workflow checks that it equals the tag.
"""

# pip's isolated build provides setuptools when it installs the hook; the dev environment has none.
from setuptools import setup  # ty: ignore[unresolved-import]

# No packages: without these, setuptools looks for them in every top-level folder and stops,
# because `crates/`, `compat/` and `fuzz/` all look like packages.
setup(
    name="pre_commit_placeholder_package",
    version="0.0.0",
    install_requires=["jsonvet==3.2.0"],
    packages=[],
    py_modules=[],
)
