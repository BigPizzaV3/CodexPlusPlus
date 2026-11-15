import assert from "node:assert/strict";
import { it } from "node:test";
import { readFile } from "node:fs/promises";

/**
 * 「Statsig 界面增强」。
 *
 * 这层只覆盖纯客户端的远程开关：升级推销、网络/流式超时、记忆开关、消息队列、时间线、侧聊 attach。
 * 这里钉死五条约束：
 *   1. 开关关着（或后端设置没读到）时原样透传，不做任何覆盖；
 *   2. 只覆盖配置里列出的闸门 / 实验 / 层 / 动态配置，其余保持远端值；
 *   3. 对象型配置只合并列出的键，其余键原样保留；
 *   4. getFeatureGate 是 memoized 的共享对象，覆盖时不能就地改；
 *   5. 设置项、内置拓展与页面清单三处登记一致；
 *   6. 逐项覆盖（codexAppStatsigGateOverrides）叠加在默认闸门上，从后端设置实时读；
 *   7. 记录模式在总开关关着时也只记不改，原值读取绕过覆盖；
 *   8. 接管后与覆盖变化时补派发 values_updated（让 Codex 把执行配置重新发布给主进程），没变不派发。
 */

const SHARD_URL = new URL("../../../assets/inject/renderer-inject/34-statsig-overrides.js", import.meta.url);

type ValueObject = {
  name: string;
  value: Record<string, unknown>;
  __value?: Record<string, unknown>;
  __evaluation?: Record<string, unknown>;
  get(key: string, fallback?: unknown): unknown;
};

type GateObject = {
  name: string;
  value: boolean;
  details: { reason: string };
  ruleID: string;
  __evaluation: { value: boolean };
};

function valueObject(name: string, value: Record<string, unknown>, extra: Partial<ValueObject> = {}): ValueObject {
  return {
    name,
    value: { ...value },
    get(key, fallback) {
      return key in this.value ? this.value[key] : fallback;
    },
    ...extra,
  };
}

function fakeClient(
  gates: Record<string, boolean> = {},
  configs: Record<string, Record<string, unknown>> = {},
  experiments: Record<string, Record<string, unknown>> = {},
  layers: Record<string, Record<string, unknown>> = {},
) {
  const gateObjects = new Map<string, GateObject>();
  for (const [name, value] of Object.entries(gates)) {
    gateObjects.set(name, { name, value, details: { reason: "Network" }, ruleID: "rule", __evaluation: { value } });
  }
  const configObjects = new Map<string, ValueObject>();
  for (const [name, value] of Object.entries(configs)) configObjects.set(name, valueObject(name, value));
  const experimentObjects = new Map<string, ValueObject>();
  for (const [name, value] of Object.entries(experiments)) {
    experimentObjects.set(name, valueObject(name, value, {
      __evaluation: { value, is_user_in_experiment: false, is_experiment_active: false },
    }));
  }
  const layerObjects = new Map<string, ValueObject>();
  for (const [name, value] of Object.entries(layers)) {
    layerObjects.set(name, {
      name,
      value: undefined as unknown as Record<string, unknown>,
      __value: { ...value },
      get(key, fallback) {
        const source = this.__value ?? {};
        return key in source ? source[key] : fallback;
      },
    });
  }

  const gateCalls: string[] = [];
  const emitted: Array<{ name: string; status: string; values: unknown }> = [];
  const client = {
    gateCalls,
    emitted,
    loadingStatus: "Ready",
    _memoCache: { stale: true } as Record<string, unknown>,
    $emt(event: { name: string; status: string; values: unknown }) {
      emitted.push(event);
    },
    getContext() {
      return { values: { source: "network" } };
    },
    getFeatureGate(name: string) {
      gateCalls.push(name);
      return gateObjects.get(name) ?? { name, value: false, details: { reason: "Unrecognized" }, ruleID: "", __evaluation: { value: false } };
    },
    checkGate(name: string) {
      return this.getFeatureGate(name).value;
    },
    getDynamicConfig(name: string) {
      const existing = configObjects.get(name);
      if (existing) return existing;
      const created = valueObject(name, {});
      configObjects.set(name, created);
      return created;
    },
    getExperiment(name: string) {
      const existing = experimentObjects.get(name);
      if (existing) return existing;
      const created = valueObject(name, {}, { __evaluation: { value: {}, is_user_in_experiment: false, is_experiment_active: false } });
      experimentObjects.set(name, created);
      return created;
    },
    getLayer(name: string) {
      const existing = layerObjects.get(name);
      if (existing) return existing;
      const created: ValueObject = {
        name,
        value: undefined as unknown as Record<string, unknown>,
        __value: {},
        get(key, fallback) {
          const source = this.__value ?? {};
          return key in source ? source[key] : fallback;
        },
      };
      layerObjects.set(name, created);
      return created;
    },
    gate(name: string) {
      return gateObjects.get(name);
    },
    config(name: string) {
      return configObjects.get(name);
    },
    experiment(name: string) {
      return experimentObjects.get(name);
    },
    layer(name: string) {
      return layerObjects.get(name);
    },
  };
  return client;
}

