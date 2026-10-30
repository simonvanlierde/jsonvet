const $ = (id) => document.getElementById(id);
const input = $("input");
const editor = $("editor");
const status = $("status");
const gutter = $("gutter");
const layer = $("layer");
const list = $("problems");
const verdict = $("verdict");
const stopped = $("stopped");
const hint = $("dialect-hint");
const strip = $("dialects");
const share = $("share");
const shareStatus = $("share-status");
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
// The files each dialect already checks, for the CI note.
const already = {
  jsonc: "`.jsonc` files are JSONC already",
  jwcc: "`.hujson` and `.jwcc` files are JWCC already",
  jsonl: "`.jsonl` and `.ndjson` files are JSON Lines already",
};
// How a related place reads in a problem's entry, as the CLI writes it after the message.
const near = { "first copy": "first at", "opened here": "opened at", "starts here": "started at" };
const DOCS = "https://github.com/simonvanlierde/jsonvet/blob/main/docs/usage.md#";
const MAX_ITEMS = 200;
// Above this many rows the gutter and line highlights are left out: they cost two DOM nodes a row
// on every check.
const MAX_ROWS = 5000;
// Above this many characters a check waits for a pause in typing, and only the chosen dialect is
// checked.
const BIG = 200_000;
// A share link longer than this is not kept in the address bar.
const MAX_LINK = 50_000;
const MAX_FILE = 10 << 20;

// pages.yml stamps the commit on main.mjs's URL, so the files it loads match the ones deployed.
const version = new URL(import.meta.url).search;
let check;
let rowStarts;
let rowOf;
let encode;
let decode;
let jsonvetVersion = "";
let loaded = false;
let dialect = "json5";
let current = [];
// Whether the text or dialect changed since the page loaded, so the address bar should hold them.
let edited = false;

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

const plural = (n, word) => `${n} ${word}${n === 1 ? "" : "s"}`;
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

// Shows a dialect's verdict in the strip: a tick, or its counts by severity. `null` means it was
// not checked.
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
  if (!parts.length) parts.push(el("span", "ok", "\u2713"));
  // Where the first problem is, so dialects that fail on different things look different.
  const at = problems[0] && `${problems[0].line}:${problems[0].column}`;
  if (at) parts.push(el("span", "none", at));
  v.replaceChildren(...parts);
  r.setAttribute("aria-label", [nameOf(d), summary(count).toLowerCase(), at && `first at ${at}`].filter(Boolean).join(", "));
}

// `cols`, when given, maps a row to the offsets in it that a problem points at, each with its
// severity and whether it is a related place: those characters get a mark.
function lines(element, texts, marks, cols) {
  element.replaceChildren(...texts.map((t, i) => {
    const div = document.createElement("div");
    if (marks.has(i + 1)) div.className = marks.get(i + 1);
    const at = cols?.get(i + 1);
    if (!at) {
      div.textContent = t;
      return div;
    }
    let last = 0;
    for (const [offset, { severity, rel }] of [...at].sort((a, b) => a[0] - b[0])) {
      if (offset < last) continue;
      // A problem at the end of a row marks the space after its last character.
      const end = offset < t.length ? offset + String.fromCodePoint(t.codePointAt(offset)).length : offset;
      const char = offset < t.length ? t.slice(offset, end) : " ";
      const cls = [severity, rel && "rel", char === "\uFEFF" && "zero"].filter(Boolean).join(" ");
      div.append(t.slice(last, offset), el("mark", cls, char));
      last = end;
    }
    div.append(t.slice(last));
    return div;
  }));
}

// Lights the rows a problem names while its entry in the list has the pointer or focus.
function light(rows, on) {
  for (const row of rows) {
    layer.children[row - 1]?.classList.toggle("active", on);
    gutter.children[row - 1]?.classList.toggle("active", on);
  }
}

function setStatus(text, cls) {
  if (status.textContent !== text) status.textContent = text;
  if (status.className !== cls) status.className = cls;
}

// Puts the caret on a problem and scrolls its row into view, as the textarea may not.
function goTo(p) {
  input.focus();
  input.setSelectionRange(p.index, p.index);
  const lh = parseFloat(getComputedStyle(input).lineHeight);
  const top = 8 + (p.row - 1) * lh;
  if (top < input.scrollTop || top + lh > input.scrollTop + input.clientHeight) {
    input.scrollTop = Math.max(0, top - input.clientHeight / 2);
  }
  sync();
}

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

function entry(p) {
  const item = el("li");
  const button = el("button");
  button.type = "button";
  const related = p.related[0];
  const relText = related && `${near[related.message] ?? related.message} ${related.line}:${related.column}`;
  button.append(
    el("span", "at", `${p.line}:${p.column}`),
    el("span", `severity ${p.severity} shape`, p.severity),
    rich(el("span", "msg"), p.message, p.path),
  );
  // The message names the path of a duplicate key already.
  if (p.path && !p.message.includes(p.path)) button.append(el("span", "path", p.path));
  if (relText) button.append(el("span", "rel-at", relText));
  button.setAttribute("aria-label", [`${p.line}:${p.column}`, p.severity, p.message, relText].filter(Boolean).join(", "));
  const rows = related ? [p.row, related.row] : [p.row];
  button.addEventListener("click", () => goTo(p));
  button.addEventListener("pointerenter", () => light(rows, true));
  button.addEventListener("pointerleave", () => light(rows, false));
  button.addEventListener("focus", () => light(rows, true));
  button.addEventListener("blur", () => light(rows, false));
  const rule = el("a", "rule", p.rule);
  rule.href = DOCS + p.rule;
  rule.title = `What ${p.rule} means`;
  item.append(button, rule);
  return item;
}

