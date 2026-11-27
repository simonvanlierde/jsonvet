// What the docs build shares: the repository's address, GitHub's heading anchors, and where a link
// in the Markdown goes on the site. The Markdown in docs/ stays the only source; links between docs
// go to the pages, links to other files in the repository go to GitHub. A link to a file that does
// not exist stops the build.
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { posix, resolve } from "node:path";
import { Marked, Parser } from "marked";

// Pages are bundled before they run, so this is not relative to the file: the build runs in
// scripts/docs-site, and so do the tests.
export const ROOT = resolve(process.cwd(), "../..");
export const REPO = "https://github.com/simonvanlierde/jsonvet";
export const SITE = "https://simonvanlierde.com/jsonvet/";
/** Where the docs live on the site, as the CLI and the playground link to them. */
export const DOCS = "/jsonvet/docs/";

// The one page that is not from Markdown: matrix.mjs renders it from this file.
export const MATRIX = "docs/conformance.json";

// The docs nav, in reading order. Every docs/*.md needs a row, so a new page cannot go unlisted.
export const NAV = [
  ["docs/usage.md", "Reference"],
  ["docs/format.md", "Formatting"],
  ["docs/editors.md", "Editors"],
  ["docs/design.md", "Design"],
  ["docs/architecture.md", "Architecture"],
  ["docs/conformance.md", "Conformance"],
  [MATRIX, "Matrix"],
  ["docs/parser-disagreements.md", "Where parsers disagree"],
  ["docs/accessibility.md", "Accessibility"],
  ["docs/assurance-case.md", "Assurance case"],
  ["docs/decisions/README.md", "Decisions"],
  ["docs/roadmap.md", "Roadmap"],
];

/** The page a doc becomes, relative to DOCS: usage.html, decisions/ for the decisions' README. */
export const pageOf = (src) =>
  src === MATRIX
    ? "matrix.html"
    : src
        .replace(/^docs\//, "")
        .replace(/(^|\/)README\.md$/, "$1")
        .replace(/\.md$/, ".html");

/** Every Markdown file of the site, from the repository root: docs/*.md and docs/decisions/*.md. */
export const sources = () => [
  ...readdirSync(resolve(ROOT, "docs"))
    .filter((f) => f.endsWith(".md"))
    .map((f) => `docs/${f}`),
  ...readdirSync(resolve(ROOT, "docs/decisions"))
    .filter((f) => f.endsWith(".md"))
    .map((f) => `docs/decisions/${f}`),
];

/** The plain text of inline tokens, as GitHub reads a heading for its anchor. */
const toText = (tokens) => {
  const parser = new Parser();
  return parser.parseInline(tokens, parser.textRenderer);
};
export const esc = (text) =>
  text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

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
export function submodules() {
  const config = existsSync(resolve(ROOT, ".gitmodules")) ? readFileSync(resolve(ROOT, ".gitmodules"), "utf8") : "";
  return [...config.matchAll(/path = (\S+)\s+url = (\S+)/g)].map(([, path, url]) => {
    const commit = execFileSync("git", ["-C", ROOT, "rev-parse", `HEAD:${path}`], { encoding: "utf8" }).trim();
    return { path, url: url.replace(/\.git$/, ""), commit };
  });
}

/** Where a link in doc `src` goes on the site: another page, GitHub, or unchanged. */
export function rewrite(href, src, pages, subs) {
  if (/^[a-z][a-z0-9+.-]*:|^\/\/|^#/i.test(href)) return href;
  const [path, hash] = href.split(/(?=#)/);
  let target = posix.normalize(posix.join(posix.dirname(src), decodeURI(path)));
  if (target.endsWith("/")) target = target.slice(0, -1);
  const full = resolve(ROOT, target);
  if (!existsSync(full))
    throw new Error(
      `${src}: the link ${href} names ${target}, which does not exist${target.startsWith("tests/") ? " (run git submodule update --init)" : ""}`,
    );
  const dir = statSync(full).isDirectory();
  if (dir && pages.has(`${target}/README.md`)) target = `${target}/README.md`;
  if (pages.has(target)) {
    return DOCS + pageOf(target) + (hash ?? "");
  }
  const sub = subs.find((s) => target.startsWith(`${s.path}/`));
  if (sub) return `${sub.url}/${dir ? "tree" : "blob"}/${sub.commit}/${target.slice(sub.path.length + 1)}${hash ?? ""}`;
  return `${REPO}/${dir ? "tree" : "blob"}/main/${target}${hash ?? ""}`;
}

/** A doc's title (its h1) and its summary (its first sentence that says something), for the
 * page's title and description. A decision record opens with "Status: accepted (...)", which says
 * nothing about the page, and a paragraph that ends in a colon only introduces a list. */
export function describe(src) {
  const tokens = new Marked().lexer(readFileSync(resolve(ROOT, src), "utf8"));
  const h1 = tokens.find((t) => t.type === "heading" && t.depth === 1);
  const text = (t) => toText(t.tokens).replace(/\s+/g, " ").trim();
  const first = tokens.find((t) => t.type === "paragraph" && !t.text.startsWith("Status:") && !text(t).endsWith(":"));
  return {
    title: toText(h1.tokens),
    summary: first ? text(first).split(/(?<=[.!?])\s+(?=[A-Z])/)[0] : "",
  };
}

/** A fragment of the header or footer that the playground's page and the docs pages share. */
export const chrome = (name) => readFileSync(resolve(ROOT, "playground", name), "utf8").trim();
