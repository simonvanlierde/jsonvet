---
name: jsonvet playground
description: The text you paste, set as a printer's galley proof and corrected by jsonvet in the browser.
colors:
  paper: "#fbfaf5"
  ink: "#1a1916"
  faint: "#6b675e"
  dim: "#77736a"
  pen: "#c0251b"
  rule: "#dcd8cc"
  rule-strong: "#b7b2a4"
  grid: "#f0ede4"
  tint: "#f2efe5"
  active: "#f5e6d6"
  select: "#f1d6c6"
  dark-paper: "#1b1915"
  dark-lamp: "#27231b"
  dark-ink: "#ebe5d6"
  dark-faint: "#a8a191"
  dark-dim: "#8b8475"
  dark-pen: "#ff8a73"
  dark-rule: "#3a362d"
  dark-rule-strong: "#575146"
  dark-grid: "#221f1a"
  dark-tint: "#26231d"
  dark-active: "#3a2a22"
  dark-select: "#5a3427"
typography:
  prose:
    fontFamily: "Charter, Bitstream Charter, Sitka Text, Cambria, Iowan Old Style, Georgia, serif"
    fontSize: "1rem"
    lineHeight: 1.5
  verdict:
    fontFamily: "Charter, Bitstream Charter, Sitka Text, Cambria, Iowan Old Style, Georgia, serif"
    fontSize: "1.125rem"
    fontWeight: 400
  note:
    fontFamily: "Charter, Bitstream Charter, Sitka Text, Cambria, Iowan Old Style, Georgia, serif"
    fontSize: "16px"
    fontStyle: "italic"
    lineHeight: "22px"
  aside:
    fontFamily: "Charter, Bitstream Charter, Sitka Text, Cambria, Iowan Old Style, Georgia, serif"
    fontSize: "1rem"
    fontStyle: "italic"
  text:
    fontFamily: "ui-monospace, Cascadia Code, Source Code Pro, Menlo, Consolas, Liberation Mono, monospace"
    fontSize: "15px"
    lineHeight: "22px"
  text-narrow:
    fontFamily: "ui-monospace, Cascadia Code, Source Code Pro, Menlo, Consolas, Liberation Mono, monospace"
    fontSize: "16px"
    lineHeight: "22px"
  slug:
    fontFamily: "ui-monospace, Cascadia Code, Source Code Pro, Menlo, Consolas, Liberation Mono, monospace"
    fontSize: "13px"
    lineHeight: 1.6
  data:
    fontFamily: "ui-monospace, Cascadia Code, Source Code Pro, Menlo, Consolas, Liberation Mono, monospace"
    fontSize: "12.5px"
    lineHeight: 1.5
rounded:
  none: "0"
spacing:
  xs: "4px"
  sm: "6px"
  md: "10px"
  lg: "18px"
  xl: "28px"
components:
  slug:
    textColor: "{colors.faint}"
    typography: "{typography.slug}"
    padding: "20px 0 9px"
  stage:
    textColor: "{colors.faint}"
    padding: "6px 14px 7px 12px"
    height: "48px"
  stage-selected:
    backgroundColor: "{colors.tint}"
    textColor: "{colors.ink}"
  proof-text:
    backgroundColor: "{colors.paper}"
    textColor: "{colors.ink}"
    typography: "{typography.text}"
  margin-note:
    backgroundColor: "{colors.paper}"
    textColor: "{colors.pen}"
    typography: "{typography.note}"
  snippet:
    backgroundColor: "{colors.tint}"
    textColor: "{colors.ink}"
    padding: "10px 12px"
---

## Design System: jsonvet playground

Product context is in [PRODUCT.md](PRODUCT.md). The tokens above are the light theme; the
`dark-` entries replace their light namesakes under `prefers-color-scheme: dark` (or
`data-theme="dark"`), in [style.css](style.css), which is the source of truth. A sun or moon in
the colophon sets `data-theme` and [theme.js](theme.js) keeps the choice; with none kept, the
system's setting holds.

### Overview

The page is a printer's galley proof with a touch of engineering drawing. The text is set with
line numbers on warm paper; every problem gets a proofreader's mark at the exact token, and the
correction is written in the margin in italic red, joined to its line by a thin leader line with a
small arrowhead. The only drawing touches are those leader lines and a very faint grid in the
margin column. Nothing else decorates the page: no title block, no boxes, no shadows.

The register is Persuade with a working checker: the first screen shows what jsonvet says about a
real config, how long it took in the visitor's browser, and how to start.

