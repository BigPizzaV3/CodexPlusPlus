import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

// 内置拓展挂在拓展内核上：扫描步骤由拓展登记、卸下即撤；打字特效的全局监听随拓展装卸；内置数据作为服务提供。
const read = (name: string) => readFileSync(new URL(`../../../assets/inject/renderer-inject/${name}`, import.meta.url), "utf8");
const builtins = read("96-builtin-extensions.js");
const scan = read("95-conversation-view.js");
const extract = (source: string, name: string) => {
  const body = source.match(new RegExp(`^  function ${name}\\([^]*?^  \\}`, "m"))?.[0];
  assert.ok(body, name);
  return body;
};

function harness() {
  const calls: string[] = [];
  const services = new Map<string, { value: any; owner: string }>();
  const handles: Array<{ id: string; dispose(): void }> = [];
  const known: Record<string, unknown> = {
    window: { __codexPlusBuiltinExtensionState: { disabled: [], replacedBy: {} } },
    codexPlusServices: services,
    codexPlusProvideService: (name: string, value: unknown, owner: string) => services.set(name, { value, owner }),
    codexPlusMarkExtensionFailure: (key: string, message: string) => calls.push(`fail:${key}:${message}`),
    codexPlusCreateExtension(definition: any) {
      const disposers: Array<() => void> = [];
      const ctx = { effect: (fn: () => () => void) => { const dispose = fn(); if (typeof dispose === "function") disposers.push(dispose); return dispose; } };
      definition.apply(ctx);
      const handle = { id: definition.id, dispose: () => disposers.reverse().forEach((dispose) => dispose()) };
      handles.push(handle);
      return handle;
    },
    installCodexPlusTypingEffects: () => calls.push("typing:install"),
    uninstallCodexPlusTypingEffects: () => calls.push("typing:uninstall"),
    refreshDreamSkin: () => calls.push("dream:refresh"),
    codexPlusUsageState: { cache: new Map([["t1", [{ total_tokens: 10 }]]]) },
    codexPlusUsageSnapshot: (events: unknown[] | undefined) => (events ? { events: events.length } : null),
    codexPlusProxyStatusState: { requests: [{ id: "r1" }] },
    codexPlusProxyStatusPostWithTimeout: async () => ({ status: "ok", requests: [{ id: "r2" }] }),
    codexPlusCurrentSessionId: () => "t1",
  };
  // 其余被引用的功能函数一律当作无副作用的桩。
  const scope = new Proxy(known, {
    has: (_target, key) => typeof key === "string" && !["globalThis", "Map", "Set", "Object", "Array", "String", "Error"].includes(key),
    get: (target, key) => (key in target ? target[key as string] : key === Symbol.unscopables ? undefined : () => undefined),
  });
  const source = [
    extract(scan, "codexPlusScanSteps"), extract(scan, "registerCodexPlusScanStep"), extract(scan, "runCodexPlusScanSteps"), extract(scan, "runScanStep"),
    builtins,
  ].join("\n");
  const api = new Function("scope", `with (scope) { ${source}\n return { sync: syncCodexPlusBuiltinExtensions, runSteps: runCodexPlusScanSteps, steps: codexPlusScanSteps }; }`)(scope);
  return { api, calls, services, known };
}

test("内置拓展装上时登记扫描步骤、挂打字特效监听；停用后撤掉", () => {
  (globalThis as any).__codexPlusScanSteps = undefined;
  (globalThis as any).__codexPlusBuiltinInactive = undefined;
  const { api, calls, known } = harness();
  api.sync();
  assert.ok(calls.includes("typing:install"));
  assert.ok(api.steps().has("dream-skin") && api.steps().has("side-chat-save") && api.steps().has("agent-pet"));
  calls.length = 0;
  api.runSteps();
  assert.ok(calls.includes("dream:refresh"), "扫描循环经登记的步骤刷新梦皮");
  (known.window as any).__codexPlusBuiltinExtensionState = { disabled: ["dream-skin", "typing-effects"], replacedBy: {} };
  calls.length = 0;
  api.sync();
  assert.ok(!api.steps().has("dream-skin"), "停用后扫描不再点名它");
  assert.ok(calls.includes("typing:uninstall"), "停用后摘掉打字特效的全局监听");
  calls.length = 0;
  api.runSteps();
  assert.ok(!calls.includes("dream:refresh"));
});

test("内置数据作为服务提供，界面那一层被顶替也能拿到", async () => {
  (globalThis as any).__codexPlusScanSteps = undefined;
  const { api, services } = harness();
  api.sync();
  assert.equal(services.get("codexpp.usage")?.owner, "builtin");
  assert.deepEqual(services.get("codexpp.usage")?.value.cached(), { events: 1 }, "默认取当前会话");
  assert.equal(services.get("codexpp.usage")?.value.cached("nope"), null);
  assert.deepEqual(services.get("codexpp.proxyStatus")?.value.requests(), [{ id: "r1" }]);
  assert.deepEqual(await services.get("codexpp.proxyStatus")?.value.fetch(), [{ id: "r2" }]);
});
