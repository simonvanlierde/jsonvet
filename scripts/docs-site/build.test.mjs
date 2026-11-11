import assert from "node:assert/strict";
import { existsSync, mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { posix, resolve } from "node:path";
import { test } from "node:test";
import { anchorsOf, build, REPO, ROOT, slugger } from "./build.mjs";

// The site's root, as deployed: playground/ with the pages in docs/.
const site = mkdtempSync(resolve(tmpdir(), "docs-site-"));
const pages = build(resolve(site, "docs"), "abc123");
const html = Object.fromEntries(pages.map((p) => [`docs/${p}`, readFileSync(resolve(site, "docs", p), "utf8")]));
const ids = (text) => new Set([...text.matchAll(/\sid="([^"]+)"/g)].map((m) => m[1]));
const fileOf = (path) => (path.startsWith("docs/") ? resolve(site, path) : resolve(ROOT, "playground", path));

test("anchors match GitHub's", () => {
  const slug = slugger();
  assert.equal(slug("Rules and severities"), "rules-and-severities");
  assert.equal(slug("Known differences from 1.4"), "known-differences-from-14");
  assert.equal(slug("--schema"), "--schema");
  assert.equal(slug("normalize-json5"), "normalize-json5");
  assert.equal(slug("JSON"), "json");
  assert.equal(slug("JSON"), "json-1");
  assert.equal(slug("json-1"), "json-1-1");
});

test("every link between the pages, and every anchor, resolves", () => {
  const broken = [];
  for (const [path, text] of Object.entries(html)) {
    for (const [, href] of text.matchAll(/\s(?:href|src)="([^"]+)"/g)) {
      if (/^[a-z]+:/i.test(href)) continue;
      const [ref, anchor] = href.replace(/\?v=abc123$/, "").split("#");
      let target = ref ? posix.normalize(posix.join(posix.dirname(path), ref)) : path;
      if (ref.endsWith("/") || ref === ".") target = posix.join(target, "index.html");
      if (!existsSync(fileOf(target))) broken.push(`${path}: ${href}`);
      else if (anchor && !ids(html[target] ?? "").has(anchor)) broken.push(`${path}: ${href} (no such anchor)`);
    }
  }
  assert.deepEqual(broken, []);
});

test("links to GitHub name files and anchors that exist", () => {
  const broken = [];
  const prefix = new RegExp(`^${REPO}/(?:blob|tree)/main/`);
  for (const [path, text] of Object.entries(html)) {
    for (const [, href] of text.matchAll(/\shref="([^"]+)"/g)) {
      if (!prefix.test(href)) continue;
      const [file, anchor] = href.replace(prefix, "").split("#");
      if (!existsSync(resolve(ROOT, file))) broken.push(`${path}: ${href}`);
      else if (anchor && file.endsWith(".md") && !anchorsOf(file).has(anchor)) broken.push(`${path}: ${href}`);
    }
  }
  assert.deepEqual(broken, []);
});

test("no link is left pointing at a Markdown file of the site", () => {
  for (const [path, text] of Object.entries(html)) {
    assert.doesNotMatch(text, /href="(?!https?:)[^"]*\.md(#[^"]*)?"/, path);
  }
});

test("each page keeps the CSP, a skip link and its landmarks, with no inline script or style", () => {
  for (const [path, text] of Object.entries(html)) {
    assert.match(
      text,
      /<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'self';/,
      path,
    );
    assert.doesNotMatch(text, /'unsafe-inline'|<script(?![^>]*\ssrc=)[^>]*>|<style|\sstyle=/, path);
    assert.match(text, /<a class="skip" href="#content">/, path);
    for (const tag of ["<header", "<nav", '<main id="content"', "<footer"])
      assert.ok(text.includes(tag), `${path} ${tag}`);
    assert.equal(text.match(/<h1[ >]/g)?.length, 1, `${path}: one h1`);
  }
});

test("tables and code scroll in a focusable box", () => {
  const page = html["docs/conformance.html"];
  assert.equal(page.match(/<table>/g).length, page.match(/<div class="scroll" tabindex="0"><table>/g).length);
  assert.match(html["docs/format.html"], /<pre tabindex="0"><code/);
});
