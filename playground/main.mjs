const $ = (id) => document.getElementById(id);
const input = $("input");
const editor = $("editor");
const proof = $("proof");
const status = $("status");
const gutter = $("gutter");
const layer = $("layer");
const margin = $("margin");
const sheet = $("sheet");
const leaders = $("leaders");
const list = $("problems");
const first = $("first");
const stopped = $("stopped");
const bigNote = $("big");
const hint = $("dialect-hint");
const strip = $("dialects");
const share = $("share");
const shareStatus = $("share-status");
const tryBig = $("try");
const back = $("back");
const radios = [...strip.querySelectorAll('[role="radio"]')];
const radio = (d) => radios.find((r) => r.dataset.dialect === d);
const nameOf = (d) => radio(d).querySelector(".name").textContent;

// What each dialect adds to JSON, and which files the CLI checks in it (README, "Dialects").
// Backticks mark CLI strings, shown in mono.
const hints = {
  json: "Plain JSON: no comments, no trailing commas. The CLI uses it only when asked, with `--dialect` or `--strict`.",
  "i-json": "Plain JSON with stricter content: every number fits a double, every string is valid Unicode. The CLI uses it only when asked, with `--dialect`.",
  jsonc: "JSON with `//` and `/* */` comments. The CLI checks `.jsonc`, `.code-workspace` and `.code-snippets` files as JSONC.",
  jwcc: "JSON with comments and trailing commas, also called HuJSON. The CLI checks `.hujson` and `.jwcc` files as JWCC.",
  json5: "Comments, trailing commas, single quotes, unquoted keys and more. The CLI checks `.json`, `.json5` and any extension with no other dialect as JSON5.",
  jsonl: "One JSON value per line. The CLI checks `.jsonl` and `.ndjson` files as JSON Lines.",
};
// The files each dialect already checks, for the note under the snippets.
const already = {
  jsonc: "`.jsonc` files are JSONC already",
  jwcc: "`.hujson` and `.jwcc` files are JWCC already",
  jsonl: "`.jsonl` and `.ndjson` files are JSON Lines already",
};
// How a related place reads in a problem's entry, as the CLI writes it after the message.
const near = { "first copy": "first at", "opened here": "opened at", "starts here": "started at" };
const DOCS = "https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md#";
const SAMPLE = input.value;
const SAMPLE_NAME = "staging.json";
const MAX_ITEMS = 200;
// Above this many rows the gutter and the marks are left out: they cost two DOM nodes a row on
// every check.
const MAX_ROWS = 5000;
// Above this many characters a check waits for a pause in typing, and only the chosen dialect is
// checked.
const BIG = 200_000;
// Past this many notes, or when they would stack past the end of the text, the margin gives way to
// a numbered list under the proof.
const MAX_NOTES = 30;
const NOTE_GAP = 6;
// The textarea's top padding, in style.css.
const PAD = 8;
// A share link longer than this is not kept in the address bar.
const MAX_LINK = 50_000;
const MAX_FILE = 10 << 20;
const wide = matchMedia("(min-width: 860px)");

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
let hosts;
let jsonvetVersion = "";
let loaded = false;
let dialect = "json5";
let current = [];
// Whether the text or dialect changed since the page loaded, so the address bar should hold them.
let edited = false;
// What the proof is of, for the slug line and the command-line snippet.
let source = SAMPLE_NAME;
let generated = false;
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

// Shows a dialect's verdict in the strip: a tick, or its counts by severity and where the first
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
  if (count.errors) parts.push(el("span", "error shape", count.errors));
  if (count.warnings) parts.push(el("span", "warning shape", count.warnings));
  if (!parts.length) parts.push(el("span", "ok", "✓"));
  // Where the first problem is, so dialects that fail on different things look different.
  const at = problems[0] && `${problems[0].line}:${problems[0].column}`;
  if (at) parts.push(el("span", "none", at));
  v.replaceChildren(...parts);
  r.setAttribute("aria-label", [nameOf(d), summary(count).toLowerCase(), at && `first at ${at}`].filter(Boolean).join(", "));
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
    for (const [offset, { severity, kind }] of [...at].sort((a, b) => a[0] - b[0])) {
      if (offset < last) continue;
      // A problem at the end of a row marks the space after its last character.
      const end = offset < t.length ? tokenEnd(t, offset) : offset;
      const text = offset < t.length ? t.slice(offset, end) : " ";
      const cls = [severity, kind, text === "\uFEFF" && "zero"].filter(Boolean).join(" ");
      div.append(t.slice(last, offset), el("mark", cls, text));
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
}

