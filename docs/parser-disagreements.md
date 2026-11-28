# Where JSON parsers disagree

The same JSONC file can parse in one tool and fail in another. In the
[conformance report](conformance.md), a single carriage return ends a line comment
for some readers. For others the comment swallows the next key and the closing
brace, so the remaining text fails to parse.

The matrix runs the same files through parsers for JSON, JSON5, JSONC, JWCC, and
JSON Lines. Most runs use the parser's default options. The
[test harness notes](../conformance/README.md#what-the-verdicts-mean) explain the
exceptions, including how the runs count duplicate keys.

## A carriage return can hide a JSONC key

In [the test file](../conformance/extra/jsonc/lone-cr-ends-comment.jsonc), the
comment ends at a lone carriage return (CR). The
[JSONC draft specification](https://jsonc.org/) recognizes CR, LF and CRLF as line
endings. A comment remover that stops only at LF can swallow the key after CR.

The Go `tidwall/jsonc` reader fixed this in v0.3.4. The matrix now shows it
accepting the fixture that v0.3.3 rejected. The pinned Node
`strip-json-comments` version still disagrees. Use LF line endings when
these files have to pass through both tools.
[Go report](https://github.com/tidwall/jsonc/issues/6)
· [Node report](https://github.com/sindresorhus/strip-json-comments/issues/66)

## JSON5 escapes still have to make valid keys

JSON5 allows unquoted keys that follow the ECMAScript 5.1 identifier rules.
Writing the first character as a Unicode escape does not change those rules.
For example, `{\u0301: 1}` starts a key with a combining accent, which is
allowed after an identifier's first character but cannot start one. Quote the
key if you intend it to contain that character.

The pinned Python `json5` and `pyjson5` versions accept this input. The
`json5` maintainer confirmed a fix on their main branch. The two Python
packages have separate implementations.
[JSON5 specification](https://spec.json5.org/#objects)
· [json5 report](https://github.com/dpranke/pyjson5/issues/161)
· [pyjson5 report](https://github.com/Kijewski/pyjson5/issues/165)

A different case is an unterminated block comment after a complete value.
The pinned Go `titanous/json5` accepts `{} /*` even though JSON5 requires a
closing `*/`. Keep the closing delimiter when emitting JSON5.
[Go JSON5 report](https://github.com/titanous/json5/issues/10)

## A final JWCC comment can need a newline in practice

The [JWCC specification](https://nigeltao.github.io/blog/2021/json-with-commas-comments.html)
allows `{} // end` with no newline at the end of the file. The pinned
`hujson` reader rejects it. Add a final newline if your consumer uses that
reader. [HuJSON report](https://github.com/tailscale/hujson/issues/56)

The [interactive matrix](https://simonvanlierde.com/jsonvet/docs/matrix.html)
shows the exact bytes, versions and verdicts. When a file crosses tools,
check it with the reader that will consume it, using that reader's settings.
