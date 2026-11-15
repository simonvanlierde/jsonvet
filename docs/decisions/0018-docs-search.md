# 18. Docs search is a build-time index in a script of its own

Status: accepted (2026-11-15)

## Context

The docs pages (28 of them, plus the contents page) had no search. The pages' CSP is
`default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'`: no inline script, no
`connect-src`, no wasm. [0017](0017-docs-pages-built-with-marked.md) kept the pages free of script
but the theme button's. Search needs a second one, and the pages must stay fully usable without it.

## Options

- **A. A generated `docs/search-index.js` and a 6.5 KB matcher, `playground/search.mjs`.** The
  build strips the Markdown, splits each page at its headings, and writes each section's words once.
  The index is an ES module of the site's own, loaded with `import()` on the first focus of the box.
  Needs no CSP change.
- **B. Pagefind.** Better ranking, snippets and stemming, but it fetches its index chunks and loads
  a wasm module: the pages would need `connect-src 'self'` and `'wasm-unsafe-eval'`, a bigger
  attack surface for pages that have none today, plus a post-build step and a dependency to pin.
  Argued, not measured.
- **C. A small library (lunr, MiniSearch, Fuse).** Pinned and dependency-free, but 8 to 25 KB of
  code for ranking this corpus does not need: 212 sections, whole-word, prefix and substring
  matching, headings above body.

## Decision

A. It adds no dependency, no CSP rule and no build step beyond `build.mjs`. Under
[0004](0004-measure-before-adopting-a-speed-up.md)'s cost rule, B and C would add a cost and have
to earn it; they cannot, as the matcher takes 0.34 ms a query and 1.8 ms to prepare on this host
(node 26, 212 sections).

- The index is 112 KB, 36.6 KB gzipped, and is fetched only when a reader focuses the box. The
  script is 2.6 KB gzipped, and loads on every docs page.
- A query matches when every word is a whole word, a prefix or a substring of a heading or body
  word (a hyphenated rule id is one word); one word at least must be in the section's heading or
  body, and the page title adds a little. Headings rank above body text.
- Body text is not stored, only its words, so a result shows the heading and the page, not a
  snippet.
- Without script the box stays hidden (`hidden` in the HTML) and the nav, the contents page and the
  browser's find work as before.
- The pages now carry two scripts, `theme.js` and `search.mjs`, both with `src`; `build.test.mjs`
  asserts that list.

## Consequences

The index grows with the docs: about 4 KB gzipped per decision record. Anything the Markdown
says only in an image is not searchable.

## Confirmation

`playground/search.test.mjs` tests the matcher, and `scripts/docs-site/build.test.mjs` checks that
the index holds every heading with an id on every page, that each page loads the script, and that
no page has an inline script, a `connect-src` or `wasm-unsafe-eval`. axe-core found no violations
on a docs page at 400 px and 1440 px, light and dark, with the results open (2026-11-15).

## Revisit if

The index passes about 150 KB gzipped, readers need snippets or typo tolerance, or the pages
gain a reason to allow `connect-src` anyway.
