import assert from "node:assert/strict";
import { test } from "node:test";
import { indexOf, lineEnd } from "./lines.mjs";

test("a column counts a character outside the BMP once", () => {
  const text = '{"😀": 1,}';
  assert.equal(text[indexOf(text, 1, 9, "json")], "}");
});

test("CRLF and a lone CR each end one line", () => {
  const text = "a\r\nb\rcX";
  assert.equal(text[indexOf(text, 3, 2, "json")], "X");
});

test("U+2028 ends a line in JSON5 only", () => {
  const text = "a\u2028bX";
  assert.equal(text[indexOf(text, 2, 2, "json5")], "X");
  assert.equal(text[indexOf(text, 1, 4, "jsonc")], "X");
  assert.equal(text.split(lineEnd("json5")).length, 2);
  assert.equal(text.split(lineEnd("jsonc")).length, 1);
});

test("a column past the line's end stops at the text's end", () => {
  assert.equal(indexOf("ab", 1, 9, "json"), 2);
});
