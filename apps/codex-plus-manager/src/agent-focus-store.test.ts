import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import * as model from "./agent-focus-model.ts";

const source = ts.transpileModule(readFileSync(new URL("./agent-focus-store.ts", import.meta.url), "utf8"), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;

function harness(initial: string | null = null, options: { tauri?: boolean; invoke?: (command: string, args: unknown) => Promise<unknown> } = {}) {
  let stored = initial;
  let readFails = false;
  let writeFails = false;
  const events = new Map<string, Set<(event: { key?: string | null }) => void>>();
  const updates: unknown[] = [];
  const exports: Record<string, any> = {};
  const invoked: Array<{ command: string; args: unknown }> = [];
  const window: Record<string, any> = {
    ...(options.tauri ? { __TAURI_INTERNALS__: {} } : {}),
    location: { search: "" },
    localStorage: {
      getItem: () => { if (readFails) throw new Error("storage denied"); return stored; },
      setItem: (_key: string, value: string) => { if (writeFails) throw new Error("storage full"); stored = value; },
    },
    addEventListener: (type: string, listener: (event: { key?: string | null }) => void) => {
      const listeners = events.get(type) ?? new Set();
      listeners.add(listener);
      events.set(type, listeners);
    },
    removeEventListener: (type: string, listener: (event: { key?: string | null }) => void) => events.get(type)?.delete(listener),
    dispatchEvent: (event: { type: string; key?: string | null }) => {
      for (const listener of events.get(event.type) ?? []) listener(event);
      return true;
    },
  };
  const eventClass = class {
    type: string;
    constructor(type: string) { this.type = type; }
  };
  const settingsEvents = {
    BACKEND_SETTINGS_CHANGED_EVENT: "codex-plus:backend-settings-changed",
    announceBackendSettingsChanged: () => window.dispatchEvent(new eventClass("codex-plus:backend-settings-changed")),
  };
  vm.runInNewContext(source, {
    exports, window, URLSearchParams,
    CustomEvent: eventClass,
    require: (name: string) => {
      if (name === "./agent-focus-model") return model;
      // #2902：备份到设置文件走 Tauri；测试里没有 __TAURI_INTERNALS__，不会真的调用。
      if (name === "@tauri-apps/api/core") return {
        invoke: async (command: string, args: unknown) => {
          invoked.push({ command, args });
          return options.invoke ? options.invoke(command, args) : undefined;
        },
      };
      if (name === "./settings-events") return settingsEvents;
      if (name === "react") return {
        useState: (read: () => unknown) => [read(), (value: unknown) => updates.push(value)],
        useEffect: (effect: () => unknown) => effect(),
      };
      throw new Error(`unexpected import ${name}`);
    },
  });
  return {
    store: exports,
    window,
    updates,
    invoked,
    stored: () => stored,
    failReads: () => { readFails = true; },
    failWrites: (value = true) => { writeFails = value; },
    externalSave: (value: string | null, key: string | null = model.AGENT_FOCUS_STORAGE_KEY) => {
      stored = value;
      window.dispatchEvent({ type: "storage", key });
    },
  };
}

test("正常保存后更新同窗口订阅者，不再询问使用模式", () => {
  const f = harness();
  f.store.useAgentFocus();
  f.store.saveAgentFocus("codex");
  assert.equal(f.store.loadAgentFocus(), "codex");
  assert.equal(f.store.needAgentFocusPrompt(), false);
  assert.equal(f.stored(), model.serializeAgentFocus("codex"));
  assert.deepEqual(f.updates, ["codex"]);
});

test("写入失败时使用本次会话选择，不会重新读回旧模式", () => {
  const f = harness(model.serializeAgentFocus("all"));
  f.store.useAgentFocus();
  f.failWrites();
  f.store.saveAgentFocus("enhance");
  assert.equal(f.stored(), model.serializeAgentFocus("all"));
  assert.equal(f.store.loadAgentFocus(), "enhance");
  assert.equal(f.store.needAgentFocusPrompt(), false);
  assert.deepEqual(f.updates, ["enhance"]);
  f.failWrites(false);
  f.store.saveAgentFocus("codex");
  assert.equal(f.store.loadAgentFocus(), "codex");
  assert.equal(f.stored(), model.serializeAgentFocus("codex"));
});

test("首次启动且存储完全不可用时，选择仍在当前会话生效", () => {
  const f = harness();
  f.failReads();
  f.failWrites();
  assert.equal(f.store.needAgentFocusPrompt(), true);
  f.store.saveAgentFocus("enhance");
  assert.equal(f.store.loadAgentFocus(), "enhance");
  assert.equal(f.store.needAgentFocusPrompt(), false);
});

test("其他窗口的更新与清空覆盖旧的会话回退值", () => {
  const f = harness(model.serializeAgentFocus("all"));
  f.store.useAgentFocus();
  f.failWrites();
  f.store.saveAgentFocus("enhance");
  f.externalSave(model.serializeAgentFocus("codex"));
  assert.equal(f.store.loadAgentFocus(), "codex");
  f.externalSave(null, null);
  assert.equal(f.store.loadAgentFocus(), null);
  assert.equal(f.store.needAgentFocusPrompt(), true);
  assert.deepEqual(f.updates, ["enhance", "codex", null]);
});

test("本地存储丢了时从设置文件补回模式，补回后不再询问（#2902）", () => {
  const f = harness(null);
  assert.equal(f.store.needAgentFocusPrompt(), true);
  assert.equal(f.store.restoreAgentFocusFromSettings("enhance"), true);
  assert.equal(f.store.needAgentFocusPrompt(), false);
  assert.equal(model.parseAgentFocus(f.stored()), "enhance");
  // 设置文件里是未知值或空：不补。
  const g = harness(null);
  assert.equal(g.store.restoreAgentFocusFromSettings("bogus"), false);
  assert.equal(g.store.restoreAgentFocusFromSettings(""), false);
  assert.equal(g.store.needAgentFocusPrompt(), true);
});

test("本地已有有效选择时以本地为准，不被设置文件覆盖（#2902）", () => {
  const f = harness(model.serializeAgentFocus("codex"));
  assert.equal(f.store.restoreAgentFocusFromSettings("all"), false);
  assert.equal(model.parseAgentFocus(f.stored()), "codex");
});

test("启动判定：设置文件里有值就补回、不弹问卷，且在读回之前不先下结论（#2936）", async () => {
  // 本地存储为空（例如 WebView2 本地存储损坏），设置文件里记着：不弹。
  const f = harness(null);
  let settled = false;
  let release!: (value: string) => void;
  const pending = f.store.resolveAgentFocusPrompt(() => new Promise<string>((resolve) => { release = resolve; }))
    .then((ask: boolean) => { settled = true; return ask; });
  await Promise.resolve();
  assert.equal(settled, false, "设置没读回来之前不能先判定要弹");
  release("codex");
  assert.equal(await pending, false);
  assert.equal(model.parseAgentFocus(f.stored()), "codex");

  // 本地已有有效选择：不读设置文件，直接不弹。
  const g = harness(model.serializeAgentFocus("enhance"));
  let loaded = false;
  assert.equal(await g.store.resolveAgentFocusPrompt(async () => { loaded = true; return "all"; }), false);
  assert.equal(loaded, false);
  assert.equal(model.parseAgentFocus(g.stored()), "enhance");
});

test("启动判定：两边都没有、或设置文件读不到时才弹问卷（#2936）", async () => {
  const f = harness(null);
  assert.equal(await f.store.resolveAgentFocusPrompt(async () => ""), true);
  const g = harness(null);
  assert.equal(await g.store.resolveAgentFocusPrompt(async () => { throw new Error("load failed"); }), true);
  const h = harness(null);
  assert.equal(await h.store.resolveAgentFocusPrompt(async () => "bogus"), true);
  // 预览模式不弹，也不读设置。
  const p = harness(null);
  p.window.location.search = "?previewRoute=home";
  assert.equal(await p.store.resolveAgentFocusPrompt(async () => { throw new Error("不该读"); }), false);
});

test("备份进设置文件后广播设置已变，主界面据此同步版本号（#2937）", async () => {
  const f = harness(null, { tauri: true, invoke: async () => ({ status: "ok" }) });
  let changed = 0;
  f.window.addEventListener("codex-plus:backend-settings-changed", () => { changed += 1; });
  f.store.backupAgentFocus("all");
  assert.equal(JSON.stringify(f.invoked), JSON.stringify([{ command: "set_manager_agent_focus", args: { mode: "all" } }]));
  // 广播在写入完成之后：先于写入广播的话，主界面重读到的还是旧版本号。
  assert.equal(changed, 0);
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.equal(changed, 1);

  // 调用失败（拿不到返回）时不广播，也不抛出。
  const g = harness(null, { tauri: true, invoke: async () => { throw new Error("ipc down"); } });
  let failedChanged = 0;
  g.window.addEventListener("codex-plus:backend-settings-changed", () => { failedChanged += 1; });
  g.store.backupAgentFocus("codex");
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.equal(failedChanged, 0);
});
