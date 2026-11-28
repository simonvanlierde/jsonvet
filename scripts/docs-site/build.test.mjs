import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync } from "node:fs";
import { posix, resolve } from "node:path";
import { after, test } from "node:test";
import { decode } from "../../playground/share.mjs";
import { policy, scriptHashes } from "./csp-hash.mjs";
import { anchorsOf, NAV, pageOf, QUICK_START, REPO, ROOT, ruleIds, SITE, slugger, sources } from "./lib.mjs";
import { shown } from "./matrix.mjs";

// The site as deployed, built into a folder of its own (DOCS_OUT), so a test never touches ../../site.
// Inside node_modules, as Astro moves files into it and /tmp may be another device.
const site = mkdtempSync(resolve(import.meta.dirname, "node_modules/.site-"));
after(() => rmSync(site, { recursive: true, force: true }));
try {
  execFileSync("astro", ["build"], { env: { ...process.env, DOCS_OUT: site }, encoding: "utf8", stdio: "pipe" });
} catch (e) {
  rmSync(site, { recursive: true, force: true });
  throw new Error(`astro build failed:\n${e.stdout}${e.stderr}`);
}

const BASE = "/jsonvet/";
const walk = (dir) =>
  readdirSync(dir, { recursive: true, withFileTypes: true })
    .filter((f) => f.isFile())
    .map((f) => posix.relative(dir, resolve(f.parentPath, f.name)));
