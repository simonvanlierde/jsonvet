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

test("notes keep their line, or move apart around their lines within the margin", () => {
  assert.deepEqual(stack([0, 100, 200], [20, 20, 20], 4), [0, 100, 200]);
  // Two notes that want the same line share the move: one goes up, one down.
  assert.deepEqual(stack([100, 100], [20, 20], 4), [88, 112]);
  // At the top of the margin they can only go down.
  assert.deepEqual(stack([0, 10, 12], [40, 20, 20], 4, 0), [0, 44, 68]);
  // At the bottom they move up into the free space instead of running past it.
  assert.deepEqual(stack([0, 100], [20, 20], 4, 0, 110), [0, 90]);
  // A note clear of the crowd below keeps its line.
  assert.deepEqual(stack([0, 200, 200], [20, 20, 20], 4, 0, 300), [0, 188, 212]);
  // When they cannot fit, the floor wins and they run past the bottom.
  assert.deepEqual(stack([0, 0], [20, 20], 4, 0, 30), [0, 24]);
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
