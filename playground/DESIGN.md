---
name: jsonvet playground
description: A terminal tool's page for the text you paste, checked by jsonvet in the browser.
colors:
  ground: "#f7f8f8"
  field: "#ffffff"
  ink: "#15191b"
  muted: "#57636a"
  rule: "#dce1e3"
  rule-strong: "#b9c2c6"
  accent: "#0b6575"
  active: "#d9ecf0"
  hover: "#edf1f2"
  select: "#bfe0e6"
  error: "#b42318"
  error-bg: "#fce8e5"
  warning: "#8f5300"
  warning-bg: "#fcf0da"
  ok: "#1a7a3c"
  dark-ground: "#0e1214"
  dark-field: "#13181b"
  dark-ink: "#e3e8ea"
  dark-muted: "#95a3a9"
  dark-rule: "#252e32"
  dark-rule-strong: "#3b484e"
  dark-accent: "#6cc6d6"
  dark-active: "#173a42"
  dark-hover: "#192226"
  dark-error: "#ff8f85"
  dark-error-bg: "#3a1714"
  dark-warning: "#f2b54b"
  dark-warning-bg: "#352509"
  dark-ok: "#6fd391"
typography:
  verdict:
    fontFamily: "system-ui, -apple-system, Segoe UI, Roboto, Helvetica Neue, Arial, sans-serif"
    fontSize: "1.5rem"
    fontWeight: 650
    lineHeight: 1.2
    letterSpacing: "-0.015em"
  title:
    fontFamily: "system-ui, -apple-system, Segoe UI, Roboto, Helvetica Neue, Arial, sans-serif"
    fontSize: "1rem"
    fontWeight: 650
    lineHeight: 1.4
  body:
    fontFamily: "system-ui, -apple-system, Segoe UI, Roboto, Helvetica Neue, Arial, sans-serif"
    fontSize: "0.9375rem"
    fontWeight: 400
    lineHeight: 1.45
  note:
    fontFamily: "system-ui, -apple-system, Segoe UI, Roboto, Helvetica Neue, Arial, sans-serif"
    fontSize: "0.875rem"
    lineHeight: 1.45
  small:
    fontFamily: "system-ui, -apple-system, Segoe UI, Roboto, Helvetica Neue, Arial, sans-serif"
    fontSize: "0.8125rem"
    lineHeight: 1.45
  code:
    fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, Liberation Mono, monospace"
    fontSize: "14px"
    lineHeight: "20px"
  code-narrow:
    fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, Liberation Mono, monospace"
    fontSize: "16px"
    lineHeight: "22px"
  cli:
    fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, Liberation Mono, monospace"
    fontSize: "0.8125rem"
    lineHeight: 1.6
rounded:
  none: "0"
spacing:
  xs: "4px"
  sm: "8px"
  md: "12px"
  lg: "16px"
  xl: "28px"
components:
  header:
    backgroundColor: "{colors.ground}"
    textColor: "{colors.ink}"
    padding: "16px 0 4px"
  dialect-option:
    backgroundColor: "{colors.field}"
    textColor: "{colors.muted}"
    rounded: "{rounded.none}"
    padding: "6px 10px 7px"
    height: "48px"
  dialect-option-selected:
    backgroundColor: "{colors.field}"
    textColor: "{colors.ink}"
  dialect-option-hover:
    backgroundColor: "{colors.hover}"
    textColor: "{colors.ink}"
  editor:
    backgroundColor: "{colors.field}"
    textColor: "{colors.ink}"
    typography: "{typography.code}"
  problem:
    textColor: "{colors.ink}"
    padding: "9px 8px"
  problem-hover:
    backgroundColor: "{colors.hover}"
  problem-focus:
    backgroundColor: "{colors.active}"
  snippet:
    backgroundColor: "{colors.field}"
    textColor: "{colors.ink}"
    typography: "{typography.cli}"
    padding: "10px 12px"
---

## Design System: jsonvet playground

Product context is in [PRODUCT.md](PRODUCT.md). The tokens above are the light theme; the
`dark-` entries replace their light namesakes under `prefers-color-scheme: dark`, in
[style.css](style.css), which is the source of truth.

### Overview

The page looks like a good terminal tool: one quiet ground, hairline rules, square corners, and
the text and the verdicts carry it. A thin header names the page on the ground itself. Below it,
the dialect strip shows how each of the six dialects judges the current text, the editor holds the
text, and the report beside it says what jsonvet says: the count, the CLI's exit code, each problem
and how to run the same check in CI.

It is an Operate surface. Character comes from precise details, not from fields of colour: mono for
every string the CLI prints, printed severity shapes, the column marks in the text, and one accent.

### Colors

Restrained: neutrals do the work, one accent, and state colours only where a problem is.

- **Ground and field:** the page sits on the ground (`#f7f8f8`); the editor, the dialect strip and
  the snippets are fields (`#ffffff`) inside a hairline. Nothing else has a fill.
- **Ink, muted, rule, rule-strong:** text, secondary text, hairlines between parts, and the outline
  of a control or a field. Muted text keeps 4.5:1 on the ground and on a field.
- **Accent** (`#0b6575`, `#6cc6d6` in dark): links, the caret, focus rings, the `{}` mark, and the
  bar under the selected dialect. Never a fill.
- **Active** and **hover:** the rows a focused or hovered problem names, a focused problem, and the
  hover ground of buttons.
- **State:** error red, warning amber, ok green, each with a pale ground for row tints.

