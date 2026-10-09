import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";
import { modelSlugFromRowName, suffixWindowString } from "./model-metadata.ts";
import { normalizeRelayModelRoutes } from "./model-routes.ts";
import { sessionProviderForProtocol } from "./relay-session.ts";

// 执行 App 中实际使用的配置 helper，避免把另一份实现复制进测试。
const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
const app = ts.createSourceFile("App.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const names = new Set([
  "withGeneratedRelayFiles", "buildRelayConfigToml", "buildRelayAuthJson", "buildOfficialRelayAuthJson",
  "deriveRelayProfileFromFiles", "applyRelayProfilePatchToFiles", "codexModelFromConfig", "parseModelSuffix",
  "codexBaseUrlFromConfig", "codexExperimentalBearerTokenFromConfig", "codexProviderStringFromConfig",
  "codexApiKeyFromAuth", "codexTopLevelIntFromConfig", "rootTomlStringValue", "tomlSectionName",
  "tomlStringAssignmentValue", "setAuthOpenAiApiKey", "setRootTomlStringKey", "setManagedOpenAiBaseUrl",
  "setRootTomlIntKey", "setRootTomlLine", "removeRootTomlKey", "setCodexProviderStringKey",
  "setCodexExperimentalBearerToken", "removeCodexExperimentalBearerToken", "ensureCodexProviderDefaults",
  "setTomlSectionBoolKey", "setTomlSectionStringKey", "setTomlSectionRawKey", "removeTomlSectionKey",
  "tomlString", "ensureTrailingNewline", "joinTomlSections", "splitTomlRootAndTables",
  "normalizeRelaySessionProvider", "relaySessionProviderFromConfig", "relaySessionProvider",
  "isAggregateRelayProfile",
  "tomlQuotedValue", "tomlDottedKeyPath", "tomlKeyPathName", "codexProviderSectionName",
  "tomlKeyAssignment", "tomlInlineComment",
]);
const helpers = app.statements.filter((node) =>
  ts.isFunctionDeclaration(node) && node.name && names.has(node.name.text));
