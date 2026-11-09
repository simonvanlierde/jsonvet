import assert from "node:assert/strict";
import { test } from "node:test";
import { withoutBuiltIn } from "./diagnostics.ts";

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
