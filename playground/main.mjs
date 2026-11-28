const $ = (id) => document.getElementById(id);
const input = $("input");
const editor = $("editor");
const proof = $("proof");
const status = $("status");
const verdictLine = $("verdict");
const verdictProblem = $("verdict-problem");
let verdictTarget;
const caretMeasure = $("caret-measure");
const dropError = $("drop-error");
// The verdict for screen readers, a second after it last changed, so typing is not interrupted.
const said = $("said");
let sayTimer;
const gutter = $("gutter");
const layer = $("layer");
const margin = $("margin");
const sheet = $("sheet");
const paths = $("paths");
const list = $("problems");
const stopped = $("stopped");
const bomNote = $("bom");
const bigNote = $("big");
const hint = $("dialect-hint");
const stages = $("dialects");
const share = $("share");
const shareLabel = share.textContent;
const shareStatus = $("share-status");
let announceFrame;
const tryBig = $("try");
const back = $("back");
const restore = $("restore");
const fixButton = $("fix");
const formatButton = $("format");
const acted = $("acted");
const radios = [...stages.querySelectorAll('[role="radio"]')];
const radio = (d) => radios.find((r) => r.dataset.dialect === d);
const nameOf = (d) => radio(d).querySelector(".name").textContent;

// What each dialect adds to JSON, a file that is one, and which files the CLI checks in it (README,
// "Dialects"), on one line each.
// Backticks mark CLI strings, shown in mono.
const hints = {
  json: "No comments and no trailing commas, as in `package.json`. The CLI uses it only with `--dialect json`.",
  "i-json": "JSON whose numbers fit a double and whose strings are valid Unicode. The CLI uses it only with `--dialect`.",
  jsonc: "JSON with comments, as in `tsconfig.json` and VS Code settings. The CLI checks `.jsonc` files this way.",
  jwcc: "JSON with comments and trailing commas, also called HuJSON. The CLI checks `.hujson` and `.jwcc` files this way.",
  json5: "Comments, trailing commas, single quotes and unquoted keys. By default, the CLI checks `.json` files as JSON5.",
  jsonl: "One JSON value per line. The CLI checks `.jsonl` and `.ndjson` files this way.",
};
// How a related place reads in a problem's entry, as the CLI writes it after the message.
const near = { "first copy": "key first set at", "opened here": "opened at", "starts here": "started at" };
// The same places in the margin, where the CLI's "first copy" would read as a term of art.
const placeLabel = { "first copy": "key first set here" };
const BOM_WHY = "RFC 8259 forbids a byte order mark in JSON; JSONC, JWCC and JSON5 readers accept one.";
const DOCS = "/jsonvet/docs/usage.html#";
const SAMPLE = input.value;
const SAMPLE_NAME = "staging.json";
const MAX_ITEMS = 200;
// Above this many rows the gutter and the marks are left out: they cost two DOM nodes a row on
// every check.
const MAX_ROWS = 5000;
// Above this many characters a check waits for a pause in typing and checks only the chosen dialect.
const BIG = 200_000;
// Past this many notes, or when they would stack past the end of the text, the margin gives way to
// a numbered list under the proof.
const MAX_NOTES = 30;
const NOTE_GAP = 6;
// The textarea's top padding, in style.css.
const PAD = 8;
const LEFT_PAD = 6;
// A share link longer than this is not kept in the address bar.
const MAX_LINK = 50_000;
const MAX_FILE = 10 << 20;
const none = new Uint32Array(0);
const wide = matchMedia("(min-width: 860px)");
$("playground").hidden = false;
function placeVerdict() {
  const focused = document.activeElement;
  if (wide.matches) proof.after(verdictLine);
  else $("editor-head").before(verdictLine);
  if (verdictLine.contains(focused)) focused.focus({ preventScroll: true });
}
placeVerdict();
wide.addEventListener("change", placeVerdict);
// The "Use this check" fold starts open on a wide screen, where it costs no first-screen room.
document.querySelector(".use").open = wide.matches;

// pages.yml stamps the commit on main.mjs's URL, so the files it loads match the ones deployed.
const version = new URL(import.meta.url).search;
let check;
let rowStarts;
let rowOf;
let encode;
let decode;
let tokenEnd;
let stack;
let formatMs;
let lineCount;
let runFix;
let runFormat;
let formatArgs;
// The options of Fix and Format, by key (settings.mjs), and the popovers that edit them. Not kept.
let fixValues = {};
let formatValues = {};
const mounted = [];
let fixesFor;
let fixSummary;
let snippets;
// What fix() found for the current text, with unsafe fixes included: the notes show which
// corrections have one, and the button counts the ones it would apply.
// `null` above BIG characters, where they are not looked for before a click.
let fixes = [];
let hosts;
let jsonvetVersion = "";
let loaded = false;
let dialect = "json5";
let current = [];
// Whether the text or dialect changed since the page loaded, so the address bar should hold them.
let edited = false;
// What the text is, for the caption above the editor and the command-line snippet.
let source = SAMPLE_NAME;
let droppedName;
// A later editor action takes precedence over a pending file read, sample or shared link.
let loadRevision = 0;
let generated = false;
let draft;
let big = false;
let rows = 1;
// The step performance.now() moves in, measured once: browsers coarsen it.
let resolution = 0.1;

