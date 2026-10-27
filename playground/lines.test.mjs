import assert from "node:assert/strict";
import { test } from "node:test";
import { rowOf, rowStarts } from "./lines.mjs";

test("rows start after each LF", () => {
  assert.deepEqual(rowStarts("a\nbc\n\nd"), [0, 2, 5, 6]);
  assert.deepEqual(rowStarts(""), [0]);
});

test("an index belongs to the row it is in", () => {
  const starts = rowStarts("a\nbc\n\nd");
  const rows = [0, 1, 2, 3, 4, 5, 6, 7].map((i) => rowOf(starts, i));
  assert.deepEqual(rows, [1, 1, 2, 2, 2, 3, 4, 4]);
});

test("U+2028 and U+2029 do not start a row, as in a textarea", () => {
  const text = "a b c\nd";
  const starts = rowStarts(text);
  assert.equal(starts.length, 2);
  assert.equal(rowOf(starts, text.indexOf("c")), 1);
  assert.equal(rowOf(starts, text.indexOf("d")), 2);
});

test("a problem after a character outside the BMP lands on its own row", () => {
  const text = '{"😀": 1,\n"😀": 2}';
  const starts = rowStarts(text);
  assert.equal(rowOf(starts, text.lastIndexOf('"😀')), 2);
});