#### Named Rules

**The State-Only Rule.** Red, amber and green mean error, warning and pass, nowhere else.

**The One-Accent Rule.** The accent marks what you can act on or where you are. It is never
decoration and never a background.

### Typography

One system sans for the interface, one system mono for the text and for anything the CLI prints:
positions, severities, rule ids, JSONPaths, options, file extensions, the snippets, and each
backticked word in a message. No web fonts: the CSP allows only the page's own files.

#### Hierarchy

- **Verdict** (1.5rem, 650): the count, such as "2 errors, 1 warning". The only large type.
- **Title** (1rem, 650): the header's name. Small on purpose: the page is the tool, not a banner.
- **Body** (0.9375rem / 1.45): problem messages, the CLI line, the syntax-stop note.
- **Note** (0.875rem): the tagline, the dialect hint, dialect names.
- **Small** (0.8125rem): the keyboard hint, CI headings and note, the footer.
- **Code** (14px / 20px, 16px / 22px under 560px so iOS does not zoom): the editor, gutter and
  overlay share it exactly, or the marks drift off their characters.
- **CLI** (0.8125rem mono): positions, severities, rule ids and the strip's verdicts.

Positions, counts and line numbers use tabular figures.

### Layout

- A centred column up to 1360px, side padding `clamp(16px, 3vw, 32px)`.
- The header is one line on wide screens: name, tagline, and "Copy link" on the right. It wraps on
  a phone.
- The dialect strip sits on a two-column grid, its "Dialect" label on the left and the hint under
  the strip. Six equal cells from 720px, three by two below.
- From 1024px the editor and the report sit side by side, the report 21 to 27rem and scrolling on
  its own. When the window is also at least 600px tall, the body is a flex column of exactly the
  window's height, so the page itself never scrolls; the editor and report take what is left after
  the header, strip and footer.
- Under 1024px the report stacks under a 45vh editor. Nothing is wider than the screen at 320px:
  long lines scroll inside the editor and the snippets.
- The problem list is one grid with shared columns (position, severity, rule), so they line up
  across rows; the message, path and related place run under severity and rule.

### Elevation & Depth

Flat. No shadows. Parts are told apart by the ground, fields and one-pixel rules. The only inset
strokes are the selected dialect's bar, the column marks and focus outlines.

### Shapes

Square corners everywhere. Severity marks are printed shapes in the state colour: a square for an
error, a triangle for a warning, a dot (or a tick in the strip) for a pass. The gutter keeps a
✖ or ▲ glyph on problem lines, so they show in forced-colors mode too.

### Components

#### Header

`{}` in mono in the accent, "jsonvet playground", the tagline in muted, and "Copy link": an outlined
button that copies the share link and says "Copied" for a moment. A hidden live region announces
the result.

#### Dialect strip

A radio group of six buttons (`role="radio"`), one tab stop, arrow keys and Home/End to move and
choose. Each cell shows the dialect's name and its verdict on the current text in mono: a tick, or
the error and warning counts with their shapes, then the first problem's position. The chosen
dialect is checked at once; the other five after a 300ms pause in typing, and not at all past
5,000 rows or 200,000 characters, where they say "not checked". The selected cell has ink text and
a 3px accent bar at its foot. The hint under the strip says in plain words what the dialect adds
and which files the CLI checks in it, with CLI strings in mono.

#### Editor

A textarea over two layers that never take input: the gutter (a fixed glyph slot, then the line
number, so the text never shifts when a mark comes or goes) and the overlay (row tints and column
marks, with transparent text). Focus draws a 2px accent outline round the field. A file dragged over
the field gets a dashed accent outline; dropping it reads it in the page.

#### Column marks

The signature. The character a problem points at gets a 3px underline in the state colour; the
related place it names (a duplicate key's first copy, the bracket an unclosed value opened at)
gets a 1px box in the same colour. A byte order mark, which has no width, gets a 3px bar. Hovering
or focusing a problem lights both rows in the active colour.

#### Report

The verdict (`#status`, a live region: "1 error", "2 errors, 1 warning", "No problems"; the browser
check reads it), the CLI's exit code, and, after a syntax error, a muted line saying checking stopped
there. Then the problem list: each entry is a button that moves the caret to the problem, with the
rule id as a link to its section in docs/usage.md beside it. A related place reads as the CLI
writes it, "first at 6:5". Under the list, the F8 hint.

#### Run this in CI

Two snippets, the pre-commit or prek hook and the GitHub Action, for the chosen dialect and the
version of the build the page runs (`version()` from the wasm). For any dialect but JSON5 they add
`--dialect DIALECT:*.json`. Each has a Copy text button; the snippet itself is a focusable,
horizontally scrolling `pre`.

#### Footer

The privacy sentence and the CLI link on the left; "jsonvet · MIT licence · Simon van Lierde ·
source" on the right. Small, muted, links in the accent.

### Do's and Don'ts

- Do keep the editor, gutter and overlay on one font size and line height.
- Do keep every string the CLI prints in mono, and everything else in the sans.
- Do mark state with a shape as well as a colour.
- Do build message text as DOM nodes; never pass it through `innerHTML`.
- Don't add inline style or script, external fonts or any other host: the CSP refuses them, and
  `smoke.mjs` checks it.
- Don't bring back a coloured band or any colour field: the ground and the hairlines are the frame.
- Don't round corners or add shadows.
- Don't use the accent or the state colours for decoration.
