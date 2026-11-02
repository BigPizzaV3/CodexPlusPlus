import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import * as model from "./agent-focus-model.ts";

const source = ts.transpileModule(readFileSync(new URL("./agent-focus-store.ts", import.meta.url), "utf8"), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;

function harness(initial: string | null = null) {
  let stored = initial;
  let readFails = false;
  let writeFails = false;
  const events = new Map<string, Set<(event: { key?: string | null }) => void>>();
  const updates: unknown[] = [];
  const exports: Record<string, any> = {};
  const window = {
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
  vm.runInNewContext(source, {
    exports, window, URLSearchParams,
    CustomEvent: class {
      type: string;
      constructor(type: string) { this.type = type; }
    },
    require: (name: string) => {
      if (name === "./agent-focus-model") return model;
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
