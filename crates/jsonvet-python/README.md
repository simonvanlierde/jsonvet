# jsonvet in Python

Use `jsonvet.check` to lint a string in a notebook, a test or a program that checks many small
documents. Each call uses the Rust parser directly.

```python
from jsonvet import check

text = "{ port: 80, port: 8080 }"
for problem in check(text, dialect="json5"):
    print(problem.line, problem.column, problem.severity, problem.rule, problem.message)
```

`check(text, *, dialect="json")` takes a Python string that can be encoded as UTF-8. It returns a
list of read-only `Diagnostic` objects in document order. A string with a lone surrogate raises
`UnicodeEncodeError`. An empty list means the check found no problems. The list
can contain warnings. Check for `problem.severity == "error"` to decide if the input fails.
Checking stops at the first syntax error and keeps earlier findings.

Use `json`, `i-json`, `jsonc`, `jwcc`, `json5` or `jsonl`. The CLI's dialect aliases also work.
An unknown name raises `ValueError`. Each diagnostic has a `severity`, `rule` and `message`.
`line` and `column` start at 1, and the column counts Unicode code points. The half-open span
`[byte_offset, byte_end)` indexes `text.encode("utf-8")` from 0. A point at the end of the input
can have an empty span.

The Python API checks text with the dialect's default rules, including inline suppressions in
dialects with comments. It reads no files or project settings. Use the CLI for file discovery,
JSON Schema checks, fixes or formatting.

## Build and test

The `jsonvet-python` distribution exposes the `jsonvet` Python module on CPython 3.11 or later.
Build a wheel from this directory with Rust 1.88 or later and uv.

```sh
uv run --no-project --with maturin==1.15.0 maturin build --release --locked --out dist
uv run --no-project --with . --with pytest pytest tests
```

The wheel and the `jsonvet` command can be installed in the same environment. The command's
distribution still provides the CLI and pre-commit hook. The bindings have their own Cargo
workspace so building and releasing the CLI does not pull in PyO3 or a Python interpreter.

When the Rust library version changes, run `cargo update --workspace` here to refresh its
entry in this lockfile. Run `cargo deny check` here to check the Python package dependencies.