// Puts the caret on a problem and scrolls its row into view, as the textarea may not.
function goTo(p) {
  input.focus();
  input.setSelectionRange(p.index, p.index);
  const lh = lineHeight();
  const top = PAD + (p.row - 1) * lh;
  if (top < input.scrollTop || top + lh > input.scrollTop + input.clientHeight) {
    input.scrollTop = Math.max(0, top - input.clientHeight / 2);
  }
  sync();
}

const lineHeight = () => parseFloat(getComputedStyle(input).lineHeight);

// Input events can come several to a frame: check once per frame.
let queued = false;
function schedule() {
  if (!loaded || queued) return;
  queued = true;
  requestAnimationFrame(() => {
    if (input.value.length > BIG) setTimeout(go, 300);
    else go();
  });
}

function go() {
  queued = false;
  run();
}

// A problem's note: a button that moves the caret to it, and its position and rule id.
function note(p, n) {
  const item = el("li", `note ${p.severity}`);
  const button = el("button");
  button.type = "button";
  const related = p.related[0];
  const relText = related && `${near[related.message] ?? related.message} ${related.line}:${related.column}`;
  const num = el("span", "n", n);
  num.setAttribute("aria-hidden", "true");
  button.append(num, rich(el("span", "msg"), p.message, p.path));
  button.setAttribute("aria-label", [`${p.line}:${p.column}`, p.severity, p.message, relText].filter(Boolean).join(", "));
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
  meta.append(el("span"));
  meta.lastChild.append(rule);
  if (relText) meta.append(el("span", "", relText));
  item.append(button, meta);
  item.dataset.row = p.row;
  return item;
}

// The related place a problem names, such as a duplicate key's first copy, as a note of its own in
// the margin. Screen readers get it in the problem's name instead.
function relNote(r) {
  const item = el("div", "note rel", r.message);
  item.setAttribute("aria-hidden", "true");
  item.append(el("span", "meta", `${r.line}:${r.column}`));
  item.dataset.row = r.row;
  return item;
}

function run() {
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
      if (!had || (had.kind === "rel" && kind !== "rel") || (had.kind === kind && severity === "error")) at.set(offset, { severity, kind });
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
  const items = shown.map((p, i) => note(p, i + 1));
  if (problems.length > MAX_ITEMS) items.push(el("li", "more", `and ${problems.length - MAX_ITEMS} more`));
  list.replaceChildren(...items);
  // One note for each related place, however many problems name it.
  const rel = new Map(shown.flatMap((p) => p.related.slice(0, 1)).map((r) => [r.index, r]));
  sheet.replaceChildren(list, ...[...rel.values()].map(relNote));
  verdict(problems);
  $("lines").textContent = plural(lineCount(starts, text.length), "line");
  $("t1").textContent = formatMs(took, resolution);
  $("took").hidden = false;
  mark(dialect, problems);
  others(text, big || text.length > BIG);
  layout();
  saveSoon();
}

