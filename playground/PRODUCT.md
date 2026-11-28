# Product

This file describes the browser playground in `playground/`, not jsonvet as a whole. What jsonvet
promises is in [docs/design.md](../docs/design.md), and how to use it in
[docs/usage.md](../docs/usage.md).

## Platform

web

## Register

Persuade, with a working checker. The page is mainly jsonvet's public face and its onboarding
page: most visitors decide here whether to add the hook, so the first screen has to show what
jsonvet does, how fast it is and how to start using it. It is not a daily tool, but everything on
it works for real: the checks, the timings and the snippets.

## Users

- **Developers deciding whether to adopt jsonvet** as a pre-commit or prek hook, a GitHub Action
  or a CI command. They arrive from the README's "Try it in your browser" link or a shared link,
  paste a file from their own repo, and want to see what jsonvet would say about it before they
  add the hook.
- **People debugging why a file fails.** A hook or CI run reported a problem, and they paste the
  file to see the line, the rule and the message next to the text, and to try fixes as they type.
- **People who want the file fixed, not only judged.** They paste a file that fails, apply the
  safe fixes or format it, and copy the result back.
- **People comparing dialects.** They keep the same text and switch between JSON, JSONC, JSON5
  and the rest to see what each one accepts.
- **People seeing what jsonvet is.** They follow a shared link, skim the page with the sample text
  and rarely paste their own. The first screen has to show what jsonvet does without any input.

Most work at a desk, on a laptop or a wide screen, often switching from an editor or a CI log. A
shared link also gets opened on a phone, so the phone layout must work, but the desk layout comes
first.

## Product Purpose

The playground runs jsonvet's own parser, built to WebAssembly, on the text you paste, and sets
it as a printer's proof: every problem gets a proofreader's mark at the exact token, and its
correction sits in the margin beside its line, with the position, rule id and, for a duplicate
key, its JSONPath and where its first copy is. Clicking a correction moves the caret to it. The
six dialects run along the top as proof stages, each with its verdict. The verdict states whether
the text parses with the selected dialect and default rules. On narrow screens it sits above the
editor controls with a button that jumps to the first problem. A fold below gives a hook config,
a check-only workflow, and command examples for the chosen dialect.

Two actions edit the proof, as the CLI's `--fix` and `jsonvet fmt` do. Fix applies the safe fixes
(duplicate keys only when set in its options, keeping the last), either all at once or one correction at a
time from its note in the margin. Format sets the text with the chosen indent, width and bracket
spacing, and normalises JSON5 when asked. Each has a settings popover that also shows the same
settings as real config (a `jsonvet.toml` `[format]` table, or the `--fix` flags), so the page
teaches how to set them in a repository. Both change the editor text so Ctrl+Z undoes them,
check again, update the share link and show the time they took.

The verdict row times the WebAssembly check and result decoding in the visitor's own browser.
The "hosts.json (2.7 MB)" sample builds a large made-up file in the page and checks it. Switching
to a sample keeps the draft, dialect and selection behind a "Restore previous text" button.
A caption above the editor says what the text is (its name and line count) and offers
"Copy share link".

A visit succeeds when the visitor sees the verdict jsonvet would give their text, understands
each problem well enough to fix it, and knows how to run the same check in their repository.

## Positioning

The playground uses the parser and default rules from the CLI, hook and Action. Project settings
and schemas can change the result outside the playground. A share link keeps the text in the URL
fragment, which browsers do not send to a server.

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
  the wasm-bindgen output in `playground/pkg/`. The docs pages are the one
  build step: `scripts/docs-site` builds `docs/*.md` with Starlight into `site/docs/` when the site
  is built, so the Markdown stays the only source. Those pages have their own policy, with hashes
  for their inline scripts ([ADR 0019](../docs/decisions/0019-docs-on-starlight.md)), and are
  usable without script. The matrix page is built the same way from `docs/conformance.json`, with
  its filters in CSS.
- Large input stays usable: above 5,000 rows the gutter and highlights turn off, above 200,000
  characters a check waits for a pause in typing, past either the other five dialects are not
  checked, and the list shows at most 200 problems.
- Only the theme choice is kept in local storage. There are no cookies, and requests load only
  the page's own files. The share link lives in the address bar (`#dialect:data`, the text deflated
  and in base64url, by `share.mjs`). The page rejects dropped files that contain invalid UTF-8.
- `playground/browser-check.sh` loads the page in headless Chrome and expects the verdict
  `Parses as JSON5, but rules found 1 error.` for the sample text.
- Fix and Format edit the text with the browser's editing commands (`insertText`), which keep
  the undo stack. Fix uses the library's `try_fix` and Format its `format`, so they give what the
  CLI gives. Options are not kept: not in the share link, not in storage.
- Timings come from `performance.now()`, which browsers coarsen; a time under one step of the
  clock is given as "under 0.1 ms" (or whatever the step is), never as an invented number.
- The 2.7 MB file is generated by `hosts.mjs` in the page: no download, no file in the
  repository, names under example.com and addresses from RFC 5737.

## Brand Commitments

- Name: jsonvet, written in lower case.
- Voice: plain, short sentences, as in the README.
- The favicon is a `{}` on a dark rounded square.

## Evidence on Hand

- The sample text in `index.html`: a config with a comment, a trailing comma and a `port` key
  repeated inside `server`, so it fails as JSON, and each dialect gives a different verdict.
- The social card `playground/og.png`: the name, one duplicate key struck and circled, and the address, rendered from `docs/og-card.html`.
- The CLI benchmarks in [the design notes](../docs/design.md#speed), linked from the timing.
- No testimonials, user counts or quotes exist; none may be invented.

The page's own words are plain: a margin item is a problem, a repeated key's first place is "key first set here",
and a fix that drops data says "changes data". "Correction" and "proof" stay as the look's metaphor, not as UI text.
Above the dialects, one line says the tool is the successor of check-json5 and what to change in a hook
(`id: check-json5` to `id: jsonvet`), with a link that opens the hook snippet. The hint under the
dialects names the common files that are JSON (`package.json`) and JSONC (`tsconfig.json`), so no line of its own is needed.
Help text is one sentence and about one line wide at 1440 px; a concept is explained once.

## Product Principles

1. The proof is the page: the text and its problems. Everything else stays quiet.
2. Say what the CLI says: same rule ids, same messages, same positions.
3. Nothing leaves the browser, and the page says so.
4. Works from a keyboard, on a phone, and in light or dark.

## Accessibility & Inclusion

- Each correction has a number in the gutter and a mark shape (struck, underlined, ringed) as
  well as a colour, so it shows in forced-colors mode and without colour vision.
- The verdict live region clears when the verdict changes. Each correction is a button that moves
  the caret. Its name starts with the visible message and includes its position, severity and related place.
- The editor text is 16px on narrow screens so iOS does not zoom on focus.
