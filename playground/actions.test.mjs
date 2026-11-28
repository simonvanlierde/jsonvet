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

test("the snippets give a complete hook config and a check-only workflow with checkout", () => {
  const s = snippets({ version: "4.0.0", dialect: "json5", file: "config.json", text: "{}" });
  assert.match(s.hook, /^repos:\n {2}- repo: .*\n {4}rev: v4\.0\.0\n {4}hooks:\n {6}- id: jsonvet$/);
  assert.match(s.action, /steps:\n {6}- uses: actions\/checkout@[0-9a-f]{40}(?:[^\n]*\n){3} {6}- uses: simonvanlierde\/jsonvet@v4$/);
  assert.doesNotMatch(s.action, /--fix|jsonvet fmt/);
  assert.equal(s.cli, "uvx jsonvet -- 'config.json'");
  assert.equal(s["cli-fix"], "uvx jsonvet --fix -- 'config.json'");
  assert.equal(s["cli-format"], "uvx jsonvet fmt -- 'config.json'");
});

test("the workflow and hook limit overrides to the filename class", () => {
  const s = snippets({ version: "4.0.0", dialect: "jsonc", file: "a.json", text: "{}" });
  assert.match(s.hook, /args: \[--dialect, 'jsonc:\*\.json'\]/);
  assert.match(s.action, /args: --dialect jsonc:\*\.json/);
  assert.equal(s["cli-fix"], "uvx jsonvet --fix --dialect jsonc -- 'a.json'");
});

test("the CLI keeps the dropped name and quotes it for the shell", () => {
  const settings = { version: "4.0.0", text: "{}" };
  const s = snippets({ ...settings, dialect: "jsonl", file: "-today's data.ndjson" });
  assert.equal(s.cli, "uvx jsonvet -- '-today'\\''s data.ndjson'");
  assert.equal(snippets({ ...settings, dialect: "jsonc", file: "data.jsonc" }).cli, "uvx jsonvet -- 'data.jsonc'");
  assert.equal(snippets({ ...settings, dialect: "jsonc", file: ".jsonc" }).cli, "uvx jsonvet -- '.jsonc'");
  assert.equal(snippets({ ...settings, dialect: "json", file: "-" }).cli, "uvx jsonvet --dialect json -- './-'");
  assert.equal(snippets({ ...settings, dialect: "json5", file: "data.ndjson" }).cli, "uvx jsonvet --dialect json5 -- 'data.ndjson'");
  assert.equal(snippets({ ...settings, dialect: "jsonl", file: "data.ndjson" }).tracked, true);
  assert.equal(snippets({ ...settings, dialect: "json", file: "-" }).tracked, false);
});

test("the CLI override accounts for the JSONC modeline", () => {
  const s = snippets({ version: "4.0.0", dialect: "json5", file: "data.json", text: "\uFEFF // -*- mode: jsonc -*-\n{}" });
  assert.equal(s.cli, "uvx jsonvet --dialect json5 -- 'data.json'");
});
