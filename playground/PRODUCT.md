# Product

This file describes the browser playground in `playground/`, not jsonvet as a whole. What jsonvet
promises is in [docs/design.md](../docs/design.md), and how to use it in
[docs/usage.md](../docs/usage.md). Facts marked *(inferred)* come from the code and docs, not from
a confirmed interview.

## Platform

web

## Users

- **Developers deciding whether to adopt jsonvet** as a pre-commit or prek hook, a GitHub Action
  or a CI command. They arrive from the README's "Try it in your browser" link or a shared link,
  paste a file from their own repo, and want to see what jsonvet would say about it before they
  add the hook. *(inferred)*
- **People debugging why a file fails.** A hook or CI run reported a problem, and they paste the
  file to see the line, the rule and the message next to the text, and to try fixes as they type.
  *(inferred)*

Both work at a desk, usually on a laptop or a wide screen, often switching from an editor or a CI
log. A shared link also gets opened on a phone. *(inferred)*

## Product Purpose

The playground runs jsonvet's own parser, built to WebAssembly, on the text you paste, and shows
every problem as you type: the line in the gutter, a highlight on the row, and a list with the
position, severity, rule id, message and, for a duplicate key, its JSONPath. Clicking a problem
moves the caret to it.

A visit succeeds when the visitor sees the verdict jsonvet would give their text, understands
each problem well enough to fix it, and knows where the command-line tool is.

## Positioning

It is the same parser and the same messages as the CLI, hook and Action, not a separate JavaScript
linter, so what the playground says is what CI will say for that dialect. The text never leaves
the page.

## Operating Context

- Dialects: JSON, I-JSON, JSONC, JWCC (HuJSON), JSON5 and JSON Lines. JSON5 is the default, as it
  is for `.json` files in the CLI.
- The rules and their default severities per dialect are the CLI's (README, "Rules").
- JSON Schema checks (`--schema`) exist only in the command-line tool.
- Deployed as a static page on GitHub Pages by `.github/workflows/pages.yml`, at
  <https://simonvanlierde.com/jsonvet/>.

## Capabilities and Constraints

- A strict Content-Security-Policy: no inline script or style, no external hosts, no network
  access but the page's own files. `playground/smoke.mjs` enforces it.
- No runtime dependencies and no build step for the page: plain HTML, CSS and ES modules beside
  the wasm-bindgen output in `playground/pkg/`.
- Large input stays usable: above 5,000 rows the gutter and highlights turn off, above 200,000
  characters a check waits for a pause in typing, and the list shows at most 200 problems.
- `playground/browser-check.sh` loads the page in headless Chrome and expects the status
  `1 problem` for the sample text.

## Brand Commitments

- Name: jsonvet, written in lower case.
- Voice: plain, short sentences, as in the README.
- The favicon is a `{}` on a dark rounded square.

## Evidence on Hand

- The sample text in `index.html` (a JSON5 config with a repeated `port` key).
- The social card `playground/og.png`, a screenshot of the page itself.
- No testimonials, user counts or quotes exist; none may be invented.

## Product Principles

1. The text and its problems are the page. Everything else stays out of their way.
2. Say what the CLI says: same rule ids, same messages, same positions.
3. Nothing leaves the browser, and the page says so.
4. Works from a keyboard, on a phone, and in light or dark.

## Accessibility & Inclusion

- Problem lines are marked by a glyph as well as a colour, so they show in forced-colors mode.
- The status line is a live region; the problem list is a list of buttons that move the caret.
- The editor text is 16px on narrow screens so iOS does not zoom on focus.
