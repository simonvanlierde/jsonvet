// No `vscode` import here, so `node --test` can load this file outside the editor.

/// The rules whose problems VS Code's own JSON validation also reports in JSON and JSONC files:
/// its parser reports syntax errors, and its validator duplicate keys.
const BUILT_IN = new Set(["syntax", "duplicate-key"]);

/// A diagnostic's code as VS Code gives it: with a link to the rule's help, an object.
type Code = string | number | { value: string | number } | undefined;

/// `diagnostics` for a document in `languageId`, without those VS Code's own JSON validation also
/// reports while `builtIn`, its `json.validate.enable`, is on. Schema problems stay: VS Code checks
/// the schemas it knows, such as SchemaStore's, and jsonvet the local ones its settings name.
export function withoutBuiltIn<D extends { code?: Code }>(languageId: string, builtIn: boolean, diagnostics: D[]): D[] {
  if (!builtIn || (languageId !== "json" && languageId !== "jsonc")) {
    return diagnostics;
  }
  return diagnostics.filter(({ code }) => !BUILT_IN.has(String(typeof code === "object" ? code.value : code)));
}