// The verdict line under the proof, worded for the CLI's exit code: 1 when a file has an error;
// warnings fail a run only past --max-warnings.
function verdict(problems) {
  const count = tally(problems);
  if (count.errors) {
    const strong = [plural(count.errors, "correction"), count.warnings && plural(count.warnings, "warning")].filter(Boolean).join(", ");
    setStatus(`${strong}.`, "The CLI would exit 1 on this file.");
  } else if (count.warnings) {
    setStatus(`${plural(count.warnings, "warning")}.`, "The CLI would exit 0, or 1 past `--max-warnings`.");
  } else {
    setStatus("No corrections.", "The CLI would exit 0 on this file.");
  }
  status.className = count.errors ? "bad" : count.warnings ? "warn" : "ok";
  first.textContent = problems[0] ? `${problems[0].rule} · ${problems[0].line}:${problems[0].column}` : "";
  // A syntax error is always the last problem: checking stops there.
  const syntax = problems.find((p) => p.rule === "syntax");
  stopped.hidden = !syntax;
  if (syntax) stopped.textContent = `Checking stopped at the syntax error on line ${syntax.line}. Fix it to see the rest.`;
  bigNote.hidden = !big && input.value.length <= BIG;
  if (!bigNote.hidden) {
    bigNote.textContent = big
      ? `The marks are off above ${MAX_ROWS.toLocaleString("en")} lines, and the other dialects are checked only for shorter texts: the problems are listed under the proof.`
      : "The other dialects are checked only for shorter texts.";
  }
}

// Checks the text in all six dialects after a pause in typing, or at once on the first check; not
// at all for a big text. The six are timed together.
let othersTimer;
let firstCheck = true;
function others(text, long) {
  clearTimeout(othersTimer);
  const rest = radios.map((r) => r.dataset.dialect).filter((d) => d !== dialect);
  $("took-all").classList.add("stale");
  if (long) {
    for (const d of rest) mark(d, null);
    $("took-all").hidden = true;
    return;
  }
  const all = () => {
    const t0 = performance.now();
    const results = radios.map((r) => check(text, r.dataset.dialect));
    $("t6").textContent = formatMs(performance.now() - t0, resolution);
    $("took-all").hidden = false;
    $("took-all").classList.remove("stale");
    radios.forEach((r, i) => r.dataset.dialect !== dialect && mark(r.dataset.dialect, JSON.parse(results[i])));
  };
  if (firstCheck) all();
  else othersTimer = setTimeout(all, 300);
  firstCheck = false;
}

// The notes that sit beside a row: one for each problem, and one for each related place.
const placed = () => [...list.children, ...sheet.children].filter((n) => n.dataset.row);

// Places the notes in the margin beside their rows, stacked where they would overlap, or lists
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
    const tops = stack(notes.map((n) => PAD + (n.dataset.row - 1) * lh), heights, NOTE_GAP);
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
    leaders.replaceChildren();
    return;
  }
  const box = proof.getBoundingClientRect();
  const ed = editor.getBoundingClientRect();
  const mx = margin.getBoundingClientRect().left - box.left;
  const paths = [];
  for (const n of placed()) {
    const eol = layer.children[n.dataset.row - 1]?.querySelector(".eol");
    if (!eol) continue;
    const e = eol.getBoundingClientRect();
    const row = eol.parentElement.getBoundingClientRect();
    const y = row.top + lineHeight() / 2;
    const r = n.getBoundingClientRect();
    // A note on its own row gets a straight line; one stacked lower gets a bend.
    const ny = Math.abs(r.top + lineHeight() / 2 - y) < 6 ? y : r.top + lineHeight() / 2;
    // Rows and notes scrolled out of the proof get no line.
    if (y < ed.top || y > ed.bottom || ny < ed.top || ny > ed.bottom) continue;
    const x = Math.min(e.left - box.left + 4, ed.right - box.left - 24);
    if (x > mx - 12) continue;
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", `M${x},${y - box.top}H${mx - 10}L${mx + 2},${ny - box.top}H${r.left - box.left - 4}`);
    path.setAttribute("marker-start", "url(#arrow)");
    if (n === lit || n.contains(document.activeElement)) path.setAttribute("class", "lit");
    paths.push(path);
  }
  leaders.replaceChildren(arrowhead(), ...paths);
}

function arrowhead() {
  const ns = "http://www.w3.org/2000/svg";
  const defs = document.createElementNS(ns, "defs");
  const marker = document.createElementNS(ns, "marker");
  for (const [k, v] of Object.entries({ id: "arrow", viewBox: "0 0 8 6", refX: "1", refY: "3", markerWidth: "8", markerHeight: "6", orient: "auto", markerUnits: "userSpaceOnUse" })) {
    marker.setAttribute(k, v);
  }
  const head = document.createElementNS(ns, "path");
  head.setAttribute("d", "M0,3L8,0V6Z");
  marker.append(head);
  defs.append(marker);
  return defs;
}

