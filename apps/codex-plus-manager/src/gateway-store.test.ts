/** 网关共享数据：一个轮询、按最短间隔、窗口不可见时暂停、读取去重、保存后丢旧答复、读失败记错误。 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = ts.transpileModule(readFileSync(new URL("./gateway-store.ts", import.meta.url), "utf8"), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;

type Pending = { resolve: (value: unknown) => void; reject: (error: unknown) => void };

function payload(port: number) {
  return { status: "ok", message: "", gateway: { bindings: {} }, pool: [], agents: [], port };
}

function harness() {
  let now = 1_000;
  let nextTimer = 0;
  const timers = new Map<number, { at: number; run: () => void }>();
  const pending: Pending[] = [];
  const visibility = new Set<() => void>();
  const document = {
    visibilityState: "visible",
    addEventListener: (_name: string, listener: () => void) => visibility.add(listener),
    removeEventListener: (_name: string, listener: () => void) => visibility.delete(listener),
  };
  const exports: Record<string, any> = {};
  vm.runInNewContext(source, {
    exports,
    document,
    Date: { now: () => now },
    setTimeout: (run: () => void, delay: number) => {
      const id = ++nextTimer;
      timers.set(id, { at: now + delay, run });
      return id;
    },
    clearTimeout: (id: number | undefined) => {
      if (id !== undefined) timers.delete(id);
    },
    require: (name: string) => {
      if (name === "@tauri-apps/api/core") return {
        invoke: (command: string) => {
          assert.equal(command, "load_gateway");
          return new Promise((resolve, reject) => pending.push({ resolve, reject }));
        },
      };
      if (name === "./gateway-model") return { rememberAgentNames: () => {} };
      if (name === "react") return { createContext: () => ({}), useEffect: () => {}, useSyncExternalStore: () => null };
      assert.fail(name);
    },
  });
  const flush = async () => {
    for (let index = 0; index < 10; index += 1) await Promise.resolve();
  };
  return {
    store: exports,
    pending,
    timers,
    document,
    flush,
    /** 时间往前走，跑到点的计时器。 */
    advance: async (ms: number) => {
      now += ms;
      for (const [id, timer] of [...timers]) {
        if (timer.at <= now) {
          timers.delete(id);
          timer.run();
        }
      }
      await flush();
    },
    setVisible: async (visible: boolean) => {
      document.visibilityState = visible ? "visible" : "hidden";
      for (const listener of visibility) listener();
      await flush();
    },
  };
}

test("多个组件订阅只有一个计时器，按最短间隔读；同时要读的共用一个请求", async () => {
  const { store, pending, timers, advance, flush } = harness();
  const stopSidebar = store.watchGateway(10_000);
  const stopHub = store.watchGateway(3_000);
  assert.equal(pending.length, 1, "首次读取只发一次");
  pending.shift()!.resolve(payload(1));
  await flush();
  assert.equal(store.getGatewayState().payload.port, 1);
  assert.equal(timers.size, 1, "只有一个计时器");
  await advance(3_000);
  assert.equal(pending.length, 1, "按最短的 3 秒读");
  pending.shift()!.resolve(payload(1));
  await flush();
  stopHub();
  assert.equal(timers.size, 1);
  await advance(3_000);
  assert.equal(pending.length, 0, "总览走了，回到 10 秒");
  await advance(7_000);
  assert.equal(pending.length, 1);
  pending.shift()!.resolve(payload(1));
  await flush();
  stopSidebar();
  assert.equal(timers.size, 0, "没人用就不再读");
});

test("数据没变不通知；轮询不加版本号", async () => {
  const { store, pending, flush, advance } = harness();
  let notified = 0;
  store.subscribeGatewayState(() => { notified += 1; });
  store.watchGateway(1_000);
  pending.shift()!.resolve(payload(1));
  await flush();
  const first = store.getGatewayState();
  assert.equal(notified, 1);
  await advance(1_000);
  pending.shift()!.resolve(payload(1));
  await flush();
  assert.equal(notified, 1, "一样的数据不重渲染");
  assert.equal(store.getGatewayState(), first);
});

test("读取期间保存过：旧答复丢掉，不盖回刚保存的状态", async () => {
  const { store, pending, flush } = harness();
  store.watchGateway(1_000);
  store.publishGatewayPayload(payload(2));
  pending.shift()!.resolve(payload(1));
  await flush();
  assert.equal(store.getGatewayState().payload.port, 2);
  // 保存之后再要读：不能共用保存前发出的那一个。
  const again = store.refreshGateway();
  assert.equal(pending.length, 1);
  pending.shift()!.resolve(payload(3));
  assert.equal((await again).port, 3);
  assert.equal(store.getGatewayState().payload.port, 3);
});

test("窗口不可见时不读，重新可见时补一轮", async () => {
  const { store, pending, timers, flush, advance, setVisible } = harness();
  store.watchGateway(1_000);
  pending.shift()!.resolve(payload(1));
  await flush();
  await setVisible(false);
  assert.equal(timers.size, 0);
  await advance(5_000);
  assert.equal(pending.length, 0);
  await setVisible(true);
  assert.equal(pending.length, 1, "回到前台立刻读");
});

test("读取失败：保留上一份数据，记下错误；成功后清掉", async () => {
  const { store, pending, flush } = harness();
  store.watchGateway(1_000);
  pending.shift()!.reject(new Error("boom"));
  await flush();
  assert.equal(store.getGatewayState().payload, null);
  assert.match(store.getGatewayState().error, /boom/);
  const retry = store.refreshGateway();
  pending.shift()!.resolve({ status: "failed", message: "读取网关状态失败：坏了" });
  await retry;
  assert.equal(store.getGatewayState().error, "读取网关状态失败：坏了");
  const ok = store.refreshGateway();
  pending.shift()!.resolve(payload(1));
  await ok;
  assert.deepEqual(store.getGatewayState().error, null);
  assert.equal(store.getGatewayState().payload.port, 1);
});