function el(tag, cls, text) {
  const node = document.createElement(tag);
  if (cls) node.className = cls;
  if (text !== undefined) node.textContent = text;
  return node;
}

// Appends `text` to `node`, with each `code` span and the JSONPath `path` in mono.
function rich(node, text, path) {
  text.split(/(`[^`]*`)/).forEach((part, i) => {
    if (i % 2) return node.append(el("code", "", part.slice(1, -1)));
    const at = path ? part.indexOf(path) : -1;
    if (at < 0) return node.append(part);
    node.append(part.slice(0, at), el("code", "", path), part.slice(at + path.length));
  });
  return node;
}

function setRich(node, text) {
  node.replaceChildren();
  rich(node, text);
}

function announce(message) {
  cancelAnimationFrame(announceFrame);
  shareStatus.textContent = "";
  announceFrame = requestAnimationFrame(() => (shareStatus.textContent = message));
}

const plural = (n, word) => `${n.toLocaleString("en")} ${word}${n === 1 ? "" : "s"}`;
function tally(problems) {
  const errors = problems.filter((p) => p.severity === "error").length;
  return { errors, warnings: problems.length - errors };
}
function summary({ errors, warnings }) {
  const parts = [];
  if (errors) parts.push(plural(errors, "error"));
  if (warnings) parts.push(plural(warnings, "warning"));
  return parts.join(", ") || "No problems";
}

// Shows a dialect's verdict in the stages: a tick, or its counts by severity and where the first
// problem is. `null` means it was not checked.
function mark(d, problems) {
  const r = radio(d);
  const v = r.querySelector(".v");
  if (!problems) {
    v.replaceChildren(el("span", "none", "not checked"));
    r.setAttribute("aria-label", `${nameOf(d)}, not checked: the text is long`);
    return;
  }
  const count = tally(problems);
  const parts = [];
  if (count.errors) parts.push(el("span", "error shape", plural(count.errors, "error")));
  if (count.warnings) parts.push(el("span", "warning shape", plural(count.warnings, "warning")));
  if (!parts.length) parts.push(el("span", "ok", "✓ no problems"));
  // Where checking stops at a syntax error, else where the first problem is, so dialects that
  // fail on different things look different.
  const syntax = problems.find((p) => p.rule === "syntax");
  const first = syntax ?? problems[0];
  const at = first && `${syntax ? "stops at" : "first at"} ${first.line}:${first.column}`;
  if (at) parts.push(el("span", "none", at));
  v.replaceChildren(...parts);
  r.setAttribute("aria-label", [nameOf(d), summary(count).toLowerCase(), at].filter(Boolean).join(", "));
}

// Sets each row in `element`. `cols`, when given, maps a row to the offsets in it that a problem
// points at, each with its severity and kind: those tokens get a proofreader's mark, and the row
// ends in an empty span that a leader line starts from.
function lines(element, texts, cols, numbers) {
  element.replaceChildren(...texts.map((t, i) => {
    const div = document.createElement("div");
    const n = numbers?.get(i + 1);
    if (n) div.dataset.n = n;
    const at = cols?.get(i + 1);
    if (!at) {
      div.textContent = t;
      return div;
    }
    let last = 0;
    let prev;
    for (const [offset, { severity, kind }] of [...at].sort((a, b) => a[0] - b[0])) {
      // A mark that starts inside the one before cuts that one short, so both show.
      if (offset < last) {
        prev.mark.textContent = t.slice(prev.offset, offset);
        last = offset;
      }
      // A problem at the end of a row marks the space after its last character.
      const end = offset < t.length ? tokenEnd(t, offset) : offset;
      const text = offset < t.length ? t.slice(offset, end) : " ";
      const cls = [severity, kind, text === "\uFEFF" && "zero"].filter(Boolean).join(" ");
      const mark = el("mark", cls, text);
      div.append(t.slice(last, offset), mark);
      prev = { mark, offset };
      last = end;
    }
    div.append(t.slice(last), el("span", "eol"));
    return div;
  }));
}

// Lights the rows a problem names, and its leader, while its note has the pointer or focus.
let lit = null;
function light(rows, note, on) {
  for (const row of rows) {
    layer.children[row - 1]?.classList.toggle("active", on);
    gutter.children[row - 1]?.classList.toggle("active", on);
  }
  lit = on ? note : null;
  drawLeaders();
}

function setStatus(strong, rest) {
  const html = `${strong}|${rest}`;
  if (status.dataset.said === html) return;
  status.dataset.said = html;
  status.replaceChildren(el("b", "", strong));
  if (rest) status.append(" ", rich(document.createDocumentFragment(), rest));
  // Empty at once, so a screen reader and the page text never hold the last verdict beside the new one.
  said.textContent = "";
  clearTimeout(sayTimer);
  sayTimer = setTimeout(() => (said.textContent = status.textContent), 1000);
}

// Sets the caret and scrolls both axes: setSelectionRange can leave the caret out of view.
function goTo(p) {
  input.focus();
  input.setSelectionRange(p.index, p.index);
  const lh = lineHeight();
  const top = PAD + (p.row - 1) * lh;
  if (top < input.scrollTop || top + lh > input.scrollTop + input.clientHeight) {
    input.scrollTop = Math.max(0, top - input.clientHeight / 2);
  }
  const rowStart = p.index > 0 ? input.value.lastIndexOf("\n", p.index - 1) + 1 : 0;
  caretMeasure.textContent = input.value.slice(rowStart, p.index);
  const left = LEFT_PAD + caretMeasure.getBoundingClientRect().width;
  caretMeasure.textContent = "";
  if (left < input.scrollLeft || left >= input.scrollLeft + input.clientWidth) {
    input.scrollLeft = Math.max(0, left - input.clientWidth / 2);
  }
  sync();
}

verdictProblem.addEventListener("click", () => {
  if (verdictTarget) goTo(verdictTarget);
});

const lineHeight = () => parseFloat(getComputedStyle(input).lineHeight);

let checkFrame;
let checkTimer;
function schedule() {
  if (!loaded) return;
  cancelAnimationFrame(checkFrame);
  clearTimeout(checkTimer);
  if (input.value.length > BIG) checkTimer = setTimeout(run, 300);
  else checkFrame = requestAnimationFrame(run);
}

// A problem's note: a button that moves the caret to it, and its position and rule id.
function note(p, n, problems) {
  const item = el("li", `note ${p.severity}`);
  const button = el("button");
  button.type = "button";
  const related = p.related[0];
  const relText = related && `${near[related.message] ?? related.message} ${related.line}:${related.column}`;
  const num = el("span", "n", n);
  num.setAttribute("aria-hidden", "true");
  button.append(rich(el("span", "msg"), p.message, p.path));
  button.setAttribute("aria-label", [p.message.replaceAll("`", ""), `${p.line}:${p.column}`, p.severity, relText].filter(Boolean).join(", "));
  const rows = related ? [p.row, related.row] : [p.row];
  button.addEventListener("click", () => goTo(p));
  button.addEventListener("pointerenter", () => light(rows, item, true));
  button.addEventListener("pointerleave", () => light(rows, item, false));
  button.addEventListener("focus", () => light(rows, item, true));
  button.addEventListener("blur", () => light(rows, item, false));
  const meta = el("span", "meta");
  const rule = el("a", "rule", p.rule);
  rule.href = DOCS + p.rule;
  rule.title = `What ${p.rule} means`;
  meta.append(el("span", "", `${p.line}:${p.column}`));
  if (p.severity === "warning") meta.append(el("span", "", "warning"));
  const ruleItem = el("span");
  ruleItem.append(rule);
  meta.append(ruleItem);
  if (relText) meta.append(el("span", "", relText));
  const own = fixesFor(p, fixes ?? [], problems);
  if (own.length) {
    const slot = el("span");
    if (own.every((f) => f.safe) || fixValues.unsafe) {
      const apply = el("button", "link apply", "apply");
      apply.type = "button";
      apply.setAttribute("aria-label", `Apply the fix for ${p.line}:${p.column}, ${p.message.replaceAll("`", "")}`);
      apply.addEventListener("click", () => act("fix", own.map((f) => f.index)));
      slot.append(apply);
    } else {
      slot.append(el("span", "unsafe", "changes data: allow in Fix options"));
      slot.title = "Dropping a key changes the data, so it is off by default. Turn on \u201cDrop duplicate keys\u201d in the Fix options (the CLI flag is --unsafe-fixes).";
    }
    meta.append(slot);
  }
  item.append(num, button, meta);
  if (p.rule === "byte-order-mark") item.append(el("p", "why", BOM_WHY));
  item.dataset.row = p.row;
  return item;
}

// The related place a problem names, such as a duplicate key's first copy, as a note of its own in
// the margin. Screen readers get it in the problem's name instead.
function relNote(r) {
  const item = el("div", "note rel", placeLabel[r.message] ?? r.message);
  item.setAttribute("aria-hidden", "true");
  item.append(el("span", "meta", `${r.line}:${r.column}`));
  item.dataset.row = r.row;
  return item;
}

function run() {
  cancelAnimationFrame(checkFrame);
  clearTimeout(checkTimer);
  const text = input.value;
  let problems;
  const t0 = performance.now();
  try {
    problems = JSON.parse(check(text, dialect));
  } catch (e) {
    setStatus("Could not check the text.", String(e));
    return;
  }
  const took = performance.now() - t0;
  const starts = rowStarts(text);
  // The row each place sits on in the textarea, which can differ from its line number.
  for (const p of problems) {
    p.row = rowOf(starts, p.index);
    for (const r of p.related) r.row = rowOf(starts, r.index);
  }
  current = problems;
  // A fix is a second scan of the text: not for a text that waits for a pause in typing.
  try {
    fixes = text.length > BIG ? null : (JSON.parse(runFix(text, dialect, true, none))?.fixes ?? []);
  } catch {
    fixes = [];
  }
  rows = starts.length;
  big = rows > MAX_ROWS;
  const shown = problems.slice(0, MAX_ITEMS);
  gutter.hidden = big;
  if (big) {
    gutter.replaceChildren();
    layer.replaceChildren();
  } else {
    const cols = new Map();
    const numbers = new Map();
    const put = (row, offset, severity, kind) => {
      if (!cols.has(row)) cols.set(row, new Map());
      const at = cols.get(row);
      const had = at.get(offset);
      // A problem's own place wins over a related one, and an error over a warning.
      if (!had || (had.kind === "rel" && kind !== "rel") || (kind !== "rel" && had.severity !== "error" && severity === "error")) at.set(offset, { severity, kind });
    };
    shown.forEach((p, i) => {
      if (!numbers.has(p.row)) numbers.set(p.row, String(i + 1));
      // A syntax error marks where checking stopped; any other rule marks a token to take out.
      put(p.row, p.index - starts[p.row - 1], p.severity, p.rule === "syntax" ? "at" : "dele");
      for (const r of p.related) put(r.row, r.index - starts[r.row - 1], p.severity, "rel");
    });
    const texts = text.split("\n");
    lines(layer, texts, cols);
    lines(gutter, texts.map((_, i) => String(i + 1)), null, numbers);
  }
  const items = shown.map((p, i) => note(p, i + 1, problems));
  if (problems.length > MAX_ITEMS) items.push(el("li", "more", `and ${problems.length - MAX_ITEMS} more`));
  list.replaceChildren(...items);
  // One note for each related place, however many problems name it.
  const rel = new Map(shown.flatMap((p) => p.related).map((r) => [r.index, r]));
  sheet.replaceChildren(list, ...[...rel.values()].map(relNote));
  verdict(problems);
  $("lines").textContent = plural(lineCount(starts, text.length), "line");
  $("t1").textContent = formatMs(took, resolution);
  $("took").hidden = false;
  mark(dialect, problems);
  fixState();
  others(text, big || text.length > BIG);
  layout();
  saveSoon();
}

// A rule can flag text that parses.
function verdict(problems) {
  const count = tally(problems);
  const name = nameOf(dialect);
  const syntax = problems.find((p) => p.rule === "syntax");
  const counts = [count.errors && plural(count.errors, "error"), count.warnings && plural(count.warnings, "warning")].filter(Boolean).join(", ");
  if (syntax) {
    setStatus(`Does not parse as ${name}.`);
  } else if (count.errors) {
    setStatus(`Parses as ${name}, but rules found ${counts}.`);
  } else if (count.warnings) {
    setStatus(`Parses as ${name} with ${plural(count.warnings, "warning")}.`);
  } else {
    setStatus(`\u2713 Valid ${name}.`);
  }
  const first = verdictTarget = syntax ?? problems.find((p) => p.severity === "error") ?? problems[0];
  verdictProblem.hidden = !first;
  if (first) {
    const related = first.related[0];
    const firstPlace = related ? `, ${near[related.message] ?? related.message} ${related.line}:${related.column}` : "";
    verdictProblem.replaceChildren();
    rich(verdictProblem, `${first.message} at ${first.line}:${first.column}${firstPlace}`, first.path);
  }
  status.className = count.errors ? "bad" : count.warnings ? "warn" : "ok";
  // A syntax error is always the last problem: checking stops there.
  stopped.hidden = !syntax;
  if (syntax) stopped.textContent = `Checking stopped at the syntax error on line ${syntax.line}, so later problems are not shown.`;
  // A byte order mark only some dialects flag: say so where nothing else does.
  const bom = input.value.startsWith("\uFEFF") && !problems.some((p) => p.rule === "byte-order-mark");
  bomNote.hidden = !bom;
  if (bom) bomNote.textContent = `The text starts with a byte order mark, which JSON and I-JSON flag and ${name} does not.`;
  bigNote.hidden = !big && input.value.length <= BIG;
  if (!bigNote.hidden) {
    bigNote.textContent = big
      ? `Above ${MAX_ROWS.toLocaleString("en")} lines the marks are off, and only this dialect is checked.`
      : "The other dialects are checked only for shorter texts.";
  }
}

// Checks the text in all six dialects after a pause in typing, or at once on the first check; not
// at all for a big text.
let othersTimer;
let firstCheck = true;
function others(text, long) {
  clearTimeout(othersTimer);
  const rest = radios.map((r) => r.dataset.dialect).filter((d) => d !== dialect);
  if (long) {
    for (const d of rest) mark(d, null);
    return;
  }
  const all = () => {
    const results = radios.map((r) => check(text, r.dataset.dialect));
    radios.forEach((r, i) => r.dataset.dialect !== dialect && mark(r.dataset.dialect, JSON.parse(results[i])));
  };
  if (firstCheck) all();
  else othersTimer = setTimeout(all, 300);
  firstCheck = false;
}

// The notes that sit beside a row: one for each problem, and one for each related place.
const placed = () => [...list.children, ...sheet.children].filter((n) => n.dataset.row);

// Places the notes in the margin beside their rows, moved apart where they would overlap, or lists
// them under the proof when they do not fit; then sizes the editor to the text.
function layout() {
  const lh = lineHeight();
  const textHeight = 2 * PAD + rows * lh;
  const cap = Math.max(12 * lh, Math.round(innerHeight * 0.7));
  const notes = placed();
  notes.sort((a, b) => a.dataset.row - b.dataset.row);
  let inMargin = wide.matches && !big && list.children.length <= MAX_NOTES;
  let bottom = 0;
  proof.classList.toggle("listed", !inMargin);
  if (inMargin && notes.length) {
    const heights = notes.map((n) => n.offsetHeight);
    const room = Math.max(textHeight, 8 * lh + 2 * PAD) - PAD;
    const tops = stack(notes.map((n) => PAD + (n.dataset.row - 1) * lh), heights, NOTE_GAP, 0, room);
    bottom = tops.at(-1) + heights.at(-1) + PAD;
    if (bottom > Math.max(textHeight, cap)) {
      inMargin = false;
      proof.classList.add("listed");
    } else {
      notes.forEach((n, i) => (n.style.top = `${tops[i]}px`));
    }
  }
  if (!inMargin) {
    for (const n of notes) n.style.top = "";
    bottom = 0;
  }
  editor.style.height = `${Math.min(cap, Math.max(textHeight, bottom, 8 * lh + 2 * PAD))}px`;
  sync();
}

// Draws a thin line from the end of each marked row to its note in the margin, with an arrowhead
// at the text.
function drawLeaders() {
  if (proof.classList.contains("listed") || big) {
    paths.replaceChildren();
    return;
  }
  const box = proof.getBoundingClientRect();
  const ed = editor.getBoundingClientRect();
  const mx = margin.getBoundingClientRect().left - box.left;
  const lh = lineHeight();
  const lines = [];
  for (const n of placed()) {
    const eol = layer.children[n.dataset.row - 1]?.querySelector(".eol");
    if (!eol) continue;
    const e = eol.getBoundingClientRect();
    const row = eol.parentElement.getBoundingClientRect();
    const y = row.top + lh / 2;
    const r = n.getBoundingClientRect();
    // A note on its own row gets a straight line; one stacked lower gets a bend.
    const ry = r.top + lh / 2;
    const ny = Math.abs(ry - y) < 6 ? y : ry;
    // Rows and notes scrolled out of the proof get no line.
    if (y < ed.top || y > ed.bottom || ny < ed.top || ny > ed.bottom) continue;
    const x = Math.min(e.left - box.left + 4, ed.right - box.left - 24);
    if (x > mx - 12) continue;
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", `M${x},${y - box.top}H${mx - 10}L${mx + 2},${ny - box.top}H${r.left - box.left - 4}`);
    path.setAttribute("marker-start", "url(#arrow)");
    if (n === lit || n.contains(document.activeElement)) path.setAttribute("class", "lit");
    lines.push(path);
  }
  paths.replaceChildren(...lines);
}

function sync() {
  layer.scrollTop = gutter.scrollTop = input.scrollTop;
  layer.scrollLeft = input.scrollLeft;
  sheet.style.transform = proof.classList.contains("listed") ? "" : `translateY(${-input.scrollTop}px)`;
  drawLeaders();
}

// The snippets that run this check, for the chosen dialect and this build.
function ci() {
  const file = droppedName ?? (source === SAMPLE_NAME || generated ? source : "config.json");
  const out = snippets({ version: jsonvetVersion, dialect, file, text: input.value });
  for (const id of ["hook", "action", "cli", "cli-fix", "cli-format"]) $(id).textContent = out[id];
  $("ci-note").hidden = out.tracked;
}

// The caption above the editor: what the text is.
function caption() {
  $("name").textContent = source;
  $("made").textContent = generated ? `, ${(input.value.length / 1e6).toFixed(1)} MB, made up in this page` : "";
  tryBig.hidden = generated;
  back.hidden = input.value === SAMPLE && dialect === "json5";
  restore.hidden = !draft;
  if (jsonvetVersion) ci();
}

function select(d, focus) {
  loadRevision++;
  dialect = d;
  for (const r of radios) {
    const on = r.dataset.dialect === d;
    r.setAttribute("aria-checked", String(on));
    r.tabIndex = on ? 0 : -1;
    if (on && focus) r.focus();
  }
  setRich(hint, hints[d]);
  for (const o of mounted) o.render();
  caption();
  schedule();
}

stages.addEventListener("click", (e) => {
  const r = e.target.closest('[role="radio"]');
  if (!r || r.dataset.dialect === dialect) return;
  edited = true;
  select(r.dataset.dialect);
});
// Arrow keys move through the dialects and choose one, as in a radio group.
stages.addEventListener("keydown", (e) => {
  const i = radios.findIndex((r) => r.dataset.dialect === dialect);
  const to = { ArrowRight: i + 1, ArrowDown: i + 1, ArrowLeft: i - 1, ArrowUp: i - 1, Home: 0, End: radios.length - 1 }[e.key];
  if (to === undefined) return;
  e.preventDefault();
  edited = true;
  select(radios[(to + radios.length) % radios.length].dataset.dialect, true);
});

// Replaces the whole text, as a dropped file, the big file or the sample do.
function load(text, name, { isGenerated = false, file } = {}) {
  loadRevision++;
  input.value = text;
  input.scrollTop = input.scrollLeft = 0;
  source = name;
  droppedName = file;
  generated = isGenerated;
  acted.hidden = $("took-act").hidden = dropError.hidden = true;
  edited = !(text === SAMPLE && dialect === "json5");
  caption();
  if (!edited) history.replaceState(null, "", location.pathname + location.search);
  run();
}

input.addEventListener("input", () => {
  loadRevision++;
  edited = true;
  if (generated || source === SAMPLE_NAME && !droppedName || !input.value) {
    source = "Your text";
    droppedName = undefined;
  }
  generated = false;
  acted.hidden = $("took-act").hidden = dropError.hidden = true;
  caption();
  schedule();
});
// Text pasted over all of the proof is a new text, not the sample or the file any more.
input.addEventListener("paste", () => {
  if (input.selectionStart === 0 && input.selectionEnd === input.value.length) {
    source = "Your text";
    droppedName = undefined;
    generated = false;
  }
});
input.addEventListener("scroll", sync);
// F8 and Shift+F8 move to the next and previous problem, as in an editor.
input.addEventListener("keydown", (e) => {
  if (e.key !== "F8" || !current.length || e.altKey || e.ctrlKey || e.metaKey) return;
  e.preventDefault();
  const caret = input.selectionStart;
  const p = e.shiftKey
    ? current.findLast((q) => q.index < caret) ?? current.at(-1)
    : current.find((q) => q.index > caret) ?? current[0];
  goTo(p);
  announce(`${p.line}:${p.column} ${p.severity}: ${p.message.replaceAll("`", "")}`);
});
// Fix and Format replace the text through the editing commands, so Ctrl+Z undoes them, and the
// input event they raise re-runs the check and updates the share link.
function replaceText(text) {
  const caret = input.selectionStart;
  input.focus({ preventScroll: true });
  input.select();
  if (!document.execCommand("insertText", false, text)) {
    input.value = text;
    input.dispatchEvent(new Event("input"));
  }
  input.setSelectionRange(Math.min(caret, text.length), Math.min(caret, text.length));
  sync();
}

// What an action did, with its time beside the verdict, under the proof, and for screen readers.
function report(verb, ms, message) {
  $("act-verb").textContent = verb;
  $("t-act").textContent = formatMs(ms, resolution);
  $("took-act").hidden = false;
  setRich(acted, message);
  acted.hidden = false;
  announce(acted.textContent);
}

// `only` limits a fix to the ones a correction names; empty fixes everything the toggle allows.
function act(kind, only = none) {
  if (!loaded) return;
  const text = input.value;
  const t0 = performance.now();
  let out;
  try {
    if (kind === "fix") {
      out = JSON.parse(runFix(text, dialect, only.length > 0 || fixValues.unsafe, only));
    } else {
      out = runFormat(text, dialect, ...formatArgs(formatValues, dialect));
    }
  } catch (e) {
    const why = String(e);
    report(kind === "fix" ? "fix failed" : "not formatted", performance.now() - t0, kind === "fix" ? why : `Not formatted: ${why}`);
    return;
  }
  const ms = performance.now() - t0;
  if (kind === "fix") {
    if (!out) {
      const syntax = current.find((p) => p.rule === "syntax");
      return report("nothing fixed", ms, syntax ? `Nothing to fix yet: the syntax error on line ${syntax.line} comes first.` : "Nothing to fix.");
    }
    replaceText(out.text);
    return report("fixed", ms, `${fixSummary(out.fixes)} Ctrl+Z undoes it.`);
  }
  if (out === text) return report("checked formatting", ms, "Already formatted.");
  replaceText(out);
  report("formatted", ms, "Formatted. Ctrl+Z undoes it.");
}

// The Fix button counts the fixes it would apply; it stays focusable at zero so it can say why.
function fixState() {
  if (fixes === null) {
    fixButton.textContent = "Fix";
    fixButton.setAttribute("aria-disabled", "false");
    return;
  }
  const n = fixes.filter((f) => f.safe || fixValues.unsafe).length;
  fixButton.textContent = n ? `Fix ${n}` : "Fix";
  fixButton.setAttribute("aria-disabled", String(!n));
}
fixButton.addEventListener("click", () => act("fix"));
formatButton.addEventListener("click", () => act("format"));

wide.addEventListener("change", layout);
let resizeFrame;
addEventListener("resize", () => {
  cancelAnimationFrame(resizeFrame);
  resizeFrame = requestAnimationFrame(layout);
});

function loadSample(text, name, isGenerated) {
  if (!generated && (droppedName || input.value !== SAMPLE || (!draft && dialect !== "json5"))) {
    draft = {
      text: input.value, name: source, file: droppedName, dialect,
      start: input.selectionStart, end: input.selectionEnd, direction: input.selectionDirection,
      top: input.scrollTop, left: input.scrollLeft,
    };
  }
  select("json5");
  load(text, name, { isGenerated });
  (draft ? restore : input).focus();
}

let building = false;
tryBig.addEventListener("click", () => {
  if (building) return;
  building = true;
  const revision = ++loadRevision;
  tryBig.textContent = "building hosts.json…";
  // A frame first, so the button can say so before the page is busy.
  requestAnimationFrame(() => setTimeout(() => {
    building = false;
    tryBig.textContent = "hosts.json (2.7 MB)";
    if (revision !== loadRevision) return;
    loadSample(hosts(), "hosts.json", true);
    announce(`Loaded a made-up 2.7 MB hosts.json. ${status.textContent}`);
  }));
});
back.addEventListener("click", () => loadSample(SAMPLE, SAMPLE_NAME, false));
restore.addEventListener("click", () => {
  const { text, name, file, dialect, start, end, direction, top, left } = draft;
  draft = undefined;
  select(dialect);
  load(text, name, { file });
  input.focus({ preventScroll: true });
  input.setSelectionRange(start, end, direction);
  input.scrollTop = top;
  input.scrollLeft = left;
  sync();
});

// The address bar holds the dialect and text once they change, so the URL is a share link. The
// fragment stays in the browser: it is never sent to the server.
let saveTimer;
function saveSoon() {
  if (!edited) return;
  clearTimeout(saveTimer);
  saveTimer = setTimeout(save, 500);
}
async function save() {
  clearTimeout(saveTimer);
  if (!edited) return true;
  const text = input.value;
  const selected = dialect;
  const fragment = text.length > BIG ? "" : await encode(selected, text);
  if (!edited || input.value !== text || dialect !== selected) return save();
  const fits = fragment && fragment.length <= MAX_LINK;
  history.replaceState(null, "", fits ? `#${fragment}` : location.pathname + location.search);
  return fits;
}
share.addEventListener("click", async () => {
  let said = "Link copied.";
  try {
    if (edited && !(await save())) throw new RangeError("too long");
    await navigator.clipboard.writeText(location.href);
    share.textContent = "Link copied";
    setTimeout(() => (share.textContent = shareLabel), 1500);
  } catch (e) {
    said = e instanceof RangeError ? "The text is too long for a link." : "Could not copy: copy the address bar instead.";
    share.textContent = e instanceof RangeError ? "Too long for a link" : "Copy failed";
    setTimeout(() => (share.textContent = shareLabel), 2500);
  }
  announce(said);
});
for (const button of document.querySelectorAll(".copy")) {
  button.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText($(button.dataset.copy).textContent);
      button.title = "Copied";
      button.classList.add("done");
    } catch {
      button.title = "Copy failed";
    }
    announce(`${button.title}.`);
    setTimeout(() => {
      button.title = "Copy";
      button.classList.remove("done");
    }, 1500);
  });
}

