const input = document.getElementById("input");
const dialect = document.getElementById("dialect");
const status = document.getElementById("status");
const gutter = document.getElementById("gutter");
const layer = document.getElementById("layer");
const list = document.getElementById("problems");
const verdict = document.getElementById("verdict");
const hint = document.getElementById("dialect-hint");
// What each dialect adds to JSON, and which files the CLI checks in it (README, "Dialects").
const hints = {
  json: "Plain JSON, RFC 8259. The CLI uses it when you choose it with --dialect or --strict.",
  "i-json": "JSON with stricter content, RFC 7493. The CLI uses it when you choose it with --dialect.",
  jsonc: "JSON with // and /* */ comments. The CLI checks .jsonc, .code-workspace and .code-snippets files as JSONC.",
  jwcc: "JSON with comments and trailing commas (HuJSON). The CLI checks .hujson and .jwcc files as JWCC.",
  json5: "The JSON5 1.0.0 grammar. The CLI checks .json, .json5 and any extension it has no other dialect for as JSON5.",
  jsonl: "One JSON value per line. The CLI checks .jsonl and .ndjson files as JSON Lines.",
};
// Marks a problem line by a glyph as well as a color, so it shows in forced-colors mode too.
const glyph = { error: "\u2716", warning: "\u25b2" };
const MAX_ITEMS = 200;
// Above this many rows the gutter and line highlights are left out: they cost two DOM nodes a row
// on every check.
const MAX_ROWS = 5000;
// Above this many characters a check waits for a pause in typing.
const BIG = 200_000;

// pages.yml stamps the commit on main.mjs's URL, so the files it loads match the ones deployed.
const version = new URL(import.meta.url).search;
let check;
let rowStarts;
let rowOf;
let loaded = false;

// `cols`, when given, maps a row to the offsets in it that a problem points at, each with its
// severity: those characters get a mark.
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
    for (const [offset, severity] of [...at].sort((a, b) => a[0] - b[0])) {
      if (offset < last) continue;
      // A problem at the end of a row marks the space after its last character.
      const end = offset < t.length ? offset + String.fromCodePoint(t.codePointAt(offset)).length : offset;
      const mark = document.createElement("mark");
      mark.className = severity;
      mark.textContent = offset < t.length ? t.slice(offset, end) : " ";
      div.append(t.slice(last, offset), mark);
      last = end;
    }
    div.append(t.slice(last));
    return div;
  }));
}

// Lights a problem's row in the text while its entry in the list has the pointer or focus.
function light(row, on) {
  layer.children[row - 1]?.classList.toggle("active", on);
  gutter.children[row - 1]?.classList.toggle("active", on);
}

function setStatus(text, cls) {
  if (status.textContent !== text) status.textContent = text;
  if (status.className !== cls) status.className = cls;
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

function run() {
  const text = input.value;
  let problems;
  try {
    problems = JSON.parse(check(text, dialect.value));
  } catch (e) {
    setStatus(String(e), "bad");
    return;
  }
  const starts = rowStarts(text);
  // The row each problem sits on in the textarea, which can differ from its line number.
  for (const p of problems) p.row = rowOf(starts, p.index);
  const big = starts.length > MAX_ROWS;
  gutter.hidden = big;
  if (big) {
    gutter.replaceChildren();
    layer.replaceChildren();
  } else {
    const marks = new Map();
    const cols = new Map();
    for (const p of problems) {
      if (marks.get(p.row) !== "error") marks.set(p.row, p.severity);
      if (!cols.has(p.row)) cols.set(p.row, new Map());
      const at = cols.get(p.row);
      const offset = p.index - starts[p.row - 1];
      if (at.get(offset) !== "error") at.set(offset, p.severity);
    }
    const texts = text.split("\n");
    lines(layer, texts, marks, cols);
    lines(gutter, texts.map((_, i) => (marks.has(i + 1) ? `${glyph[marks.get(i + 1)]} ` : "") + (i + 1)), marks);
  }
  const items = problems.slice(0, MAX_ITEMS).map((p) => {
    const item = document.createElement("li");
    const button = document.createElement("button");
    const parts = [
      ["at", `${p.line}:${p.column}`],
      [`severity ${p.severity}`, p.severity],
      ["rule", p.rule],
      ["msg", p.message],
    ];
    // The message names the path of a duplicate key already.
    if (p.path && !p.message.includes(p.path)) parts.push(["path", p.path]);
    for (const [cls, value] of parts) {
      const span = document.createElement("span");
      span.className = cls;
      span.textContent = value;
      button.append(span);
    }
    button.addEventListener("click", () => {
      input.focus();
      input.setSelectionRange(p.index, p.index);
      // Bring the line into view, as the textarea may not scroll to the caret itself.
      const lh = parseFloat(getComputedStyle(input).lineHeight);
      const top = 8 + (p.row - 1) * lh;
      if (top < input.scrollTop || top + lh > input.scrollTop + input.clientHeight) {
        input.scrollTop = Math.max(0, top - input.clientHeight / 2);
      }
      sync();
    });
    button.addEventListener("pointerenter", () => light(p.row, true));
    button.addEventListener("pointerleave", () => light(p.row, false));
    button.addEventListener("focus", () => light(p.row, true));
    button.addEventListener("blur", () => light(p.row, false));
    item.append(button);
    return item;
  });
  if (problems.length > MAX_ITEMS) {
    const more = document.createElement("li");
    more.className = "more";
    more.textContent = `and ${problems.length - MAX_ITEMS} more`;
    items.push(more);
  }
  list.replaceChildren(...items);
  const errors = problems.filter((p) => p.severity === "error").length;
  const count = problems.length === 0 ? "No problems" : `${problems.length} problem${problems.length === 1 ? "" : "s"}`;
  setStatus(
    big ? `${count} (line highlights are off above ${MAX_ROWS} lines)` : count,
    errors ? "bad" : problems.length ? "warn" : "ok",
  );
  // The CLI exits 1 when a file has an error; warnings fail a run only past --max-warnings.
  if (errors) verdict.textContent = "The CLI would exit 1 on a file with this text.";
  else if (problems.length === 0) verdict.textContent = "The CLI would exit 0 on a file with this text.";
  else {
    const option = document.createElement("code");
    option.textContent = "--max-warnings";
    verdict.replaceChildren("Only warnings: the CLI would exit 0, or 1 past ", option, ".");
  }
  sync();
}

function sync() {
  layer.scrollTop = gutter.scrollTop = input.scrollTop;
  layer.scrollLeft = input.scrollLeft;
}

input.addEventListener("input", schedule);
input.addEventListener("scroll", sync);
dialect.addEventListener("change", () => {
  hint.textContent = hints[dialect.value];
  schedule();
});
try {
  ({ rowStarts, rowOf } = await import(`./lines.mjs${version}`));
  const wasm = await import(`./pkg/jsonvet_wasm.js${version}`);
  await wasm.default({ module_or_path: new URL(`./pkg/jsonvet_wasm_bg.wasm${version}`, import.meta.url) });
  check = wasm.check;
  loaded = true;
  run();
} catch (e) {
  console.error(e);
  input.disabled = dialect.disabled = true;
  setStatus("Could not load the checker. Reload the page to try again.", "bad");
}
