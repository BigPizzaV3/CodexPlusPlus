import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";
import vm from "node:vm";

// issue #2910：Codex 的模型菜单只请求一页 `model/list {cursor: null, limit: 100}`，不翻页。
// 网关目录超过 100 条时，第 100 条以后的供应商、自动组、模型组都靠注入层把目录里缺的模型补进这一页；
// 这一页带着非空 nextCursor 也必须补，否则菜单在第 100 条处截断。
const bundle = readFileSync(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

function node() {
  return {
    appendChild() {}, prepend() {}, remove() {}, setAttribute() {}, removeAttribute() {}, toggleAttribute() {},
    addEventListener() {}, querySelector() { return null; }, querySelectorAll() { return []; }, closest() { return null; },
    classList: { add() {}, remove() {}, toggle() {}, contains() { return false; } },
    dataset: {}, style: {}, children: [], isConnected: true, textContent: "", innerHTML: "",
  };
}

function runtime() {
  const store = new Map<string, string>();
  const context: Record<string, unknown> = {
    __CODEX_PLUS_TEST_SERVICE_TIER__: true,
    // 产物开头按 `process` 判断是否在 node 测试里，没有它会在非 Codex 页面直接退出。
    process,
    console, setTimeout, clearTimeout, setInterval, clearInterval, queueMicrotask, Promise, URL, JSON,
    addEventListener() {}, removeEventListener() {},
    document: {
      scripts: [], documentElement: node(), body: node(), createElement: () => node(), getElementById: () => null,
      querySelector: () => null, querySelectorAll: () => [], addEventListener() {}, removeEventListener() {},
    },
    localStorage: {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: unknown) => store.set(key, String(value)),
      removeItem: (key: string) => store.delete(key),
    },
    location: { href: "https://codex.test/", pathname: "/", search: "", hash: "" },
    navigator: { userAgent: "node-test" },
    performance: { getEntriesByType: () => [] },
  };
  context.window = context;
  context.globalThis = context;
  vm.runInNewContext(bundle, context);
  return context as any;
}

describe("model/list first page beyond 100 entries (#2910)", () => {
  it("tops up the single native page with every catalog model after the first 100", async () => {
    const window = runtime();
    const api = window.__codexPlusServiceTierTest;
    api.setBackendSettings({
      relayProfilesEnabled: true, codexAppModelWhitelistUnlock: true, activeRelayId: "codex-plus-gateway",
      relayProfiles: [{ id: "codex-plus-gateway", relayMode: "pureApi" }],
    });
    // 网关目录：13 家供应商各 10 个，后面是自动组和模型组，共 132 条。
    const names = [
      ...Array.from({ length: 130 }, (_, index) => `relay${Math.floor(index / 10)}/model-${index}`),
      "deepseek-v4-flash", "group/fallback",
    ];
    api.setModelCatalog({
      status: "ok", model_provider: "codex-plus-gateway", model: names[0], default_model: names[0], models: names,
      sources: [{ id: "relay-profile:codex-plus-gateway" }],
    });
    const requests: unknown[] = [];
    class Client {
      hostId = "local";
      async sendRequest(method: string, params: { includeHidden?: boolean; cursor?: string | null; limit?: number }) {
        requests.push({ method, params });
        const limit = params?.limit ?? 100;
        return {
          data: names.slice(0, limit).map((model) => ({ model, id: model, displayName: model, hidden: false })),
          nextCursor: String(limit),
        };
      }
    }
    window.__codexPlusAppServerClientClass = Client;
    assert.equal(api.installAppServerClientPrototypePatch(), true);

    const page = await new Client().sendRequest("model/list", { includeHidden: true, cursor: null, limit: 100 });

    assert.deepEqual(requests, [{ method: "model/list", params: { includeHidden: true, cursor: null, limit: 100 } }]);
    assert.deepEqual(page.data.map((entry: { model: string }) => entry.model), names);
  });
});
