"""Smoke tests against the installed native wheel."""

import pytest
from jsonvet import Diagnostic, check


def test_defaults_to_strict_json() -> None:
    """JSON5 needs an explicit dialect."""
    text = "{ port: 8080 }"
    assert check(text, dialect="json5") == []
    (diagnostic,) = check(text)
    assert diagnostic.rule == "syntax"


def test_positions_use_code_points_and_utf8_bytes() -> None:
    """A non-ASCII key makes the display column differ from the byte offset."""
    text = '{"é": 1, "é": 2}'
    (diagnostic,) = check(text)
    assert isinstance(diagnostic, Diagnostic)
    assert diagnostic.rule == "duplicate-key"
    assert diagnostic.severity == "error"
    assert "duplicate key" in diagnostic.message
    assert (diagnostic.line, diagnostic.column) == (1, 10)
    span = text.encode()[diagnostic.byte_offset : diagnostic.byte_end]
    assert span.decode() == '"é"'
    assert diagnostic.byte_offset == text.encode().rindex(span)


def test_warning_at_eof_has_empty_span() -> None:
    """JSON Lines can report a warning with no bytes left to highlight."""
    (diagnostic,) = check('{"active":true}', dialect="ndjson")
    assert diagnostic.rule == "missing-final-newline"
    assert diagnostic.severity == "warning"
    assert diagnostic.byte_offset == diagnostic.byte_end


def test_unknown_dialect_raises_value_error() -> None:
    """An invalid option is distinct from a finding in the document."""
    with pytest.raises(ValueError, match="yaml"):
        check("{}", dialect="yaml")
