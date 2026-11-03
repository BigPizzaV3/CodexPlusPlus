import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";

const source = readFileSync(
  new URL("../../../assets/inject/renderer-inject/10-style.js", import.meta.url),
  "utf8",
);

function slice(startMarker: string, endMarker: string) {
  const start = source.indexOf(startMarker);
  const end = source.indexOf(endMarker, start);
  assert.ok(start >= 0 && end > start, `missing ${startMarker}`);
  return source.slice(start, end);
}

function harness() {
  const calls = {
    fullScans: 0, queries: 0, writes: 0, installs: 0, ensures: 0,
    companions: 0, cleanups: 0, disconnects: 0, revocations: 0,
  };
  function element(classes: string[] = []) {
    const names = new Set(classes);
    return {
      classList: {
        [Symbol.iterator]: () => names.values(),
        contains: (name: string) => names.has(name),
        add: (name: string) => { names.add(name); },
        remove: (...removed: string[]) => {
          calls.writes += 1;
          removed.forEach((name) => names.delete(name));
        },
      },
      style: { removeProperty: () => { calls.writes += 1; } },
      removeAttribute: () => { calls.writes += 1; },
    };
  }
  const legacy = element(["theme-legacy-home", "native-class"]);
  const document = {
    documentElement: element(["codex-dream-skin", "electron-dark"]),
    querySelectorAll: (selector: string) => {
      calls.queries += 1;
      if (selector !== "[class]") return [];
      calls.fullScans += 1;
      return [document.documentElement, legacy];
    },
    getElementById: () => null,
  };
  const window: Record<string, any> = {
    __CODEX_PLUS_EXTERNAL_DREAM_SKIN_RUNTIME__: true,
    removeEventListener: () => {},
  };
  const settings = { dreamSkinEnabled: false, dreamSkinPaused: false, dreamSkinThemeConfig: {} };
  const context = vm.createContext({
    window, document,
    codexPlusBackendSettingsLoaded: true,
    codexPlusSettings: () => settings,
    codexPlusDreamSkinStyleId: "codex-dream-skin-style",
    clearInterval: () => {},
    clearTimeout: () => {},
    URL: { revokeObjectURL: () => { calls.revocations += 1; } },
    removeDreamSkinCompanion: () => {},
    clearDreamSkinMainSurfaceCompatibility: () => {},
    ensureDreamSkinMainSurface: () => {},
    ensureDreamSkinCompanion: () => { calls.companions += 1; },
    installDreamSkin: () => { calls.installs += 1; },
  });
  const runtimeSource = [
    slice("  function clearDreamSkinPresentation(", "  function dreamSkinContentSignature("),
    slice("  function dreamSkinRefreshAction(", "  function applyDreamSkinLiveUpdate("),
  ].join("\n");
  // 每次注入有独立闭包，但沿用页面与外部运行时。
  const inject = () => vm.runInContext(
    `(() => { ${runtimeSource}\n return refreshDreamSkin; })()`,
    context,
  ) as () => void;
  return { calls, document, window, settings, context, element, legacy, inject, refresh: inject() };
}

test("disabled dream skin cleans legacy remnants once without rescanning or writing on later scans", () => {
  const runtime = harness();
  runtime.refresh();
  assert.equal(runtime.calls.fullScans, 1);
  assert.equal(runtime.legacy.classList.contains("theme-legacy-home"), false);
  assert.equal(runtime.legacy.classList.contains("native-class"), true);
  assert.equal(runtime.document.documentElement.classList.contains("codex-dream-skin"), false);
  assert.equal(runtime.document.documentElement.classList.contains("electron-dark"), true);
  const afterCleanup = { ...runtime.calls };

  for (let index = 0; index < 100; index += 1) runtime.refresh();
  assert.deepEqual(runtime.calls, afterCleanup);
});

test("enabling dream skin rearms cleanup even when its external runtime disappears before disabling", () => {
  const runtime = harness();
  runtime.refresh();
  runtime.settings.dreamSkinEnabled = true;
  runtime.window.__CODEX_DREAM_SKIN_STATE__ = {
    ensure: () => { runtime.calls.ensures += 1; },
  };
  runtime.refresh();
  assert.equal(runtime.calls.ensures, 1);

  delete runtime.window.__CODEX_DREAM_SKIN_STATE__;
  runtime.legacy.classList.add("theme-legacy-home");
  runtime.settings.dreamSkinEnabled = false;
  runtime.refresh();
  runtime.refresh();
  assert.equal(runtime.calls.fullScans, 2);
  assert.equal(runtime.legacy.classList.contains("theme-legacy-home"), false);
});

