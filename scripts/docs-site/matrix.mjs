// The conformance matrix page: docs/conformance.json (written by conformance/report.py) as one
// table per dialect, with filters and a detail panel for each file and parser. No script: the
// dialect and filter choices are radio inputs that src/styles/matrix.css reads with :has(), and each detail is
// a section shown by :target, so every file and parser has its own link (#file/..., #parser/...).
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { deflateRawSync } from "node:zlib";
import { Marked } from "marked";
import { DOCS, esc, REPO, ROOT, submodules } from "./lib.mjs";

// A file's text is shown up to this many characters; the playground link carries all of it.
const MAX_CHARS = 2000;
const VERDICT = { accept: "accepts", reject: "rejects", crash: "crashes", timeout: "times out" };
const MARK = {
  "✓": "agrees",
  "✗": "disagrees",
  "‼": "crash or timeout",
  A: "either verdict conforms",
  R: "either verdict conforms",
};
// The filters, in legend order: [value, label, test on a row].
const FILTERS = [
  ["all", "all files", () => true],
  ["some", "✗ some parser disagrees", (f) => Object.values(f.runs).some((r) => r.mark === "✗" || r.mark === "‼")],
  [
    "jsonvet",
    "jsonvet disagrees",
    (f) => Object.entries(f.runs).some(([c, r]) => c.startsWith("jsonvet-") && (r.mark === "✗" || r.mark === "‼")),
  ],
  ["crash", "‼ a parser crashed", (f) => Object.values(f.runs).some((r) => r.mark === "‼")],
  ["spec", "† spec-decided", (f) => f.why !== ""],
];

