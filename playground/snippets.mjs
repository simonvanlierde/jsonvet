// The snippets under "Use this check": the check, then the same with `--fix`, then `jsonvet fmt`.
// `glob` is `DIALECT:*.json` for any dialect but JSON5, else empty.

export function snippets({ version, glob, file }) {
  const repo = ["- repo: https://github.com/simonvanlierde/jsonvet", `  rev: v${version}`, "  hooks:"];
  const hook = (...lines) => [...repo, ...lines].join("\n");
  const id = ["    - id: jsonvet"];
  const args = (...a) => (glob ? [`      args: [${[...a, "--dialect", `"${glob}"`].join(", ")}]`] : a.length ? [`      args: [${a.join(", ")}]`] : []);
  const use = `- uses: simonvanlierde/jsonvet@v${version.split(".")[0]}`;
  const withArgs = (extra) => {
    const all = [extra, glob && `--dialect ${glob}`].filter(Boolean).join(" ");
    return all ? ["  with:", `    args: ${all}`] : [];
  };
  const run = (...words) => ["uvx jsonvet", ...words, file].filter(Boolean).join(" ");
  const dialect = glob && `--dialect '${glob}'`;
  return {
    hook: [
      hook(...id, ...args()),
      "# Fix what is safe, in place (add --unsafe-fixes to drop duplicate keys)",
      hook(...id, ...args("--fix")),
      "# Format, keeping comments (it takes its dialect from the file name)",
      hook("    - id: jsonvet-format"),
    ].join("\n\n"),
    action: [
      [use, ...withArgs("")].join("\n"),
      "# Fix what is safe, in the runner's checkout",
      [use, ...withArgs("--fix")].join("\n"),
      "# Fail when a file is not formatted",
      "- run: uvx jsonvet fmt --check .",
    ].join("\n\n"),
    cli: [run(dialect), run("--fix", dialect), `uvx jsonvet fmt ${file}`].join("\n"),
  };
}
