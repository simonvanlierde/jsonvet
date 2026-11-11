import assert from "node:assert/strict";
import { test } from "node:test";
import { clean, configText, defaults, formatArgs, rowsFor, SETTINGS } from "./settings.mjs";

const form = { ...defaults("format"), normalize: true };

test("defaults come from the table", () => {
  assert.deepEqual(defaults("format"), { indent: "2", width: 80, spacing: true, normalize: false, fill: false });
  assert.deepEqual(defaults("fix"), { unsafe: false });
});

test("every row has a default its own control accepts", () => {
  for (const r of SETTINGS) assert.equal(clean(r, r.default), r.default, r.key);
});

test("dialects filter the rows", () => {
  assert.deepEqual(rowsFor("format", "json").map((r) => r.key), ["indent", "width", "spacing", "fill"]);
  assert.deepEqual(rowsFor("format", "json5").map((r) => r.key), ["indent", "width", "spacing", "normalize", "fill"]);
  assert.deepEqual(rowsFor("fix", "jsonl").map((r) => r.key), ["unsafe"]);
});

test("the controls become format()'s arguments", () => {
  assert.deepEqual(formatArgs(form, "json"), [2, 80, true, false, false]);
  assert.deepEqual(formatArgs({ ...form, indent: "tab", spacing: false }, "json5"), [0, 80, false, true, false]);
  assert.deepEqual(formatArgs({ ...form, indent: "4", width: "100" }, "jsonc"), [4, 100, true, false, false]);
});

test("a bad width is the default", () => {
  for (const width of ["", "0", "-3", "1.5", "abc", "5000", undefined]) {
    assert.equal(formatArgs({ ...form, width }, "json")[1], 80, String(width));
  }
});

test("all defaults give no config", () => {
  assert.equal(configText("format", defaults("format"), "json5"), "");
  assert.equal(configText("fix", defaults("fix"), "json"), "");
  assert.equal(configText("format", { ...defaults("format"), width: "abc" }, "json"), "");
});

test("format settings become a jsonvet.toml table", () => {
  const values = { indent: "4", width: "100", spacing: false, normalize: true, fill: true };
  assert.equal(
    configText("format", values, "json5"),
    "[format]\nindent = 4\nwidth = 100\nbracket-spacing = false\nnormalize-json5 = true\nfill-numbers = true",
  );
  assert.equal(configText("format", { ...defaults("format"), indent: "tab" }, "json"), '[format]\nindent = "tab"');
});

test("a setting that does not apply to the dialect is left out", () => {
  assert.equal(configText("format", { ...defaults("format"), normalize: true }, "jsonc"), "");
});

test("fix settings become CLI flags", () => {
  assert.equal(configText("fix", { unsafe: true }, "json"), "--fix --unsafe-fixes");
});
