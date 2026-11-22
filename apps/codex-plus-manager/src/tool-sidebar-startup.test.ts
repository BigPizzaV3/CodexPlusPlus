import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { it } from "node:test";

it("广告请求持续未完成时，侧栏工具列表、设置等本地加载照常完成并释放隐藏窗口", async () => {
  const source = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
  const effect = source.slice(source.indexOf("    void refreshTools(true);"));
  const startup = effect.slice(0, effect.indexOf("  }, []);"));
  const events: string[] = [];
  let releaseAds!: () => void;
  const pendingAds = new Promise<void>((resolve) => { releaseAds = resolve; });
  const noOp = async () => {};
  const dependencies = {
    refreshTools: async () => { events.push("tools"); },
    run: async (task: () => Promise<unknown>) => await task(),
    call: async () => ({ showUpdate: false }),
    consumePendingManagerNavigation: async () => false,
    setRoute: noOp,
    checkUpdate: noOp,
    refreshOverview: noOp,
    refreshAds: async () => { events.push("ads"); await pendingAds; },
    refreshSettings: async () => { events.push("settings"); return null; },
    agentFocusChecking: false,
    resolveAgentFocusPrompt: async () => false,
    setAgentFocusOpen: noOp,
    setAgentFocusChecking: noOp,
    setSettingsFocus: noOp,
    refreshEnvConflicts: noOp,
    refreshProviderSyncTargets: noOp,
    refreshPendingProviderImport: noOp,
    refreshPendingDreamSkinCommunity: noOp,
    releaseMainWindowWhenHidden: () => { events.push("release"); },
  };
  // 仅去掉 TS 泛型，让实际启动 effect 在受控异步依赖中运行。
  new Function(...Object.keys(dependencies), startup.replace(/call<StartupResult>/g, "call"))(
    ...Object.values(dependencies),
  );
  try {
    await new Promise((resolve) => setImmediate(resolve));
    assert.ok(events.includes("ads"), "测试必须真的到达未完成的广告请求");
    assert.ok(events.includes("tools"), "广告尚未返回，工具列表也应已加载");
    assert.ok(events.includes("settings"), "广告尚未返回，设置也应已加载");
    assert.ok(events.includes("release"), "启动流程不等广告，走完就按轻量模式释放隐藏窗口");
  } finally {
    releaseAds();
  }
});