assert.equal(helpers.length, names.size);
const code = ts.transpileModule(helpers.map((node) => node.getText(app)).join("\n"), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText;
const context = vm.createContext({
  exports: {},
  PROTOCOL_PROXY_BASE_URL: "http://127.0.0.1:57321/v1",
  CHAT_UPSTREAM_BASE_URL_KEY: "codex_plus_chat_base_url",
  normalizeRelayModelRoutes, sessionProviderForProtocol, modelSlugFromRowName, suffixWindowString,
});
vm.runInContext(code, context);

function profile(flag?: boolean) {
  const authFlag = flag === undefined ? "" : "requires_openai_auth = " + flag + "\n";
  return {
    relayMode: "pureApi", sessionProvider: "custom", protocol: "responses", noAuth: false,
    model: "fixture-model", baseUrl: "https://relay.example/v1", upstreamBaseUrl: "https://relay.example/v1",
    apiKey: "fixture-provider-key", officialMixApiKey: false, modelRoutes: [],
    configContents: 'model_provider = "vendor"\n\n[model_providers.vendor]\nname = "Friendly vendor"\nwire_api = "responses"\nbase_url = "https://relay.example/v1"\n' + authFlag,
    authContents: JSON.stringify({ OPENAI_API_KEY: "fixture-provider-key", vendor: "stored" }),
  };
}

test("new pure API files use explicit provider auth without requiring official login", () => {
  const generated = context.withGeneratedRelayFiles(profile());
  assert.match(generated.configContents, /requires_openai_auth = false/);
  assert.match(generated.configContents, /experimental_bearer_token = "fixture-provider-key"/);
  assert.equal(JSON.parse(generated.authContents).OPENAI_API_KEY, "fixture-provider-key");
});

test("legacy false, missing, and true templates gain provider auth while preserving their settings", () => {
  for (const flag of [false, undefined, true]) {
    const normalized = context.deriveRelayProfileFromFiles(profile(flag));
    assert.match(normalized.configContents, /experimental_bearer_token = "fixture-provider-key"/);
    assert.match(normalized.configContents, /name = "Friendly vendor"/);
    assert.equal(context.rootTomlStringValue(normalized.configContents, "model_provider"), "vendor");
    if (flag === undefined) assert.doesNotMatch(normalized.configContents, /requires_openai_auth/);
    else assert.match(normalized.configContents, new RegExp("requires_openai_auth = " + flag));
  }
});

test("editing and clearing the API key keeps provider and auth snapshots consistent", () => {
  for (const flag of [false, undefined, true]) {
    const updated = context.applyRelayProfilePatchToFiles(profile(flag), { apiKey: "fixture-edited-key" });
    assert.equal(updated.apiKey, "fixture-edited-key");
    assert.match(updated.configContents, /experimental_bearer_token = "fixture-edited-key"/);
    assert.equal(JSON.parse(updated.authContents).OPENAI_API_KEY, "fixture-edited-key");
    assert.equal(JSON.parse(updated.authContents).vendor, "stored");
    const cleared = context.applyRelayProfilePatchToFiles(updated, { apiKey: "" });
    assert.doesNotMatch(cleared.configContents, /experimental_bearer_token/);
    assert.equal(JSON.parse(cleared.authContents).OPENAI_API_KEY, "");
    if (flag === undefined) assert.doesNotMatch(updated.configContents, /requires_openai_auth/);
    else assert.match(updated.configContents, new RegExp("requires_openai_auth = " + flag));
  }
});

test("inline imports and backfilled auth snapshots keep explicit provider credentials", () => {
  const imported = profile(false);
  imported.authContents = "{}";
  imported.configContents += 'experimental_bearer_token = "fixture-import-key"\n';
  const normalized = context.deriveRelayProfileFromFiles(imported);
  assert.equal(normalized.apiKey, "fixture-import-key");
  assert.match(normalized.configContents, /experimental_bearer_token = "fixture-import-key"/);
  const backfilled = context.deriveRelayProfileFromFiles(profile(false));
  assert.match(backfilled.configContents, /experimental_bearer_token = "fixture-provider-key"/);
  assert.equal(JSON.parse(backfilled.authContents).vendor, "stored");
});

test("protocol and model-route edits retain explicit auth for local transport", () => {
  for (const patch of [
    { protocol: "chatCompletions" },
    { modelRoutes: [{ model: "fixture-model", targetRelayId: "other", targetModel: "upstream" }] },
  ]) {
    const updated = context.applyRelayProfilePatchToFiles(profile(false), patch);
    assert.match(updated.configContents, /base_url = "http:\/\/127\.0\.0\.1:57321\/v1"/);
    assert.match(updated.configContents, /experimental_bearer_token = "fixture-provider-key"/);
    assert.match(updated.configContents, /requires_openai_auth = false/);
  }
});

test("no-auth snapshots do not expose a real provider bearer", () => {
  const noAuth = { ...profile(), noAuth: true };
  noAuth.configContents += 'experimental_bearer_token = "fixture-provider-key"\n';
  assert.doesNotMatch(context.deriveRelayProfileFromFiles(noAuth).configContents, /experimental_bearer_token/);
  assert.doesNotMatch(context.withGeneratedRelayFiles(noAuth).configContents, /experimental_bearer_token/);
});

test("official mixed API still uses explicit provider auth while preserving OAuth", () => {
  const mixed = {
    ...profile(), relayMode: "official", officialMixApiKey: true,
    authContents: JSON.stringify({ auth_mode: "chatgpt", tokens: { access_token: "fixture-oauth" } }),
  };
  const generated = context.withGeneratedRelayFiles(mixed);
  assert.match(generated.configContents, /requires_openai_auth = true/);
  assert.match(generated.configContents, /experimental_bearer_token = "fixture-provider-key"/);
  assert.equal(JSON.parse(generated.authContents).tokens.access_token, "fixture-oauth");
  assert.doesNotMatch(generated.configContents, /fixture-oauth/);
});

const tableCases = [
  { id: "vendor", header: '[model_providers."vendor"] # keep-header' },
  { id: "vendor", header: "[model_providers.'vendor'] # keep-header" },
  { id: "vendor", header: "[ model_providers . vendor ] # keep-header" },
  { id: "vendor", header: '[ "model_providers" . "\\u0076endor" ] # keep-header' },
  { id: "vendor.one", header: '[model_providers."vendor.one"] # keep-header' },
  { id: 'vendor"quote', header: '[model_providers."vendor\\"quote"] # keep-header' },
  { id: "vendor\\path", header: "[model_providers.'vendor\\path'] # keep-header" },
  { id: "vendor]hash#", header: '[model_providers."vendor]hash#"] # keep-header' },
  { id: "vendor🐋", header: '[model_providers."vendor\\U0001F40B"] # keep-header' },
];

// 同一份输出也供独立 Rust TOML parser 验证，测试不依赖额外的 JS TOML 库。
export const authRoundTripFixtures = tableCases.map(({ id, header }) => {
  const original = {
    ...profile(false),
    configContents: '"model_provider" = ' + JSON.stringify(id) + ' # keep-root\n\n' + header
      + '\nname = "Friendly vendor"\nwire_api = "responses"\nbase_url = "https://relay.example/v1"\nrequires_openai_auth = false\n'
      + "'experimental_bearer_token' = \"fixture-old-key\" # keep-token\n",
  };
  const loaded = context.deriveRelayProfileFromFiles(original);
  const edited = context.applyRelayProfilePatchToFiles(loaded, { apiKey: "fixture-edited-key" });
  const reloaded = context.deriveRelayProfileFromFiles(edited);
  return { id, header, loaded: loaded.configContents, edited: edited.configContents, reloaded: reloaded.configContents };
});

test("quoted and dotted provider headers survive loading, editing, and saving without duplicate tables", () => {
  for (const fixture of authRoundTripFixtures) {
    const section = context.codexProviderSectionName(fixture.id);
    for (const config of [fixture.loaded, fixture.edited, fixture.reloaded]) {
      assert.ok(config.includes(fixture.header), fixture.header);
      assert.ok(config.includes("# keep-root"));
      assert.ok(config.includes("# keep-token"));
      assert.match(config, /name = "Friendly vendor"/);
      assert.equal(context.rootTomlStringValue(config, "model_provider"), fixture.id);
      assert.equal(config.split("\n").filter((line: string) => context.tomlSectionName(line) === section).length, 1);
    }
    assert.equal(context.codexExperimentalBearerTokenFromConfig(fixture.reloaded), "fixture-edited-key");
    assert.equal(fixture.edited, fixture.reloaded);
  }
});

test("a dotted provider id is distinct from a nested inactive provider table", () => {
  const input = {
    ...profile(false),
    configContents: 'model_provider = "vendor.one"\n\n[model_providers.vendor.one]\nexperimental_bearer_token = "fixture-inactive-key"\n\n[model_providers."vendor.one"]\nbase_url = "https://active.example/v1"\n',
  };
  const loaded = context.deriveRelayProfileFromFiles(input);
  assert.equal(context.codexExperimentalBearerTokenFromConfig(loaded.configContents), "fixture-provider-key");
  assert.equal(context.tomlSectionName("[model_providers.vendor.one]"), "model_providers.vendor.one");
  assert.equal(context.tomlSectionName('[model_providers."vendor.one"]'), 'model_providers."vendor.one"');
  assert.equal(loaded.configContents.split('experimental_bearer_token = "fixture-inactive-key"').length - 1, 1);
});

test("missing active credentials never borrow an inactive provider key or OAuth token", () => {
  const config = 'model_provider = "vendor"\n\n[model_providers."vendor"]\nbase_url = "https://active.example/v1"\n\n[model_providers.other]\nexperimental_bearer_token = "fixture-inactive-key"\n';
  for (const authContents of ["{}", JSON.stringify({ tokens: { access_token: "fixture-oauth" } })]) {
    const loaded = context.deriveRelayProfileFromFiles({ ...profile(false), authContents, configContents: config });
    assert.equal(loaded.apiKey, "");
    assert.equal(loaded.configContents, config);
    assert.equal(context.codexExperimentalBearerTokenFromConfig(config), "");
  }
  const own = context.deriveRelayProfileFromFiles({ ...profile(false), configContents: config });
  assert.equal(context.codexExperimentalBearerTokenFromConfig(own.configContents), "fixture-provider-key");
  assert.equal(own.configContents.split("fixture-inactive-key").length - 1, 1);
});

test("custom transport fallback requires an explicit managed OpenAI endpoint", () => {
  const transport = '\n[model_providers."custom"]\nexperimental_bearer_token = "fixture-transport-key"\n';
  assert.equal(context.codexExperimentalBearerTokenFromConfig('model_provider = "openai"\n' + transport), "");
  assert.equal(context.codexExperimentalBearerTokenFromConfig('model_provider = "openai"\nopenai_base_url = "http://127.0.0.1:57321/v1"\n' + transport), "fixture-transport-key");
});

test("array tables end the provider scope instead of receiving its new token", () => {
  const input = {
    ...profile(false),
    configContents: 'model_provider = "vendor"\n\n[model_providers."vendor"] # keep-header\nbase_url = "https://active.example/v1"\n\n[[extras]]\nname = "unchanged"\n',
  };
  const loaded = context.deriveRelayProfileFromFiles(input);
  assert.equal(context.codexExperimentalBearerTokenFromConfig(loaded.configContents), "fixture-provider-key");
  assert.ok(loaded.configContents.indexOf("experimental_bearer_token") < loaded.configContents.indexOf("[[extras]]"));
  assert.ok(loaded.configContents.endsWith('[[extras]]\nname = "unchanged"\n'));
});