function overridesConfig(enabled: boolean) {
  return {
    enabled,
    gates: { "1174206456": true, "2120612410": true },
    experiments: { "3359351081": { enabled: true } },
    layers: { "1574672957": { desktop_context_section: "x" } },
    dynamicConfigs: {
      "3165814200": { MIN_RETRY_INTERVAL: 400, MAX_RETRY_COUNT: 15 },
      "3608673692": { sse_initial_open_timeout_ms: 90000 },
      "2808262316": { enabled: true },
    },
  };
}

async function runtime(
  client: ReturnType<typeof fakeClient>,
  settings: { loaded: boolean; enabled: boolean; configEnabled?: boolean; gateOverrides?: Record<string, unknown>; recording?: boolean },
) {
  const source = await readFile(SHARD_URL, "utf8");
  const window: Record<string, unknown> = {
    __STATSIG__: { firstInstance: client, instance: () => client, instances: { a: client } },
    __CODEX_PLUS_STATSIG_OVERRIDES__: overridesConfig(settings.configEnabled ?? settings.enabled),
    __codexPlusStatsigRecording: settings.recording === true,
  };
  const backend: { codexAppStatsigGateOverrides?: Record<string, unknown> } = { codexAppStatsigGateOverrides: settings.gateOverrides };
  const api = new Function(
    "window",
    "codexPlusBackendSettingsLoaded",
    "codexPlusSettings",
    "codexPlusBackendSettings",
    `${source}\nreturn {
      tick: codexPlusStatsigOverridesTick,
      extension: codexPlusStatsigOverridesExtension,
      effective: codexPlusStatsigEffectiveGates,
      gates: codexPlusStatsigGateOverrides,
      user: codexPlusStatsigUserGateOverrides,
      native: codexPlusStatsigNativeGateValue,
    };`,
  )(window, settings.loaded, () => ({ statsigOverrides: settings.enabled }), backend) as {
    tick(): void;
    extension(ctx: { setInterval(fn: () => void, ms: number): void }): void;
    effective(defaults: Record<string, boolean>, user: Record<string, string>): Record<string, boolean>;
    gates(): Record<string, boolean>;
    user(): Record<string, string>;
    native(name: string): boolean | null;
  };
  return { ...api, window, settings, backend };
}

it("开关打开时强制列出的闸门，checkGate 与 getFeatureGate 都能读到", async () => {
  const client = fakeClient({ "1174206456": false, "2120612410": false, "999": false });
  const app = await runtime(client, { loaded: true, enabled: true });
  app.tick();
  assert.equal(client.checkGate("1174206456"), true, "checkGate 覆盖为 true");
  assert.equal(client.getFeatureGate("1174206456").value, true, "getFeatureGate 覆盖为 true");
  assert.equal(client.getFeatureGate("2120612410").value, true, "消息队列闸门");
  assert.equal(client.checkGate("999"), false, "未列出的闸门原样透传");
  assert.equal(client.getFeatureGate("999").value, false);
});

it("覆盖 getFeatureGate 时不就地改 memoized 的共享对象", async () => {
  const client = fakeClient({ "1174206456": false });
  const app = await runtime(client, { loaded: true, enabled: true });
  app.tick();
  client.getFeatureGate("1174206456");
  assert.equal(client.gate("1174206456")!.value, false, "SDK 缓存的对象保持远端值");
});

it("开关关着或后端设置没读到时完全透传", async () => {
  for (const settings of [
    { loaded: true, enabled: false },
    { loaded: false, enabled: true },
    { loaded: true, enabled: true, configEnabled: false },
  ]) {
    const client = fakeClient({ "1174206456": false }, {}, { "3359351081": { enabled: false } }, { "1574672957": { desktop_context_section: "远端" } });
    const app = await runtime(client, settings);
    app.tick();
    assert.equal(client.checkGate("1174206456"), false, JSON.stringify(settings));
    assert.equal(client.getFeatureGate("1174206456").value, false, JSON.stringify(settings));
    assert.equal(client.getExperiment("3359351081").get("enabled", false), false, JSON.stringify(settings));
    assert.equal(client.getLayer("1574672957").get("desktop_context_section"), "远端", JSON.stringify(settings));
  }
});