### Colors

Warm paper, near-black ink, and one proofreader's red. Nothing else is coloured.

- **Paper**: the whole page. **Ink**: text and the selected stage's bar.
- **Faint** (5.4:1 on paper): the slug, hints, notes under the verdict, meta lines.
  **Dim**: line numbers and the middle dots between slug parts.
- **Pen** (6:1 on paper): proofreader's marks, margin notes, leader lines, the
  timings, the correction count and the numbers in the gutter. Never a fill, never decoration.
- **Rule** and **rule-strong:** hairlines between parts, link underlines. **Grid:** the margin's
  20px drafting grid, barely there. **Tint:** the selected stage and the snippet ground.
  **Active:** the rows a hovered or focused correction names.
- **Dark, under a desk lamp:** the same roles on warm dark paper, with a soft pool of
  light at the top of the page. Every pair keeps AA.
- **Forced colours:** pen becomes `Highlight`, ink `CanvasText`, rules `GrayText`.

#### Named Rules

**The One-Pen Rule.** Red marks a correction and only that: the mark, its note, its leader, its
count. Severity is told by the word ("warning") and the stages' shapes, not by a second colour.

**The Light-Drawing Rule.** The drawing world stops at the leader lines and the margin grid. No
title block, dimension lines, crop marks or heavier strokes.

### Typography

