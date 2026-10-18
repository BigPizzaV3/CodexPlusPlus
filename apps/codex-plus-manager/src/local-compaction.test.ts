import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import {
  requestCompatibilityEnabled,
  modelRouteSaveRequiresRestart,
  normalizeRelayModelRoutes,
  PROTOCOL_PROXY_BASE_URL,
  settingsRequireLocalHelper,
  localCompactionPromptAtEndEnabled,
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
    officialMixApiKey: false, localCompactionPromptAtEnd: enabled,
    configContents: "original-config", authContents: "original-auth", modelRoutes: [],
  };
}

function settings(enabled = false) {
  return {
    relayProfilesEnabled: true, enhancementsEnabled: false, activeRelayId: "source",
    relayProfiles: [profile("source", enabled), profile("other", true)],
  };
}

test("experimental local compaction is per-provider and starts the helper only when enabled", () => {
  assert.equal(settingsRequireLocalHelper(settings()), false);
  assert.equal(settingsRequireLocalHelper(settings(true)), true);
  assert.equal(modelRouteSaveRequiresRestart(settings(), settings(true), profile().baseUrl), true);
  assert.equal(modelRouteSaveRequiresRestart(settings(true), settings(true), profile().baseUrl), true);
  assert.equal(modelRouteSaveRequiresRestart(settings(true), settings(true), PROTOCOL_PROXY_BASE_URL), false);
  assert.equal(modelRouteSaveRequiresRestart(settings(true), settings(), PROTOCOL_PROXY_BASE_URL), false);
  assert.equal(localCompactionPromptAtEndEnabled({ ...profile(), localCompactionPromptAtEnd: undefined }), false);
  assert.equal(localCompactionPromptAtEndEnabled({ ...profile("source", true), protocol: "chatCompletions" }), true);
  assert.equal(localCompactionPromptAtEndEnabled({ ...profile("source", true), protocol: "anthropicMessages" }), false);
  assert.equal(localCompactionPromptAtEndEnabled({ ...profile("source", true), relayMode: "official" }), false);
  assert.equal(localCompactionPromptAtEndEnabled({ ...profile("source", true), relayMode: "official", officialMixApiKey: true }), true);
  assert.equal(localCompactionPromptAtEndEnabled({ ...profile("source", true), relayMode: "aggregate" }), false);
});

test("checkbox draft and generated config route to proxy and restore the real upstream on disable", () => {
  const context = vm.createContext({
    requestCompatibilityEnabled, PROTOCOL_PROXY_BASE_URL, localCompactionPromptAtEndEnabled, normalizeRelayModelRoutes,
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
  const enabled = context.applyRelayProfilePatchToFiles(original, { localCompactionPromptAtEnd: true });
  assert.equal(enabled.localCompactionPromptAtEnd, true);
  assert.equal(enabled.upstreamBaseUrl, original.upstreamBaseUrl);
  assert.equal(enabled.configContents, `base_url = "${PROTOCOL_PROXY_BASE_URL}"`);
  const disabled = context.applyRelayProfilePatchToFiles(enabled, { localCompactionPromptAtEnd: false });
  assert.equal(disabled.localCompactionPromptAtEnd, false);
  assert.equal(disabled.configContents, `base_url = "${original.upstreamBaseUrl}"`);
  assert.equal(original.localCompactionPromptAtEnd, false, "draft edits must not mutate the original");
  for (const value of [original, enabled, disabled]) {
    const config = context.buildRelayConfigToml(value, { includeBearerToken: false });
    assert.ok(config.includes(`base_url = "${value.localCompactionPromptAtEnd ? PROTOCOL_PROXY_BASE_URL : original.baseUrl}"`));
  }
  const retired = { ...original, baseUrl: PROTOCOL_PROXY_BASE_URL, webSearchHistoryCompat: true };
  const config = context.buildRelayConfigToml(retired, { includeBearerToken: false });
  assert.ok(config.includes(`base_url = "${original.upstreamBaseUrl}"`));
  assert.equal(settingsRequireLocalHelper({ ...settings(), relayProfiles: [retired] }), false);
});

test("provider checkbox binds draft state and is hidden for Anthropic and aggregate profiles", () => {
  const start = source.indexOf("relay-field-local-compaction");
  assert.ok(start >= 0);
  const checkbox = source.slice(start, source.indexOf("</label>", start));
  assert.match(checkbox, /checked=\{profile\.localCompactionPromptAtEnd\}/);
  assert.match(source.slice(start - 180, start), /showApiFields && profile\.relayMode !== "aggregate" && profile\.protocol !== "anthropicMessages"/);
  assert.match(checkbox, /updateDraft\(\{ localCompactionPromptAtEnd: event\.currentTarget\.checked \}\)/);
  assert.match(checkbox, /type="checkbox"/);
  assert.match(checkbox, /本地压缩缓存复用（实验）/);
  assert.match(checkbox, /不保证每次命中/);
  assert.match(checkbox, /不保证后置压缩提示不影响摘要质量/);
  assert.doesNotMatch(source, /webSearchHistoryCompat|搜索历史压缩兼容/);
  assert.match(source, /localCompactionPromptAtEnd: profile\.localCompactionPromptAtEnd === true/);
});