function run() {
  const text = input.value;
  let problems;
  try {
    problems = JSON.parse(check(text, dialect));
  } catch (e) {
    setStatus(String(e), "bad");
    return;
  }
  const starts = rowStarts(text);
  // The row each place sits on in the textarea, which can differ from its line number.
  for (const p of problems) {
    p.row = rowOf(starts, p.index);
    for (const r of p.related) r.row = rowOf(starts, r.index);
  }
  current = problems;
  const big = starts.length > MAX_ROWS;
  gutter.hidden = big;
  if (big) {
    gutter.replaceChildren();
    layer.replaceChildren();
  } else {
    const marks = new Map();
    const cols = new Map();
    const put = (row, offset, severity, rel) => {
      if (!cols.has(row)) cols.set(row, new Map());
      const at = cols.get(row);
      const had = at.get(offset);
      // A problem's own place wins over a related one, and an error over a warning.
      if (!had || (had.rel && !rel) || (had.rel === rel && severity === "error")) at.set(offset, { severity, rel });
    };
    for (const p of problems) {
      if (marks.get(p.row) !== "error") marks.set(p.row, p.severity);
      put(p.row, p.index - starts[p.row - 1], p.severity, false);
      for (const r of p.related) put(r.row, r.index - starts[r.row - 1], p.severity, true);
    }
    const texts = text.split("\n");
    lines(layer, texts, marks, cols);
    lines(gutter, texts.map((_, i) => String(i + 1)), marks);
  }
  const items = problems.slice(0, MAX_ITEMS).map(entry);
  if (problems.length > MAX_ITEMS) items.push(el("li", "more", `and ${problems.length - MAX_ITEMS} more`));
  list.replaceChildren(...items);
  const count = tally(problems);
  const said = summary(count);
  setStatus(
    big ? `${said} (line highlights are off above ${MAX_ROWS.toLocaleString("en")} lines)` : said,
    count.errors ? "bad" : count.warnings ? "warn" : "ok",
  );
  // The CLI exits 1 when a file has an error; warnings fail a run only past --max-warnings.
  if (count.errors) verdict.textContent = "The CLI would exit 1 on a file with this text.";
  else if (!count.warnings) verdict.textContent = "The CLI would exit 0 on a file with this text.";
  else setRich(verdict, "Only warnings: the CLI would exit 0, or 1 past `--max-warnings`.");
  // A syntax error is always the last problem: checking stops there.
  const syntax = problems.find((p) => p.rule === "syntax");
  stopped.hidden = !syntax;
  if (syntax) stopped.textContent = `Checking stopped at the syntax error on line ${syntax.line}. Fix it to see the rest.`;
  mark(dialect, problems);
  others(text, big || text.length > BIG);
  sync();
  saveSoon();
}

// Checks the text in the other five dialects after a pause in typing, or at once on the first
// check; not at all for a big text.
let othersTimer;
let first = true;
function others(text, big) {
  clearTimeout(othersTimer);
  const rest = radios.map((r) => r.dataset.dialect).filter((d) => d !== dialect);
  if (big) {
    for (const d of rest) mark(d, null);
    return;
  }
  const all = () => {
    for (const d of rest) mark(d, JSON.parse(check(text, d)));
  };
  if (first) all();
  else othersTimer = setTimeout(all, 300);
  first = false;
}

function sync() {
  layer.scrollTop = gutter.scrollTop = input.scrollTop;
  layer.scrollLeft = input.scrollLeft;
}

// The snippets that run this check in a repository, for the chosen dialect and this build.
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
  const name = nameOf(dialect);
  const note = glob
    ? `\`--dialect ${glob}\` checks every \`.json\` file as ${name}${already[dialect] ? `; ${already[dialect]}` : ""}.`
    : "Both check `.json` files as JSON5 by default, as here.";
  setRich($("ci-note"), note);
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
  if (jsonvetVersion) ci();
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

input.addEventListener("input", () => {
  edited = true;
  schedule();
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
    share.textContent = "Copied";
    setTimeout(() => (share.textContent = "Copy link"), 1500);
  } catch (e) {
    said = e instanceof RangeError ? "The text is too long for a link." : "Could not copy: copy the address bar instead.";
    share.textContent = e instanceof RangeError ? "Too long for a link" : "Copy failed";
    setTimeout(() => (share.textContent = "Copy link"), 2500);
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

async function open(hash) {
  const shared = hash ? await decode(hash) : null;
  if (!shared) return false;
  input.value = shared.text;
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
    setStatus(`${file.name} is over 10 MB: check it with the command-line tool.`, "bad");
    return;
  }
  // Not file.text(): it drops a byte order mark, which jsonvet reports.
  input.value = new TextDecoder("utf-8", { ignoreBOM: true }).decode(await file.arrayBuffer());
  input.scrollTop = input.scrollLeft = 0;
  edited = true;
  input.focus();
  schedule();
});

try {
  ({ rowStarts, rowOf } = await import(`./lines.mjs${version}`));
  ({ encode, decode } = await import(`./share.mjs${version}`));
  const wasm = await import(`./pkg/jsonvet_wasm.js${version}`);
  await wasm.default({ module_or_path: new URL(`./pkg/jsonvet_wasm_bg.wasm${version}`, import.meta.url) });
  check = wasm.check;
  jsonvetVersion = wasm.version();
  loaded = true;
  share.disabled = false;
  if (!(await open(location.hash))) select(dialect);
  // Check at once rather than on the next frame, which a background tab may not get.
  run();
} catch (e) {
  console.error(e);
  input.disabled = share.disabled = true;
  for (const r of radios) r.disabled = true;
  setStatus("Could not load the checker. Reload the page to try again.", "bad");
}
