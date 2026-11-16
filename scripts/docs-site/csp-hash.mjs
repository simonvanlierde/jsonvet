// An Astro integration that runs after the build: it gives every docs page a Content-Security-Policy
// meta tag that allows exactly the inline scripts the page has, by their sha256, and it lists the
// pages' canonical addresses in the sitemap, and takes the 404 page out of search engines. Starlight writes data-theme="dark" into every page and
// its inline script corrects it; the attribute goes, so a reader without script gets the system's
// theme from jsonvet.css. The playground's index.html, which the build copies in
// as it is, keeps its own strict policy: no inline script or style at all.
import { createHash } from "node:crypto";
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

// A script with a `src` is the site's own file, which 'self' covers. A data block is not run.
const INLINE = /<script(?![^>]*\ssrc=)([^>]*)>([\s\S]*?)<\/script>/g;
const DATA = /\stype="(?:application\/json|application\/ld\+json|importmap)"/;

/** The sha256 source expressions of a page's inline scripts, each once. */
export function scriptHashes(html) {
  const hashes = new Set();
  for (const [, attrs, code] of html.matchAll(INLINE))
    if (!DATA.test(attrs)) hashes.add(`'sha256-${createHash("sha256").update(code).digest("base64")}'`);
  return [...hashes].sort();
}

/**
 * The docs pages' policy. Pagefind (the search) needs `connect-src 'self'` for its index and
 * 'wasm-unsafe-eval' for its WebAssembly; Astro and Starlight set `style` attributes, so styles may
 * be inline. Nothing is loaded from another origin.
 */
export function policy(html) {
  return [
    "default-src 'none'",
    `script-src 'self' 'wasm-unsafe-eval' ${scriptHashes(html).join(" ")}`.trim(),
    "style-src 'self' 'unsafe-inline'",
    "img-src 'self' data:",
    "font-src 'self'",
    "connect-src 'self'",
    "base-uri 'none'",
    "form-action 'none'",
  ].join("; ");
}

/** The page with its policy as a meta tag straight after the charset, before anything can run. */
export function withPolicy(html) {
  const tag = `<meta http-equiv="Content-Security-Policy" content="${policy(html)}"/>`;
  const clean = html.replace(/<meta http-equiv="Content-Security-Policy"[^>]*>/, "");
  const out = clean.replace(/(<meta charset="utf-8"\s*\/?>)/i, `$1${tag}`);
  if (out === clean) throw new Error("no charset meta to put the content security policy after");
  return out;
}

const htmlFiles = (dir) =>
  readdirSync(dir, { recursive: true, withFileTypes: true })
    .filter((f) => f.isFile() && f.name.endsWith(".html"))
    .map((f) => resolve(f.parentPath, f.name));

/** Applies the policy to each Starlight page under `dir`, and writes the sitemap from their canonical links. */
export function finish(dir, site) {
  const urls = [site];
  let pages = 0;
  for (const file of htmlFiles(dir)) {
    const html = readFileSync(file, "utf8");
    if (!html.includes('<meta name="generator" content="Starlight')) continue;
    pages++;
    let page = withPolicy(html).replace(/(<html[^>]*?) data-theme="dark"/, "$1");
    if (file === resolve(dir, "404.html")) {
      // GitHub Pages serves it for any address with no page, so it has no address of its own.
      page = page
        .replace(/<link rel="canonical"[^>]*>|<meta property="og:url"[^>]*>/g, "")
        .replace("</title>", '</title><meta name="robots" content="noindex"/>');
    } else {
      const canonical = html.match(/<link rel="canonical" href="([^"]+)"/)?.[1];
      if (canonical) urls.push(canonical);
    }
    writeFileSync(file, page);
  }
  if (!pages) throw new Error(`no Starlight page under ${dir}, so no page got its content security policy`);
  const sitemap = [...new Set(urls)].sort().map((u) => `<url><loc>${u}</loc></url>`);
  writeFileSync(
    resolve(dir, "sitemap-0.xml"),
    `<?xml version="1.0" encoding="UTF-8"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">${sitemap.join("")}</urlset>`,
  );
}

export default function cspHash() {
  return {
    name: "csp-hash",
    hooks: {
      "astro:build:done": ({ dir }) => {
        finish(fileURLToPath(dir), "https://simonvanlierde.com/jsonvet/");
      },
    },
  };
}