// The snippet tabs, with arrow keys between them as in a tab list.
const tabs = [...document.querySelectorAll('[role="tab"]')];
function showTab(tab, focus) {
  for (const t of tabs) {
    const on = t === tab;
    t.setAttribute("aria-selected", String(on));
    t.tabIndex = on ? 0 : -1;
    $(t.getAttribute("aria-controls")).hidden = !on;
  }
  if (focus) tab.focus();
}
for (const tab of tabs) tab.addEventListener("click", () => showTab(tab));
tabs[0].parentElement.addEventListener("keydown", (e) => {
  const i = tabs.indexOf(document.activeElement);
  const to = { ArrowRight: i + 1, ArrowLeft: i - 1, Home: 0, End: tabs.length - 1 }[e.key];
  if (to === undefined || i < 0) return;
  e.preventDefault();
  showTab(tabs[(to + tabs.length) % tabs.length], true);
});

// "Show the hook" opens the fold on the hook tab and moves there, also where it starts closed.
$("to-hook").addEventListener("click", (e) => {
  e.preventDefault();
  document.querySelector(".use").open = true;
  showTab(tabs[0], true);
  tabs[0].scrollIntoView({ block: "center", behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "instant" : "smooth" });
});

async function open(hash) {
  const revision = ++loadRevision;
  const shared = hash ? await decode(hash) : null;
  if (revision !== loadRevision || !shared || !Object.hasOwn(hints, shared.dialect)) return false;
  input.value = shared.text;
  input.scrollTop = input.scrollLeft = 0;
  droppedName = undefined;
  source = shared.text === SAMPLE ? SAMPLE_NAME : "Shared text";
  generated = false;
  acted.hidden = $("took-act").hidden = dropError.hidden = true;
  edited = true;
  select(shared.dialect);
  return true;
}
window.addEventListener("hashchange", () => {
  if (!loaded) return;
  if (location.hash) open(location.hash);
  else {
    select("json5");
    load(SAMPLE, SAMPLE_NAME);
  }
});

