import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import {
  independentProxyEnabled,
  localCompactionPromptAtEndEnabled,
  normalizeRelayModelRoutes,
  PROTOCOL_PROXY_BASE_URL,
  pureApiImageGenerationEnabled,
  requestCompatibilityEnabled,
  settingsRequireLocalHelper,
  stripImageGenerationToolEnabled,
  stripResponsesImageDetailEnabled,
  stripResponsesOutputEncryptedContentEnabled,
  webSearchHistoryCompatEnabled,
} from "./model-routes.ts";

const source = (await readFile(new URL("./App.tsx", import.meta.url), "utf8")) + "\n" + (await readFile(new URL("./relay-profile-editing.ts", import.meta.url), "utf8"));
const app = ts.createSourceFile("App.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const names = ["applyRelayProfilePatchToFiles", "buildRelayConfigToml"];
const functions = names.map((name) => {
  const node = app.statements.find((entry) => ts.isFunctionDeclaration(entry) && entry.name?.text === name);
  assert.ok(node, `missing ${name}`);
  return node.getText(app).replace(/^export /, "");
});
const code = ts.transpileModule(functions.join("\n"), {
  compilerOptions: { target: ts.ScriptTarget.ES2022 },
}).outputText;

function profile(enabled = false) {
  return {
    id: "source",
    name: "source",
    model: "glm-5.3-flash",
    apiKey: "test-key",
    baseUrl: "https://strict.example/v1",
    upstreamBaseUrl: "https://strict.example/v1",
    protocol: "responses" as const,
    relayMode: "pureApi" as const,
    officialMixApiKey: false,
    stripResponsesOutputEncryptedContent: enabled,
    configContents: "original-config",
    authContents: "original-auth",
    modelRoutes: [],
  };
}

test("output.encrypted_content compatibility is per-provider and starts the local helper", () => {
  const enabled = profile(true);
  assert.equal(stripResponsesOutputEncryptedContentEnabled(enabled), true);
  assert.equal(stripResponsesOutputEncryptedContentEnabled({ ...enabled, protocol: "chatCompletions" }), false);
  assert.equal(stripResponsesOutputEncryptedContentEnabled({ ...enabled, relayMode: "aggregate" }), false);
  assert.equal(stripResponsesOutputEncryptedContentEnabled({ ...enabled, relayMode: "official" }), false);
  assert.equal(stripResponsesOutputEncryptedContentEnabled({ ...enabled, relayMode: "official", officialMixApiKey: true }), true);
  const settings = { relayProfilesEnabled: true, enhancementsEnabled: false, activeRelayId: "source", relayProfiles: [enabled] };
  assert.equal(settingsRequireLocalHelper(settings), true);
  assert.equal(settingsRequireLocalHelper({ ...settings, relayProfiles: [profile(false)] }), false);
});

test("checkbox edits route config to the proxy and restore the real upstream on disable", () => {
  const context = vm.createContext({
    requestCompatibilityEnabled,
    stripImageGenerationToolEnabled,
    stripResponsesImageDetailEnabled,
    stripResponsesOutputEncryptedContentEnabled,
    pureApiImageGenerationEnabled,
    independentProxyEnabled,
    PROTOCOL_PROXY_BASE_URL,
    localCompactionPromptAtEndEnabled,
    webSearchHistoryCompatEnabled,
    normalizeRelayModelRoutes,
    normalizeRelaySessionProvider: (provider: string) => provider,
    tomlString: (value: string) => value,
    relaySessionProvider: () => "custom",
    sessionProviderForProtocol: (provider: string) => provider,
    isAggregateRelayProfile: () => false,
    deriveRelayProfileFromFiles: (value: unknown) => value,
    setCodexProviderStringKey: (_contents: string, _key: string, value: string) => `base_url = "${value}"`,
    removeRootTomlKey: (contents: string) => contents,
    CHAT_UPSTREAM_BASE_URL_KEY: "chat_upstream_base_url",
  });
  vm.runInContext(code, context);

  const original = profile(false);
  const enabled = context.applyRelayProfilePatchToFiles(original, {
    stripResponsesOutputEncryptedContent: true,
  });
  assert.equal(enabled.stripResponsesOutputEncryptedContent, true);
  assert.equal(enabled.configContents, `base_url = "${PROTOCOL_PROXY_BASE_URL}"`);
  assert.ok(context.buildRelayConfigToml(enabled, { includeBearerToken: false })
    .includes(`base_url = "${PROTOCOL_PROXY_BASE_URL}"`));

  const disabled = context.applyRelayProfilePatchToFiles(enabled, {
    stripResponsesOutputEncryptedContent: false,
  });
  assert.equal(disabled.stripResponsesOutputEncryptedContent, false);
  assert.equal(disabled.configContents, `base_url = "${original.upstreamBaseUrl}"`);
  assert.equal(original.stripResponsesOutputEncryptedContent, false, "draft edits must not mutate the original");
});
