import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import {
  independentProxyEnabled,
  setIndependentProxyConfigured,
  requestCompatibilityEnabled,
  stripImageGenerationToolEnabled,
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
    requestCompatibilityEnabled, stripImageGenerationToolEnabled, independentProxyEnabled, PROTOCOL_PROXY_BASE_URL, localCompactionPromptAtEndEnabled, normalizeRelayModelRoutes,
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
  for (const search of [false, true]) {
    for (const local of [false, true]) {
      const value = { ...original, webSearchHistoryCompat: search, localCompactionPromptAtEnd: local };
      const config = context.buildRelayConfigToml(value, { includeBearerToken: false });
      assert.ok(config.includes(`base_url = "${search || local ? PROTOCOL_PROXY_BASE_URL : original.baseUrl}"`));
      assert.equal(settingsRequireLocalHelper({ ...settings(), relayProfiles: [value] }), search || local);
    }
  }
  const differentSavedUpstream = { ...original, upstreamBaseUrl: "https://other.example/v1" };
  assert.ok(context.buildRelayConfigToml(differentSavedUpstream, { includeBearerToken: false })
    .includes(`base_url = "${original.baseUrl}"`), "disabled feature preserves main's baseUrl selection");
});

test("independent Responses proxy enables the helper per provider and clearing it removes cached routing", () => {
  const original = settings();
  setIndependentProxyConfigured("source", true);
  try {
    assert.equal(independentProxyEnabled(profile()), true);
    assert.equal(independentProxyEnabled({ ...profile(), relayMode: "official" }), false);
    assert.equal(independentProxyEnabled(profile("other")), false);
    assert.equal(settingsRequireLocalHelper(original), true);
    assert.equal(modelRouteSaveRequiresRestart(original, original, profile().baseUrl), true);
  } finally {
    setIndependentProxyConfigured("source", false);
  }
  assert.equal(settingsRequireLocalHelper(original), false);
});

test("provider checkbox binds draft state and is hidden for Anthropic and aggregate profiles", () => {
  const start = source.indexOf("relay-field-local-compaction");
  assert.ok(start >= 0);
  const checkbox = source.slice(start, source.indexOf("</label>", start));
  assert.match(checkbox, /checked=\{profile\.localCompactionPromptAtEnd === true\}/);
  assert.match(source.slice(start - 180, start), /showApiFields && profile\.relayMode !== "aggregate" && profile\.protocol !== "anthropicMessages"/);
  assert.match(checkbox, /updateDraft\(\{ localCompactionPromptAtEnd: event\.currentTarget\.checked \}\)/);
  assert.match(checkbox, /type="checkbox"/);
  assert.match(checkbox, /本地压缩缓存复用（实验）/);
  assert.match(checkbox, /仅支持旧版直连供应商，暂不支持 2\.0 网关/);
  assert.match(checkbox, /缓存按 Key 隔离，切换或轮换 Key 可能降低命中率/);
  assert.match(checkbox, /实测可改善压缩请求的缓存命中概率/);
  assert.doesNotMatch(checkbox, /用户实测/);
  assert.match(checkbox, /不保证每次命中/);
  assert.match(checkbox, /不保证后置压缩提示不影响摘要质量/);
  assert.match(checkbox, /未命中供应商缓存时，补回的工具声明会增加输入 token 开销/);
  assert.match(checkbox, /上游若忽略禁用工具调用要求，可能返回空摘要/);
  assert.match(source, /webSearchHistoryCompat|搜索历史压缩兼容/);
  assert.match(source, /localCompactionPromptAtEnd: profile\.localCompactionPromptAtEnd === true/);
});