// Every Starlight page: the docs and the 404, not the playground's index.html.
const pagePaths = walk(site).filter((p) => p.endsWith(".html") && (p.startsWith("docs/") || p === "404.html"));
const html = Object.fromEntries(pagePaths.map((p) => [p, readFileSync(resolve(site, p), "utf8")]));
const read = (path) => readFileSync(resolve(site, path), "utf8");
// Cached: the matrix page alone has thousands of links into its own ids.
const idCache = new Map();
const ids = (text) => {
  if (!idCache.has(text)) idCache.set(text, new Set([...text.matchAll(/\sid="([^"]+)"/g)].map((m) => m[1])));
  return idCache.get(text);
};
const descriptionOf = (page) => html[page].match(/<meta name="description" content="([^"]*)"/)[1];

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

test("every address of the old pages still serves a page, as a file", () => {
  const expected = [...sources().map(pageOf), "matrix.html"].map((p) => `docs/${p}`);
  assert.ok(expected.length >= 29);
  for (const path of expected) {
    // The decisions' README and the contents are the folders' index.
    const file = path.endsWith("/") ? `${path}index.html` : path;
    assert.ok(existsSync(resolve(site, file)), file);
  }
  for (const file of ["docs/index.html", "docs/decisions/index.html", "404.html", "index.html"])
    assert.ok(existsSync(resolve(site, file)), file);
});

test("every link between the pages, every asset and every anchor resolves", () => {
  const broken = [];
  for (const [path, text] of Object.entries(html)) {
    for (const [, href] of text.matchAll(/\s(?:href|src)="([^"]+)"/g)) {
      if (/^[a-z]+:/i.test(href)) continue;
      const [ref, anchor] = href.split("#");
      // A link is absolute under the base, or relative to the page (the matrix page's playground links).
      const rooted = ref.startsWith(BASE) ? ref.slice(BASE.length) : posix.join(posix.dirname(path), ref);
      const target = !ref ? path : rooted.endsWith("/") || rooted === "" ? `${rooted}index.html` : rooted;
      const file = posix.normalize(target);
      // A playground share link (`#dialect:data`) is read by the page's script, not an id.
      const shared = file === "index.html" && /^[a-z0-9-]+:/.test(anchor ?? "");
      if (ref.startsWith("/") && !ref.startsWith(BASE)) broken.push(`${path}: ${href} (outside the base)`);
      else if (!existsSync(resolve(site, file))) broken.push(`${path}: ${href}`);
      else if (anchor && !shared && !ids(html[file] ?? read(file)).has(anchor))
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

test("each page has a skip link and its landmarks, and one h1", () => {
  for (const [path, text] of Object.entries(html)) {
    assert.match(text, /<a [^>]*href="#_top"/, `${path}: skip link`);
    for (const tag of ["<header", "<main", ...(path === "404.html" ? [] : ["<nav"])])
      assert.ok(text.includes(tag), `${path} ${tag}`);
    assert.equal(text.match(/<h1[ >]/g)?.length, 1, `${path}: one h1`);
  }
});

test("tables and code scroll in a focusable, named box, and the names are unique on each page", () => {
  const page = html["docs/conformance.html"];
  const box = '<div class="scroll" tabindex="0" role="region" aria-label="Table[^"]*"><table>';
  assert.equal(page.match(/<table>/g).length, page.match(new RegExp(box, "g")).length);
  assert.match(html["docs/format.html"], /<pre tabindex="0" role="region" aria-label="Code(: [^"]*)?">/);
  for (const [path, text] of Object.entries(html)) {
    const labels = [...text.matchAll(/role="region" aria-label="([^"]+)"/g)].map((m) => m[1]);
    assert.deepEqual(
      labels.filter((l, i) => labels.indexOf(l) !== i),
      [],
      path,
    );
  }
  assert.match(page, /aria-label="Table: /);
});

test("ids are unique on each page", () => {
  for (const [path, text] of Object.entries(html)) {
    const all = [...text.matchAll(/\sid="([^"]+)"/g)].map((m) => m[1]);
    assert.deepEqual(
      all.filter((id, i) => all.indexOf(id) !== i),
      [],
      path,
    );
  }
});

// What the policy is for: a page may carry inline script only if its hash is in the policy, and
// nothing may come from another origin.
const SOURCE = /^(?:'self'|'none'|'unsafe-inline'|'wasm-unsafe-eval'|data:|'sha256-[A-Za-z0-9+/]+=*')$/;
const policyOf = (text) => text.match(/<meta http-equiv="Content-Security-Policy" content="([^"]*)"/)?.[1];
/** The inline scripts of a page that its policy does not allow. */
const unhashed = (text) => {
  const allowed = new Set((policyOf(text)?.match(/script-src ([^;]*)/)?.[1] ?? "").split(" "));
  return scriptHashes(text).filter((h) => !allowed.has(h));
};

test("the CSP check sees an inline script that is not hashed", () => {
  const page = '<meta charset="utf-8"><script>one()</script><script type="module">two()</script>';
  assert.equal(scriptHashes(page).length, 2);
  assert.equal(scriptHashes('<script src="/a.js"></script><script type="application/json">{}</script>').length, 0);
  const policed = `<meta http-equiv="Content-Security-Policy" content="${policy(page)}">${page}`;
  assert.deepEqual(unhashed(policed), []);
  assert.equal(unhashed(`${policed}<script>three()</script>`).length, 1);
  assert.equal(unhashed(policed.replace("one()", "one( )")).length, 1);
});

test("every inline script of a docs page is hashed in its CSP, and nothing comes from another origin", () => {
  let inline = 0;
  for (const [path, text] of Object.entries(html)) {
    const csp = policyOf(text);
    assert.ok(csp, `${path}: a policy`);
    assert.match(csp, /^default-src 'none'; script-src 'self' 'wasm-unsafe-eval'/, path);
    assert.deepEqual(unhashed(text), [], `${path}: an inline script is not in the policy`);
    inline += scriptHashes(text).length;
    // Every source is the site itself, a hash or one of the few keywords: no host, no `*`.
    for (const directive of csp.split("; ")) {
      const [name, ...sources] = directive.split(" ");
      for (const s of sources) assert.match(s, SOURCE, `${path}: ${name} ${s}`);
      if (name === "script-src") assert.ok(!sources.includes("'unsafe-inline'"), `${path}: script-src`);
    }
    assert.ok(csp.includes("connect-src 'self'") && csp.includes("base-uri 'none'"), path);
    // The policy cannot hash an event handler attribute.
    assert.doesNotMatch(text, /\son[a-z]+="/, `${path}: an event handler attribute`);
    assert.doesNotMatch(text, /javascript:/, path);
    // A resource is the site's own file: a script, stylesheet, image, frame or font by address.
    for (const [tag] of text.matchAll(/<(?:script|link|img|iframe|source|video|audio|embed|object)\b[^>]*>/gi)) {
      for (const [, url] of tag.matchAll(/\s(?:src|href|srcset|data)="(https?:\/\/[^"]+)"/g)) {
        // Only an address a reader can follow is allowed: the canonical and the sitemap are not loaded.
        assert.ok(/rel="(?:canonical|sitemap)"/.test(tag), `${path}: loads ${url}`);
      }
    }
  }
  assert.ok(inline > 0);
  for (const file of walk(site).filter((f) => f.endsWith(".css") && f !== "style.css" && !f.startsWith("pagefind")))
    assert.doesNotMatch(read(file), /url\(\s*["']?(?:https?:)?\/\/|@import/, file);
});

test("the playground page keeps its strict CSP and its own files, with no inline script or style", () => {
  const page = read("index.html");
  assert.equal(page, readFileSync(resolve(ROOT, "playground/index.html"), "utf8"));
  assert.equal(
    policyOf(page),
    "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; connect-src 'self'; style-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'",
  );
  assert.doesNotMatch(page, /<script(?![^>]*\ssrc=)[^>]*>|<style|\sstyle=|unsafe-inline/);
});

test("without script the system's theme holds: no page says it is dark", () => {
  for (const [path, text] of Object.entries(html))
    assert.doesNotMatch(text.match(/<html[^>]*>/)[0], /data-theme/, path);
  const css = walk(site)
    .filter((f) => f.endsWith(".css"))
    .map(read)
    .join("");
  assert.match(css, /prefers-color-scheme:\s*dark\)\s*{\s*:root:not\(\[data-theme\]\)/);
});

test("search: the index is built over the docs, and the matrix is left out", () => {
  assert.ok(existsSync(resolve(site, "pagefind/pagefind.js")));
  for (const [path, text] of Object.entries(html)) {
    const indexed = text.includes("data-pagefind-body");
    assert.equal(indexed, !["docs/matrix.html", "404.html"].includes(path), path);
  }
  assert.ok(existsSync(resolve(site, "index.html")) && !read("index.html").includes("data-pagefind-body"));
  const index = html["docs/index.html"];
  assert.ok(index.includes(`<a href="${QUICK_START}">Install jsonvet</a>`));
});

test("page descriptions omit status lines, review dates, and list introductions", () => {
  assert.match(descriptionOf("docs/decisions/0002-no-unsafe-code.html"), /\s.*\s/);
  for (const path of Object.keys(html).filter((p) => p !== "404.html")) {
    assert.doesNotMatch(descriptionOf(path), /^(?:Status|Last reviewed):/, path);
    assert.doesNotMatch(descriptionOf(path), /:$/, path);
  }
});

test("the playground's links into the pages resolve", () => {
  const broken = [];
  const links = [
    ...readFileSync(resolve(ROOT, "playground/index.html"), "utf8").matchAll(/href="\/jsonvet\/(docs\/[^"]*)"/g),
  ].map((m) => m[1]);
  // main.mjs links each problem's rule to docs/usage.html#<rule>. The ids are the rules' in
  // rule.rs, and the CLI's own two.
  const rules = readFileSync(resolve(ROOT, "crates/jsonvet/src/rule.rs"), "utf8").matchAll(/Rule::\w+ => "([a-z-]+)"/g);
  for (const id of [...new Set([...rules].map((m) => m[1])), "unreadable", "schema"])
    links.push(`docs/usage.html#${id}`);
  assert.ok(links.length > 10);
  for (const href of links) {
    const [ref, anchor] = href.split("#");
    const file = ref.endsWith("/") ? `${ref}index.html` : ref;
    const page = html[file];
    if (!page) broken.push(href);
    else if (anchor && !ids(page).has(anchor)) broken.push(`${href} (no such anchor)`);
  }
  assert.deepEqual(broken, []);
});

test("literal code and matrix fixtures opt out of Cloudflare email obfuscation", () => {
  for (const path of ["docs/usage.html", "docs/matrix.html"]) {
    const blocks = [...html[path].matchAll(/<pre(?=[\s>])[^>]*>[\s\S]*?<\/pre>/g)];
    assert.ok(blocks.length > 0, path);
    for (const block of blocks) {
      assert.ok(html[path].startsWith("<!--email_off-->", block.index - "<!--email_off-->".length), path);
      assert.ok(html[path].startsWith("<!--/email_off-->", block.index + block[0].length), path);
    }
  }
  assert.match(html["docs/usage.html"], /python@\d+(?:\.\d+)+/);
  assert.match(html["docs/usage.html"], /jsonvet@\d+(?:\.\d+)+/);
});

test("the CLI links each rule to a page the site publishes", () => {
  const src = readFileSync(resolve(ROOT, "crates/jsonvet-cli/src/rules.rs"), "utf8");
  const [, ref, prefix] = src.match(/"https:\/\/simonvanlierde\.com\/jsonvet\/(docs\/usage\.html)(#)\{id\}"/) ?? [];
  assert.equal(prefix, "#", "help_uri points at the site's usage page");
  assert.ok(html[ref]);
});

test("the docs nav lists every page, in order", () => {
  const nav =
    html["docs/usage.html"].match(/<nav[^>]*aria-label="Main"[^>]*>[\s\S]*?<\/nav>/)?.[0] ?? html["docs/usage.html"];
  const links = [...nav.matchAll(/href="\/jsonvet\/docs\/([^"#]*)"/g)].map((m) => m[1]);
  const wanted = NAV.map(([src]) => pageOf(src).replace(/\/$/, "/index.html"));
  assert.deepEqual(
    wanted.filter((w) => links.includes(w)),
    wanted,
  );
  assert.ok(html["docs/usage.html"].includes('aria-current="page"'));
});

test("exact search matches lead to the quick start or the relevant reference section", () => {
  const usage = html["docs/usage.html"];
  const [, results] = usage.match(/data-direct-results="([^"]+)"/);
  const matches = new Map(JSON.parse(results.replaceAll("&quot;", '"')));
  assert.equal(matches.get("install jsonvet").href, QUICK_START);
  assert.ok(ids(usage).has("sarif"));
  assert.equal(matches.get("sarif").href, `${BASE}docs/usage.html#sarif`);
  assert.ok(ids(usage).has("gitlab-code-quality"));
  assert.equal(matches.get("gitlab").href, `${BASE}docs/usage.html#gitlab-code-quality`);
  assert.equal(matches.get("gitlab code quality").href, matches.get("gitlab").href);
  const rules = ruleIds();
  assert.ok(rules.includes("duplicate-key"));
  for (const rule of rules) {
    assert.ok(ids(usage).has(rule), rule);
    assert.equal(matches.get(rule).href, `${BASE}docs/usage.html#${rule}`);
  }
});

test("the matrix page has a table per dialect, and details for every file and parser", () => {
  const page = html["docs/matrix.html"];
  const data = JSON.parse(readFileSync(resolve(ROOT, "docs/conformance.json"), "utf8"));
  const css = readFileSync(resolve(import.meta.dirname, "src/styles/matrix.css"), "utf8");
  for (const d of data.dialects) {
    // matrix.css shows a dialect's table by its id, so a new dialect needs a rule there.
    assert.ok(css.includes(`[value="${d.dialect}"]:checked) #dialect-${d.dialect}`), `matrix.css: ${d.dialect}`);
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

test("the report includes duplicate-key variants and links to the syntax matrix", () => {
  const report = html["docs/conformance.html"];
  const matrix = html["docs/matrix.html"];
  assert.ok(report.includes("<title>Conformance report | jsonvet docs</title>"));
  assert.ok(matrix.includes("<title>Conformance matrix | jsonvet docs</title>"));
  assert.match(descriptionOf("docs/conformance.html"), /including duplicate-key variants/);
  assert.match(descriptionOf("docs/matrix.html"), /syntax runs/);
  assert.ok(matrix.includes('href="/jsonvet/docs/conformance.html">conformance report</a>'));
  assert.ok(report.includes(`href="${SITE}docs/matrix.html">conformance matrix</a>`));
});

test("a file's text names its invisible characters, and its share link opens the same text", async () => {
  const bom = String.fromCharCode(0xfeff);
  assert.equal(
    shown(new TextEncoder().encode(`${bom}{"a":\r\n1}\r`)).html,
    '<span class="ch">U+FEFF</span>{&quot;a&quot;:<span class="ch">CR</span>\n1}<span class="ch">CR</span>\n',
  );
  assert.equal(shown(new Uint8Array([0x7b, 0xff, 0x7d])), null);
  const [, dialect, data] = html["docs/matrix.html"].match(
    /<section class="detail" id="file\/corpus\/json5\/bom.json"[^>]*>[\s\S]*?href="\.\.\/#([a-z0-9-]+):([\w-]+)"/,
  );
  const text = readFileSync(resolve(ROOT, "tests/corpus/json5/bom.json"), "utf8");
  assert.ok(text.startsWith(bom));
  assert.deepEqual(await decode(`${dialect}:${data}`), { dialect: "json5", text });
});

test("each page has a canonical address and the social card's tags", () => {
  const card = readFileSync(resolve(ROOT, "playground/index.html"), "utf8").match(
    /property="og:image" content="([^"]+)"/,
  )[1];
  for (const [path, text] of Object.entries(html).filter(([p]) => p !== "404.html")) {
    for (const tag of [
      'rel="canonical"',
      'property="og:title"',
      'property="og:description"',
      'property="og:url"',
      'name="twitter:card"',
    ])
      assert.ok(text.includes(tag), `${path} ${tag}`);
    assert.ok(text.includes(`property="og:image" content="${card}"`), `${path}: the playground's card`);
    const canonical = text.match(/rel="canonical" href="([^"]+)"/)[1];
    assert.equal(text.match(/property="og:url" content="([^"]+)"/)[1], canonical, path);
  }
  // The folders' pages keep their old address, not the file that serves them.
  assert.match(html["docs/index.html"], /rel="canonical" href="https:\/\/simonvanlierde.com\/jsonvet\/docs\/"/);
  assert.match(html["docs/decisions/index.html"], /rel="canonical" href="[^"]*\/docs\/decisions\/"/);
  assert.match(
    html["docs/usage.html"],
    /rel="canonical" href="https:\/\/simonvanlierde.com\/jsonvet\/docs\/usage.html"/,
  );
});

test("the sitemap lists each page at its canonical address, and not the 404", () => {
  const urls = [...read("sitemap-0.xml").matchAll(/<loc>([^<]+)<\/loc>/g)].map((m) => m[1]);
  const canonical = Object.entries(html)
    .filter(([p]) => p !== "404.html")
    .map(([, t]) => t.match(/rel="canonical" href="([^"]+)"/)[1]);
  assert.deepEqual(urls.filter((u) => u !== "https://simonvanlierde.com/jsonvet/").sort(), canonical.sort());
  assert.equal(urls.filter((u) => u === "https://simonvanlierde.com/jsonvet/").length, 1);
});

test("a 404 page sits at the site's root, styled by absolute paths and kept out of search", () => {
  const text = html["404.html"];
  assert.match(text, /<h1[^>]*>[^<]*404/i);
  assert.match(text, /href="\/jsonvet\/_astro\/[^"]+\.css"/);
  assert.match(text, /name="robots" content="noindex"/);
  assert.doesNotMatch(text, /rel="canonical"/);
  assert.ok(statSync(resolve(site, "404.html")).size > 0);
});

// The header bar and the footer line are written once (playground/header.html, footer.html), and the
// playground's static page and every docs page carry them as they are.
const fragment = (name) => readFileSync(resolve(ROOT, "playground", name), "utf8").trim();
const header = fragment("header.html");
const footer = fragment("footer.html");
const [headerBefore, headerAfter] = header.split("<!--search-->");

test("the playground and the docs pages share one header bar and one footer line", () => {
  const playground = read("index.html");
  assert.ok(playground.includes(`<div class="bar">${headerBefore}${headerAfter}</div>`), "index.html: header.html");
  assert.ok(
    playground.includes(`<footer class="wrap colophon">`) && playground.includes(footer),
    "index.html: footer.html",
  );
  for (const [path, text] of Object.entries(html)) {
    const bar = text.match(/<div class="bar">([\s\S]*?)<\/header>/)?.[1] ?? "";
    assert.ok(bar.startsWith(headerBefore), `${path}: the bar starts with header.html`);
    assert.ok(bar.includes(headerAfter), `${path}: the bar ends with header.html`);
    assert.ok(text.includes(`<div class="colophon">${footer}</div>`), `${path}: footer.html`);
  }
});

test("the docs pages take their theme from the playground's theme.js, under its one key", () => {
  assert.match(read("theme.js"), /getItem\("theme"\)/);
  for (const [path, text] of Object.entries(html)) {
    assert.match(text, /<script src="\/jsonvet\/theme\.js"/, path);
    assert.doesNotMatch(text, /getItem\(['"`]starlight-theme/, path);
  }
});
