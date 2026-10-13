import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import ts from "typescript";

test("官方本机配置保留可编辑快照并接入现有文件编辑路径", async () => {
  const source = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
  const file = ts.createSourceFile("App.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const fn = file.statements.find((node) => ts.isFunctionDeclaration(node)
    && node.name?.text === "relayProfileUsesLiveFiles");
  assert.ok(fn);
  const compiled = ts.transpileModule(fn.getText(file), {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const usesFiles = new Function(`${compiled}; return relayProfileUsesLiveFiles;`)();
  assert.equal(usesFiles({ relayMode: "official", officialMixApiKey: false, configContents: 'model = "gpt-5.5"' }), true);
  assert.equal(usesFiles({ relayMode: "official", officialMixApiKey: false, configContents: "" }), false);
  assert.equal(usesFiles({ relayMode: "pureApi", officialMixApiKey: false, configContents: "" }), true);
});
