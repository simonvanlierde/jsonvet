// The docs site: docs/*.md as Starlight pages under /jsonvet/docs/, built into ../../site next to
// the playground's own files, which are copied in as they are (publicDir). `build.format: "file"`
// keeps the published addresses: docs/usage.html, not docs/usage/.

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { satteri } from "@astrojs/markdown-satteri";
import starlight from "@astrojs/starlight";
import { defineConfig } from "astro/config";
import cspHash from "./csp-hash.mjs";
import { NAV, pageOf, REPO, SITE } from "./lib.mjs";
import { docBoxes, docLinks } from "./markdown.mjs";

const playground = resolve(import.meta.dirname, "../../playground");
// The social card is the playground's: the same image, alt text and cache-busting hash.
const card = readFileSync(resolve(playground, "og.png"));
const alt = readFileSync(resolve(playground, "index.html"), "utf8").match(/og:image:alt" content="([^"]*)"/)[1];
const cardUrl = `${SITE}og.png?v=${createHash("sha256").update(card).digest("hex").slice(0, 8)}`;
const meta = (attrs) => ({ tag: "meta", attrs });

// The old pages opened the search with "/", as GitHub does; Starlight's own key is Ctrl+K.
const slash = `addEventListener("keydown",e=>{if(e.key!=="/"||e.ctrlKey||e.metaKey||e.altKey||e.target.closest("input,textarea,select,[contenteditable]"))return;e.preventDefault();document.querySelector("button[data-open-modal]")?.click()})`;

export default defineConfig({
  site: "https://simonvanlierde.com",
  base: "/jsonvet",
  outDir: process.env.DOCS_OUT ?? "../../site", // DOCS_OUT: the tests build elsewhere
  publicDir: "../../playground",
  build: { format: "file", inlineStylesheets: "never" },
  markdown: {
    // No highlighting: the docs' code is mostly command lines and output, and each token would
    // carry an inline style.
    syntaxHighlight: false,
    processor: satteri({ mdastPlugins: [docLinks], hastPlugins: [docBoxes] }),
  },
  integrations: [
    starlight({
      title: "jsonvet docs",
      description: "The jsonvet reference, design notes, conformance matrix and decision records.",
      favicon: "/favicon.svg",
      social: [{ icon: "github", label: "jsonvet on GitHub", href: REPO }],
      sidebar: [
        { label: "Docs", link: "/docs/index.html" },
        ...NAV.map(([src, label]) => ({ label, link: `/docs/${pageOf(src).replace(/\/$/, "/index.html")}` })),
      ],
      // Expressive Code adds an inline script and a `style` attribute to every token.
      expressiveCode: false,
      customCss: ["./src/styles/jsonvet.css"],
      components: { Footer: "./src/components/Footer.astro" },
      head: [
        meta({ property: "og:image", content: cardUrl }),
        meta({ property: "og:image:width", content: "1200" }),
        meta({ property: "og:image:height", content: "630" }),
        meta({ property: "og:image:alt", content: alt }),
        meta({ name: "twitter:card", content: "summary_large_image" }),
        { tag: "script", content: slash },
        // The search needs script; without it the button would do nothing.
        { tag: "noscript", content: "<style>site-search{display:none}</style>" },
      ],
    }),
    cspHash(),
  ],
});
