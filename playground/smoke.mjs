// Loads the built wasm and compares check() with the strings the Rust tests in
// crates/jsonvet-wasm/src/lib.rs expect, then checks the page keeps its CSP.
// Run from the repository root, after wasm-bindgen wrote playground/pkg.
import { readFileSync } from "node:fs";
import init, { check, fix, format, version } from "./pkg/jsonvet_wasm.js";

await init({ module_or_path: readFileSync("playground/pkg/jsonvet_wasm_bg.wasm") });

const cases = [
  ["{}", "jsonc", "[]"],
  [
    '{"a": 1, "a": 2, "b": [1 2]}',
    "json",
    '[{"line":1,"column":10,"index":9,"severity":"error","rule":"duplicate-key","message":"duplicate key $.a","path":"$.a","related":[{"message":"first copy","line":1,"column":2,"offset":1,"index":1}]},{"line":1,"column":25,"index":24,"severity":"error","rule":"syntax","message":"missing `,`","path":null,"related":[]}]',
  ],
  [
    '{"a": 1,\u2028"a": 2}',
    "json5",
    '[{"line":2,"column":1,"index":9,"severity":"error","rule":"duplicate-key","message":"duplicate key $.a","path":"$.a","related":[{"message":"first copy","line":1,"column":2,"offset":1,"index":1}]}]',
  ],
];
let failed = false;
const crate = /^version = "([^"]+)"/m.exec(readFileSync("Cargo.toml", "utf8"))?.[1];
if (version() !== crate) {
  console.error(`version() is ${version()}, Cargo.toml says ${crate}`);
  failed = true;
}
for (const [text, dialect, expected] of cases) {
  const got = check(text, dialect);
  if (got !== expected) {
    console.error(`check(${JSON.stringify(text)}, ${dialect})\n  got      ${got}\n  expected ${expected}`);
    failed = true;
  }
}

// fix() and format() answer as the Rust tests in the wasm crate expect.
const fixed = '{"text":"[1, 2]","fixes":[{"rule":"syntax","index":5,"safe":true}]}';
const wasmCases = [
  ["fix", fix("[1, 2,]", "json", false, new Uint32Array(0)), fixed],
  ["fix of a clean text", fix("[1]", "json", true, new Uint32Array(0)), "null"],
  ["fix of one place", fix("[1,]", "json", false, new Uint32Array([2])), '{"text":"[1]","fixes":[{"rule":"syntax","index":2,"safe":true}]}'],
  ["format", format('{"a":[1,2]}', "json", 2, 80, true, false, false), '{ "a": [1, 2] }\n'],
  // The last flag is fill_numbers: swapped with normalize_json5 (which JSON ignores) this fails.
  ["format with fill_numbers", format("[1,2,3,4,5,6,7]", "json", 2, 12, false, false, true), "[\n  1, 2, 3,\n  4, 5, 6, 7\n]\n"],
];
for (const [name, got, expected] of wasmCases) {
  if (got !== expected) {
    console.error(`${name}\n  got      ${JSON.stringify(got)}\n  expected ${JSON.stringify(expected)}`);
    failed = true;
  }
}
try {
  format("[1 2]", "json", 2, 80, true, false, false);
  console.error("format of a syntax error did not refuse");
  failed = true;
} catch (e) {
  if (!String(e).startsWith("1:")) {
    console.error(`format refused with ${e}`);
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