function sync() {
  layer.scrollTop = gutter.scrollTop = input.scrollTop;
  layer.scrollLeft = input.scrollLeft;
  sheet.style.transform = proof.classList.contains("listed") ? "" : `translateY(${-input.scrollTop}px)`;
  drawLeaders();
}

// The snippets that run this check, for the chosen dialect and this build.
function ci() {
  const glob = dialect === "json5" ? "" : `${dialect}:*.json`;
  $("hook").textContent = [
    "- repo: https://github.com/simonvanlierde/jsonvet",
    `  rev: v${jsonvetVersion}`,
    "  hooks:",
    "    - id: jsonvet",
    ...(glob ? [`      args: [--dialect, "${glob}"]`] : []),
  ].join("\n");
  $("action").textContent = [
    `- uses: simonvanlierde/jsonvet@v${jsonvetVersion.split(".")[0]}`,
    ...(glob ? ["  with:", `    args: --dialect ${glob}`] : []),
  ].join("\n");
  const file = /^[\w.-]+\.json$/.test(source) ? source : "config.json";
  $("cli").textContent = ["uvx jsonvet", glob && `--dialect '${glob}'`, file].filter(Boolean).join(" ");
  const name = nameOf(dialect);
  const note = glob
    ? `\`--dialect ${glob}\` checks every \`.json\` file as ${name}${already[dialect] ? `; ${already[dialect]}` : ""}.`
    : "All three check `.json` files as JSON5 by default, as here.";
  setRich($("ci-note"), note);
}

function slug() {
  $("name").textContent = source;
  $("made").textContent = generated ? `, ${(input.value.length / 1e6).toFixed(1)} MB of made-up hosts built in this page` : "";
  $("set").textContent = nameOf(dialect);
  $("try-wrap").hidden = generated;
  $("back-wrap").hidden = input.value === SAMPLE;
  if (jsonvetVersion) ci();
}

function select(d, focus) {
  if (!hints[d]) return;
  dialect = d;
  for (const r of radios) {
    const on = r.dataset.dialect === d;
    r.setAttribute("aria-checked", String(on));
    r.tabIndex = on ? 0 : -1;
    if (on && focus) r.focus();
  }
  setRich(hint, hints[d]);
  slug();
  schedule();
}

strip.addEventListener("click", (e) => {
  const r = e.target.closest('[role="radio"]');
  if (!r || r.dataset.dialect === dialect) return;
  edited = true;
  select(r.dataset.dialect);
});
// Arrow keys move through the dialects and choose one, as in a radio group.
strip.addEventListener("keydown", (e) => {
  const i = radios.findIndex((r) => r.dataset.dialect === dialect);
  const to = { ArrowRight: i + 1, ArrowDown: i + 1, ArrowLeft: i - 1, ArrowUp: i - 1, Home: 0, End: radios.length - 1 }[e.key];
  if (to === undefined) return;
  e.preventDefault();
  edited = true;
  select(radios[(to + radios.length) % radios.length].dataset.dialect, true);
});

// Replaces the whole text, as a dropped file, the big file or the sample do.
function load(text, name, isGenerated) {
  input.value = text;
  input.scrollTop = input.scrollLeft = 0;
  source = name;
  generated = isGenerated;
  edited = name !== SAMPLE_NAME;
  slug();
  if (!edited) history.replaceState(null, "", location.pathname + location.search);
  run();
}

