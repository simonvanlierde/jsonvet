import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { test } from "node:test";

test("og:image carries the hash of og.png, so link previews refresh when the card changes", () => {
  const hash = createHash("sha256").update(readFileSync(new URL("./og.png", import.meta.url))).digest("hex").slice(0, 8);
  const html = readFileSync(new URL("./index.html", import.meta.url), "utf8");
  assert.match(html, new RegExp(`property="og:image" content="[^"]*/og\\.png\\?v=${hash}"`));
});
