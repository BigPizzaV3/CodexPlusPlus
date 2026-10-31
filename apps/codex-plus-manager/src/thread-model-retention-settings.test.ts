import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";
import { enhancementCatalog } from "./enhancement-catalog.ts";
import { EN_PLAIN } from "./i18n-en.ts";

const style = await readFile(new URL("../../../assets/inject/renderer-inject/10-style.js", import.meta.url), "utf8");
const settings = await readFile(new URL("../../../assets/inject/renderer-inject/40-backend-settings.js", import.meta.url), "utf8");
const app = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
const shim = await readFile(new URL("../web-preview/tauri-web-shim.ts", import.meta.url), "utf8");

function rendererSettings() {
  const persisted: Record<string, unknown> = { enhancementsEnabled: true, codexAppThreadModelRetention: false };
  const posts: unknown[] = [];
  let refreshes = 0;
  const window = { __codexPlusThreadModelRetentionRefresh: () => { refreshes++; } };
  const context = vm.createContext({
    window, conversationViewDefaultWidth: 900, codexPlusSettingsKey: "settings",
    localStorage: { getItem: () => "{}" },
    postJson: async (path: string, patch: Record<string, unknown>) => {
      assert.equal(path, "/settings/set");posts.push(patch);Object.assign(persisted,patch);return { ...persisted };
    },
    refreshCodexPlusBackendToggles: () => {},
  });
  const first = style.indexOf("  function defaultCodexPlusSettings()");
  const last = style.indexOf("  // Dream skin runtime", first);
  const setter = settings.slice(settings.indexOf("  async function setBackendSetting("), settings.indexOf("  /**\n   * 把直接写后端字段"));
  const api = vm.runInContext(`(()=>{
    let codexPlusBackendSettings = { enhancementsEnabled: true };
    let codexPlusBackendSettingsLoaded = false, codexPlusBackendSettingsSeq = 0;
    ${style.slice(first,last)}
    ${setter}
    return { codexPlusSettings, setBackendSetting,
      load: (value) => { codexPlusBackendSettings = value; codexPlusBackendSettingsLoaded = true; },
      snapshot: () => codexPlusBackendSettings };
  })()`, context);
  return { api, persisted, posts, refreshes: () => refreshes };
}

test("retention is a translated, master-gated switch on both settings surfaces", () => {
  const item = enhancementCatalog.items.find((entry) => entry.key === "codexAppThreadModelRetention");
  assert.ok(item);assert.equal(item.control,"bool");
  assert.deepEqual(item.surfaces,["manager","codex"]);assert.notEqual(item.masterGated,false);
  assert.ok(EN_PLAIN[item.title]);assert.ok(EN_PLAIN[item.description]);
  assert.match(app,/codexAppThreadModelRetention:\s*boolean/);
  assert.match(app,/codexAppThreadModelRetention:\s*false/);
  assert.match(shim,/codexAppThreadModelRetention:\s*false/);
});

test("renderer defaults off, maps backend settings, honors the master switch and observes transitions", async () => {
  const r=rendererSettings();
  assert.equal(r.api.codexPlusSettings().threadModelRetention,false);
  r.api.load({ ...r.persisted });
  assert.equal(r.api.codexPlusSettings().threadModelRetention,false);
  await r.api.setBackendSetting("codexAppThreadModelRetention",true);
  assert.equal(r.persisted.codexAppThreadModelRetention,true);
  assert.equal(r.api.codexPlusSettings().threadModelRetention,true);
  await r.api.setBackendSetting("enhancementsEnabled",false);
  assert.equal(r.api.codexPlusSettings().threadModelRetention,false);
  await r.api.setBackendSetting("enhancementsEnabled",true);
  await r.api.setBackendSetting("codexAppThreadModelRetention",false);
  assert.equal(r.api.codexPlusSettings().threadModelRetention,false);
  assert.equal(r.persisted.codexAppThreadModelRetention,false);
  assert.equal(r.posts.length,4);assert.ok(r.refreshes()>=4);
});
