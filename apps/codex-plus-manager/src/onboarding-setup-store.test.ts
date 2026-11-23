import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import * as setupModel from "./onboarding-setup-model.ts";
import * as tourModel from "./onboarding-model.ts";

const source = ts.transpileModule(readFileSync(new URL("./onboarding-setup-store.ts", import.meta.url), "utf8"), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;

const flush = () => new Promise<void>((resolve) => setImmediate(resolve));

/** 真实的存储层，只替换 localStorage、地址栏和桌面命令。 */
function harness(initial: Record<string, string> = {}, options: { tauri?: boolean; search?: string; invoke?: () => Promise<unknown> } = {}) {
  const stored = new Map(Object.entries(initial));
  let readFails = false;
  let writeFails = false;
  const invoked: Array<{ command: string; args: unknown }> = [];
  const events: string[] = [];
  const exports: Record<string, any> = {};
  const window: Record<string, any> = {
    ...(options.tauri ? { __TAURI_INTERNALS__: {} } : {}),
    location: { search: options.search ?? "" },
    localStorage: {
      getItem: (key: string) => { if (readFails) throw new Error("storage denied"); return stored.get(key) ?? null; },
      setItem: (key: string, value: string) => { if (writeFails) throw new Error("storage full"); stored.set(key, value); },
    },
  };
  vm.runInNewContext(source, {
    exports, window, URLSearchParams,
    require: (name: string) => {
      if (name === "./onboarding-setup-model") return setupModel;
      if (name === "./onboarding-model") return tourModel;
      if (name === "@tauri-apps/api/core") return {
        invoke: async (command: string, args: unknown) => {
          invoked.push({ command, args });
          return options.invoke ? options.invoke() : undefined;
        },
      };
      if (name === "./settings-events") return { announceBackendSettingsChanged: () => events.push("settings-changed") };
      throw new Error(`unexpected import ${name}`);
    },
  });
  return {
    store: exports, invoked, events,
    setup: () => stored.get(setupModel.SETUP_STORAGE_KEY) ?? null,
    failReads: () => { readFails = true; },
    failWrites: () => { writeFails = true; },
  };
}

test("新用户：侧栏有进度卡，向导自动打开；预览模式两样都不出", () => {
  const fresh = harness();
  assert.equal(fresh.store.setupCardNeeded(), true);
  assert.equal(fresh.store.setupAutoOpenPossible(), true);
  const preview = harness({}, { search: "?previewRoute=relay" });
  assert.equal(preview.store.setupCardNeeded(), false);
  assert.equal(preview.store.setupAutoOpenPossible(), false);
  // 本地存储读不了：按新用户处理，和以前一样。
  const denied = harness();
  denied.failReads();
  assert.equal(denied.store.setupCardNeeded(), true);
  assert.equal(denied.store.setupAutoOpenPossible(), true);
});

test("向导出现过之后只留进度卡；走完、关掉、看过旧导览、答过旧版使用模式问卷的都不再自动打开", () => {
  const later = harness({ [setupModel.SETUP_STORAGE_KEY]: setupModel.serializeSetupState("later") });
  assert.equal(later.store.setupCardNeeded(), true);
  assert.equal(later.store.setupAutoOpenPossible(), false);
  for (const state of ["completed", "dismissed"] as const) {
    const done = harness({ [setupModel.SETUP_STORAGE_KEY]: setupModel.serializeSetupState(state) });
    assert.equal(done.store.setupCardNeeded(), false);
    assert.equal(done.store.setupAutoOpenPossible(), false);
  }
  const toured = harness({ [tourModel.TOUR_STORAGE_KEY]: "1" });
  assert.equal(toured.store.setupCardNeeded(), false);
  assert.equal(toured.store.setupAutoOpenPossible(), false);
  // 老用户答过使用模式问卷、清单还没做完：进度卡照旧，向导不弹。
  const legacy = harness({ [setupModel.LEGACY_FOCUS_STORAGE_KEY]: '{"version":1,"mode":"enhance"}' });
  assert.equal(legacy.store.setupCardNeeded(), true);
  assert.equal(legacy.store.setupAutoOpenPossible(), false);
});

test("启动时先读设置文件里的记号再决定弹不弹：本地存储丢了也不会每次都弹（#2902 / #2936）", async () => {
  // 本地说没出现过，设置文件里留着记号（包括旧版写下的模式名）：不弹。
  for (const marker of ["all", "codex", "enhance"]) {
    assert.equal(await harness().store.resolveSetupAutoOpen(async () => marker), false, marker);
  }
  // 两处都没有：弹。空白、读不到、读取抛错都按没有记号处理。
  for (const marker of ["", "  ", null, undefined]) {
    assert.equal(await harness().store.resolveSetupAutoOpen(async () => marker), true, String(marker));
  }
  assert.equal(await harness().store.resolveSetupAutoOpen(async () => { throw new Error("settings unavailable"); }), true);
  // 本地已经知道出现过：不用等设置，也不读。
  let asked = false;
  const seen = harness({ [setupModel.SETUP_STORAGE_KEY]: setupModel.serializeSetupState("later") });
  assert.equal(await seen.store.resolveSetupAutoOpen(async () => { asked = true; return ""; }), false);
  assert.equal(asked, false);
});

test("向导自动打开时两处都记下出现过，并通知主界面重读设置（#2937）", async () => {
  const f = harness({}, { tauri: true });
  f.store.markSetupSeen();
  assert.deepEqual(JSON.parse(f.setup()!), { version: 1, state: "later" });
  assert.equal(f.store.setupAutoOpenPossible(), false);
  assert.equal(f.store.setupCardNeeded(), true, "进度卡留着");
  assert.equal(f.invoked.length, 1);
  assert.equal(f.invoked[0].command, "set_manager_agent_focus");
  assert.equal((f.invoked[0].args as { mode: string }).mode, "all", "后端只认旧的三个模式名，记号写 all");
  await flush();
  assert.deepEqual(f.events, ["settings-changed"], "后端改了 settings.json 的版本号，主界面要重读");

  // 网页预览里没有 Tauri：只记本地。
  const web = harness();
  web.store.markSetupSeen();
  assert.equal(web.invoked.length, 0);
  assert.deepEqual(JSON.parse(web.setup()!), { version: 1, state: "later" });

  // 写设置失败不抛错，也不广播；本地存不了同样不抛。
  const failing = harness({}, { tauri: true, invoke: async () => { throw new Error("settings locked"); } });
  failing.failWrites();
  failing.store.markSetupSeen();
  await flush();
  assert.deepEqual(failing.events, []);
  assert.equal(failing.setup(), null);
});

test("走完和关掉各自记下，之后进度卡不再出现", () => {
  for (const state of ["completed", "dismissed"] as const) {
    const f = harness({ [setupModel.SETUP_STORAGE_KEY]: setupModel.serializeSetupState("later") });
    f.store.saveSetupState(state);
    assert.deepEqual(JSON.parse(f.setup()!), { version: 1, state });
    assert.equal(f.store.setupCardNeeded(), false);
  }
});
