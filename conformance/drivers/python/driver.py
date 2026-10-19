"""Conformance driver for the Python parsers. Usage: python driver.py <parser> < paths.

Prints "ready", then "<path> TAB <accept|reject|crash>" per path. See conformance/README.md.
"""

import json
import sys
from collections.abc import Callable
from pathlib import Path

import jiter
import json5
import jsonlines
import pyjson5


class DuplicateKeyError(ValueError):
    """Raised by the object hook that turns duplicate detection on for the json module."""


def no_duplicates(pairs: list[tuple[str, object]]) -> dict[str, object]:
    """Build an object, refusing a key seen twice."""
    if len({key for key, _ in pairs}) != len(pairs):
        raise DuplicateKeyError
    return dict(pairs)


def read_jsonlines(path: str) -> None:
    """Read every line with the library's own file reader."""
    with jsonlines.open(path) as reader:
        for _ in reader:
            pass


def text(data: bytes) -> str:
    """Decode as strict UTF-8 for the parsers that take text; verdict() counts a failure as a reject."""
    return data.decode("utf-8")


# Each parser takes a path and the file's bytes, and raises one of its error types to reject.
PARSERS: dict[str, tuple[Callable[[str, bytes], object], tuple[type[Exception], ...]]] = {
    "py-json": (lambda _, data: json.loads(text(data)), (json.JSONDecodeError,)),
    "py-json5": (lambda _, data: json5.loads(text(data)), (ValueError,)),
    "py-pyjson5": (lambda _, data: pyjson5.loads(text(data)), (pyjson5.Json5DecoderException,)),
    "py-jsonlines": (lambda path, _: read_jsonlines(path), (jsonlines.InvalidLineError,)),
    "py-jiter": (lambda _, data: jiter.from_json(data), (ValueError,)),
    # Variants with duplicate detection on, for the duplicate-key table only.
    "py-json-dupes": (
        lambda _, data: json.loads(text(data), object_pairs_hook=no_duplicates),
        (json.JSONDecodeError, DuplicateKeyError),
    ),
    "py-json5-dupes": (lambda _, data: json5.loads(text(data), allow_duplicate_keys=False), (ValueError,)),
    "py-jiter-dupes": (lambda _, data: jiter.from_json(data, catch_duplicate_keys=True), (ValueError,)),
}


def verdict(path: str, parse: Callable[[str, bytes], object], errors: tuple[type[Exception], ...]) -> str:
    """Parse one file and name the outcome."""
    try:
        parse(path, Path(path).read_bytes())
    except UnicodeDecodeError:
        return "reject"  # not UTF-8, so a text parser never sees it
    except errors:
        return "reject"
    except Exception:  # noqa: BLE001 - any other exception type is a crash by definition
        return "crash"
    return "accept"


def main() -> None:
    """Read paths from stdin and print one verdict per path."""
    parse, errors = PARSERS[sys.argv[1]]
    print("ready", flush=True)
    for line in sys.stdin:
        path = line.rstrip("\n")
        if path:
            print(f"{path}\t{verdict(path, parse, errors)}", flush=True)


if __name__ == "__main__":
    main()
