import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import { test } from "node:test";
import { setImmediate } from "node:timers/promises";

test("replays settings changed during startup or a restart without overlapping restarts", async () => {
  const handshakes = Array.from({ length: 4 }, () => Promise.withResolvers());
  const limits = [];
  const listeners = [];
  let maxProblems = 0;
  let stops = 0;
  const change = (key) => {
    for (const notify of listeners) {
      notify({ affectsConfiguration: (section) => section === key });
    }
  };
  const workspace = {
    getConfiguration: (section) => ({
      get: (key, fallback) => (section === "jsonvet" && key === "maxProblems" ? maxProblems : fallback),
    }),
    onDidChangeConfiguration: (notify) => {
      listeners.push(notify);
      return { dispose: () => undefined };
    },
  };
  class LanguageClient {
    constructor(_id, _name, _server, options) {
      this.options = options;
    }
    start() {
      const handshake = handshakes[limits.length];
      limits.push(this.options.initializationOptions().maxProblems);
      return handshake.promise;
    }
    restart() {
      return this.start();
    }
    async stop() {
      stops += 1;
    }
  }

  // Import the real extension while controlling its VS Code API and LSP handshake.
  const mocks = Object.assign(globalThis, { jsonvetTestModules: { workspace, LanguageClient } });
  const hooks = registerHooks({
    resolve(specifier, context, nextResolve) {
      const source =
        specifier === "vscode"
          ? "export const {workspace} = globalThis.jsonvetTestModules; export const window = {}; export const ConfigurationTarget = {};"
          : specifier === "vscode-languageclient/node"
            ? "export const {LanguageClient} = globalThis.jsonvetTestModules;"
            : undefined;
      return source
        ? { url: `data:text/javascript,${encodeURIComponent(source)}`, shortCircuit: true }
        : nextResolve(specifier, context);
    },
  });
  try {
    const extension = await import("./extension.ts");
    const context = { subscriptions: [], workspaceState: { get: () => undefined } };
    const activated = extension.activate(context);
    assert.deepEqual(limits, [0]);

    maxProblems = 1000;
    change("jsonvet.maxProblems");
    assert.deepEqual(limits, [0]);
    handshakes[0].resolve();
    await activated;
    assert.deepEqual(limits, [0, 1000]);

    maxProblems = 500;
    change("jsonvet.maxProblems");
    maxProblems = 200;
    change("jsonvet.maxProblems");
    assert.deepEqual(limits, [0, 1000]);
    handshakes[1].resolve();
    await setImmediate();
    assert.deepEqual(limits, [0, 1000, 200]);
    handshakes[2].resolve();
    await setImmediate();

    change("json.validate.enable");
    assert.deepEqual(limits, [0, 1000, 200, 200]);
    maxProblems = 1;
    change("jsonvet.maxProblems");
    const stopped = extension.deactivate();
    assert.equal(stops, 0);
    handshakes[3].resolve();
    await stopped;
    assert.deepEqual(limits, [0, 1000, 200, 200]);
    assert.equal(stops, 1);
  } finally {
    hooks.deregister();
    Reflect.deleteProperty(mocks, "jsonvetTestModules");
  }
});