/** The short name of an input file, as docs/conformance.md shows it, and its fragment. */
const short = (path) => path.replace(/^tests\//, "").replace(/^conformance\//, "");
const fileId = (path) => `file/${short(path)}`;
const base = (path) => path.slice(path.lastIndexOf("/") + 1);
const folder = (path) => short(path).slice(0, short(path).lastIndexOf("/") + 1);
const inline = (md) => new Marked().parseInline(md);
let subs;
/** A file on GitHub: in this repository, or in a submodule's at its pinned commit. */
function fileUrl(path) {
  subs ??= submodules();
  const sub = subs.find((s) => path.startsWith(`${s.path}/`));
  return sub ? `${sub.url}/blob/${sub.commit}/${path.slice(sub.path.length + 1)}` : `${REPO}/blob/main/${path}`;
}

/** The text of a file with its invisible characters named, or null when it is not UTF-8. */
export function shown(bytes) {
  let text;
  try {
    text = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(bytes);
  } catch {
    return null;
  }
  const chars = [...text];
  const cut = chars.length > MAX_CHARS;
  const html = esc(chars.slice(0, MAX_CHARS).join("")).replace(
    // \r\n reads as CR, then the line break; a lone CR gets a line break of its own.
    /\r\n?|(?![\n\t])[\p{Cc}\p{Cf}\p{Zl}\p{Zp}\u00a0]/gu,
    (c) =>
      c.startsWith("\r")
        ? '<span class="ch">CR</span>\n'
        : `<span class="ch">U+${c.codePointAt(0).toString(16).toUpperCase().padStart(4, "0")}</span>`,
  );
  return { text, html, cut, length: chars.length };
}

function cellOf(column, run) {
  const words = `${VERDICT[run.verdict]}, ${MARK[run.mark]}`;
  const bad = run.mark === "✗" || run.mark === "‼" ? ' class="bad"' : "";
  return `<td${bad} title="${esc(`${column} ${words}`)}"><span aria-hidden="true">${run.mark}</span><span class="vh">${words}</span></td>`;
}

function dialectTable(d) {
  const counts = FILTERS.map(([value, label, test]) => [value, label, d.files.filter(test).length]);
  const legend = counts
    .map(
      ([value, label, n], i) =>
        `<label><input type="radio" name="f-${d.dialect}" value="${value}"${i ? "" : " checked"}${n ? "" : " disabled"}> ${esc(label)} <span class="count">(${n})</span></label>`,
    )
    .join("\n");
  const head = d.columns
    .map((c) => `<th scope="col" class="run"><a href="#parser/${c}"><span>${c}</span></a></th>`)
    .join("");
  const groups = Map.groupBy(d.files, (f) => folder(f.path));
  const bodies = [...groups].map(([name, files]) => {
    const rows = files.map((f) => {
      const flags = FILTERS.slice(1)
        .filter(([, , test]) => test(f))
        .map(([value]) => value);
      return `<tr${flags.length ? ` class="${flags.join(" ")}"` : ""}><th scope="row"><a href="#${esc(fileId(f.path))}">${esc(base(f.path))}</a></th><td>${f.expected}${f.why ? " †" : ""}</td>${d.columns.map((c) => cellOf(c, f.runs[c])).join("")}</tr>`;
    });
    // A group row per folder; the filters hide a group with no row left.
    const kinds = FILTERS.slice(1)
      .filter(([, , test]) => files.some(test))
      .map(([value]) => value);
    return `<tbody${kinds.length ? ` class="${kinds.join(" ")}"` : ""}><tr class="group"><th scope="rowgroup" colspan="${d.columns.length + 2}">${esc(name)}</th></tr>\n${rows.join("\n")}\n</tbody>`;
  });
  const scored = d.files.filter((f) => f.expected !== "either").length;
  return `<section class="dialect" id="dialect-${d.dialect}" aria-labelledby="h-${d.dialect}">
<h2 id="h-${d.dialect}">${esc(d.title)}</h2>
<p>Spec: <a href="${esc(d.spec)}">${esc(d.spec.replace(/^https:\/\//, ""))}</a>. ${d.files.length} files, ${scored} scored, ${d.columns.length} parser runs.</p>
<fieldset class="filter"><legend>Show</legend>
${legend}
</fieldset>
<div class="scroll" tabindex="0" role="region" aria-label="Matrix: ${esc(d.title)}">
<table>
<thead><tr><th scope="col">file</th><th scope="col">expected</th>${head}</tr></thead>
${bodies.join("\n")}
</table>
</div>
</section>`;
}

function fileDetail(path, entries) {
  const bytes = readFileSync(resolve(ROOT, path));
  const text = shown(bytes);
  const shownText = text
    ? `<pre tabindex="0" role="region" aria-label="Text: ${esc(short(path))}"><code>${text.html}</code></pre>${
        text.cut
          ? `<p class="aside">The first ${MAX_CHARS.toLocaleString("en")} of ${text.length.toLocaleString("en")} characters.</p>`
          : ""
      }`
    : `<p class="aside">The file is not valid UTF-8, so its text is not shown.</p>`;
  const dialects = entries.map(({ d, f }) => {
    const play = text
      ? ` <a href="../#${d.dialect}:${deflateRawSync(text.text).toString("base64url")}">Open it in the playground as ${esc(d.title)}</a>.`
      : "";
    const source =
      f.source === "the spec" ? `the spec: ${esc(f.why)}` : `${esc(f.source)}${f.why ? `: ${esc(f.why)}` : ""}`;
    const runs = d.columns
      .map(
        (c) =>
          `<tr><th scope="row"><a href="#parser/${c}">${c}</a></th><td>${VERDICT[f.runs[c].verdict]}</td><td>${f.runs[c].mark} ${MARK[f.runs[c].mark]}</td></tr>`,
      )
      .join("");
    return `<h3>${esc(d.title)}: expected ${f.expected}</h3>
<p>From ${source}.${play}</p>
<table class="runs"><thead><tr><th scope="col">parser</th><th scope="col">verdict</th><th scope="col">against expected</th></tr></thead><tbody>${runs}</tbody></table>
${f.note ? `<p class="why">${inline(f.note)}</p>` : ""}`;
  });
  return `<section class="detail" id="${esc(fileId(path))}">
<h2>${esc(short(path))}</h2>
<p class="back"><a href="#matrix">Back to the matrix</a></p>
<p class="meta"><span>${bytes.length.toLocaleString("en")} bytes</span><span><a href="${fileUrl(path)}">on GitHub</a></span></p>
${shownText}
${dialects.join("\n")}
</section>`;
}

function parserDetail(name, info, dialects) {
  const parts = dialects
    .filter((d) => d.columns.includes(name))
    .map((d) => {
      const scored = d.files.filter((f) => f.expected !== "either");
      const n = (mark) => scored.filter((f) => f.runs[name].mark === mark).length;
      const crashes = d.files.filter((f) => f.runs[name].mark === "‼");
      const off = d.files.filter((f) => ["✗", "‼"].includes(f.runs[name].mark));
      const list = off.length
        ? `<ul>${off.map((f) => `<li><a href="#${esc(fileId(f.path))}">${esc(short(f.path))}</a>: ${VERDICT[f.runs[name].verdict]}, expected ${f.expected}</li>`).join("")}</ul>`
        : "<p>It agrees on every scored file.</p>";
      return `<h3>${esc(d.title)}</h3>
<p>Agrees on ${n("✓")} of ${scored.length} scored files, disagrees on ${n("✗")}, crashed or timed out on ${crashes.length}.</p>
${list}`;
    });
  return `<section class="detail" id="parser/${name}">
<h2>${name}</h2>
<p class="back"><a href="#matrix">Back to the matrix</a></p>
<p class="meta"><span>${esc(info.language)}</span>${info.package ? `<span>${esc(info.package)}</span>` : ""}<span>${esc(info.version)}</span></p>
${parts.join("\n")}
</section>`;
}

/** The page's title, description and body. */
export function matrixPage() {
  const data = JSON.parse(readFileSync(resolve(ROOT, "docs/conformance.json"), "utf8"));
  const files = new Map();
  for (const d of data.dialects) for (const f of d.files) files.set(f.path, [...(files.get(f.path) ?? []), { d, f }]);
  const shownParsers = Object.keys(data.parsers).filter((p) => data.dialects.some((d) => d.columns.includes(p)));
  const choices = data.dialects
    .map(
      (d, i) =>
        `<label><input type="radio" name="dialect" value="${d.dialect}"${i ? "" : " checked"}> ${esc(d.title)} <span class="count">(${d.files.length})</span></label>`,
    )
    .join("\n");
  // The page's h1 is Starlight's title.
  const body = `<p>How ${shownParsers.length} parser runs treat the same files in five JSON dialects. Results updated on ${data.date}. Pick a dialect and what to show, then a file or a parser for its details. <a href="${DOCS}conformance.html">Conformance</a> has the same results as text, with the duplicate-key table and every version.</p>
<ul class="key not-content" aria-label="Legend">
<li><b>✓</b> agrees with the expected verdict</li>
<li><b>✗</b> disagrees</li>
<li><b>‼</b> crashed or took longer than ${data.timeout}&nbsp;s</li>
<li><b>A</b> accepted, where the spec allows both</li>
<li><b>R</b> rejected, where the spec allows both</li>
<li><b>†</b> the spec decides the expected verdict</li>
</ul>
<div class="matrix not-content" id="matrix">
<fieldset class="dialects"><legend>Dialect</legend>
${choices}
</fieldset>
<a class="skip-matrix" href="#details">Skip past the matrix</a>
<div class="tables">
${data.dialects.map(dialectTable).join("\n")}
</div>
<div class="details" id="details">
<p class="hint">Choose a file name or a parser to see its details here.</p>
${[...files].map(([path, entries]) => fileDetail(path, entries)).join("\n")}
${shownParsers.map((p) => parserDetail(p, data.parsers[p], data.dialects)).join("\n")}
</div>
</div>
`;
  return {
    title: "Conformance matrix",
    summary: `How ${shownParsers.length} parser runs treat the same files in five JSON dialects, file by file, with each file's text.`,
    body,
    h2s: [],
  };
}
