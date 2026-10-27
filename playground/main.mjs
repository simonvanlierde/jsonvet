import init, { check } from "./pkg/jsonvet_wasm.js";
import { indexOf, lineEnd } from "./lines.mjs";

const input = document.getElementById("input");
const dialect = document.getElementById("dialect");
const status = document.getElementById("status");
const gutter = document.getElementById("gutter");
const layer = document.getElementById("layer");
const list = document.getElementById("problems");
// Marks a problem line by a glyph as well as a color, so it shows in forced-colors mode too.
const glyph = { error: "\u2716", warning: "\u25b2" };
const MAX_ITEMS = 200;

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
  if (queued) return;
  queued = true;
  requestAnimationFrame(() => {
    queued = false;
    run();
  });
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
  const marks = new Map();
  for (const p of problems) if (marks.get(p.line) !== "error") marks.set(p.line, p.severity);
  const texts = text.split(lineEnd(dialect.value));
  lines(layer, texts, marks);
  lines(gutter, texts.map((_, i) => (marks.has(i + 1) ? `${glyph[marks.get(i + 1)]} ` : "") + (i + 1)), marks);
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
    button.title = `${p.severity}: go to line ${p.line}, column ${p.column}`;
    button.addEventListener("click", () => {
      const i = indexOf(input.value, p.line, p.column, dialect.value);
      input.focus();
      input.setSelectionRange(i, i);
      // Bring the line into view, as the textarea may not scroll to the caret itself.
      const lh = parseFloat(getComputedStyle(input).lineHeight);
      const top = 8 + (p.line - 1) * lh;
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
  setStatus(
    problems.length === 0 ? "No problems" : `${problems.length} problem${problems.length === 1 ? "" : "s"}`,
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
  await init();
  run();
} catch (e) {
  setStatus(`Could not load jsonvet: ${e}`, "bad");
}
