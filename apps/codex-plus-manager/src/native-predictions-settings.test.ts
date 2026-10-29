import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";
import { EN_PLAIN } from "./i18n-en.ts";

const read = (path: string) => readFileSync(new URL(path, import.meta.url), "utf8");
const catalog = JSON.parse(read("../../../crates/codex-plus-core/assets/enhancement-settings.json"));
const app = read("./App.tsx");
const settingsPage = read("../../../assets/inject/renderer-inject/40-backend-settings.js");
const serviceTier = read("../../../assets/inject/renderer-inject/30-service-tier.js");

test("native predictions is an experimental opt-in shown on both settings pages that explains cost and scope", () => {
  const item = catalog.items.find((entry: { key: string }) => entry.key === "codexAppNativePredictions");
  assert.ok(item);
  assert.equal(item.group, "plugins");
  assert.equal(item.control, "bool");
  assert.equal(item.title, "原生输入预测（实验）");
  assert.deepEqual(item.surfaces, ["manager", "codex"]);
  for (const phrase of ["实验", "默认关闭", "额度", "API Key / 自定义供应商", "本机对话", "重新启动 Codex 后生效"]) {
    assert.ok(item.description.includes(phrase), phrase);
  }
  assert.ok(EN_PLAIN[item.title]);
  assert.match(EN_PLAIN[item.description], /quota/);
  assert.match(app, /codexAppNativePredictions: boolean;/);
  assert.match(app, /codexAppNativePredictions: false,/);
});

test("backend loads and saves publish only the native predictions policy flags", async () => {
  const extract = (fragment: string, name: string) => {
    const body = fragment.match(new RegExp(`^  async function ${name}\\([^]*?^  \\}`, "m"))?.[0];
    assert.ok(body, name);
    return body;
  };
  const publications: Array<Record<string, boolean>> = [];
  let response: Record<string, unknown> = {
    enhancementsEnabled: true, codexAppNativePredictions: true, nativePredictionsLocalApiConfigured: true,
    relayProfiles: [{ configContents: "private backend fixture" }],
  };
  const context = vm.createContext({
    window: {
      __CODEX_PLUS_TEST_SERVICE_TIER__: true,
      __codexPlusNativePredictions: { runtime: { update: (value: Record<string, boolean>) => publications.push({ ...value }) } },
    },
    codexPlusBackendSettings: { enhancementsEnabled: true },
    codexPlusBackendSettingsLoaded: false,
    codexPlusBackendSettingsSeq: 0,
    postJson: async () => response,
    refreshCodexPlusBackendToggles: () => {},
  });
  vm.runInContext([extract(serviceTier, "loadBackendSettingsState"), extract(settingsPage, "setBackendSetting")].join("\n"), context);
  assert.equal(await context.loadBackendSettingsState(), true);
  response = { ...response, codexAppNativePredictions: false };
  await context.setBackendSetting("codexAppNativePredictions", false);
  assert.deepEqual(publications, [
    { enhancementsEnabled: true, codexAppNativePredictions: true, nativePredictionsLocalApiConfigured: true },
    { enhancementsEnabled: true, codexAppNativePredictions: false, nativePredictionsLocalApiConfigured: true },
  ]);
});
