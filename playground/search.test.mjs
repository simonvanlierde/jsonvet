import assert from "node:assert/strict";
import { test } from "node:test";
import { prepare, search, tokens } from "./search.mjs";

const sections = prepare([
  [
    "usage.html",
    "jsonvet reference",
    [
      ["rules", "Rules and severities", "duplicate-key trailing-comma error warning"],
      ["no-trailing-comma", "no-trailing-comma", "comma before a closing bracket"],
    ],
  ],
  ["format.html", "Formatting", [["indent", "Indent", "spaces tabs trailing whitespace"]]],
]);
const ids = (q) => search(sections, q).map((s) => s.id);

test("tokens keep a hyphenated rule id whole and drop the dashes around a flag", () => {
  assert.deepEqual(tokens("Use --schema for no-trailing-comma, 'JSON5'"), ["use", "schema", "for", "no-trailing-comma", "json5"]);
});

test("a prefix and a substring match, in any case", () => {
  assert.deepEqual(ids("DUPLI"), ["rules"]);
  assert.deepEqual(ids("ailing-com"), ["no-trailing-comma", "rules"]);
});

test("every word has to match, in any order", () => {
  assert.deepEqual(ids("severities rules"), ["rules"]);
  assert.deepEqual(ids("rules tabs"), []);
});

test("a heading match ranks above a body match, and a whole word above a prefix", () => {
  assert.deepEqual(ids("trailing"), ["no-trailing-comma", "indent", "rules"]);
});

test("the page title counts beside a word of the section, never alone", () => {
  assert.deepEqual(ids("formatting indent"), ["indent"]);
  assert.deepEqual(ids("formatting"), []);
});

test("one letter, blanks and punctuation find nothing", () => {
  for (const q of ["", " ", "a", "--", "zzzz"]) assert.deepEqual(ids(q), [], q);
});

test("the list is capped", () => {
  const many = prepare([["p.html", "P", Array.from({ length: 30 }, (_, i) => [`s${i}`, `Thing ${i}`, ""])]]);
  assert.equal(search(many, "thing").length, 8);
  assert.equal(search(many, "thing", 3).length, 3);
});
