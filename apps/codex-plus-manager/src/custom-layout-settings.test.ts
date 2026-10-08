import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";

const fragment = (name: string) => readFileSync(new URL(`../../../assets/inject/renderer-inject/${name}`, import.meta.url), "utf8");

function settingsFixture() {
  const style = fragment("10-style.js");
  const stored = new Map<string, string>();
  const writes: Array<[string, unknown]> = [];
  const context = vm.createContext({
    window: {}, conversationViewDefaultWidth: 900, codexPlusSettingsKey: "settings",
    codexPlusBackendSettings: {}, codexPlusBackendSettingsLoaded: true,
    localStorage: { getItem: (key: string) => stored.get(key) || null, setItem: (key: string, value: string) => stored.set(key, value) },
    setBackendSetting: async (key: string, value: unknown) => { writes.push([key, value]); },
    loadBackendSettings: async () => {}, syncStepwisePanel: () => {},
  });
  const start = style.indexOf("  function defaultCodexPlusSettings()");
  const end = style.indexOf("  // Dream skin runtime", start);
  const setterStart = style.indexOf("  function setCodexPlusSetting(");
  const setterEnd = style.indexOf("  function syncStepwisePanel(", setterStart);
  assert.ok(start >= 0 && end > start && setterStart >= 0 && setterEnd > setterStart);
  vm.runInContext(`${style.slice(start, end)}\n${style.slice(setterStart, setterEnd)}`, context);
  return { context, stored, writes };
}

test("custom layout defaults off and the enhancement master switch overrides its saved enablement", () => {
  const { context } = settingsFixture();
  assert.equal(context.codexPlusSettings().customLayout, false);
  context.codexPlusBackendSettings = { enhancementsEnabled: true, codexAppCustomLayoutEnabled: true };
  assert.equal(context.codexPlusSettings().customLayout, true);
  context.codexPlusBackendSettings.enhancementsEnabled = false;
  assert.equal(context.codexPlusSettings().customLayout, false);
});

test("custom layout enablement is saved through the backend partial-update contract", async () => {
  const { context, stored, writes } = settingsFixture();
  context.setCodexPlusSetting("customLayout", true);
  context.setCodexPlusSetting("customLayout", false);
  await Promise.resolve();
  assert.deepEqual(writes, [["codexAppCustomLayoutEnabled", true], ["codexAppCustomLayoutEnabled", false]]);
  assert.equal(stored.size, 0);
});

test("backend heartbeat applies custom layout updates and failed loads retain the current layout", async () => {
  let loaded = true;
  let syncs = 0;
  const context = vm.createContext({
    codexPlusSettings: () => ({ conversationView: false }),
    loadBackendSettingsState: async () => loaded,
    syncOfficialUsagePolicy: () => {}, refreshConversationView: () => {},
    syncCodexPlusTypingEffects: () => {}, installCodexPlusCustomLayout: () => { syncs += 1; },
    renderCodexPlusMenu: () => {}, runScanStep: (callback: () => void) => callback(),
  });
  const source = fragment("40-backend-settings.js");
  const start = source.indexOf("  let syncBackendSettingsInFlight");
  const end = source.indexOf("  async function setBackendSetting(", start);
  assert.ok(start >= 0 && end > start);
  vm.runInContext(source.slice(start, end), context);
  await context.syncBackendSettingsFromHeartbeat();
  assert.equal(syncs, 1);
  loaded = false;
  await context.syncBackendSettingsFromHeartbeat();
  assert.equal(syncs, 1);
});
