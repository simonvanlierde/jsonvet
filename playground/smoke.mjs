// Loads the built wasm and compares check() with the strings the Rust tests in
// crates/jsonvet-wasm/src/lib.rs expect, then checks the page keeps its CSP.
// Run from the repository root, after wasm-bindgen wrote playground/pkg.
import { readFileSync } from "node:fs";
import init, { check } from "./pkg/jsonvet_wasm.js";

await init({ module_or_path: readFileSync("playground/pkg/jsonvet_wasm_bg.wasm") });

const cases = [
  ["{}", "jsonc", "[]"],
  [
    '{"a": 1, "a": 2, "b": [1 2]}',
    "json",
    '[{"line":1,"column":10,"index":9,"severity":"error","rule":"duplicate-key","message":"duplicate key $.a","path":"$.a"},{"line":1,"column":25,"index":24,"severity":"error","rule":"syntax","message":"missing `,`","path":null}]',
  ],
  [
    '{"a": 1,\u2028"a": 2}',
    "json5",
    '[{"line":2,"column":1,"index":9,"severity":"error","rule":"duplicate-key","message":"duplicate key $.a","path":"$.a"}]',
  ],
];
let failed = false;
for (const [text, dialect, expected] of cases) {
  const got = check(text, dialect);
  if (got !== expected) {
    console.error(`check(${JSON.stringify(text)}, ${dialect})\n  got      ${got}\n  expected ${expected}`);
    failed = true;
  }
}

// The CSP allows no inline script or style, so any left in the page would not run.
const page = readFileSync("playground/index.html", "utf8");
const csp = /<meta http-equiv="Content-Security-Policy" content="([^"]*)"/.exec(page)?.[1] ?? "";
for (const need of ["default-src 'none'", "script-src 'self' 'wasm-unsafe-eval'", "connect-src 'self'", "style-src 'self';"]) {
  if (!`${csp};`.includes(need)) {
    console.error(`playground/index.html: the CSP lacks ${need}`);
    failed = true;
  }
}
if (/'unsafe-inline'/.test(csp) || /<script(?![^>]*\ssrc=)[^>]*>/.test(page) || /<style|\sstyle=/.test(page)) {
  console.error("playground/index.html has an inline script or style");
  failed = true;
}
process.exit(failed ? 1 : 0);