test("a late external runtime is cleaned even if the disabled settings never changed", () => {
  const runtime = harness();
  runtime.refresh();
  runtime.window.__CODEX_DREAM_SKIN_STATE__ = {
    cleanup: () => { runtime.calls.cleanups += 1; },
    observer: { disconnect: () => { runtime.calls.disconnects += 1; } },
    timer: 1,
    scheduler: { timeout: 2 },
    artUrl: "blob:theme-test",
  };
  runtime.window.__CODEX_GLASS_VISION_SKIN_STATE__ = {
    cleanup: () => { runtime.calls.cleanups += 1; },
  };

  runtime.refresh();
  runtime.refresh();
  assert.equal(runtime.calls.fullScans, 2);
  assert.equal(runtime.calls.cleanups, 2);
  assert.equal(runtime.calls.disconnects, 1);
  assert.equal(runtime.calls.revocations, 1);
  assert.equal(runtime.window.__CODEX_DREAM_SKIN_STATE__, undefined);
  assert.equal(runtime.window.__CODEX_GLASS_VISION_SKIN_STATE__, undefined);
});

test("root replacement and reinjection each retain their first legacy cleanup", () => {
  const runtime = harness();
  runtime.refresh();
  runtime.document.documentElement = runtime.element(["codex-dream-skin"]);
  runtime.refresh();
  runtime.refresh();
  assert.equal(runtime.calls.fullScans, 2);
  assert.equal(runtime.document.documentElement.classList.contains("codex-dream-skin"), false);

  runtime.legacy.classList.add("theme-legacy-home");
  const reinjectedRefresh = runtime.inject();
  reinjectedRefresh();
  reinjectedRefresh();
  assert.equal(runtime.calls.fullScans, 3);
  assert.equal(runtime.legacy.classList.contains("theme-legacy-home"), false);
});

test("explicit external cleanup still removes newly introduced remnants while ordinary scans stay idle", () => {
  const runtime = harness();
  runtime.refresh();
  runtime.legacy.classList.add("theme-legacy-home");
  runtime.window.__CODEX_PLUS_CLEAR_DREAM_SKIN__();
  assert.equal(runtime.calls.fullScans, 2);
  assert.equal(runtime.legacy.classList.contains("theme-legacy-home"), false);
  runtime.refresh();
  assert.equal(runtime.calls.fullScans, 2);
});

test("backend loading preserves an installed external theme until settings explicitly disable it", () => {
  const runtime = harness();
  runtime.context.codexPlusBackendSettingsLoaded = false;
  runtime.window.__CODEX_DREAM_SKIN_STATE__ = {
    ensure: () => { runtime.calls.ensures += 1; },
  };
  runtime.refresh();
  assert.equal(runtime.calls.fullScans, 0);
  assert.equal(runtime.calls.ensures, 1);

  runtime.context.codexPlusBackendSettingsLoaded = true;
  runtime.refresh();
  runtime.refresh();
  assert.equal(runtime.calls.fullScans, 1);
});

test("internal theme installation and pause transitions rearm cleanup", () => {
  const runtime = harness();
  runtime.window.__CODEX_PLUS_EXTERNAL_DREAM_SKIN_RUNTIME__ = false;
  runtime.refresh();
  runtime.settings.dreamSkinEnabled = true;
  runtime.refresh();
  assert.equal(runtime.calls.installs, 1);

  runtime.settings.dreamSkinPaused = true;
  runtime.refresh();
  runtime.refresh();
  assert.equal(runtime.calls.fullScans, 2);
  assert.equal(runtime.calls.installs, 1);

  runtime.settings.dreamSkinPaused = false;
  runtime.refresh();
  runtime.settings.dreamSkinEnabled = false;
  runtime.refresh();
  assert.equal(runtime.calls.fullScans, 3);
  assert.equal(runtime.calls.installs, 2);
});
