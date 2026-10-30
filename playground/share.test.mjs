import assert from "node:assert/strict";
import { test } from "node:test";
import { decode, encode } from "./share.mjs";

test("a fragment gives back the dialect and the text", async () => {
  const text = '{\n  // café ☕ 😀\n  "a": 1,\n}\n';
  const fragment = await encode("jsonc", text);
  assert.match(fragment, /^jsonc:[A-Za-z0-9_-]+$/);
  assert.deepEqual(await decode(`#${fragment}`), { dialect: "jsonc", text });
});

test("a long text compresses", async () => {
  const text = '{"key": "value"},\n'.repeat(5000);
  const fragment = await encode("json5", text);
  assert.ok(fragment.length < text.length / 20, `${fragment.length} characters`);
  assert.equal((await decode(fragment)).text, text);
});

test("a byte order mark survives, as jsonvet reports it", async () => {
  const text = "\uFEFF{}";
  assert.equal((await decode(await encode("json", text))).text, text);
});

test("an empty text round-trips", async () => {
  assert.deepEqual(await decode(await encode("json", "")), { dialect: "json", text: "" });
});

test("a broken or foreign fragment gives nothing", async () => {
  for (const bad of ["", "#", "#top", "#json5:", "#json5:!!!", "#yaml:AAAA", "#json5:AAAA"]) {
    assert.equal(await decode(bad), null, bad);
  }
});

test("a link that inflates past the limit gives nothing", async () => {
  assert.equal(await decode(await encode("json", "a".repeat(9 << 20))), null);
});
