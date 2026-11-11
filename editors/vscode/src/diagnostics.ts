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

/// The language of the open document at `uri`, or "" when it is not open, which keeps everything.
/// Compares by string, as VS Code hands the middleware a different `Uri` object than the document's.
export function languageIdOf(
  documents: readonly { uri: { toString(): string }; languageId: string }[],
  uri: { toString(): string },
): string {
  return documents.find((open) => open.uri.toString() === uri.toString())?.languageId ?? "";
}

/// Whether to ask about turning VS Code's JSON validation off: only with a folder open, while it
/// is on, and when the workspace has not answered before.
export function shouldAskAboutBuiltIn(hasFolder: boolean, builtIn: boolean, asked: boolean): boolean {
  return hasFolder && builtIn && !asked;
}

/// What to do with the answer to that question. No choice, a closed message, remembers nothing.
export function afterAnswer<T extends string>(
  choice: T | undefined,
  turnOff: T,
): { remember: boolean; disable: boolean } {
  return { remember: choice !== undefined, disable: choice === turnOff };
}
