import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { it } from "node:test";
import { createStableCallbacks } from "./use-stable-callbacks.ts";

it("稳定回调对象引用不变，调用时转到最新一次传入的函数", () => {
  let latest = { read: () => "old", add: (a: number, b: number) => a + b };
  const stable = createStableCallbacks(Object.keys(latest) as (keyof typeof latest)[], () => latest);
  const read = stable.read;
  assert.equal(read(), "old");
  latest = { read: () => "new", add: (a: number, b: number) => a * b };
  assert.equal(read(), "new", "同一个方法引用读到最新闭包");
  assert.equal(stable.add(3, 4), 12);
});

it("App 的 actions 方法走稳定回调，切换中等会变的值单独进 useMemo 依赖", async () => {
  const app = (await readFile(new URL("./App.tsx", import.meta.url), "utf8")).replace(/\r\n/g, "\n");
  assert.match(app, /const actionCallbacks = useStableCallbacks\(\{/);
  assert.match(app, /\[actionCallbacks, multiLaunchPending, multiLaunchInstances, multiLaunchInstancesLoading, multiLaunchActionPending, relaySwitching\],/);
  // 切换供应商同步占住，连点两下只发起一次。
  assert.match(app, /if \(relaySwitchingRef\.current\) \{\n\s*showNotice\(t\("供应商切换中"\)/);
  assert.match(app, /relaySwitchingRef\.current = true;\n\s*try \{\n\s*return await performRelaySwitch\(/);
});
