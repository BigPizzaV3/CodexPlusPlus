import assert from "node:assert/strict";
import { it } from "node:test";
import { readFile } from "node:fs/promises";

/**
 * 「Statsig 界面增强」。
 *
 * 这层只覆盖纯客户端的远程开关：升级推销、网络/流式超时、记忆开关。
 * 这里钉死四条约束：
 *   1. 开关关着（或后端设置没读到）时原样透传，不做任何覆盖；
 *   2. 只覆盖配置里列出的闸门与动态配置，其余保持远端值；
 *   3. 动态配置只合并列出的键，其余键原样保留；
 *   4. 设置项、内置拓展与页面清单三处登记一致。
 */

const SHARD_URL = new URL("../../../assets/inject/renderer-inject/34-statsig-overrides.js", import.meta.url);

type DynamicConfig = { value: Record<string, unknown>; get(key: string, fallback?: unknown): unknown };

function fakeClient(gates: Record<string, boolean> = {}, configs: Record<string, Record<string, unknown>> = {}) {
  const configObjects = new Map<string, DynamicConfig>();
  for (const [name, value] of Object.entries(configs)) {
    const config: DynamicConfig = {
      value: { ...value },
      get(key, fallback) {
        return key in this.value ? this.value[key] : fallback;
      },
    };
    configObjects.set(name, config);
  }
  const gateCalls: string[] = [];
  const client = {
    gateCalls,
    checkGate(name: string) {
      gateCalls.push(name);
      return gates[name] === true;
    },
    getDynamicConfig(name: string) {
      const existing = configObjects.get(name);
      if (existing) return existing;
      const created: DynamicConfig = { value: {}, get(key, fallback) { return key in this.value ? this.value[key] : fallback; } };
      configObjects.set(name, created);
      return created;
    },
    config(name: string) {
      return configObjects.get(name);
    },
  };
  return client;
}

function overridesConfig(enabled: boolean) {
  return {
    enabled,
    gates: { "1174206456": true },
    dynamicConfigs: {
      "3165814200": { MIN_RETRY_INTERVAL: 400, MAX_RETRY_COUNT: 15 },
      "3608673692": { sse_initial_open_timeout_ms: 90000 },
      "2808262316": { enabled: true },
    },
  };
}

async function runtime(
  client: ReturnType<typeof fakeClient>,
  settings: { loaded: boolean; enabled: boolean; configEnabled?: boolean },
) {
  const source = await readFile(SHARD_URL, "utf8");
  const window: Record<string, unknown> = {
    __STATSIG__: { firstInstance: client, instance: () => client, instances: { a: client } },
    __CODEX_PLUS_STATSIG_OVERRIDES__: overridesConfig(settings.configEnabled ?? settings.enabled),
  };
  const api = new Function(
    "window",
    "codexPlusBackendSettingsLoaded",
    "codexPlusSettings",
    `${source}\nreturn { tick: codexPlusStatsigOverridesTick, extension: codexPlusStatsigOverridesExtension };`,
  )(window, settings.loaded, () => ({ statsigOverrides: settings.enabled })) as {
    tick(): void;
    extension(ctx: { setInterval(fn: () => void, ms: number): void }): void;
  };
  return api;
}

it("开关打开时强制升级推销闸门，其余闸门保持远端值", async () => {
  const client = fakeClient({ "1174206456": false, "999": false });
  const app = await runtime(client, { loaded: true, enabled: true });
  app.tick();
  assert.equal(client.checkGate("1174206456"), true, "覆盖为 true");
  assert.equal(client.checkGate("999"), false, "未列出的闸门原样透传");
});

it("开关关着或后端设置没读到时完全透传", async () => {
  for (const settings of [
    { loaded: true, enabled: false },
    { loaded: false, enabled: true },
    { loaded: true, enabled: true, configEnabled: false },
  ]) {
    const client = fakeClient({ "1174206456": false });
    const app = await runtime(client, settings);
    app.tick();
    assert.equal(client.checkGate("1174206456"), false, JSON.stringify(settings));
  }
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
  const patched = client.checkGate;
  app.tick();
  assert.equal(client.checkGate, patched, "同一客户端只包一层");
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
