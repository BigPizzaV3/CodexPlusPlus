// 用量状态栏、请求状态条改成了官方拓展包（assets/extensions/usage-statusline、proxy-request-status）。
// 这里按拓展内核加载包的方式（各 renderer 文件拼进同一作用域）把包放进沙箱，给一个最小的 codexPlus / ctx，
// 验证只经公开接口工作：插槽、服务、清单里声明的路由，以及停用时撤干净。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const packageSource = (dir: string, files: string[]) =>
  files.map((file) => readFileSync(new URL(`../../../assets/extensions/${dir}/${file}`, import.meta.url), "utf8")).join("\n");

function environment() {
  const styles = new Map<string, { id: string; dataset: Record<string, string>; remove(): void }>();
  const timers = new Map<number, () => void>();
  let timerId = 0;
  const schedule = (handler: () => void) => { timers.set(++timerId, handler); return timerId; };
  const clear = (id: number) => { timers.delete(id); };
  const document = {
    hidden: false,
    head: { appendChild: (node: any) => styles.set(node.id, node) },
    body: {},
    getElementById: (id: string) => styles.get(id) || null,
    createElement: (tag: string) => {
      const node: any = { tag, id: "", dataset: {}, textContent: "", remove: () => styles.delete(node.id) };
      return node;
    },
    querySelector: () => null,
    querySelectorAll: () => [],
  };
  class FakeMutationObserver {
    observed = false;
    observe() { this.observed = true; }
    disconnect() { this.observed = false; }
  }
  // 旧版注入（拆包前）留在页面里的内置运行时：拓展包装上时要先停掉它们。
  const legacyDisposed: string[] = [];
  const window = {
    setTimeout: schedule, setInterval: schedule, clearInterval: clear, clearTimeout: clear,
    __codexPlusUsageStatusLineRuntime: { dispose: () => legacyDisposed.push("usage") },
    __codexPlusProxyRequestStatusRuntime: { dispose: () => legacyDisposed.push("proxy") },
  };
  return { document, window, styles, timers, FakeMutationObserver, clear, legacyDisposed };
}

/** 一个最小的拓展实例：只有这两个包会用到的 ctx 接口；dispose 按登记逆序撤销。 */
function harness(dir: string, files: string[], exportNames: string[], callImpl: (route: string, payload: unknown, options?: unknown) => Promise<unknown>) {
  const env = environment();
  const calls: Array<{ route: string; payload: unknown; options?: unknown }> = [];
  const slots: Array<{ slot: string; render: (args: { container: unknown }) => (() => void) | void }> = [];
  const services = new Map<string, any>();
  const disposers: Array<() => void> = [];
  const ctx = {
    call: (route: string, payload: unknown, options?: unknown) => {
      calls.push({ route, payload, options });
      return callImpl(route, payload, options);
    },
    registerSlot: (definition: any) => { slots.push(definition); },
    effect: (fn: () => (() => void) | void) => { const dispose = fn(); if (typeof dispose === "function") disposers.push(dispose); },
    provide: (name: string, value: unknown) => { services.set(name, value); disposers.push(() => services.delete(name)); },
  };
  const codexPlus = {
    extension: (apply: (value: typeof ctx) => void) => apply(ctx),
    currentSession: () => ({ id: "thread-current-1" }),
    currentSessionId: () => "thread-current-1",
  };
  const scope = new Function(
    "codexPlus", "document", "window", "navigator", "MutationObserver", "clearTimeout", "Element",
    `${packageSource(dir, files)}\nreturn { ${exportNames.join(", ")} };`,
  )(codexPlus, env.document, env.window, { language: "zh-CN" }, env.FakeMutationObserver, env.clear, class {});
  const dispose = () => disposers.splice(0).reverse().forEach((fn) => fn());
  return { ...env, calls, slots, services, scope, dispose };
}

