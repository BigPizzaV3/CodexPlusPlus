import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import ts from "typescript";
import { normalizeRelayApiKeys, relayProfileWithNormalizedApiKeys } from "./provider-api-keys.ts";

test("编辑命名 Key 同步选择、列表和既有 API 字段", async () => {
  const source = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
  const file = ts.createSourceFile("App.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  let code = "";
  const visit = (node: ts.Node) => {
    if (ts.isVariableDeclaration(node) && node.name.getText(file) === "updateApiKeys") {
      code = node.initializer!.getText(file);
    }
    ts.forEachChild(node, visit);
  };
  visit(file);
  assert.ok(code);
  const profile = { apiKey: "a", apiKeys: [{ id: "a", name: "A", apiKey: "a" }, { id: "b", name: "B", apiKey: "b" }], activeApiKeyId: "a" };
  const normalized = normalizeRelayApiKeys(profile);
  let patch: unknown;
  const compiled = ts.transpileModule(`const update = ${code};`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const update = new Function("normalizedApiKeys", "updateDraft", `${compiled}; return update;`)(
    normalized, (value: unknown) => { patch = value; },
  );
  update(normalized.apiKeys, "b");
  assert.deepEqual(patch, { apiKeys: normalized.apiKeys, activeApiKeyId: "b", apiKey: "b" });
  assert.equal(relayProfileWithNormalizedApiKeys({ ...profile, activeApiKeyId: "b" } as never).apiKey, "b");
});
