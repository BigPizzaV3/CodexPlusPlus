import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const text = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
const app = ts.createSourceFile("App.tsx", text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const relay = app.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === "RelayScreen");
assert.ok(relay && ts.isFunctionDeclaration(relay) && relay.body);
const saver = relay.body.statements.filter(ts.isVariableStatement).flatMap((node) => node.declarationList.declarations)
  .find((node) => node.name.getText(app) === "saveCcsDbPath");
assert.ok(saver);
const saveSource = ts.transpileModule(`const ${saver.getText(app)}; globalThis.savePath = saveCcsDbPath;`, {
  compilerOptions: { target: ts.ScriptTarget.ES2022 },
}).outputText;

test("CCS database path is persisted before refreshing the source, preserving providers", async () => {
  const events: string[] = [];
  const providers = [{ id: "keep", name: "existing provider" }];
  let persisted: { ccsDbPath: string; relayProfiles: typeof providers } | undefined;
  const context = vm.createContext({
    ccsDbPathDraft: "  C:/custom/cc-switch.db  ",
    normalized: { ccsDbPath: "", relayProfiles: providers },
    setCcsPathSaving: (saving: boolean) => events.push(`busy:${saving}`),
    actions: {
      saveSettingsValue: async (settings: typeof persisted) => { events.push("save"); persisted = settings; return settings; },
      refreshCcsProviders: async () => { events.push("refresh"); assert.equal(persisted?.ccsDbPath, "C:/custom/cc-switch.db"); },
    },
  });
  vm.runInContext(saveSource, context);
  await context.savePath();
  assert.equal(persisted?.ccsDbPath, "C:/custom/cc-switch.db");
  assert.equal(persisted?.relayProfiles, providers);
  assert.deepEqual(events, ["busy:true", "save", "refresh", "busy:false"]);
});

test("failed CCS path save does not refresh or import from the old source", async () => {
  const events: string[] = [];
  const context = vm.createContext({
    ccsDbPathDraft: "", normalized: {},
    setCcsPathSaving: (saving: boolean) => events.push(`busy:${saving}`),
    actions: {
      saveSettingsValue: async () => { events.push("save"); return null; },
      refreshCcsProviders: async () => events.push("refresh"),
    },
  });
  vm.runInContext(saveSource, context);
  await context.savePath();
  assert.deepEqual(events, ["busy:true", "save", "busy:false"]);
});