System fonts only (the CSP allows only the page's own files). A serif stack for prose and the
corrections, a mono stack for the text and every string the CLI prints.

- **Prose** (serif 1rem): the verdict, "Use this check", tabs, and the stages' names.
- **Note** (italic serif 16px / 22px): margin corrections. Its line height equals the text's, so a
  note's first line sits on its row.
- **Aside** (italic serif 1rem): the dialect hint, the syntax-stop note, the keyboard hint,
  the CI note.
- **Text** (mono 15px / 22px; 16px / 22px under 560px so iOS does not zoom): the editor, gutter
  and mark layer share it exactly, or the marks drift off their characters.
- **Slug** (mono 13px): the job line and the colophon. **Data** (mono 12.5px): stage verdicts,
  note meta lines, list numbers.

Timings, positions and counts use tabular figures.

### Layout

- A centred column up to 1240px, side padding `clamp(16px, 4vw, 48px)`.
- The slug line runs across the top, its two halves at either end over a dashed rule. They wrap
  to two lines on a phone.
- The stages are one bordered row, each stage at least 7.5rem wide; three by two under 800px.
- From 860px the proof is two columns: the text, 80 characters of the editor's font wide (`80ch` plus the gutter and padding, so it follows the font) and a margin of at least 18.5rem on the faint grid. The
  editor grows with the text (at least 8 rows, at most 70% of the window, then it scrolls inside)
  and with the notes, which keep their row or stack below the one above.
- Past 30 corrections, when the stacked notes would run past the text, above 5,000 rows, or under
  860px, the margin gives way to a numbered list under the proof, keyed to red numbers in the
  gutter.
- The verdict line, the asides, the "Use this check" fold and the colophon follow in one column.

### Elevation & Depth

Flat paper. No shadows, no rounded corners. Parts are told apart by hairlines, the dashed slug
rule and the margin's grid. The only inset strokes are the selected stage's and tab's 2px ink bar.

### Components

#### Slug

`jsonvet · proof of staging.json · set in JSON5 · 11 lines · copy link` on the left; `checked in
0.08 ms · all six in 0.4 ms · try a 2.7 MB file · back to the sample` on the right. The proof's
name is the sample's, a dropped file's, "hosts.json" with a note that it was made in the page, "a
shared text" or "your text". "copy link" and the two links on the right are buttons styled as
underlined text. The all-six timing dims while the other dialects wait for a pause in typing, and
is hidden for a long text, which only the chosen dialect checks.

#### Proof stages

A radio group of six buttons (`role="radio"`), one tab stop, arrow keys and Home/End to move and
choose. Each shows its name and, in red mono, its error and warning counts with a square or a
triangle and the first problem's position; a tick when clean; "not checked" for a long text. The
selected stage has the tint and a 2px ink bar at its foot. An italic hint under the row says what
the dialect adds and which files the CLI checks in it.

#### Actions

A row of small mono controls between the stages and the proof, set like the slug: two split
buttons, **Fix** (with its count of fixes, "Fix 2") and **Format**. The main part runs the action
with the current settings. The small part beside it, a caret with a dot and a count when any
setting differs from its default, opens a popover under the button (a bottom sheet under 560px).
Both parts are outlined controls: a 1px ink border, no fill, the tint on hover and while open.

The popover is a native `popover`, set in the same mono. A heading, then one row per setting that
applies to the dialect: a faint label and a compact control, between dashed rules. A choice (indent:
2, 4, tab) and an on/off setting (bracket spacing, drop duplicate keys, normalize JSON5, fill number arrays, JSON5 only)
are segmented controls with `role="radiogroup"`, one tab stop each and arrow keys inside; the
chosen part has the tint and a 2px ink bar at its foot, like a stage. A number (width) is a small
field. The foot shows the same settings as real config on the tint, with a copy icon in its corner: a `[format]` table
for `jsonvet.toml`, or the `jsonvet --fix` flags. At the defaults it says there is nothing to set.
Esc closes it and returns focus to the button. The settings come from one table in
`settings.mjs`, so a new setting is a new row.

Fix stays focusable at zero and says why when used. The result is an italic aside under the proof
("Fixed 2 problems: ... Ctrl+Z undoes it.", "Already formatted.", or the syntax error that stopped
Format), and the slug line gains "fixed in 0.2 ms" or "formatted in 1.1 ms". The two groups wrap.

#### Proof

A textarea over two layers that never take input: the gutter (a fixed slot for a correction's
number, then the line number, so the text never moves) and the mark layer (transparent text).
Focus draws a 2px ink rule down the proof's left edge. A dragged file gets a dashed ink outline.

#### Proofreader's marks

The signature. A caret `‸` under the line where the problem starts. A problem from any rule but
`syntax` has its token struck through (dele); a syntax error has its token underlined, as the
place checking stopped. The place a problem names (a duplicate key's first copy, the bracket an
unclosed value opened at) is ringed. A byte order mark, which has no width, gets a 3px bar. The
token is a quoted string, a run of word characters, or one character (`tokenEnd` in `proof.mjs`).

#### Margin notes

Each correction is a list item: a button with the CLI's message in italic red (JSONPaths in mono)
whose accessible name is its position, severity, message and related place, and a mono meta line
with the position, "warning" when it is one, the rule id linked to its section in docs/usage.md,
"first at 6:5" and, for a correction with a fix, an underlined "apply" in ink (not pen: the pen
marks the correction, not the cure). A fix that is not safe reads "unsafe fix" until the
"drop duplicate keys" option is on. A related place gets its own short note ("first copy 6:5"), hidden from screen
readers. A thin red leader runs from the end of the row to the note, with an arrowhead at the
text; a stacked note's leader bends at the margin's edge. Hovering or focusing a correction tints
its rows and darkens its leader. In the list form, each note carries the number shown in the
gutter.

#### Verdict

One line: the count in bold red ("1 correction.", "2 corrections, 1 warning.", "1 warning.", or
"No corrections." in ink), then what the CLI would exit with, as a live region; the first
problem's `rule · line:col` on the right in mono. Italic asides follow: where checking stopped
after a syntax error, why the marks are off for a big text, and the drop and F8 hint.

#### Use this check

A `details` fold, closed by default, with a tab list (pre-commit or prek hook, GitHub Action,
command line), arrow keys between tabs. Each panel holds a focusable, scrolling `pre` on the tint
and a copy icon in its corner, as in the options popovers. The snippets use the version from the wasm's `version()` and add
`--dialect DIALECT:*.json` for any dialect but JSON5. Each panel holds the check, then the same
with `--fix`, then the format variant (the `jsonvet-format` hook, `jsonvet fmt --check` in a
workflow step, `jsonvet fmt` on the command line), each under a `#` comment line.

#### Colophon

The privacy and JSON Schema sentence in small italic, then two mono lines: "jsonvet <version> · MIT or Apache-2.0
licence" and "Simon van Lierde · source on GitHub · docs · speed figures".

### Do's and Don'ts

- Do keep the editor, gutter and mark layer on one font size and line height.
- Do keep every string the CLI prints in mono, and prose in the serif.
- Do show measured timings only, and a bound ("under 0.1 ms") below the clock's step.
- Do build message text as DOM nodes; never pass it through `innerHTML`.
- Don't add inline style or script, external fonts or any other host: the CSP refuses them, and
  `smoke.mjs` checks it. Positions set from script go through the CSSOM.
- Don't add a second accent, a title block, boxes round the proof, rounded corners or shadows.
- Don't use the pen for anything that is not a correction.