input.addEventListener("input", () => {
  edited = true;
  if (generated || !input.value) {
    generated = false;
    source = input.value ? source : "your text";
  }
  slug();
  schedule();
});
// Text pasted over all of the proof is a new text, not the sample or the file any more.
input.addEventListener("paste", () => {
  if (input.selectionStart === 0 && input.selectionEnd === input.value.length) {
    source = "your text";
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
  shareStatus.textContent = `${p.line}:${p.column} ${p.severity}: ${p.message}`;
});
wide.addEventListener("change", layout);
let resizeFrame;
addEventListener("resize", () => {
  cancelAnimationFrame(resizeFrame);
  resizeFrame = requestAnimationFrame(layout);
});

tryBig.addEventListener("click", () => {
  tryBig.textContent = "building the file…";
  // A frame first, so the button can say so before the page is busy.
  requestAnimationFrame(() => setTimeout(() => {
    load(hosts(), "hosts.json", true);
    tryBig.textContent = "try a 2.7 MB file";
    shareStatus.textContent = `Loaded a made-up 2.7 MB hosts.json. ${status.textContent}`;
  }));
});
back.addEventListener("click", () => {
  load(SAMPLE, SAMPLE_NAME, false);
  input.focus();
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
  const text = input.value;
  const fragment = text.length > BIG ? "" : await encode(dialect, text);
  if (input.value !== text) return false;
  const fits = fragment && fragment.length <= MAX_LINK;
  history.replaceState(null, "", fits ? `#${fragment}` : location.pathname + location.search);
  return fits;
}
share.addEventListener("click", async () => {
  let said = "Link copied.";
  try {
    if (edited && !(await save())) throw new RangeError("too long");
    await navigator.clipboard.writeText(location.href);
    share.textContent = "link copied";
    setTimeout(() => (share.textContent = "copy link"), 1500);
  } catch (e) {
    said = e instanceof RangeError ? "The text is too long for a link." : "Could not copy: copy the address bar instead.";
    share.textContent = e instanceof RangeError ? "too long for a link" : "copy failed";
    setTimeout(() => (share.textContent = "copy link"), 2500);
  }
  shareStatus.textContent = said;
});
for (const button of document.querySelectorAll(".copy")) {
  button.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText($(button.dataset.copy).textContent);
      button.textContent = "Copied";
    } catch {
      button.textContent = "Copy failed";
    }
    setTimeout(() => (button.textContent = "Copy"), 1500);
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

async function open(hash) {
  const shared = hash ? await decode(hash) : null;
  if (!shared) return false;
  input.value = shared.text;
  source = shared.text === SAMPLE ? SAMPLE_NAME : "a shared text";
  generated = false;
  edited = true;
  select(shared.dialect);
  return true;
}
window.addEventListener("hashchange", () => open(location.hash));

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
document.addEventListener("drop", async (e) => {
  if (!e.dataTransfer?.types.includes("Files")) return;
  e.preventDefault();
  editor.classList.remove("drop");
  const file = e.dataTransfer.files[0];
  if (!file || !editor.contains(e.target) || !loaded) return;
  if (file.size > MAX_FILE) {
    setStatus(`${file.name} is over 10 MB.`, "Check it with the command-line tool.");
    status.className = "bad";
    return;
  }
  // Not file.text(): it drops a byte order mark, which jsonvet reports.
  load(new TextDecoder("utf-8", { ignoreBOM: true }).decode(await file.arrayBuffer()), file.name, false);
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
  ({ rowStarts, rowOf } = await import(`./lines.mjs${version}`));
  ({ encode, decode } = await import(`./share.mjs${version}`));
  ({ tokenEnd, stack, formatMs, lineCount } = await import(`./proof.mjs${version}`));
  ({ hosts } = await import(`./hosts.mjs${version}`));
  const wasm = await import(`./pkg/jsonvet_wasm.js${version}`);
  await wasm.default({ module_or_path: new URL(`./pkg/jsonvet_wasm_bg.wasm${version}`, import.meta.url) });
  check = wasm.check;
  jsonvetVersion = wasm.version();
  $("version").textContent = ` ${jsonvetVersion}`;
  resolution = clockStep();
  loaded = true;
  share.disabled = tryBig.disabled = false;
  if (!(await open(location.hash))) select(dialect);
  // Check at once rather than on the next frame, which a background tab may not get.
  run();
} catch (e) {
  console.error(e);
  input.disabled = share.disabled = tryBig.disabled = true;
  for (const r of radios) r.disabled = true;
  setStatus("Could not load the checker.", "Reload the page to try again.");
  status.className = "bad";
}
