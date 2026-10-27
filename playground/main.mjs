const input = document.getElementById("input");
const dialect = document.getElementById("dialect");
const status = document.getElementById("status");
const gutter = document.getElementById("gutter");
const layer = document.getElementById("layer");
const list = document.getElementById("problems");
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

function lines(element, texts, marks) {
  element.replaceChildren(...texts.map((t, i) => {
    const div = document.createElement("div");
    div.textContent = t;
    if (marks.has(i + 1)) div.className = marks.get(i + 1);
    return div;
  }));
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
    for (const p of problems) if (marks.get(p.row) !== "error") marks.set(p.row, p.severity);
    const texts = text.split("\n");
    lines(layer, texts, marks);
    lines(gutter, texts.map((_, i) => (marks.has(i + 1) ? `${glyph[marks.get(i + 1)]} ` : "") + (i + 1)), marks);
  }
  const items = problems.slice(0, MAX_ITEMS).map((p) => {
    const item = document.createElement("li");
    const button = document.createElement("button");
    const parts = [
      ["at", `${p.line}:${p.column}`],
      [`rule ${p.severity}`, `${p.severity} ${p.rule}`],
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
    errors ? "bad" : problems.length ? "" : "ok",
  );
  sync();
}

function sync() {
  layer.scrollTop = gutter.scrollTop = input.scrollTop;
  layer.scrollLeft = input.scrollLeft;
}

input.addEventListener("input", schedule);
input.addEventListener("scroll", sync);
dialect.addEventListener("change", schedule);
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
