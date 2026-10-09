import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import {
  modelRouteSaveRequiresRestart,
  normalizeRelayModelRoutes,
  PROTOCOL_PROXY_BASE_URL,
  settingsRequireLocalHelper,
  webSearchHistoryCompatEnabled,
} from "./model-routes.ts";

const source = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
const app = ts.createSourceFile("App.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const names = ["applyRelayProfilePatchToFiles", "buildRelayConfigToml", "normalizeRelaySessionProvider", "tomlString"];
const functions = names.map((name) => {
  const node = app.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === name);
  assert.ok(node, `missing ${name}`);
  return node.getText(app);
});
const code = ts.transpileModule(functions.join("\n"), {
  compilerOptions: { target: ts.ScriptTarget.ES2022 },
}).outputText;

function profile(id = "source", enabled = false) {
  return {
    id, name: id, model: "test-model", apiKey: "test-key",
    baseUrl: "https://upstream.example/v1", upstreamBaseUrl: "https://upstream.example/v1",
    protocol: "responses" as const, relayMode: "pureApi" as const,
    officialMixApiKey: false, webSearchHistoryCompat: enabled,
    configContents: "original-config", authContents: "original-auth", modelRoutes: [],
  };
}

function settings(enabled = false) {
  return {
    relayProfilesEnabled: true, enhancementsEnabled: false, activeRelayId: "source",
    relayProfiles: [profile("source", enabled), profile("other", true)],
  };
}

test("experimental search history compatibility is per-provider and starts the helper only when enabled", () => {
  assert.equal(settingsRequireLocalHelper(settings()), false);
  assert.equal(settingsRequireLocalHelper(settings(true)), true);
  assert.equal(modelRouteSaveRequiresRestart(settings(), settings(true), profile().baseUrl), true);
  assert.equal(modelRouteSaveRequiresRestart(settings(true), settings(true), profile().baseUrl), true);
  assert.equal(modelRouteSaveRequiresRestart(settings(true), settings(true), PROTOCOL_PROXY_BASE_URL), false);
  assert.equal(modelRouteSaveRequiresRestart(settings(true), settings(), PROTOCOL_PROXY_BASE_URL), false);
  assert.equal(webSearchHistoryCompatEnabled({ ...profile(), webSearchHistoryCompat: undefined }), false);
  assert.equal(webSearchHistoryCompatEnabled({ ...profile("source", true), protocol: "chatCompletions" }), false);
  assert.equal(webSearchHistoryCompatEnabled({ ...profile("source", true), relayMode: "official" }), false);
  assert.equal(webSearchHistoryCompatEnabled({ ...profile("source", true), relayMode: "aggregate" }), false);
});

test("checkbox draft and generated config route to proxy and restore the real upstream on disable", () => {
  const context = vm.createContext({
    PROTOCOL_PROXY_BASE_URL, webSearchHistoryCompatEnabled, normalizeRelayModelRoutes,
    relaySessionProvider: () => "custom",
    sessionProviderForProtocol: (provider: string) => provider,
    isAggregateRelayProfile: () => false,
    deriveRelayProfileFromFiles: (value: unknown) => value,
    setCodexProviderStringKey: (_contents: string, _key: string, value: string) => `base_url = "${value}"`,
    removeRootTomlKey: (contents: string) => contents,
    CHAT_UPSTREAM_BASE_URL_KEY: "chat_upstream_base_url",
  });
  vm.runInContext(code, context);
  const original = profile();
  const enabled = context.applyRelayProfilePatchToFiles(original, { webSearchHistoryCompat: true });
  assert.equal(enabled.webSearchHistoryCompat, true);
  assert.equal(enabled.upstreamBaseUrl, original.upstreamBaseUrl);
  assert.equal(enabled.configContents, `base_url = "${PROTOCOL_PROXY_BASE_URL}"`);
  const disabled = context.applyRelayProfilePatchToFiles(enabled, { webSearchHistoryCompat: false });
  assert.equal(disabled.webSearchHistoryCompat, false);
  assert.equal(disabled.configContents, `base_url = "${original.upstreamBaseUrl}"`);
  assert.equal(original.webSearchHistoryCompat, false, "draft edits must not mutate the original");
  for (const value of [original, enabled, disabled]) {
    const config = context.buildRelayConfigToml(value, { includeBearerToken: false });
    assert.ok(config.includes(`base_url = "${value.webSearchHistoryCompat ? PROTOCOL_PROXY_BASE_URL : original.baseUrl}"`));
  }
});

test("provider checkbox binds checked state, draft update and the Responses-only disabled state", () => {
  const start = source.indexOf("relay-field-search-history");
  assert.ok(start >= 0);
  const checkbox = source.slice(start, source.indexOf("</label>", start));
  assert.match(checkbox, /checked=\{profile\.webSearchHistoryCompat\}/);
  assert.match(checkbox, /disabled=\{profile\.protocol !== "responses"\}/);
  assert.match(checkbox, /updateDraft\(\{ webSearchHistoryCompat: event\.currentTarget\.checked \}\)/);
  assert.match(checkbox, /type="checkbox"/);
  assert.match(checkbox, /搜索历史压缩兼容（实验）/);
  assert.match(source, /webSearchHistoryCompat: profile\.webSearchHistoryCompat === true/);
});
