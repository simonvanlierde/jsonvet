import assert from "node:assert/strict";
import { test } from "node:test";
import { formatMs, lineCount, stack, tokenEnd } from "./proof.mjs";

test("a mark covers a quoted string, a word or one character", () => {
  const row = '    "port": 8443, \'a\\\'b\' x';
  assert.equal(tokenEnd(row, 4), 10); // "port"
  assert.equal(tokenEnd(row, 12), 16); // 8443
  assert.equal(tokenEnd(row, 16), 17); // ,
  assert.equal(tokenEnd(row, 18), 24); // 'a\'b', past the escaped quote
  assert.equal(tokenEnd('  "open', 2), 7); // an unclosed string runs to the end of the row
  assert.equal(tokenEnd("a😀b", 1), 3); // one code point, two UTF-16 units
  assert.equal(tokenEnd("ab", 2), 2); // the end of the row
});

test("notes keep their line, or stack under the one above", () => {
  assert.deepEqual(stack([0, 100, 200], [20, 20, 20], 4), [0, 100, 200]);
  assert.deepEqual(stack([0, 10, 12], [40, 20, 20], 4), [0, 44, 68]);
  assert.deepEqual(stack([], [], 4), []);
});

test("a time below the timer's resolution is not given as a number", () => {
  assert.equal(formatMs(0, 0.1), "under 0.1 ms");
  assert.equal(formatMs(0.05, 0.1), "under 0.1 ms");
  assert.equal(formatMs(0.1, 0.1), "0.1 ms");
  assert.equal(formatMs(0.34, 0.1), "0.34 ms");
  assert.equal(formatMs(0.012, 0.005), "0.012 ms");
  assert.equal(formatMs(9.24, 0.1), "9.2 ms");
  assert.equal(formatMs(11.8, 0.1), "12 ms");
  assert.equal(formatMs(1234.5, 1), "1,235 ms");
  assert.equal(formatMs(0.4, 1), "under 1 ms");
});

test("lines are counted as the CLI counts them", () => {
  assert.equal(lineCount([0]), 1);
  assert.equal(lineCount([0, 5], 5), 1); // "{...}\n"
  assert.equal(lineCount([0, 5], 6), 2);
});
