// A thin client for `jsonvet lsp`: the server does the checking, fixing and formatting.

import { ConfigurationTarget, type ExtensionContext, window, workspace } from "vscode";
import { LanguageClient, type ServerOptions } from "vscode-languageclient/node";
import { withoutBuiltIn } from "./diagnostics.ts";

let client: LanguageClient | undefined;

export async function activate(context: ExtensionContext): Promise<void> {
  const command = workspace.getConfiguration("jsonvet").get<string>("path") || "jsonvet";
  // The server takes its config search's stop folders from the workspace folders; the working
  // directory is its fallback, so it is a folder too where there is one.
  const cwd = workspace.workspaceFolders?.find((folder) => folder.uri.scheme === "file")?.uri.fsPath;
  const server: ServerOptions = { command, args: ["lsp"], options: { cwd } };
  // Files only: jsonvet picks the dialect and settings by the file's path.
  const documentSelector = ["json", "jsonc", "json5", "jsonl"].map((language) => ({
    scheme: "file",
    language,
  }));
  const watcher = workspace.createFileSystemWatcher("**/{jsonvet.toml,pyproject.toml}");
  context.subscriptions.push(watcher);
  client = new LanguageClient("jsonvet", "jsonvet", server, {
    documentSelector,
    // The server checks the open files again when a settings file changes.
    synchronize: { fileEvents: watcher },
    middleware: {
      handleDiagnostics(uri, diagnostics, next) {
        const document = workspace.textDocuments.find((open) => open.uri.toString() === uri.toString());
        const builtIn = workspace.getConfiguration("json", uri).get<boolean>("validate.enable", true);
        next(uri, withoutBuiltIn(document?.languageId ?? "", builtIn, diagnostics));
      },
    },
  });
  await client.start();
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
  if (!workspace.workspaceFolders || !json.get<boolean>("validate.enable", true) || context.workspaceState.get(asked)) {
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
  if (choice === undefined) {
    return;
  }
  await context.workspaceState.update(asked, true);
  if (choice === turnOff) {
    await json.update("validate.enable", false, ConfigurationTarget.Workspace);
  }
}
