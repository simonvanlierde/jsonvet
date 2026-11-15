import assert from "node:assert/strict";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { posix, resolve } from "node:path";
import { after, test } from "node:test";
import { prepare, search } from "../../playground/search.mjs";
import { decode } from "../../playground/share.mjs";
import { anchorsOf, build, REPO, ROOT, slugger } from "./build.mjs";
import { shown } from "./matrix.mjs";

// The site's root, as deployed: playground/ with the pages in docs/.
const site = mkdtempSync(resolve(tmpdir(), "docs-site-"));
after(() => rmSync(site, { recursive: true, force: true }));
const pages = build(resolve(site, "docs"), "abc123");
const html = Object.fromEntries(pages.map((p) => [`docs/${p}`, readFileSync(resolve(site, "docs", p), "utf8")]));
// Cached: the matrix page alone has thousands of links into its own ids.
const idCache = new Map();
const ids = (text) => {
  if (!idCache.has(text)) idCache.set(text, new Set([...text.matchAll(/\sid="([^"]+)"/g)].map((m) => m[1])));
  return idCache.get(text);
};
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
      // A playground share link (`#dialect:data`) is read by the page's script, not an id.
      const shared = target === "index.html" && /^[a-z0-9-]+:/.test(anchor);
      if (!existsSync(fileOf(target))) broken.push(`${path}: ${href}`);
      else if (anchor && !shared && !ids(html[target] ?? "").has(anchor))
        broken.push(`${path}: ${href} (no such anchor)`);
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

test("search: the index covers every page and heading, each page loads the script, and nothing is inline", async () => {
  const text = readFileSync(resolve(site, "docs/search-index.js"), "utf8");
  assert.match(text, /^export default \[/);
  const index = (await import(`data:text/javascript,${encodeURIComponent(text)}`)).default;
  const byUrl = new Map(index.map(([url, , sections]) => [url, sections]));
  for (const [path, page] of Object.entries(html)) {
    const url = path.slice("docs/".length).replace(/(^|\/)index\.html$/, "$1");
    // The matrix page's own headings have no ids, so only the ones with an id can be a target.
    const headings = [...page.matchAll(/<h[1-4] id="([^"]+)"/g)].map((m) => m[1]);
    if (url === "") {
      assert.ok(!byUrl.has(url), "the contents page is not indexed");
      continue;
    }
    assert.deepEqual(
      (byUrl.get(url) ?? []).map(([id]) => id),
      headings,
      path,
    );
    assert.match(page, /<script type="module" src="(\.\.\/)*search\.mjs\?v=abc123"><\/script>/, path);
    assert.match(page, /<search id="docs-search" class="find" hidden>/, path);
  }
  for (const [, , sections] of index)
    for (const [id, heading, words] of sections) {
      assert.ok(heading && id);
      assert.doesNotMatch(words, /[<>&;]|\s\s|^\s|\s$/, id);
      assert.equal(new Set(words.split(" ")).size, words ? words.split(" ").length : 1, `${id}: a word twice`);
    }
  // Body text is found through the index, as a reader would.
  const found = search(prepare(index), "duplicate-key");
  assert.deepEqual([found[0].url, found[0].id], ["usage.html", "duplicate-key"]);
  // The index is a file of the site's own, so the CSP's `script-src 'self'` covers it with no
  // connect-src, and the search script stays the only one besides theme.js.
  for (const page of Object.values(html)) {
    const scripts = [...page.matchAll(/<script[^>]*\ssrc="([^"]+)"/g)].map((m) =>
      m[1].replace(/^(\.\.\/)*|\?.*$/g, ""),
    );
    assert.deepEqual(scripts, ["theme.js", "search.mjs"]);
    assert.doesNotMatch(
      page.match(/http-equiv="Content-Security-Policy" content="([^"]*)"/)[1],
      /connect-src|wasm-unsafe-eval/,
    );
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

test("the CLI links each rule to a page the site publishes", () => {
  const src = readFileSync(resolve(ROOT, "crates/jsonvet-cli/src/rules.rs"), "utf8");
  const [, ref, prefix] = src.match(/"https:\/\/simonvanlierde\.com\/jsonvet\/(docs\/usage\.html)(#)\{id\}"/) ?? [];
  assert.equal(prefix, "#", "help_uri points at the site's usage page");
  assert.ok(html[ref]);
});

test("the matrix page has a table per dialect, and details for every file and parser", () => {
  const page = html["docs/matrix.html"];
  const data = JSON.parse(readFileSync(resolve(ROOT, "docs/conformance.json"), "utf8"));
  const css = readFileSync(resolve(ROOT, "playground/docs.css"), "utf8");
  for (const d of data.dialects) {
    // docs.css shows a dialect's table by its id, so a new dialect needs a rule there.
    assert.ok(css.includes(`[value="${d.dialect}"]:checked) #dialect-${d.dialect}`), `docs.css: ${d.dialect}`);
    for (const c of d.columns) assert.ok(ids(page).has(`parser/${c}`), c);
    const table = page.slice(page.indexOf(`<section class="dialect" id="dialect-${d.dialect}"`)).split("</section>")[0];
    const rows = (flag) =>
      table.match(new RegExp(`<tr( class="[^"]*\\b${flag}\\b[^"]*")?><th scope="row">`, "g"))?.length ?? 0;
    assert.equal(rows(""), d.files.length, d.dialect);
    // Each filter's count is the number of rows that carry its class.
    const counts = [...table.matchAll(/value="(\w+)"[^>]*> [^<]+<span class="count">\((\d+)\)/g)];
    assert.equal(counts.length, 5);
    for (const [, value, n] of counts) {
      const flagged = table.match(new RegExp(`<tr class="[^"]*\\b${value}\\b`, "g"))?.length ?? 0;
      assert.equal(value === "all" ? rows("") : flagged, Number(n), `${d.dialect} ${value}`);
    }
  }
  for (const f of data.dialects.flatMap((d) => d.files)) {
    const id = `file/${f.path.replace(/^tests\//, "").replace(/^conformance\//, "")}`;
    assert.ok(ids(page).has(id), id);
  }
});

test("a file's text names its invisible characters, and its share link opens the same text", async () => {
  const bom = String.fromCharCode(0xfeff);
  assert.equal(
    shown(new TextEncoder().encode(`${bom}{"a":\r\n1}\r`)).html,
    '<span class="ch">U+FEFF</span>{&quot;a&quot;:<span class="ch">CR</span>\n1}<span class="ch">CR</span>\n',
  );
  assert.equal(shown(new Uint8Array([0x7b, 0xff, 0x7d])), null);
  const [, dialect, data] = html["docs/matrix.html"].match(
    /<section class="detail" id="file\/corpus\/json5\/bom.json">[\s\S]*?href="\.\.\/#([a-z0-9-]+):([\w-]+)"/,
  );
  const text = readFileSync(resolve(ROOT, "tests/corpus/json5/bom.json"), "utf8");
  assert.ok(text.startsWith(bom));
  assert.deepEqual(await decode(`${dialect}:${data}`), { dialect: "json5", text });
});