// A file dropped on the text is read here, not opened by the browser.
document.addEventListener("dragover", (e) => {
  if (!e.dataTransfer?.types.includes("Files")) return;
  e.preventDefault();
  const on = editor.contains(e.target) && loaded;
  e.dataTransfer.dropEffect = on ? "copy" : "none";
  editor.classList.toggle("drop", on);
});
document.addEventListener("dragleave", (e) => {
  if (!e.relatedTarget || !editor.contains(e.relatedTarget)) editor.classList.remove("drop");
});
function rejectDrop(message) {
  dropError.textContent = message;
  dropError.hidden = false;
  announce(message);
}
document.addEventListener("drop", async (e) => {
  if (!e.dataTransfer?.types.includes("Files")) return;
  e.preventDefault();
  editor.classList.remove("drop");
  const file = e.dataTransfer.files[0];
  if (!file || !editor.contains(e.target) || !loaded) return;
  const revision = ++loadRevision;
  if (file.size > MAX_FILE) {
    rejectDrop(`${file.name} is over 10 MB. Check it with the command-line tool.`);
    return;
  }
  const bytes = await file.arrayBuffer().catch(() => null);
  if (revision !== loadRevision) return;
  if (!bytes) {
    rejectDrop(`Could not read ${file.name}. Try dropping it again.`);
    return;
  }
  let text;
  try {
    // Keep the BOM for jsonvet and reject invalid bytes before the browser can replace them.
    text = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(bytes);
  } catch {
    rejectDrop(`${file.name} contains invalid UTF-8. Check the original file with the command-line tool.`);
    return;
  }
  load(text, file.name, { file: file.name });
  input.focus();
});

