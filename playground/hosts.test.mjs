import assert from "node:assert/strict";
import { test } from "node:test";
import { hosts } from "./hosts.mjs";

test("the generated file is about 2.7 MB of JSON, the same every time", () => {
  const text = hosts();
  assert.ok(text.length > 2.6e6 && text.length < 2.8e6, `${text.length} characters`);
  assert.equal(text, hosts());
  assert.match(text, /^[\x0a\x20-\x7e]*$/); // ASCII, so characters are bytes
  assert.doesNotThrow(() => JSON.parse(text));
});

test("it has one duplicate key and only made-up places", () => {
  const dupes = text().split("\n").filter((row) => (row.match(/"port":/g) ?? []).length > 1);
  assert.equal(dupes.length, 1);
  for (const ip of text().match(/"ip": "[^"]*"/g)) {
    assert.match(ip, /"(192\.0\.2|198\.51\.100|203\.0\.113)\.\d+"/);
  }
  for (const host of text().match(/[a-z0-9.-]+\.(com|net|org)\b/g)) assert.match(host, /(^|\.)example\.com$/);
});

const text = () => hosts();
