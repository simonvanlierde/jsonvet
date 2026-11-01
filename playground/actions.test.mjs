import assert from "node:assert/strict";
import { test } from "node:test";
import { fixesFor, fixSummary } from "./actions.mjs";
import { snippets } from "./snippets.mjs";

test("a correction finds its fix at its place or its first copy", () => {
  const fixes = [
    { rule: "duplicate-key", index: 3, safe: false },
    { rule: "syntax", index: 30, safe: true },
  ];
  const dup = { rule: "duplicate-key", index: 12, related: [{ index: 3 }] };
  assert.deepEqual(fixesFor(dup, fixes), [fixes[0]]);
  assert.deepEqual(fixesFor({ rule: "syntax", index: 30, related: [] }, fixes), [fixes[1]]);
  assert.deepEqual(fixesFor({ rule: "syntax", index: 3, related: [] }, fixes), []);
  assert.deepEqual(fixesFor({ rule: "duplicate-key", index: 12, related: [] }, fixes), []);
});

test("a note on any copy of a key applies the fixes for every earlier copy", () => {
  // {"a":1,"a":2,"a":3}: the fixes drop the copies at 1 and 7 and keep the last.
  const fixes = [
    { rule: "duplicate-key", index: 1, safe: false },
    { rule: "duplicate-key", index: 7, safe: false },
  ];
  const problems = [
    { rule: "duplicate-key", index: 7, related: [{ index: 1 }] },
    { rule: "duplicate-key", index: 13, related: [{ index: 1 }] },
  ];
  assert.deepEqual(fixesFor(problems[1], fixes, problems), fixes);
  assert.deepEqual(fixesFor(problems[0], fixes, problems), fixes);
});

test("a trailing comma's fix is the comma before the bracket it is reported at", () => {
  const fixes = [{ rule: "syntax", index: 5, safe: true }, { rule: "syntax", index: 20, safe: true }];
  const p = { rule: "syntax", index: 6, message: "trailing comma before `]` is not allowed in json", related: [] };
  assert.deepEqual(fixesFor(p, fixes), [fixes[0]]);
  assert.deepEqual(fixesFor({ ...p, index: 21 }, fixes), [fixes[1]]);
  assert.deepEqual(fixesFor({ ...p, index: 4 }, fixes), []);
});

test("a summary counts the fixes by kind", () => {
  const f = (rule) => ({ rule, index: 0, safe: true });
  assert.equal(fixSummary([f("syntax")]), "Fixed 1 problem: 1 trailing comma.");
  assert.equal(
    fixSummary([f("syntax"), f("syntax"), f("duplicate-key")]),
    "Fixed 3 problems: 2 trailing commas, 1 duplicate key.",
  );
});

test("a summary names mixed line endings", () => {
  const f = { rule: "mixed-line-endings", index: 4, safe: true };
  assert.equal(fixSummary([f]), "Fixed 1 problem: 1 mixed line ending.");
});

test("the snippets add --fix and the format hook", () => {
  const s = snippets({ version: "3.3.0", glob: "", file: "config.json" });
  assert.match(s.hook, /rev: v3\.3\.0\n {2}hooks:\n {4}- id: jsonvet\n\n/);
  assert.match(s.hook, /- id: jsonvet\n {6}args: \[--fix\]/);
  assert.match(s.hook, /- id: jsonvet-format$/);
  assert.match(s.action, /- uses: simonvanlierde\/jsonvet@v3\n {2}with:\n {4}args: --fix/);
  assert.match(s.action, /- run: uvx jsonvet fmt --check \.$/);
  assert.equal(s.cli, "uvx jsonvet config.json\nuvx jsonvet --fix config.json\nuvx jsonvet fmt config.json");
});

test("the snippets keep the dialect glob", () => {
  const s = snippets({ version: "3.3.0", glob: "jsonc:*.json", file: "a.json" });
  assert.match(s.hook, /args: \[--dialect, "jsonc:\*\.json"\]/);
  assert.match(s.hook, /args: \[--fix, --dialect, "jsonc:\*\.json"\]/);
  assert.match(s.action, /args: --fix --dialect jsonc:\*\.json/);
  assert.equal(s.cli.split("\n")[1], "uvx jsonvet --fix --dialect 'jsonc:*.json' a.json");
});
