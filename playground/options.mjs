// The options popover of a split button: one row per setting that applies to the dialect (from
// settings.mjs), each a label and a compact control, and the real config that gives the same
// result, with a copy button. `values` is the tool's live state: this module writes to it.
import { changed, clean, configText, rowsFor, TOOLS } from "./settings.mjs";

function el(tag, cls, text) {
  const node = document.createElement(tag);
  if (cls) node.className = cls;
  if (text !== undefined) node.textContent = text;
  return node;
}

// `onChange` runs after a value changes; `say` announces a short message to screen readers.
export function mountOptions({ tool, button, pop, values, dialect, onChange, say }) {
  const title = TOOLS[tool].title;
  const rows = pop.querySelector(".rows");
  const code = pop.querySelector(".conf-text");
  const copy = pop.querySelector(".conf-copy");
  const count = button.querySelector(".n");
  let paints = [];

  function set(key, value) {
    values[key] = value;
    paint();
    onChange();
  }

  // Two or three choices side by side, as a radio group with arrow keys between them.
  function segmented(row, labelId, choices) {
    const group = el("div", "seg");
    group.setAttribute("role", "radiogroup");
    group.setAttribute("aria-labelledby", labelId);
    const buttons = choices.map(([value, text]) => {
      const b = el("button", "", text);
      b.type = "button";
      b.setAttribute("role", "radio");
      b.addEventListener("click", () => set(row.key, value));
      return b;
    });
    group.addEventListener("keydown", (e) => {
      const step = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[e.key];
      if (!step) return;
      e.preventDefault();
      const i = buttons.indexOf(document.activeElement);
      const next = (i + step + buttons.length) % buttons.length;
      buttons[next].focus();
      set(row.key, choices[next][0]);
    });
    group.append(...buttons);
    paints.push(() => {
      const now = clean(row, values[row.key]);
      choices.forEach(([value], i) => {
        buttons[i].setAttribute("aria-checked", String(value === now));
        buttons[i].tabIndex = value === now ? 0 : -1;
      });
    });
    return group;
  }

  function number(row, labelId) {
    const input = el("input", "num");
    Object.assign(input, { type: "number", min: row.min, max: row.max, inputMode: "numeric" });
    input.setAttribute("aria-labelledby", labelId);
    input.addEventListener("input", () => {
      set(row.key, input.value);
      // Until the field is left, a width it cannot hold is marked; the default is in use meanwhile.
      input.setAttribute("aria-invalid", String(input.value === "" || clean(row, input.value) !== Number(input.value)));
    });
    // A value the field cannot hold goes back to what is in use once the field is left.
    input.addEventListener("blur", () => {
      input.value = clean(row, values[row.key]);
      input.removeAttribute("aria-invalid");
    });
    paints.push(() => {
      if (document.activeElement !== input) input.value = clean(row, values[row.key]);
    });
    return input;
  }

  // Builds the rows for the dialect again. Not on every change, which would drop the focus.
  function render() {
    paints = [];
    rows.replaceChildren(
      ...rowsFor(tool, dialect()).map((row) => {
        const line = el("div", "row");
        const id = `${tool}-${row.key}`;
        const label = el("span", "lab", row.label);
        label.id = id;
        const control =
          row.kind === "number"
            ? number(row, id)
            : segmented(row, id, row.kind === "bool" ? [[false, "off"], [true, "on"]] : row.choices);
        line.append(label, control);
        return line;
      }),
    );
    paint();
  }

  function paint() {
    for (const p of paints) p();
    const text = configText(tool, values, dialect());
    const n = changed(tool, values, dialect()).length;
    code.textContent = text || "Nothing to set: these are the defaults.";
    code.classList.toggle("none", !text);
    copy.hidden = !text;
    count.textContent = n || "";
    button.setAttribute("aria-label", `${title} options, ${n ? `${n} changed from the defaults` : "defaults"}`);
  }

  copy.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(code.textContent);
      copy.textContent = "Copied";
    } catch {
      copy.textContent = "Copy failed";
    }
    say(`${copy.textContent}.`);
    setTimeout(() => (copy.textContent = "Copy"), 1500);
  });

  // Fixed under the button; style.css keeps it inside the viewport, and the phone's sheet ignores it.
  pop.addEventListener("beforetoggle", (e) => {
    if (e.newState === "open") pop.style.setProperty("--x", `${button.closest(".split").getBoundingClientRect().left}px`);
    if (e.newState === "open") pop.style.setProperty("--y", `${button.closest(".split").getBoundingClientRect().bottom + 4}px`);
  });
  render();
  button.disabled = false;
  return { render };
}
