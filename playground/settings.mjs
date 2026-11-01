// The options of the Fix and Format actions, as one table. A row gives the control (kind, choices,
// limits), the default, the dialects it applies to, how it reaches the wasm call (`arg`) and how
// the real config spells it (`toml` for a `jsonvet.toml` key, `flag` for a CLI flag). A new option
// is a new row.

export const TOOLS = {
  fix: {
    title: "Fix",
    // The CLI flags that the changed rows make.
    config: (rows) => ["--fix", ...rows.map((r) => r.flag)].join(" "),
  },
  format: {
    title: "Format",
    // A `[format]` table for jsonvet.toml.
    config: (rows) => ["[format]", ...rows.map((r) => `${r.toml} = ${tomlValue(r, r.value)}`)].join("\n"),
  },
};

// The rows of a tool run in the order of format()'s arguments after the text and dialect.
export const SETTINGS = [
  { tool: "fix", key: "unsafe", label: "Drop duplicate keys", kind: "bool", default: false, flag: "--unsafe-fixes" },
  {
    tool: "format", key: "indent", label: "Indent", kind: "enum", choices: [["2", "2"], ["4", "4"], ["tab", "tab"]],
    default: "2", toml: "indent", arg: (v) => (v === "tab" ? 0 : Number(v)),
  },
  { tool: "format", key: "width", label: "Width", kind: "number", min: 1, max: 1000, default: 80, toml: "width", arg: (v) => v },
  { tool: "format", key: "spacing", label: "Bracket spacing", kind: "bool", default: true, toml: "bracket-spacing", arg: (v) => v },
  {
    tool: "format", key: "normalize", label: "Normalize JSON5", kind: "bool", default: false, dialects: ["json5"],
    toml: "normalize-json5", arg: (v) => v,
  },
];

const rowsOf = (tool) => SETTINGS.filter((r) => r.tool === tool);

// The default value of every option of a tool, by key.
export function defaults(tool) {
  return Object.fromEntries(rowsOf(tool).map((r) => [r.key, r.default]));
}

// The rows that apply to a dialect.
export function rowsFor(tool, dialect) {
  return rowsOf(tool).filter((r) => !r.dialects || r.dialects.includes(dialect));
}

// A control's raw value as the row's own type; an unusable one is the default. A number must be a
// whole number inside the row's limits.
export function clean(row, raw) {
  if (row.kind === "bool") return Boolean(raw);
  if (row.kind === "enum") return row.choices.some(([v]) => v === raw) ? raw : row.default;
  const n = Number(raw);
  return raw !== "" && raw != null && Number.isInteger(n) && n >= row.min && n <= row.max ? n : row.default;
}

function tomlValue(row, v) {
  if (row.kind === "enum") return /^\d+$/.test(v) ? v : JSON.stringify(v);
  return String(v);
}

// The rows that apply to the dialect and differ from the default, each with its cleaned `value`.
export function changed(tool, values, dialect) {
  return rowsFor(tool, dialect)
    .map((r) => ({ ...r, value: clean(r, values[r.key]) }))
    .filter((r) => r.value !== r.default);
}

// The real config that gives the same result as the options, or "" when all are at the defaults.
export function configText(tool, values, dialect) {
  const rows = changed(tool, values, dialect);
  return rows.length ? TOOLS[tool].config(rows) : "";
}

// The arguments of wasm's format() after the text and dialect. An option that does not apply to
// the dialect is at its default.
export function formatArgs(values, dialect) {
  const on = new Set(rowsFor("format", dialect).map((r) => r.key));
  return rowsOf("format").map((r) => r.arg(clean(r, on.has(r.key) ? values[r.key] : r.default)));
}
