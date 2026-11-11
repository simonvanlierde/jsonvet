// Renders docs/*.md and docs/decisions/*.md into playground/docs/, as HTML pages in the
// playground's header, theme and CSP. The Markdown stays the only source: links between docs go
// to the pages, links to other files in the repository go to GitHub. A link to a file that does
// not exist stops the build. COMMIT, when set, is stamped on the style and script URLs as pages.yml
// stamps the playground's.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { posix, resolve } from "node:path";
import { Marked, Parser, Renderer } from "marked";

export const ROOT = resolve(import.meta.dirname, "../..");
export const REPO = "https://github.com/simonvanlierde/jsonvet";
const SITE = "https://simonvanlierde.com/jsonvet/";
const CSP =
  "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'";

// The docs nav, in reading order. Every docs/*.md needs a row, so a new page cannot go unlisted.
const NAV = [
  ["docs/usage.md", "Reference"],
  ["docs/format.md", "Formatting"],
  ["docs/editors.md", "Editors"],
  ["docs/design.md", "Design"],
  ["docs/conformance.md", "Conformance"],
  ["docs/accessibility.md", "Accessibility"],
  ["docs/assurance-case.md", "Assurance case"],
  ["docs/decisions/README.md", "Decisions"],
];

/** The page a doc becomes, relative to the output folder: usage.html, decisions/index.html. */
const pageOf = (src) =>
  src
    .replace(/^docs\//, "")
    .replace(/(^|\/)README\.md$/, "$1index.html")
    .replace(/\.md$/, ".html");
/** A relative link from one page to another, to the folder for an index page. */
const linkTo = (from, to) => posix.relative(posix.dirname(from), to).replace(/(^|\/)index\.html$/, "$1") || "./";

/** The plain text of inline tokens, as GitHub reads a heading for its anchor. */
const toText = (tokens) => {
  const parser = new Parser();
  return parser.parseInline(tokens, parser.textRenderer);
};
const esc = (text) => text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

/** GitHub's heading anchors (github-slugger): lower case, punctuation but `-` and `_` dropped,
 * spaces to `-`, and a repeat gets `-1`, `-2` and so on. */
export function slugger() {
  const seen = new Map();
  return (text) => {
    const base = text
      .toLowerCase()
      .replace(/[^\p{L}\p{M}\p{N}\p{Pc} -]/gu, "")
      .replace(/ /g, "-");
    let slug = base;
    while (seen.has(slug)) {
      const n = seen.get(base) + 1;
      seen.set(base, n);
      slug = `${base}-${n}`;
    }
    seen.set(slug, 0);
    return slug;
  };
}

/** The anchors GitHub gives a Markdown file's headings. */
export function anchorsOf(file) {
  const slug = slugger();
  const marked = new Marked();
  return new Set(
    marked
      .lexer(readFileSync(resolve(ROOT, file), "utf8"))
      .filter((t) => t.type === "heading")
      .map((t) => slug(toText(t.tokens))),
  );
}

// Submodules are not in this repository on GitHub, so their files link to their own repository at
// the commit pinned here.
function submodules() {
  const config = existsSync(resolve(ROOT, ".gitmodules")) ? readFileSync(resolve(ROOT, ".gitmodules"), "utf8") : "";
  return [...config.matchAll(/path = (\S+)\s+url = (\S+)/g)].map(([, path, url]) => {
    const commit = execFileSync("git", ["-C", ROOT, "rev-parse", `HEAD:${path}`], { encoding: "utf8" }).trim();
    return { path, url: url.replace(/\.git$/, ""), commit };
  });
}

/** Where a link in doc `src` goes on the site: another page, GitHub, or unchanged. */
function rewrite(href, src, pages, subs) {
  if (/^[a-z][a-z0-9+.-]*:|^\/\/|^#/i.test(href)) return href;
  const [path, hash] = href.split(/(?=#)/);
  let target = posix.normalize(posix.join(posix.dirname(src), decodeURI(path)));
  if (target.endsWith("/")) target = target.slice(0, -1);
  const full = resolve(ROOT, target);
  if (!existsSync(full)) throw new Error(`${src}: the link ${href} names ${target}, which does not exist`);
  const dir = statSync(full).isDirectory();
  if (dir && pages.has(`${target}/README.md`)) target = `${target}/README.md`;
  if (pages.has(target)) {
    return linkTo(pageOf(src), pageOf(target)) + (hash ?? "");
  }
  const sub = subs.find((s) => target.startsWith(`${s.path}/`));
  if (sub) return `${sub.url}/${dir ? "tree" : "blob"}/${sub.commit}/${target.slice(sub.path.length + 1)}${hash ?? ""}`;
  return `${REPO}/${dir ? "tree" : "blob"}/main/${target}${hash ?? ""}`;
}

/** One doc as HTML, with its title and first paragraph. */
function render(src, pages, subs) {
  const slug = slugger();
  const h2s = [];
  // Boxes are labelled by the heading above them, so each landmark name is unique on the page.
  let heading = "";
  const uses = new Map();
  const label = (kind) => {
    const key = `${kind}: ${heading}`;
    const n = (uses.get(key) ?? 0) + 1;
    uses.set(key, n);
    return esc((heading ? key : kind) + (n > 1 ? ` (${n})` : ""));
  };
  const marked = new Marked({
    gfm: true,
    renderer: {
      link(token) {
        return Renderer.prototype.link.call(this, { ...token, href: rewrite(token.href, src, pages, subs) });
      },
      heading({ tokens, depth }) {
        const text = toText(tokens);
        const id = slug(text);
        heading = text;
        if (depth === 2) h2s.push({ id, text });
        return `<h${depth} id="${esc(id)}">${this.parser.parseInline(tokens)}</h${depth}>\n`;
      },
      // Wide tables and code scroll on their own, and take focus so a keyboard can scroll them.
      table(token) {
        return `<div class="scroll" tabindex="0" role="region" aria-label="${label("Table")}">${Renderer.prototype.table.call(this, token)}</div>\n`;
      },
      code(token) {
        return Renderer.prototype.code
          .call(this, token)
          .replace(/^<pre>/, `<pre tabindex="0" role="region" aria-label="${label("Code")}">`);
      },
    },
  });
  const markdown = readFileSync(resolve(ROOT, src), "utf8");
  const tokens = marked.lexer(markdown);
  const h1 = tokens.find((t) => t.type === "heading" && t.depth === 1);
  // A decision record opens with "Status: accepted (...)", which says nothing about the page, and
  // a paragraph that ends in a colon only introduces a list.
  const text = (t) => toText(t.tokens).replace(/\s+/g, " ").trim();
  const first = tokens.find((t) => t.type === "paragraph" && !t.text.startsWith("Status:") && !text(t).endsWith(":"));
  const body = marked.parser(tokens);
  return {
    body,
    h2s,
    title: toText(h1.tokens),
    summary: first ? text(first).split(/(?<=[.!?])\s+(?=[A-Z])/)[0] : "",
  };
}

function page({ out, src, title, summary, body, h2s = [] }, stamp) {
  const up = "../".repeat(out.split("/").length);
  const section = src.startsWith("docs/decisions/") ? "docs/decisions/README.md" : src;
  const nav = NAV.map(([file, label]) => {
    const href = linkTo(out, pageOf(file));
    const current = file === src ? ' aria-current="page"' : file === section ? ' class="here"' : "";
    // A long page lists its sections under its own item, for the wide layout.
    const inner =
      file === src && h2s.length >= 3
        ? `<ul>${h2s.map((h) => `<li><a href="#${esc(h.id)}">${esc(h.text)}</a></li>`).join("")}</ul>`
        : "";
    return `<li><a href="${href}"${current}>${label}</a>${inner}</li>`;
  }).join("");
  const source = src ? `<span><a href="${REPO}/blob/main/${src}">this page on GitHub</a></span>` : "";
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="${CSP}">
<title>${esc(title)} · jsonvet docs</title>
<link rel="icon" href="${up}favicon.svg" type="image/svg+xml">
<meta name="description" content="${esc(summary)}">
<link rel="canonical" href="${SITE}docs/${out.replace(/(^|\/)index\.html$/, "$1")}">
<meta name="theme-color" content="#fbfaf5" media="(prefers-color-scheme: light)">
<meta name="theme-color" content="#1b1915" media="(prefers-color-scheme: dark)">
<link rel="stylesheet" href="${up}style.css${stamp}">
<link rel="stylesheet" href="${up}docs.css${stamp}">
<script src="${up}theme.js${stamp}"></script>
</head>
<body>
<a class="skip" href="#content">Skip to the text</a>
<header class="wrap slug">
  <p class="slug-part"><span><a href="${up}"><b>jsonvet</b></a></span><span>docs</span><span>${esc(title)}</span></p>
  <p class="slug-part"><span><a href="${up}">playground</a></span>${source}</p>
</header>
<div class="wrap docs">
<nav aria-label="Docs"><ul>${nav}</ul></nav>
<main id="content" class="prose">
${body}</main>
</div>
<footer class="wrap colophon">
  <p class="quiet">These pages are built from the Markdown in <a href="${REPO}/tree/main/docs">docs/</a>. They load only this site's own files, and keep only your theme choice in your browser.</p>
  <p class="slug-part"><span>jsonvet</span><span><a href="${REPO}#credits-and-license">MIT or Apache-2.0 licence</a></span><span><button type="button" id="theme" class="theme" aria-label="Switch theme" title="Switch theme"><svg class="i-sun" viewBox="0 0 16 16" aria-hidden="true"><circle cx="8" cy="8" r="3"/><path d="M8 1.5v1.5M8 13v1.5M1.5 8H3M13 8h1.5M3.4 3.4l1.06 1.06M11.54 11.54l1.06 1.06M3.4 12.6l1.06-1.06M11.54 4.46l1.06-1.06"/></svg><svg class="i-moon" viewBox="0 0 16 16" aria-hidden="true"><path d="M13.5 9.6A5.5 5.5 0 0 1 6.4 2.5a5.5 5.5 0 1 0 7.1 7.1z"/></svg></button></span></p>
  <p class="slug-part"><span><a href="https://github.com/simonvanlierde">Simon van Lierde</a></span><span><a href="${REPO}">source on GitHub</a></span></p>
</footer>
</body>
</html>
`;
}

/** Writes every page into `outDir`, emptied first, and returns their paths relative to it. */
export function build(outDir, commit = "") {
  const docs = readdirSync(resolve(ROOT, "docs")).filter((f) => f.endsWith(".md"));
  const decisions = readdirSync(resolve(ROOT, "docs/decisions")).filter((f) => f.endsWith(".md"));
  const sources = [...docs.map((f) => `docs/${f}`), ...decisions.map((f) => `docs/decisions/${f}`)];
  const unlisted = docs.filter((f) => !NAV.some(([file]) => file === `docs/${f}`));
  if (unlisted.length) throw new Error(`build.mjs: add ${unlisted.join(", ")} to NAV`);
  const pages = new Set(sources);
  const subs = submodules();
  const stamp = commit ? `?v=${commit}` : "";
  const rendered = sources.map((src) => ({ src, out: pageOf(src), ...render(src, pages, subs) }));
  const list = NAV.map(([file, label]) => {
    const doc = rendered.find((r) => r.src === file);
    return `<li><a href="${linkTo("index.html", pageOf(file))}">${label}</a>: ${esc(doc.summary)}</li>`;
  }).join("\n");
  rendered.push({
    src: "",
    out: "index.html",
    title: "Contents",
    summary: "The jsonvet reference, design notes, conformance matrix and decision records.",
    body: `<h1 id="jsonvet-docs">jsonvet docs</h1>\n<ul class="contents">\n${list}\n</ul>\n`,
  });
  rmSync(outDir, { recursive: true, force: true });
  for (const doc of rendered) {
    mkdirSync(resolve(outDir, posix.dirname(doc.out)), { recursive: true });
    writeFileSync(resolve(outDir, doc.out), page(doc, stamp));
  }
  return rendered.map((r) => r.out);
}

if (import.meta.main) {
  const out = resolve(ROOT, "playground/docs");
  console.log(`${build(out, process.env.COMMIT ?? "").length} pages in ${out}`);
}
