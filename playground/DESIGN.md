---
name: jsonvet playground
description: A data sheet for the text you paste, checked by jsonvet in the browser.
colors:
  paper: "#f4f6f6"
  sheet: "#ffffff"
  side: "#f8fafa"
  ink: "#14191c"
  muted: "#56636b"
  rule: "#d3dadd"
  band: "#0e4a57"
  band-ink: "#f2f8f9"
  band-muted: "#bcd6db"
  accent: "#0b6575"
  active: "#d5ebef"
  hover: "#edf2f3"
  select: "#b9dde4"
  error: "#b42318"
  error-bg: "#fde7e4"
  warning: "#9a5a00"
  warning-bg: "#fdf0d8"
  ok: "#1a7a3c"
  dark-paper: "#0b1012"
  dark-sheet: "#11181b"
  dark-side: "#0e1416"
  dark-ink: "#e2e9eb"
  dark-muted: "#94a4ab"
  dark-rule: "#253238"
  dark-band: "#0f3f4a"
  dark-accent: "#6cc6d6"
  dark-error: "#ff8f85"
  dark-warning: "#f2b54b"
  dark-ok: "#6fd391"
typography:
  title:
    fontFamily: "system-ui, -apple-system, Segoe UI, Roboto, Helvetica Neue, Arial, sans-serif"
    fontSize: "1.25rem"
    fontWeight: 700
    lineHeight: 1.3
    letterSpacing: "-0.01em"
  verdict:
    fontFamily: "system-ui, -apple-system, Segoe UI, Roboto, Helvetica Neue, Arial, sans-serif"
    fontSize: "1.75rem"
    fontWeight: 700
    lineHeight: 1.15
    letterSpacing: "-0.02em"
  body:
    fontFamily: "system-ui, -apple-system, Segoe UI, Roboto, Helvetica Neue, Arial, sans-serif"
    fontSize: "1rem"
    fontWeight: 400
    lineHeight: 1.5
  note:
    fontFamily: "system-ui, -apple-system, Segoe UI, Roboto, Helvetica Neue, Arial, sans-serif"
    fontSize: "0.875rem"
    lineHeight: 1.4
  code:
    fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, Liberation Mono, monospace"
    fontSize: "14px"
    lineHeight: "20px"
  code-narrow:
    fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, Liberation Mono, monospace"
    fontSize: "16px"
    lineHeight: "22px"
rounded:
  none: "0"
spacing:
  xs: "4px"
  sm: "8px"
  md: "12px"
  lg: "16px"
  xl: "24px"
components:
  band:
    backgroundColor: "{colors.band}"
    textColor: "{colors.band-ink}"
    padding: "14px 32px"
  select:
    backgroundColor: "{colors.sheet}"
    textColor: "{colors.ink}"
    rounded: "{rounded.none}"
    padding: "6px 10px"
    height: "40px"
  editor:
    backgroundColor: "{colors.sheet}"
    textColor: "{colors.ink}"
    typography: "{typography.code}"
  report:
    backgroundColor: "{colors.side}"
    textColor: "{colors.ink}"
    padding: "16px 20px 20px"
  problem:
    textColor: "{colors.ink}"
    padding: "10px 8px"
  problem-hover:
    backgroundColor: "{colors.hover}"
  problem-focus:
    backgroundColor: "{colors.active}"
---

## Design System: jsonvet playground

Product context is in [PRODUCT.md](PRODUCT.md). The tokens above are the light theme; the
`dark-` entries replace their light namesakes under `prefers-color-scheme: dark`, in
[style.css](style.css), which is the source of truth.

### Overview

The page is a technical data sheet for the pasted text. A petrol title band carries the name and
the source link. Below it, one ruled frame holds the text on the left and the report on the
right: the verdict set large, what the CLI would do, and a keyed list of problems. Every problem
is tied to its place: a mark in the gutter, a tint on the row and an underline on the exact
character the position names. Nothing decorates; colour appears where a problem is.

It is an Operate surface. Familiar controls (a native select, a textarea, a list of buttons)
carry the task, and the world shows only in type, palette, density and the column mark.

