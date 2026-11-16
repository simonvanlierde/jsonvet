# 19. The docs are built with Starlight

Status: accepted (2026-11-16)

Supersedes [0017](0017-docs-pages-built-with-marked.md) and [0018](0018-docs-search.md). Their
policy was no inline script, no inline style, no `connect-src`, no wasm. The docs pages give that up
for a theme, a sidebar and a search that other people maintain. The playground keeps the strict
policy.

## Context

The marked build was 335 lines of renderer, nav, label and search-index code, plus a matcher and a
stylesheet of our own, for pages that look like the playground. Each new need (a contents list, a
search, a 404, social tags) was more of it. The docs were also the one part of the site that mixed
a build step into a page with no build step.

## Options

Measured on 2026-11-16 (node 24.22, this host, load about 4; Astro 7.4.5, Starlight 0.42.8, pinned
exactly), the first two only:

- **A. Keep marked** (0017, 0018). Visitors load 3.3 KB of script gzipped on a docs page.
- **B. Starlight**, with the pages in the playground's tokens through `customCss` and no forked
  components but the footer. Brings a sidebar, a mobile menu, a table of contents, a theme switch
  and Pagefind search, which are the parts A grew by hand.
- **C. mdBook, Docusaurus, VitePress.** Argued, not measured. mdBook has its own theme and search
  script and a second toolchain. Docusaurus and VitePress are Vue or React apps shipping a client
  router, more script than B for static pages.

| docs page, gzipped | A (marked) | B (Starlight) |
| --- | --- | --- |
| build, 30 pages | 0.22 s | 1.9 s, cold or warm |
| usage: HTML, CSS, JS | 16.2, 9.3, 3.3 KB (28.8 KB) | 21.6, 13.3, 31.3 KB (66.1 KB) |
| contents page, total | 14.8 KB | 49.6 KB |
| matrix, total | 49.0 KB | 85.9 KB |
| inline scripts per page | 0 | 10, 9 distinct hashes site-wide |
| `node_modules` | 0.2 MB | 211 MB |

Of the 31 KB of script on usage, 27 KB is the search UI, which Starlight loads when the page is
idle. The index itself (768 KB on disk) is fetched in chunks only when a reader searches. axe-core
found no violations at 400 px and 1440 px, light and dark, on the docs pages, the matrix, the
decisions' contents and the 404.

## Decision

B, built with `build.format: "file"` so every address of the old pages still serves a file:
`docs/usage.html`, `docs/decisions/0017-….html`, and `docs/` and `docs/decisions/` through routes
named `index`. The Markdown stays the only source: a Satteri plugin rewrites links between docs
and to GitHub as 0017's renderer did, and the h1 becomes the page's title.

The docs pages' policy is `default-src 'none'; script-src 'self' 'wasm-unsafe-eval' <sha256 of each
inline script>; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self';
connect-src 'self'; base-uri 'none'; form-action 'none'`. An Astro integration
(`scripts/docs-site/csp-hash.mjs`) hashes every inline script after the build and writes the meta
tag, because Astro's own hashing is not reliable for Starlight's pages (withastro/astro#16664,
#18606, #18607).

- `'unsafe-inline'` for styles is the cost: Astro and Starlight write `style` attributes (about 200 on
  the reference page) that a hash cannot cover. A style cannot run script, and the pages take no
  input but the search box.
- `'wasm-unsafe-eval'` and `connect-src 'self'` are Pagefind's: it fetches its index chunks and
  compiles a wasm module. Both stay on this site's own files, and nothing loads from another origin.
- The playground page is unchanged and byte-identical to `playground/index.html`: its own files
  only, no inline script or style.
- Without script the pages read as before: the system's theme holds (the build drops Starlight's
  `data-theme="dark"`, which its script otherwise corrects), the sidebar opens from the menu button
  (a popover), and the search button is hidden.
- Expressive Code is off and code is not highlighted, as before: it adds an inline script and a
  `style` attribute to every token, and a scroll box with no name that axe flags. Code and wide
  tables scroll in focusable boxes named by their heading, as 0017's renderer made them.

## Consequences

Worse than A, and accepted: five times the build time and a 66 KB docs page against 29 KB. The
search ranks a reader's `duplicate-key` below the decision record that names it, where the matcher
of 0018 put the reference section first. Focus goes to the page, not the search button, when the
search closes. The header bar and footer line are the playground's (`playground/header.html`,
`footer.html`, `chrome.css`), and the theme is its `theme.js`, so one choice (`theme`) serves both;
Starlight's own picker and its inline theme script are replaced. The 404 page and the matrix are not searchable (the matrix is excluded on
purpose: 800 KB of table).

Better: the nav, mobile menu, table of contents, previous and next links, search with snippets and
the sitemap are not ours to keep. Pinned versions update through Renovate after seven days, and the
tests build the site on each update, so a changed inline script is hashed again, not allowed
by a stale list.

## Confirmation

`scripts/docs-site/build.test.mjs` builds the site into a folder of its own and checks that every
address of the old pages serves a file, that every link, asset and anchor resolves (the rules'
anchors in `docs/usage.html` that the CLI and the playground link to among them), that every inline
script of a docs page is in its policy and no source is a host or `*`, that the playground page
keeps its policy and has no inline script or style, and that no page says it is dark before its
script runs. CI runs it in the `vscode` job and in `pages.yml` before a deploy;
`playground/browser-check.sh` loads a docs page in Chrome under its policy and checks that the
theme and search scripts ran.

## Revisit if

Starlight's inline scripts change faster than the build hashes them, Pagefind needs a source the
policy refuses, a docs page passes 100 KB gzipped, or an advisory hits Astro or Starlight.
