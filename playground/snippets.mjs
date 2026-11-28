// Keep these suffixes in sync with Dialect::EXTENSIONS in crates/jsonvet/src/dialect.rs.
const byExtension = new Map([
  ["json", "json5"], ["json5", "json5"], ["geojson", "json5"], ["jsonld", "json5"],
  ["webmanifest", "json5"], ["jsonc", "jsonc"], ["code-workspace", "jsonc"],
  ["code-snippets", "jsonc"], ["hujson", "jwcc"], ["jwcc", "jwcc"],
  ["jsonl", "jsonl"], ["ndjson", "jsonl"],
]);

export function snippets({ version, dialect, file, text }) {
  const dot = file.lastIndexOf(".");
  const ext = dot < 0 ? "" : file.slice(dot + 1);
  const jsoncModeline = file.endsWith(".json") && /^\uFEFF?[ \t]*\/\/ -\*- (?:mode: )?jsonc -\*-[ \t]*(?:\r|\n|$)/.test(text);
  const automatic = jsoncModeline ? "jsonc" : byExtension.get(ext) ?? "json5";
  const override = dialect !== automatic;
  const pattern = ext && byExtension.has(ext) ? `*.${ext}` : file === ".babelrc" ? ".babelrc" : "";
  const glob = override && pattern ? `${dialect}:${pattern}` : "";
  const path = file === "-" ? "./-" : file;
  const quoted = "'" + path.replaceAll("'", "'\\''") + "'";
  const run = (...words) => ["uvx jsonvet", ...words, override && `--dialect ${dialect}`, "--", quoted].filter(Boolean).join(" ");
  return {
    hook: [
      "repos:",
      "  - repo: https://github.com/simonvanlierde/jsonvet",
      `    rev: v${version}`,
      "    hooks:",
      "      - id: jsonvet",
      ...(glob ? [`        args: [--dialect, '${glob}']`] : []),
    ].join("\n"),
    action: [
      "name: jsonvet",
      "on: [push, pull_request]",
      "permissions:",
      "  contents: read",
      "jobs:",
      "  jsonvet:",
      "    runs-on: ubuntu-latest",
      "    steps:",
      "      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1",
      "        with:",
      "          persist-credentials: false",
      `      - uses: simonvanlierde/jsonvet@v${version.split(".")[0]}`,
      ...(glob ? ["        with:", `          args: --dialect ${glob}`] : []),
    ].join("\n"),
    cli: run(),
    "cli-fix": run("--fix"),
    "cli-format": run("fmt"),
    tracked: Boolean(pattern),
  };
}