test("用量状态栏：经 composer.below 插槽挂载、提供 codexpp.usage，取数只走清单里的 /thread-usage-live", async () => {
  const live = { status: "ok", events: [] };
  const h = harness("usage-statusline", ["statusline.js", "main.js"], ["codexPlusUsageState", "codexPlusUsageHost"], async () => live);
  assert.deepEqual(h.slots.map((slot) => slot.slot), ["composer.below"]);
  assert.equal(h.scope.codexPlusUsageState.running, true, "拓展装上即开始");
  assert.deepEqual(h.legacyDisposed, ["usage"], "先停掉旧版注入里的内置用量状态栏");
  assert.ok(h.styles.has("codex-plus-usage-statusline-style"));
  assert.equal(h.styles.get("codex-plus-usage-statusline-style")!.dataset.codexPlusExt, "usage-statusline", "插入的节点带归属标记");

  // 后端调用、诊断、当前会话都走公开接口。
  await h.scope.codexPlusUsageHost.call("/thread-usage-live", { session_id: "t1" });
  await h.scope.codexPlusUsageHost.diagnose("usage_statusline_probe", { a: 1 });
  assert.deepEqual(h.calls.map((item) => item.route), ["/thread-usage-live", "/diagnostics/log"]);
  assert.deepEqual(h.calls[1].payload, { event: "usage_statusline_probe", detail: { a: 1 } });
  assert.equal(h.scope.codexPlusUsageHost.legacySessionId(), "thread-current-1");

  // 插槽容器登记后状态栏放进去；撤销时移出。
  const container = {};
  const cleanup = h.slots[0].render({ container });
  assert.ok(h.scope.codexPlusUsageState.slotHosts.has(container));
  (cleanup as () => void)();
  assert.equal(h.scope.codexPlusUsageState.slotHosts.size, 0);

  // 服务：默认取当前会话，读的是状态栏缓存的事件。
  const usage = h.services.get("codexpp.usage");
  assert.equal(usage.cached(), null);
  h.scope.codexPlusUsageState.cache.set("thread-current-1", [{ t: new Date().toISOString(), total_tokens: 500, context_window: 1000 }]);
  assert.equal(usage.cached().context, 50);
  assert.equal(usage.cached("other"), null);

  // 停用：停掉轮询、摘掉样式、撤回服务。
  h.dispose();
  assert.equal(h.scope.codexPlusUsageState.running, false);
  assert.equal(h.scope.codexPlusUsageState.timer, 0);
  assert.equal(h.styles.size, 0);
  assert.equal(h.services.has("codexpp.usage"), false);
});

test("请求状态条：经 composer.above 插槽挂载、提供 codexpp.proxyStatus，轮询 /proxy-status 带超时，停用即停", async () => {
  let reply: Promise<unknown> = Promise.resolve({ status: "ok", requests: [{ id: "r1", threadId: "t1" }] });
  const h = harness("proxy-request-status", ["status.js", "main.js"], ["codexPlusProxyStatusState"], () => reply);
  assert.deepEqual(h.slots.map((slot) => slot.slot), ["composer.above"]);
  assert.equal(h.scope.codexPlusProxyStatusState.running, true);
  assert.deepEqual(h.legacyDisposed, ["proxy"], "先停掉旧版注入里的内置请求状态条");
  assert.ok(h.styles.has("codex-plus-proxy-request-status-style"));
  assert.equal(h.timers.size, 2, "轮询与倒计时两个定时器");

  // 装上时立刻问一次；超时交给 ctx.call，不再自己拼 Promise.race。
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(h.calls[0], { route: "/proxy-status", payload: {}, options: { timeout: 2500 } });
  const service = h.services.get("codexpp.proxyStatus");
  assert.deepEqual(service.requests(), [{ id: "r1", threadId: "t1" }]);
  assert.notEqual(service.requests(), h.scope.codexPlusProxyStatusState.requests, "给出去的是副本");

  // fetch 失败时回空数组，不抛给调用方。
  reply = Promise.reject(new Error("调用 /proxy-status 超时"));
  assert.deepEqual(await service.fetch(), []);
  reply = Promise.resolve({ status: "ok", requests: [{ id: "r2" }] });
  assert.deepEqual(await service.fetch(), [{ id: "r2" }]);

  h.dispose();
  assert.equal(h.scope.codexPlusProxyStatusState.running, false);
  assert.equal(h.timers.size, 0, "停用后不再轮询");
  assert.equal(h.styles.size, 0);
  assert.deepEqual(h.scope.codexPlusProxyStatusState.requests, []);
  assert.equal(h.services.has("codexpp.proxyStatus"), false);
});
