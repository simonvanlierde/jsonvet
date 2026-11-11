// A thin client for `jsonvet lsp`: the server does the checking, fixing and formatting.

import { ConfigurationTarget, type ExtensionContext, window, workspace } from "vscode";
import { LanguageClient, type ServerOptions } from "vscode-languageclient/node";
import { afterAnswer, languageIdOf, shouldAskAboutBuiltIn, withoutBuiltIn } from "./diagnostics.ts";

let client: LanguageClient | undefined;

export async function activate(context: ExtensionContext): Promise<void> {
  const command = workspace.getConfiguration("jsonvet").get<string>("path") || "jsonvet";
  // The server takes its config search's stop folders from the workspace folders, not from its
  // working directory.
  const server: ServerOptions = { command, args: ["lsp"] };
  // Files only: jsonvet picks the dialect and settings by the file's path.
  const documentSelector = ["json", "jsonc", "jwcc", "json5", "jsonl"].map((language) => ({
    scheme: "file",
    language,
  }));
  client = new LanguageClient("jsonvet", "jsonvet", server, {
    // No `synchronize`: the server registers its own watchers for the settings files.
    documentSelector,
    middleware: {
      handleDiagnostics(uri, diagnostics, next) {
        const builtIn = workspace.getConfiguration("json", uri).get<boolean>("validate.enable", true);
        next(uri, withoutBuiltIn(languageIdOf(workspace.textDocuments, uri), builtIn, diagnostics));
      },
    },
  });
  await client.start();
  // The filter applies as diagnostics arrive, so the server sends them again after a change.
  context.subscriptions.push(
    workspace.onDidChangeConfiguration((change) => {
      if (change.affectsConfiguration("json.validate.enable")) {
        void client?.restart();
      }
    }),
  );
  // Not awaited: the question waits for an answer, and activation should not.
  void askAboutBuiltInValidation(context);
}

export function deactivate(): Promise<void> | undefined {
  return client?.stop();
}

// While VS Code's own JSON validation is on, jsonvet leaves syntax errors and duplicate keys in
// JSON and JSONC files to it. Ask once per workspace whether to turn it off instead, which also
// turns off its SchemaStore schema checks. Without a folder open there is no workspace to set it
// in. Closing the message without a choice asks again next time.
async function askAboutBuiltInValidation(context: ExtensionContext): Promise<void> {
  const json = workspace.getConfiguration("json");
  const asked = "askedAboutBuiltInValidation";
  const builtIn = json.get<boolean>("validate.enable", true);
  if (
    !shouldAskAboutBuiltIn(Boolean(workspace.workspaceFolders), builtIn, Boolean(context.workspaceState.get(asked)))
  ) {
    return;
  }
  const turnOff = "Turn it off";
  const choice = await window.showInformationMessage(
    "VS Code's JSON validation reports syntax errors and duplicate keys, so jsonvet leaves those to it. " +
      "Turn it off for this workspace to see jsonvet's instead? That also turns off its checks of " +
      "schemas from SchemaStore, which jsonvet does not fetch.",
    turnOff,
    "Keep it",
  );
  const { remember, disable } = afterAnswer(choice, turnOff);
  if (!remember) {
    return;
  }
  await context.workspaceState.update(asked, true);
  if (disable) {
    await json.update("validate.enable", false, ConfigurationTarget.Workspace);
  }
}
