import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

// 打字特效拓展的入口（assets/extensions/typing-effects/main.js）：按拓展内核的真实顺序驱动——
// 先调 onConfigUpdate(next, prev)，接住后才把 ctx.config 换成新值。
const main = readFileSync(new URL("../../../assets/extensions/typing-effects/main.js", import.meta.url), "utf8");

function load(options: { legacy?: boolean } = {}) {
  const calls: string[] = [];
  let getMode: () => unknown = () => undefined;
  let config: Record<string, unknown> = { effect: "stars" };
  const effects: Array<() => void | (() => void)> = [];
  const cleanups: Array<() => void> = [];
  const updates: Array<(next: Record<string, unknown>) => unknown> = [];
  const window: Record<string, unknown> = {};
  if (options.legacy) window.__codexPlusTypingEffectsRuntime = { dispose: () => calls.push("legacy:dispose") };
  const ctx = {
    get config() { return config; },
    effect: (fn: () => void | (() => void)) => effects.push(fn),
    onConfigUpdate: (fn: (next: Record<string, unknown>) => unknown) => updates.push(fn),
  };
  const context = createContext({
    window,
    codexPlus: { extension: (apply: (ctx: unknown) => void) => apply(ctx) },
    createCodexPlusTypingEffects: (mode: () => unknown) => {
      getMode = mode;
      return {
        install: () => calls.push(`install:${mode()}`),
        uninstall: () => calls.push("uninstall"),
        sync: () => calls.push(`sync:${mode()}`),
      };
    },
  });
  runInContext(main, context);
  const start = () => effects.forEach((fn) => { const cleanup = fn(); if (typeof cleanup === "function") cleanups.push(cleanup); });
  const update = (next: Record<string, unknown>) => {
    const accepted = updates.every((fn) => fn(Object.freeze({ ...next })) === true);
    if (accepted) config = Object.freeze({ ...next });
    return accepted;
  };
  return { calls, start, update, stop: () => cleanups.forEach((fn) => fn()), mode: () => getMode() };
}

test("changing the effect takes the new style immediately instead of the stale ctx.config", () => {
  const f = load();
  f.start();
  assert.deepEqual(f.calls, ["install:stars"]);
  assert.equal(f.update({ effect: "fireworks" }), true, "接住新配置，不整体热重载");
  assert.deepEqual(f.calls, ["install:stars", "sync:fireworks"]);
  assert.equal(f.mode(), "fireworks");
  f.stop();
  assert.equal(f.calls.at(-1), "uninstall");
});

test("an older injection's built-in typing effect is stopped so the two never stack", () => {
  const f = load({ legacy: true });
  f.start();
  assert.deepEqual(f.calls, ["legacy:dispose", "install:stars"]);
  const plain = load();
  plain.start();
  assert.deepEqual(plain.calls, ["install:stars"]);
});
