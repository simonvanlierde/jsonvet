import assert from "node:assert/strict";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { posix, resolve } from "node:path";
import { after, test } from "node:test";
import { anchorsOf, build, REPO, ROOT, slugger } from "./build.mjs";

// The site's root, as deployed: playground/ with the pages in docs/.
const site = mkdtempSync(resolve(tmpdir(), "docs-site-"));
after(() => rmSync(site, { recursive: true, force: true }));
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

test("tables and code scroll in a focusable, named box", () => {
  const page = html["docs/conformance.html"];
  const table = '<div class="scroll" tabindex="0" role="region" aria-label="Table[^"]*"><table>';
  assert.equal(page.match(/<table>/g).length, page.match(new RegExp(table, "g")).length);
  assert.match(html["docs/format.html"], /<pre tabindex="0" role="region" aria-label="Code(: [^"]*)?"><code/);
});

test("region labels are unique on each page", () => {
  for (const [path, text] of Object.entries(html)) {
    const labels = [...text.matchAll(/role="region" aria-label="([^"]+)"/g)].map((m) => m[1]);
    assert.deepEqual(
      labels.filter((l, i) => labels.indexOf(l) !== i),
      [],
      path,
    );
  }
  assert.match(html["docs/conformance.html"], /aria-label="Table: /);
});

test("a long page lists its sections in the nav, and they exist", () => {
  const page = html["docs/usage.html"];
  const nav = page.match(/<a href="usage.html" aria-current="page">Reference<\/a><ul>(.*?)<\/ul>/)[1];
  const links = [...nav.matchAll(/<a href="#([^"]+)">/g)].map((m) => m[1]);
  const h2s = [...page.matchAll(/<h2 id="([^"]+)"/g)].map((m) => m[1]);
  assert.ok(links.length >= 3);
  assert.deepEqual(links, h2s);
  for (const id of links) assert.ok(ids(page).has(id), id);
  assert.doesNotMatch(html["docs/index.html"], /aria-current="page">[^<]*<\/a><ul>/);
});

test("ids are unique on each page, the template's among them", () => {
  for (const [path, text] of Object.entries(html)) {
    const all = [...text.matchAll(/\sid="([^"]+)"/g)].map((m) => m[1]);
    assert.deepEqual(
      all.filter((id, i) => all.indexOf(id) !== i),
      [],
      path,
    );
  }
});

test("a decision record's description is not its status line", () => {
  const description = (page) => html[page].match(/<meta name="description" content="([^"]*)"/)[1];
  for (const [path] of Object.entries(html).filter(([p]) => p.startsWith("docs/decisions/")))
    assert.doesNotMatch(description(path), /^Status:/, path);
  assert.match(description("docs/decisions/0002-no-unsafe-code.html"), /\s.*\s/);
});

test("no description ends in a colon", () => {
  for (const path of Object.keys(html))
    assert.doesNotMatch(html[path].match(/<meta name="description" content="([^"]*)"/)[1], /:$/, path);
});

test("the playground's links into the pages resolve", () => {
  const broken = [];
  const links = [...readFileSync(resolve(ROOT, "playground/index.html"), "utf8").matchAll(/href="(docs\/[^"]*)"/g)].map(
    (m) => m[1],
  );
  // main.mjs links each problem's rule to docs/usage.html#<rule>. The ids are the rules' in
  // rule.rs, and the CLI's own two.
  const rules = readFileSync(resolve(ROOT, "crates/jsonvet/src/rule.rs"), "utf8").matchAll(/Rule::\w+ => "([a-z-]+)"/g);
  for (const id of [...new Set([...rules].map((m) => m[1])), "unreadable", "schema"])
    links.push(`docs/usage.html#${id}`);
  assert.ok(links.length > 10);
  for (const href of links) {
    const [ref, anchor] = href.split("#");
    const page = html[ref.endsWith("/") ? `${ref}index.html` : ref];
    if (!page) broken.push(href);
    else if (anchor && !ids(page).has(anchor)) broken.push(`${href} (no such anchor)`);
  }
  assert.deepEqual(broken, []);
});
