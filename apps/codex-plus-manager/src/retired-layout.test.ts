import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";

const style = readFileSync(new URL("../../../assets/inject/renderer-inject/10-style.js", import.meta.url), "utf8");

function fixture() {
  const stored = new Map<string, string>();
  const writes: Array<[string, unknown]> = [];
  const context = vm.createContext({
    window: {}, conversationViewDefaultWidth: 900, codexPlusSettingsKey: "settings",
    codexPlusBackendSettings: {}, codexPlusBackendSettingsLoaded: true,
    localStorage: { getItem: (key: string) => stored.get(key) || null, setItem: (key: string, value: string) => stored.set(key, value) },
    setBackendSetting: async (key: string, value: unknown) => {
      writes.push([key, value]);
      context.codexPlusBackendSettings = { ...context.codexPlusBackendSettings, [key]: value };
    },
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

test("retired layout preferences are hidden while existing local enhancement choices remain intact", () => {
  const { context, stored } = fixture();
  const oldPreferences = JSON.stringify({
    customLayout: true, conversationView: true, whaleWidget: true, typingEffect: "stars",
    nativeMenuPlacement: false, threadScrollRestore: true,
  });
  stored.set("settings", oldPreferences);
  const settings = context.codexPlusSettings();
  assert.equal("customLayout" in settings, false);
  assert.equal(settings.conversationView, true); assert.equal(settings.whaleWidget, true);
  assert.equal(settings.typingEffect, "stars"); assert.equal(settings.nativeMenuPlacement, false);
  assert.equal(settings.threadScrollRestore, true);
  assert.equal(stored.get("settings"), oldPreferences, "reading preferences must not rewrite other saved choices");
});

test("a stale backend layout flag cannot reappear during partial updates to retained enhancements", async () => {
  const { context, stored, writes } = fixture();
  stored.set("settings", JSON.stringify({ customLayout: true, conversationView: true, whaleWidget: true }));
  context.codexPlusBackendSettings = { enhancementsEnabled: true, codexAppCustomLayoutEnabled: true, codexAppTypingEffect: "stars" };
  assert.equal("customLayout" in context.codexPlusSettings(), false);
  context.setCodexPlusSetting("threadIdBadge", true);
  context.setCodexPlusSetting("typingEffect", "rainbow");
  await Promise.resolve();
  assert.deepEqual(writes, [["codexAppThreadIdBadge", true], ["codexAppTypingEffect", "rainbow"]]);
  const settings = context.codexPlusSettings();
  assert.equal("customLayout" in settings, false);
  assert.equal(settings.threadIdBadge, true); assert.equal(settings.typingEffect, "rainbow");
  assert.equal(settings.conversationView, true); assert.equal(settings.whaleWidget, true);
  assert.equal(JSON.parse(stored.get("settings")!).conversationView, true);
  assert.equal(JSON.parse(stored.get("settings")!).whaleWidget, true);
});
