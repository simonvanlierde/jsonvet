// Pure helpers for the Fix and Format actions: the wasm arguments from the controls, the fixes a
// correction names, and the sentence that says what an action did.

// The controls' values as format()'s arguments after the text and dialect: spaces per level (0
// for a tab), width, bracket spacing and JSON5 normalising. A width that is not a whole number
// from 1 to 1000 is 80, the CLI's default; normalising applies to JSON5 only.
export function formatArgs({ indent, width, spacing, normalize }, dialect) {
  const n = Number(width);
  return [
    indent === "tab" ? 0 : indent === "4" ? 4 : 2,
    Number.isInteger(n) && n >= 1 && n <= 1000 ? n : 80,
    Boolean(spacing),
    Boolean(normalize) && dialect === "json5",
  ];
}

// The fixes (from fix()) that belong to a problem: those at its own place or at the place it
// names, such as a duplicate key's first copy, which is the member the fix drops.
export function fixesFor(problem, fixes) {
  // A trailing comma is reported at the closing bracket; its fix is the comma before it.
  if (problem.rule === "syntax" && problem.message?.startsWith("trailing comma")) {
    const comma = fixes.findLast((f) => f.rule === "syntax" && f.index < problem.index);
    return comma ? [comma] : [];
  }
  const at = [problem.index, ...problem.related.map((r) => r.index)];
  return fixes.filter((f) => f.rule === problem.rule && at.includes(f.index));
}

const KINDS = {
  "byte-order-mark": ["byte order mark", "byte order marks"],
  "missing-final-newline": ["missing final newline", "missing final newlines"],
  "duplicate-key": ["duplicate key", "duplicate keys"],
  syntax: ["trailing comma", "trailing commas"],
};

// "Fixed 3 problems: 2 trailing commas, 1 duplicate key."
export function fixSummary(fixes) {
  const counts = new Map();
  for (const f of fixes) counts.set(f.rule, (counts.get(f.rule) ?? 0) + 1);
  const parts = [...counts].map(([rule, n]) => `${n} ${KINDS[rule]?.[n === 1 ? 0 : 1] ?? rule}`);
  const n = fixes.length;
  return `Fixed ${n} ${n === 1 ? "problem" : "problems"}: ${parts.join(", ")}.`;
}