// The smallest step performance.now() takes here: browsers coarsen it to resist timing attacks.
function clockStep() {
  let step = Infinity;
  let last = performance.now();
  for (let seen = 0, i = 0; seen < 10 && i < 1e6; i++) {
    const now = performance.now();
    if (now > last) {
      step = Math.min(step, now - last);
      last = now;
      seen++;
    }
  }
  return Number.isFinite(step) ? step : 1;
}

try {
  // Start the wasm download now, so it runs while the modules load.
  const wasmFile = fetch(new URL(`./pkg/jsonvet_wasm_bg.wasm${version}`, import.meta.url));
  const [lines, shareMod, proof, hostsMod, actions, settings, options, snippetsMod, wasm] = await Promise.all([
    import(`./lines.mjs${version}`),
    import(`./share.mjs${version}`),
    import(`./proof.mjs${version}`),
    import(`./hosts.mjs${version}`),
    import(`./actions.mjs${version}`),
    import(`./settings.mjs${version}`),
    import(`./options.mjs${version}`),
    import(`./snippets.mjs${version}`),
    import(`./pkg/jsonvet_wasm.js${version}`),
  ]);
  ({ rowStarts, rowOf } = lines);
  ({ encode, decode } = shareMod);
  ({ tokenEnd, stack, formatMs, lineCount } = proof);
  ({ hosts } = hostsMod);
  ({ fixesFor, fixSummary } = actions);
  ({ formatArgs } = settings);
  fixValues = settings.defaults("fix");
  formatValues = settings.defaults("format");
  const { mountOptions } = options;
  ({ snippets } = snippetsMod);
  // Without popover support (Safari before 17, Firefox before 125) the panels would sit open
  // over the page, so those browsers get the actions with their defaults and no options.
  if (!Object.hasOwn(HTMLElement.prototype, "popover")) {
    for (const id of ["fix-opts", "fix-pop", "format-opts", "format-pop"]) $(id).hidden = true;
  }
  mounted.push(
    mountOptions({ tool: "fix", button: $("fix-opts"), pop: $("fix-pop"), values: fixValues, dialect: () => dialect, onChange: () => loaded && run(), say: announce }),
    mountOptions({ tool: "format", button: $("format-opts"), pop: $("format-pop"), values: formatValues, dialect: () => dialect, onChange() {}, say: announce }),
  );
  await wasm.default({ module_or_path: wasmFile });
  check = wasm.check;
  runFix = wasm.fix;
  runFormat = wasm.format;
  jsonvetVersion = wasm.version();
  // The footer line is shared with the docs pages, which have no checker: the playground adds its version.
  document.querySelector(".colophon .slug-part span").textContent += ` ${jsonvetVersion}`;
  resolution = clockStep();
  if (!(await open(location.hash))) select(dialect);
  // Check at once rather than on the next frame, which a background tab may not get.
  run();
  loaded = true;
  input.disabled = share.disabled = tryBig.disabled = fixButton.disabled = formatButton.disabled = false;
  $("to-hook").hidden = $("use").hidden = $("scope").hidden = false;
} catch (e) {
  console.error(e);
  input.disabled = share.disabled = tryBig.disabled = fixButton.disabled = formatButton.disabled = true;
  for (const r of radios) r.disabled = true;
  setStatus("Could not load the checker.", "Reload the page to try again.");
  status.className = "bad";
  $("checker-unavailable").hidden = false;
}
