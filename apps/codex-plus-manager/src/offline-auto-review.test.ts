import assert from "node:assert/strict";
import { it } from "node:test";
import { readFile } from "node:fs/promises";

/**
 * 「拿不到远程开关时补开『替我审批』」（#2817）。
 *
 * 真实 Codex 里的端到端验证（屏蔽 OpenAI 域名后权限菜单重新出现 Approve for me）靠 CDP 实测；
 * 这里钉死三条不能退化的约束：
 *   1. 只在 Statsig 完全没有数据时补，拿到真实数据就不碰；
 *   2. 只补「替我审批」这一个闸门，其余远程开关保持「没下发 = 关」；
 *   3. 开关关着（或后端设置还没读到）时什么都不做。
 */

const SHARD_URL = new URL("../../../assets/inject/renderer-inject/33-offline-auto-review.js", import.meta.url);

type FakeClient = {
  loadingStatus: string;
  values: unknown;
  setDataCalls: string[];
  updateCalls: number;
  getContext(): { values: unknown; user: Record<string, unknown> };
  dataAdapter: { setData(data: string): void };
  updateUserSync(user: unknown): void;
};

function fakeClient(values: unknown, loadingStatus = "Ready"): FakeClient {
  const client: FakeClient = {
    loadingStatus,
    values,
    setDataCalls: [],
    updateCalls: 0,
    getContext() {
      return { values: client.values, user: { userID: "user-1" } };
    },
    dataAdapter: {
      setData(data: string) {
        client.setDataCalls.push(data);
      },
    },
    updateUserSync() {
      client.updateCalls += 1;
      client.values = JSON.parse(client.setDataCalls.at(-1) ?? "null");
    },
  };
  return client;
}

async function runtime(client: FakeClient, settings: { loaded: boolean; enabled: boolean }) {
  const source = await readFile(SHARD_URL, "utf8");
  const window = { __STATSIG__: { firstInstance: client, instance: () => client, instances: { a: client } } };
  const intervals: number[] = [];
  const api = new Function(
    "window", "codexPlusBackendSettingsLoaded", "codexPlusSettings",
    `${source}\nreturn { tick: codexPlusOfflineAutoReviewTick, extension: codexPlusOfflineAutoReviewExtension, gate: codexPlusOfflineAutoReviewGate };`,
  )(window, settings.loaded, () => ({ offlineAutoReview: settings.enabled })) as {
    tick(): void;
    extension(ctx: { setInterval(fn: () => void, ms: number): void }): void;
    gate: string;
  };
  return { ...api, intervals, start: () => api.extension({ setInterval: (_fn, ms) => intervals.push(ms) }) };
}

it("Statsig 没有数据时只补「替我审批」闸门，且只补一次", async () => {
  const client = fakeClient(null);
  const app = await runtime(client, { loaded: true, enabled: true });
  app.start();
  app.tick();
  assert.equal(client.setDataCalls.length, 1, "同一个客户端只补一次（实例在多个入口里重复出现也一样）");
  assert.equal(client.updateCalls, 1);
  const values = JSON.parse(client.setDataCalls[0]);
  assert.equal(app.gate, "3902016271");
  assert.deepEqual(Object.keys(values.feature_gates), ["3902016271"], "只补这一个闸门");
  assert.equal(values.feature_gates["3902016271"].value, true);
  assert.deepEqual(values.dynamic_configs, {});
  assert.deepEqual(values.layer_configs, {});
  assert.equal(values.hash_used, "none");
  assert.deepEqual(app.intervals, [3000], "Statsig 晚于注入 Ready 时靠低频复查补上");
});

it("拿到真实数据（有网络）时完全不碰", async () => {
  const client = fakeClient({ feature_gates: {} });
  const app = await runtime(client, { loaded: true, enabled: true });
  app.start();
  assert.equal(client.setDataCalls.length, 0);
  assert.equal(client.updateCalls, 0);
});

it("客户端还没 Ready 时先不补", async () => {
  const client = fakeClient(null, "Loading");
  const app = await runtime(client, { loaded: true, enabled: true });
  app.start();
  assert.equal(client.setDataCalls.length, 0);
});

it("开关关着、或后端设置还没读到时什么都不做", async () => {
  for (const settings of [{ loaded: true, enabled: false }, { loaded: false, enabled: true }]) {
    const client = fakeClient(null);
    const app = await runtime(client, settings);
    app.start();
    assert.equal(client.setDataCalls.length, 0, JSON.stringify(settings));
  }
});

it("设置项、内置拓展和分片清单三处登记一致", async () => {
  const read = (path: string) => readFile(new URL(path, import.meta.url), "utf8");
  const registry = JSON.parse(await read("../../../crates/codex-plus-core/assets/enhancement-settings.json")) as {
    items: Array<{ key: string; builtin?: string }>;
  };
  const item = registry.items.find((entry) => entry.key === "codexAppOfflineAutoReview");
  assert.equal(item?.builtin, "offline-auto-review");
  const catalog = await read("../../../assets/inject/renderer-inject/96-builtin-extensions.js");
  assert.match(catalog, /id: "offline-auto-review", name: "[^"]+", apply: codexPlusOfflineAutoReviewExtension/);
  const style = await read("../../../assets/inject/renderer-inject/10-style.js");
  assert.match(style, /offlineAutoReview: "codexAppOfflineAutoReview"/);
  assert.match(style, /offlineAutoReview: "offline-auto-review"/);
  const manifest = JSON.parse(await read("../../../assets/inject/renderer-inject/manifest.json")) as { fragments: Array<{ name: string }> };
  const names = manifest.fragments.map((fragment) => fragment.name);
  assert.ok(names.indexOf("33-offline-auto-review.js") < names.indexOf("96-builtin-extensions.js"));
});
