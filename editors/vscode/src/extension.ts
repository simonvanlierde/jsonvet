// A thin client for `jsonvet lsp`: the server does the checking, fixing and formatting.

import { ConfigurationTarget, type ExtensionContext, window, workspace } from "vscode";
import { LanguageClient, type ServerOptions } from "vscode-languageclient/node";

let client: LanguageClient | undefined;

export async function activate(context: ExtensionContext): Promise<void> {
  const command = workspace.getConfiguration("jsonvet").get<string>("path") || "jsonvet";
  const server: ServerOptions = { command, args: ["lsp"] };
  // Files only: jsonvet picks the dialect and settings by the file's path.
  const documentSelector = ["json", "jsonc", "json5", "jsonl"].map((language) => ({
    scheme: "file",
    language,
  }));
  client = new LanguageClient("jsonvet", "jsonvet", server, { documentSelector });
  await client.start();
  // Not awaited: the question waits for an answer, and activation should not.
  void askAboutBuiltInValidation(context);
}

export function deactivate(): Promise<void> | undefined {
  return client?.stop();
}

// VS Code's own JSON validation also reports syntax errors and duplicate keys, so they would show
// twice. Ask once per workspace whether to turn it off, as it also checks SchemaStore schemas,
// which jsonvet does not fetch. Without a folder open there is no workspace to set it in.
async function askAboutBuiltInValidation(context: ExtensionContext): Promise<void> {
  const json = workspace.getConfiguration("json");
  const asked = "askedAboutBuiltInValidation";
  if (!workspace.workspaceFolders || !json.get<boolean>("validate.enable", true) || context.workspaceState.get(asked)) {
    return;
  }
  await context.workspaceState.update(asked, true);
  const turnOff = "Turn it off";
  const choice = await window.showInformationMessage(
    "VS Code's JSON validation reports some of the problems jsonvet reports, so they show twice. " +
      "Turn it off for this workspace? It also checks schemas from SchemaStore, which jsonvet does not fetch.",
    turnOff,
    "Keep both",
  );
  if (choice === turnOff) {
    await json.update("validate.enable", false, ConfigurationTarget.Workspace);
  }
}