### Colors

Restrained, with one committed field.

- **Band** (`#0e4a57`): the title band only. It is the one colour field that is not a state.
- **Ink, muted, rule:** text, secondary text and one-pixel rules on a cool white sheet. Muted
  text keeps 4.5:1 on every ground it sits on.
- **Accent** (`#0b6575`): links, the caret and focus rings. Never a fill.
- **Active** (`#d5ebef`): the row a focused or hovered problem points at, and a focused problem.
- **State:** error red, warning amber and ok green, each with a pale ground for row tints. They
  appear only on problems, the verdict and the gutter.

#### Named Rules

**The State-Only Rule.** Red, amber and green mean error, warning and pass, nowhere else.

### Typography

One system sans for the interface, one system mono for the text and anything the CLI prints (the
position, severity, rule id and JSONPath). No web fonts: the CSP allows only the page's own files,
and the platform faces load instantly.

#### Hierarchy

- **Verdict** (1.75rem, 700): the problem count. The only large type on the page.
- **Title** (1.25rem, 700): the band.
- **Body** (1rem / 1.5): the intro, problem messages (0.9375rem).
- **Note** (0.875rem): the dialect hint, the footer, the CLI codes in the list.
- **Code** (14px / 20px, 16px / 22px under 560px so iOS does not zoom): the editor, gutter and
  overlay share it exactly, or the marks drift off their characters.

Positions and line numbers use tabular figures.

### Layout

- A centred column up to 1280px, side padding `clamp(16px, 3vw, 32px)`.
- From 1024px the frame has two columns: the editor takes the rest, the report 20 to 28rem.
  The frame fills the viewport height (`clamp(420px, 100dvh - 190px, 1000px)`) and the report
  scrolls on its own.
- Under 1024px the report stacks under a 45vh editor. Under 560px the band stacks, the select
  fills its row at 44px high, and the code size steps up.
- The problem list is one grid with shared columns (position, severity, rule), so the columns line
  up across rows; the message and path run under severity and rule.

### Elevation & Depth

Flat. No shadows. Regions are told apart by flat grounds (band, paper, sheet, side) and one-pixel
rules. The only inset strokes are the column mark and focus outlines.

### Shapes

Square corners everywhere, the select included. Severity marks are printed shapes in the state
colour: a square for an error, a triangle for a warning, a dot for a pass. The `{}` mark in the
band repeats the favicon.

### Components

#### Title band

Name with the `{}` mark, the one-line intro, and "Source and CLI" on the right (under it on
phones). Links in the band use the band ink; focus rings there are band ink too.

#### Dialect picker

A native select with a label and a hint beside it. The hint says what the dialect adds to JSON
and which files the CLI checks in it, from the README's dialect table; `main.mjs` swaps it on
change.

#### Editor

A textarea over two layers that never take input: the gutter (line numbers, a ✖ or ▲ glyph on
problem lines, kept for forced-colors mode) and the overlay (row tints and column marks, with
transparent text). Row tints run the full width and meet the gutter's. Focus draws a 2px accent
ring inside the frame.

#### Column mark

The signature. The character a problem's position names gets a 3px underline in the state colour.
A problem at the end of a line marks the space after it. Hovering or focusing a problem in the list
lights its row in the active colour.

#### Report

The verdict (`#status`, a live region, its text kept as "1 problem" / "No problems" for the
browser check), one line on what the CLI would exit with, and the problem list. Each problem is a
button: hover takes the hover ground, focus an accent outline and the active ground. Clicking
moves the caret to the problem. Above 200 problems a last row says how many more.

### Do's and Don'ts

- Do keep the editor, gutter and overlay on one font size and line height.
- Do keep every string the CLI prints in mono, and everything else in the sans.
- Do mark state with a shape as well as a colour.
- Don't add inline style or script, external fonts or any other host: the CSP refuses them, and
  `smoke.mjs` checks it.
- Don't round corners or add shadows.
- Don't use the band colour or the state colours for decoration.
