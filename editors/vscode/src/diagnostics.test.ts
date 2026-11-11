import assert from "node:assert/strict";
import { test } from "node:test";
import { afterAnswer, languageIdOf, shouldAskAboutBuiltIn, withoutBuiltIn } from "./diagnostics.ts";

const linked = (value: string) => ({ code: { value } });
const all = [linked("syntax"), linked("duplicate-key"), { code: "duplicate-key" }, linked("schema"), {}];

test("drops what VS Code's validation reports in JSON and JSONC while it is on", () => {
  for (const languageId of ["json", "jsonc"]) {
    assert.deepEqual(withoutBuiltIn(languageId, true, all), [linked("schema"), {}]);
  }
});

test("keeps everything while VS Code's validation is off", () => {
  assert.deepEqual(withoutBuiltIn("json", false, all), all);
});

test("keeps everything in languages VS Code does not validate", () => {
  for (const languageId of ["json5", "jsonl"]) {
    assert.deepEqual(withoutBuiltIn(languageId, true, all), all);
  }
});

test("finds the open document's language by the uri's string, not its identity", () => {
  const documents = [
    { uri: { toString: () => "file:///a.json" }, languageId: "json" },
    { uri: { toString: () => "file:///b.jsonc" }, languageId: "jsonc" },
  ];
  assert.equal(languageIdOf(documents, { toString: () => "file:///b.jsonc" }), "jsonc");
});

test("falls back to an empty language when the document is not open", () => {
  assert.equal(languageIdOf([], { toString: () => "file:///a.json" }), "");
});

test("asks only with a folder open, validation on and no earlier answer", () => {
  assert.equal(shouldAskAboutBuiltIn(true, true, false), true);
  assert.equal(shouldAskAboutBuiltIn(false, true, false), false);
  assert.equal(shouldAskAboutBuiltIn(true, false, false), false);
  assert.equal(shouldAskAboutBuiltIn(true, true, true), false);
});

test("a closed message remembers nothing and changes nothing", () => {
  assert.deepEqual(afterAnswer(undefined, "Turn it off"), { remember: false, disable: false });
});

test("keeping it is remembered without turning validation off", () => {
  assert.deepEqual(afterAnswer("Keep it", "Turn it off"), { remember: true, disable: false });
});

test("turning it off is remembered and turns validation off", () => {
  assert.deepEqual(afterAnswer("Turn it off", "Turn it off"), { remember: true, disable: true });
});