it("实验只合并列出的键，并让 __evaluation 认为实验生效", async () => {
  const client = fakeClient({}, {}, { "3359351081": { enabled: false, other: 7 } });
  const app = await runtime(client, { loaded: true, enabled: true });
  app.tick();
  const experiment = client.experiment("3359351081")!;
  assert.equal(experiment.get("enabled", false), true, "覆盖值生效");
  assert.equal(experiment.get("other"), 7, "未列出的键保留");
  assert.equal(experiment.value.enabled, true);
  assert.equal(experiment.__evaluation!.is_user_in_experiment, true);
});

it("Layer 合并到 __value 与 get()", async () => {
  const client = fakeClient({}, {}, {}, { "1574672957": { desktop_context_section: "远端", keep: 1 } });
  const app = await runtime(client, { loaded: true, enabled: true });
  app.tick();
  const layer = client.layer("1574672957")!;
  assert.equal(layer.get("desktop_context_section"), "x");
  assert.equal(layer.get("keep"), 1);
  assert.equal(layer.__value!.desktop_context_section, "x");
});

it("动态配置只合并列出的键，其余键原样保留", async () => {
  const client = fakeClient({}, { "3165814200": { MIN_RETRY_INTERVAL: 300, MAX_RETRY_INTERVAL: 5000, RETRY_FACTOR: 1.5, MAX_RETRY_COUNT: 12 } });
  const app = await runtime(client, { loaded: true, enabled: true });
  app.tick();
  const config = client.config("3165814200")!;
  assert.equal(config.get("MIN_RETRY_INTERVAL"), 400, "覆盖值生效");
  assert.equal(config.get("RETRY_FACTOR"), 1.5, "未列出的键保留");
  assert.equal(config.get("MAX_RETRY_COUNT"), 15);
});

it("重复 tick 不会重复包装", async () => {
  const client = fakeClient({ "1174206456": false });
  const app = await runtime(client, { loaded: true, enabled: true });
  app.tick();
  const patchedGate = client.checkGate;
  const patchedFeatureGate = client.getFeatureGate;
  const patchedExperiment = client.getExperiment;
  const patchedLayer = client.getLayer;
  app.tick();
  assert.equal(client.checkGate, patchedGate, "checkGate 只包一层");
  assert.equal(client.getFeatureGate, patchedFeatureGate, "getFeatureGate 只包一层");
  assert.equal(client.getExperiment, patchedExperiment, "getExperiment 只包一层");
  assert.equal(client.getLayer, patchedLayer, "getLayer 只包一层");
  assert.equal(client.checkGate("1174206456"), true);
});

it("扩展按低频复查等待 Statsig / 后端设置就绪", async () => {
  const client = fakeClient({ "1174206456": false });
  const app = await runtime(client, { loaded: true, enabled: true });
  const intervals: number[] = [];
  app.extension({ setInterval: (_fn, ms) => intervals.push(ms) });
  assert.deepEqual(intervals, [3000]);
});

it("设置项、内置拓展和页面清单三处登记一致", async () => {
  const read = (path: string) => readFile(new URL(path, import.meta.url), "utf8");
  const registry = JSON.parse(
    await read("../../../crates/codex-plus-core/assets/enhancement-settings.json"),
  ) as { items: Array<{ key: string; builtin?: string }> };
  const item = registry.items.find((entry) => entry.key === "codexAppStatsigOverrides");
  assert.ok(item, "清单里应有 codexAppStatsigOverrides");
  assert.equal(item.builtin, "statsig-overrides");

  const page = await read("../../../assets/inject/renderer-inject/96-builtin-extensions.js");
  assert.match(page, /id: "statsig-overrides"/);

  const style = await read("../../../assets/inject/renderer-inject/10-style.js");
  assert.match(style, /statsigOverrides: "codexAppStatsigOverrides"/);
  assert.match(style, /statsigOverrides: "statsig-overrides"/);
});

it("逐项覆盖叠加在默认闸门上：on 强开、off 强关、native 取消默认覆盖", async () => {
  const client = fakeClient({ "1174206456": false, "2120612410": false, "983842163": false, "623263923": true });
  const app = await runtime(client, {
    loaded: true,
    enabled: true,
    gateOverrides: { "983842163": "on", "623263923": "off", "2120612410": "native", "bad.key": "on", "111": "maybe" },
  });
  assert.deepEqual(app.user(), { "983842163": "on", "623263923": "off", "2120612410": "native" }, "非法条目被丢掉");
  app.tick();
  assert.equal(client.checkGate("983842163"), true, "on：不在默认表里也能强开");
  assert.equal(client.getFeatureGate("623263923").value, false, "off：强关");
  assert.equal(client.checkGate("2120612410"), false, "native：取消默认强开，回到远端值");
  assert.equal(client.checkGate("1174206456"), true, "没设过的默认项照旧强开");
});

