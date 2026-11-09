import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";

const source = readFileSync(new URL("../../../assets/inject/renderer-inject/32-fork-readiness-fallback.js", import.meta.url), "utf8");
// 真实注入时整份脚本包在一个 IIFE 里，重新注入会得到新的作用域。
const inject = `(() => {\n${source}\nreturn installCodexForkReadinessFallback();\n})()`;

// 仿照 Codex 26.1027 的原生实现：getForkCapabilities 把 this.settings.read 绑定后交给 readPlacement。
function nativeModule() {
  class AppServerManager {
    hostId = "local";
    disposed = false;
    #secret = "private";
    runtime: any;
    settings: any;
    constructor(runtime: any, settings: any) {
      this.runtime = runtime;
      this.settings = settings;
    }
    secret() { return this.#secret; }
    getForkCapabilities() {
      const readSetting = this.settings.read.bind(this.settings);
      const isActive = () => !this.disposed;
      return {
        hostId: this.hostId,
        isActive,
        readPlacement() {
          const sections = readSetting("sidebarCustomSectionsEnabled");
          const appServer = readSetting("useAppServerCustomSections");
          if (sections === undefined || appServer === undefined) return { status: "not-ready", reason: "execution-config-loading" };
          return { status: "ready", placement: { sections, appServer } };
        },
      };
    }
  }
  return { rYn: AppServerManager, other: { getForkCapabilities() {} } };
}

class Settings {
  #gates: Record<string, boolean | undefined>;
  constructor(gates: Record<string, boolean | undefined>) { this.#gates = gates; }
  read(key: string) { return this.#gates[key]; }
}

async function install(enhancementsEnabled = true) {
  const module = nativeModule();
  const diagnostics: unknown[][] = [];
  const window: any = { setTimeout: () => 0 };
  const context = vm.createContext({
    window,
    codexPlusBackendSettings: { enhancementsEnabled },
    loadOptionalCodexAppModule: async (name: string) => (name === "app-initial-" ? module : null),
    sendCodexPlusDiagnostic: (...args: unknown[]) => diagnostics.push(args),
  });
  const patched = await vm.runInContext(inject, context);
  return { Manager: module.rYn, patched, diagnostics, context };
}

test("无账号且 Statsig 门控读不到时，Fork 放置按缺省 false 继续，不再停在 not-ready", async () => {
  const { Manager, patched, diagnostics } = await install();
  assert.equal(patched, true);
  const manager = new Manager({ accountId: () => null }, new Settings({ sidebarCustomSectionsEnabled: false, useAppServerCustomSections: undefined }));
  const capabilities = manager.getForkCapabilities();
  assert.deepEqual(JSON.parse(JSON.stringify(capabilities.readPlacement())), { status: "ready", placement: { sections: false, appServer: false } });
  // 其他字段照常从原实例读取，实例本身不被改动。
  assert.equal(capabilities.hostId, "local");
  assert.equal(capabilities.isActive(), true);
  manager.disposed = true;
  assert.equal(capabilities.isActive(), false);
  assert.equal(manager.secret(), "private");
  assert.equal(diagnostics.length, 1);
});

test("登录账号时保持原生行为：门控没就绪就照常返回 not-ready", async () => {
  const { Manager } = await install();
  const manager = new Manager({ accountId: () => "account-1" }, new Settings({ sidebarCustomSectionsEnabled: true, useAppServerCustomSections: undefined }));
  assert.equal(manager.getForkCapabilities().readPlacement().status, "not-ready");
});

test("门控有值时原样使用，增强总开关关闭时完全不介入", async () => {
  const ready = await install();
  const manager = new ready.Manager({ accountId: () => null }, new Settings({ sidebarCustomSectionsEnabled: true, useAppServerCustomSections: true }));
  assert.deepEqual(JSON.parse(JSON.stringify(manager.getForkCapabilities().readPlacement())), { status: "ready", placement: { sections: true, appServer: true } });
  assert.equal(ready.diagnostics.length, 0);

  const off = await install(false);
  const plain = new off.Manager({ accountId: () => null }, new Settings({ sidebarCustomSectionsEnabled: false, useAppServerCustomSections: undefined }));
  assert.equal(plain.getForkCapabilities().readPlacement().status, "not-ready");
});

test("重复注入不会叠加包装", async () => {
  const { Manager, context } = await install();
  const first: any = Manager.prototype.getForkCapabilities;
  vm.runInContext(`window.__codexPlusForkReadinessState.patched = false;`, context);
  await vm.runInContext(inject, context);
  assert.equal(Manager.prototype.getForkCapabilities, first);
  assert.equal(typeof first.__codexPlusOriginal, "function");
  assert.equal(first.__codexPlusOriginal.__codexPlusForkReadiness, undefined);
});

// #2898：启动时没联网，Statsig 是 Ready 但没有任何数据（NoValues），原生不发布执行配置，
// readPermissionRefreshEnabled() 恒为 null，分支对话在 thread/fork 之前就判 not-ready。
const offlineSource = readFileSync(new URL("../../../assets/inject/renderer-inject/33-offline-auto-review.js", import.meta.url), "utf8");
const statsigInject = `(() => {\n${source}\n${offlineSource}\ninstallCodexForkStatsigFallback();\nreturn { tick: codexForkStatsigFallbackTick, autoReviewTick: codexPlusOfflineAutoReviewTick, autoReviewGate: codexPlusOfflineAutoReviewGate };\n})()`;

type StatsigClient = {
  loadingStatus: string;
  values: any;
  user: Record<string, unknown>;
  writes: string[];
  getContext(): { values: any; user: Record<string, unknown> };
  dataAdapter: { setData(data: string): void };
  updateUserSync(user: unknown): void;
};

function statsigClient(values: any, user: Record<string, unknown> = { userID: "user-1", customIDs: { stableID: "s" } }, loadingStatus = "Ready"): StatsigClient {
  const client: StatsigClient = {
    loadingStatus,
    values,
    user,
    writes: [],
    getContext() { return { values: client.values, user: client.user }; },
    dataAdapter: { setData(data: string) { client.writes.push(data); } },
    // 与真实 SDK 一致：getContext().values 是解析后的原始 JSON，未知字段原样保留。
    updateUserSync() { client.values = JSON.parse(client.writes.at(-1) ?? "null"); },
  };
  return client;
}

// 原生 readPermissionRefreshEnabled 的判定：Statsig 有数据才发布执行配置，没有就是 null。
function nativeReadPermissionRefreshEnabled(client: StatsigClient) {
  const values = client.getContext().values;
  if (client.loadingStatus !== "Ready" || values == null) return null;
  return values.feature_gates?.permissionRefresh?.value ?? false;
}

function installStatsig(client: StatsigClient, options: { loaded?: boolean; enhancementsEnabled?: boolean; offlineAutoReview?: boolean } = {}) {
  const diagnostics: unknown[][] = [];
  const intervals: Array<() => void> = [];
  const settings = { offlineAutoReview: options.offlineAutoReview === true };
  const window: any = {
    __STATSIG__: { firstInstance: client, instance: () => client, instances: { a: client } },
    setTimeout: () => 0,
    setInterval: (fn: () => void) => { intervals.push(fn); return intervals.length; },
    clearInterval: () => {},
  };
  const context = vm.createContext({
    window,
    codexPlusBackendSettingsLoaded: options.loaded ?? true,
    codexPlusBackendSettings: { enhancementsEnabled: options.enhancementsEnabled ?? true },
    codexPlusSettings: () => settings,
    loadOptionalCodexAppModule: async () => null,
    sendCodexPlusDiagnostic: (...args: unknown[]) => diagnostics.push(args),
  });
  const api = vm.runInContext(statsigInject, context);
  return { ...api, diagnostics, intervals, settings, context };
}

test("没账号、启动时没联网（Statsig 无数据）时补一份空数据，分支对话的权限门控读到 false 而不是 null", () => {
  const client = statsigClient(null);
  assert.equal(nativeReadPermissionRefreshEnabled(client), null);
  const app = installStatsig(client);
  assert.equal(client.writes.length, 1, "同一个客户端在多个入口里出现也只补一次");
  const values = JSON.parse(client.writes[0]);
  assert.deepEqual(values.feature_gates, {}, "不补任何闸门，所有开关仍是「没下发 = 关」");
  assert.deepEqual(values.user, client.user, "带上当前用户，原生才认这份数据属于当前用户");
  assert.equal(values.has_updates, true);
  assert.equal(nativeReadPermissionRefreshEnabled(client), false);
  assert.equal(app.diagnostics.filter(([name]: any) => name === "fork_statsig_offline_fallback").length, 1);
  assert.equal(app.intervals.length, 1, "Statsig 晚于注入 Ready 时靠低频复查补上");
  app.tick();
  assert.equal(client.writes.length, 1, "补过之后复查是空操作");
});

test("拿到真实数据、登录了账号、客户端还没 Ready、增强关闭或后端设置未读到时都不碰", () => {
  const cases: Array<[string, StatsigClient, Parameters<typeof installStatsig>[1]]> = [
    ["真实数据", statsigClient({ feature_gates: {} }), {}],
    ["登录账号", statsigClient(null, { userID: "u", customIDs: { account_id: "acct-1" } }), {}],
    ["还没 Ready", statsigClient(null, undefined, "Loading"), {}],
    ["增强关闭", statsigClient(null), { enhancementsEnabled: false }],
    ["设置未读到", statsigClient(null), { loaded: false }],
  ];
  for (const [label, client, options] of cases) {
    installStatsig(client, options);
    assert.equal(client.writes.length, 0, label);
  }
});

test("先补了空数据，之后再开「替我审批」时合并闸门，而不是被当成真实数据跳过", () => {
  const client = statsigClient(null);
  const app = installStatsig(client);
  assert.equal(client.writes.length, 1);
  app.settings.offlineAutoReview = true;
  app.autoReviewTick();
  assert.equal(client.writes.length, 2);
  assert.equal(client.values.feature_gates[app.autoReviewGate].value, true);
  // 反过来：「替我审批」先补过，分支兜底不再重复写。
  app.tick();
  assert.equal(client.writes.length, 2);
});