it("合并函数不改入参，未知选择不起作用", async () => {
  const app = await runtime(fakeClient(), { loaded: true, enabled: true });
  const defaults = { "1": true, "2": true };
  const merged = app.effective(defaults, { "2": "native", "3": "off", "4": "weird" });
  assert.deepEqual(merged, { "1": true, "3": false });
  assert.deepEqual(defaults, { "1": true, "2": true });
});

it("总开关关着时逐项覆盖也不生效", async () => {
  const client = fakeClient({ "983842163": false });
  const app = await runtime(client, { loaded: true, enabled: false, gateOverrides: { "983842163": "on" } });
  app.tick();
  assert.equal(client.checkGate("983842163"), false);
});

it("记录模式：总开关关着也接管，只记录原值、不改返回值", async () => {
  const client = fakeClient({ "555": true, "777": false });
  const app = await runtime(client, { loaded: true, enabled: false, recording: true });
  app.tick();
  assert.equal(client.checkGate("555"), true);
  assert.equal(client.getFeatureGate("777").value, false);
  client.checkGate("555");
  const log = app.window.__codexPlusStatsigGateLog as Map<string, { native: boolean | null; count: number }>;
  assert.equal(log.get("555")?.native, true);
  assert.equal(log.get("555")?.count, 2, "checkGate 内部走 getFeatureGate，只记一次");
  assert.equal(log.get("777")?.native, false);
});

it("不在记录模式时不记，也不为了记录去接管客户端", async () => {
  const client = fakeClient({ "555": true });
  const app = await runtime(client, { loaded: true, enabled: false, recording: false });
  app.tick();
  client.checkGate("555");
  const log = app.window.__codexPlusStatsigGateLog as Map<string, unknown>;
  assert.equal(log.size, 0);
  assert.equal((client as Record<string, unknown>).__codexPlusStatsigFeatureGatePatched, undefined);
});

it("读原值绕过覆盖", async () => {
  const client = fakeClient({ "1174206456": false });
  const app = await runtime(client, { loaded: true, enabled: true });
  app.tick();
  assert.equal(client.checkGate("1174206456"), true, "界面读到的是覆盖值");
  assert.equal(app.native("1174206456"), false, "原值仍是远端的 false");
});

it("接管后补派发一次 values_updated，覆盖没变时不再派发", async () => {
  const client = fakeClient({ "1174206456": false });
  const app = await runtime(client, { loaded: true, enabled: true });
  app.tick();
  assert.equal(client.emitted.length, 1, "接管后派发一次");
  assert.deepEqual(client.emitted[0], { name: "values_updated", status: "Ready", values: { source: "network" } });
  assert.deepEqual(client._memoCache, {}, "派发前清 memo");
  app.tick();
  app.tick();
  assert.equal(client.emitted.length, 1, "覆盖没变不重复派发");
});

it("逐项覆盖改了、总开关关了都会再派发；只开记录不派发", async () => {
  const client = fakeClient({ "983842163": false });
  const app = await runtime(client, { loaded: true, enabled: true });
  app.tick();
  assert.equal(client.emitted.length, 1);
  app.backend.codexAppStatsigGateOverrides = { "983842163": "on" };
  app.tick();
  assert.equal(client.emitted.length, 2, "Feature Lab 改了逐项覆盖");
  assert.equal(client.checkGate("983842163"), true);
  app.settings.enabled = false;
  app.tick();
  assert.equal(client.emitted.length, 3, "关掉总开关也通知，回到远端值");
  assert.equal(client.checkGate("983842163"), false);
  app.tick();
  assert.equal(client.emitted.length, 3);

  const recorder = fakeClient({ "555": true });
  const recorded = await runtime(recorder, { loaded: true, enabled: false, recording: true });
  recorded.tick();
  assert.equal(recorder.emitted.length, 0, "只记录、值没变，不打扰 Codex");
});

it("客户端还没就绪时先不派发，就绪后补上", async () => {
  const client = fakeClient({ "1174206456": false });
  client.loadingStatus = "Loading";
  const app = await runtime(client, { loaded: true, enabled: true });
  app.tick();
  assert.equal(client.emitted.length, 0);
  client.loadingStatus = "Ready";
  app.tick();
  assert.equal(client.emitted.length, 1);
});
