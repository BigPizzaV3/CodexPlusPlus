import assert from "node:assert/strict";
import { it } from "node:test";
import { readFile } from "node:fs/promises";

/**
 * 内置拓展「Feature Lab」。
 *
 * 钉死：
 *   1. experimentalFeature/list 的分页、去重、排序；
 *   2. 开关写入走 config/batchWrite 的 features.<name>，不碰别的键；
 *   3. 非法 feature 名（会写坏 config 键路径）直接拒绝；
 *   4. 内置拓展清单、设置清单与分片清单三处登记一致；
 *   5. 服务端几乎不给说明（实测 215 项只有 5 项），用内置中文说明表补，服务端有说明时以服务端为准；
 *   6. config/batchWrite 对任何 key 都回 ok，写完必须回读 enabled 才能判断是否真的生效；
 *   7. Codex 启动时经 enablement/set 推送的 15 项带提示（config.toml 优先，memories 总被推成关闭）。
 */

const SHARD_URL = new URL("../../../assets/inject/renderer-inject/96-feature-lab.js", import.meta.url);

type Feature = {
  name: string;
  title: string;
  stage: string;
  description: string;
  enabled: boolean;
  defaultEnabled: boolean;
  risky: boolean;
  locked: boolean;
  known: boolean;
  pushNote: string;
};

async function runtime() {
  const source = await readFile(SHARD_URL, "utf8");
  return new Function(
    `${source}\nreturn {
      normalize: codexPlusFeatureLabNormalizeFeature,
      fetch: codexPlusFeatureLabFetch,
      write: codexPlusFeatureLabWrite,
      verify: codexPlusFeatureLabVerify,
      catalog: codexPlusFeatureLabCatalog,
      pushed: codexPlusFeatureLabPushedByApp,
      extension: codexPlusFeatureLabExtension,
      stage: codexPlusFeatureLabStage,
      labChoice: codexPlusStatsigLabChoice,
      labNext: codexPlusStatsigLabNextOverrides,
      labDefault: codexPlusStatsigLabDefaultLabel,
      labUnknown: codexPlusStatsigLabUnknownRows,
    };`,
  )() as {
    normalize(raw: unknown): Feature | null;
    fetch(client: unknown): Promise<Feature[]>;
    write(client: unknown, feature: Feature, enabled: boolean): Promise<unknown>;
    verify(client: unknown, name: string): Promise<boolean | null>;
    catalog: Record<string, { title: string; description: string }>;
    pushed: Set<string>;
    extension(ctx: unknown): void;
    stage(raw: unknown): string;
    labChoice(overrides: unknown, id: string): string;
    labNext(overrides: unknown, id: string, choice: string): Record<string, string>;
    labDefault(entry: unknown): string;
    labUnknown(
      catalog: Array<{ id: string }>,
      overrides: Record<string, string>,
      log: Array<{ id: string; native?: boolean | null; count?: number }>,
    ): Array<{ id: string; native: boolean | null; count: number }>;
  };
}

it("归一化 feature：缺字段给空值，未知 stage 按开发中处理", async () => {
  const api = await runtime();
  const feature = api.normalize({
    name: "some_feature",
    stage: "beta",
    displayName: "Some Feature",
    description: "说明",
    enabled: true,
    defaultEnabled: false,
  });
  assert.ok(feature);
  assert.equal(feature.name, "some_feature");
  assert.equal(feature.title, "Some Feature");
  assert.equal(feature.stage, "beta");
  assert.equal(feature.enabled, true);
  assert.equal(feature.defaultEnabled, false);
  assert.equal(feature.risky, false);
  assert.equal(feature.locked, false);

  const unknown = api.normalize({ name: "mystery", stage: "future-stage" });
  assert.ok(unknown);
  assert.equal(unknown.stage, "underDevelopment");
  assert.equal(unknown.risky, true);
  assert.equal(unknown.title, "mystery");

  // removed 阶段不静态锁：实测有的写了会生效（search_tool），有的不会，交给写后回读判断。
  const removed = api.normalize({ name: "removed_one", stage: "removed" });
  assert.equal(removed?.locked, false);
  assert.equal(removed?.risky, true);
});

it("服务端没给说明时用内置中文说明，服务端给了以服务端为准", async () => {
  const api = await runtime();
  const fromCatalog = api.normalize({ name: "code_mode", stage: "underDevelopment", displayName: null, description: null });
  assert.ok(fromCatalog);
  assert.equal(fromCatalog.title, "Code mode");
  assert.match(fromCatalog.description, /JS/);
  assert.equal(fromCatalog.known, true);

  const fromServer = api.normalize({ name: "code_mode", stage: "underDevelopment", displayName: "Server title", description: "Server desc" });
  assert.equal(fromServer?.title, "Server title");
  assert.equal(fromServer?.description, "Server desc");
  assert.equal(fromServer?.known, true);

  const unknown = api.normalize({ name: "brand_new_flag", stage: "beta" });
  assert.equal(unknown?.title, "brand_new_flag");
  assert.equal(unknown?.description, "");
  assert.equal(unknown?.known, false);
});

it("Codex 启动时推送的 feature 带提示，memories 单独说明总被推成关闭", async () => {
  const api = await runtime();
  const pushed = api.normalize({ name: "mcp_connection_sharing", stage: "beta" });
  assert.match(pushed?.pushNote ?? "", /config\.toml/);
  const memories = api.normalize({ name: "memories", stage: "beta" });
  assert.match(memories?.pushNote ?? "", /关闭/);
  const plain = api.normalize({ name: "code_mode", stage: "underDevelopment" });
  assert.equal(plain?.pushNote, "");
  assert.equal(api.pushed.size, 15);
  for (const name of api.pushed) assert.match(name, /^[A-Za-z][A-Za-z0-9_]*$/, name);
});

it("内置说明表的 key 都是合法 feature 名，标题与说明都不为空", async () => {
  const api = await runtime();
  const entries = Object.entries(api.catalog);
  assert.ok(entries.length >= 100, "说明表应覆盖主要 feature");
  for (const [name, entry] of entries) {
    assert.match(name, /^[A-Za-z][A-Za-z0-9_]*$/, name);
    assert.ok(entry.title.trim(), `${name} 缺标题`);
    assert.ok(entry.description.trim(), `${name} 缺说明`);
  }
});

it("写入后回读：按列表里的 enabled 判断是否真的生效", async () => {
  const api = await runtime();
  const pages: Record<string, { data: Array<{ name: string; enabled: boolean }>; nextCursor: string | null }> = {
    first: { data: [{ name: "a", enabled: false }], nextCursor: "p2" },
    p2: { data: [{ name: "js_repl", enabled: false }, { name: "search_tool", enabled: true }], nextCursor: null },
  };
  const client = {
    async sendRequest(method: string, params: { cursor?: string | null }) {
      assert.equal(method, "experimentalFeature/list");
      return pages[params.cursor ?? "first"];
    },
  };
  assert.equal(await api.verify(client, "search_tool"), true);
  assert.equal(await api.verify(client, "js_repl"), false);
  assert.equal(await api.verify(client, "not_in_list"), null);
  assert.equal(await api.verify(null, "anything"), null);
});

it("拒绝会写坏 config 键路径的 feature 名", async () => {
  const api = await runtime();
  for (const name of ["", "1st", "foo-bar", "foo.bar", "foo bar", "中文"]) {
    assert.equal(api.normalize({ name }), null, name);
  }
});

it("分页拉全量、按名称去重、按 stage 和标题排序", async () => {
  const api = await runtime();
  const calls: Array<{ method: string; params: Record<string, unknown> }> = [];
  const client = {
    async sendRequest(method: string, params: Record<string, unknown>) {
      calls.push({ method, params });
      if (params.cursor == null) {
        return {
          data: [
            { name: "dev_b", stage: "underDevelopment" },
            { name: "stable_z", stage: "stable" },
          ],
          nextCursor: "page-2",
        };
      }
      return {
        data: [
          { name: "beta_a", stage: "beta" },
          { name: "stable_z", stage: "stable" },
          { name: "stable_a", stage: "stable" },
        ],
        nextCursor: null,
      };
    },
  };
  const features = await api.fetch(client);
  assert.deepEqual(calls.map((call) => call.method), ["experimentalFeature/list", "experimentalFeature/list"]);
  assert.equal(calls[0].params.cursor, null);
  assert.equal(calls[1].params.cursor, "page-2");
  assert.deepEqual(features.map((feature) => feature.name), ["stable_a", "stable_z", "beta_a", "dev_b"]);
});

it("写入只动 features.<name> 并按 upsert 合并", async () => {
  const api = await runtime();
  const calls: Array<{ method: string; params: Record<string, unknown> }> = [];
  const client = {
    async sendRequest(method: string, params: Record<string, unknown>) {
      calls.push({ method, params });
      return { ok: true };
    },
  };
  const feature = api.normalize({ name: "some_feature", stage: "beta" })!;
  await api.write(client, feature, true);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].method, "config/batchWrite");
  assert.deepEqual(calls[0].params.edits, [
    { keyPath: "features.some_feature", value: true, mergeStrategy: "upsert" },
  ]);
  assert.equal(calls[0].params.filePath, null);
  assert.equal(calls[0].params.expectedVersion, null);
  assert.equal(calls[0].params.reloadUserConfig, true);

  await assert.rejects(() => api.write(client, { name: "bad.name" } as Feature, true), /名称不合法/);
});

it("注册为 Codex 设置页分区", async () => {
  const api = await runtime();
  const captured: { definition?: { id?: string; title?: string; render?: unknown } } = {};
  api.extension({
    registerSettingsSection(value: { id?: string; title?: string; render?: unknown }) {
      captured.definition = value;
      return () => {};
    },
  });
  assert.ok(captured.definition);
  assert.equal(captured.definition.id, "feature-lab");
  assert.equal(captured.definition.title, "Feature Lab");
  assert.equal(typeof captured.definition.render, "function");
});

it("内置拓展清单、设置清单与分片清单三处登记一致", async () => {
  const read = (path: string) => readFile(new URL(path, import.meta.url), "utf8");

  const registry = JSON.parse(await read("../../../crates/codex-plus-core/assets/enhancement-settings.json")) as {
    builtins: Array<{ id: string }>;
  };
  assert.ok(registry.builtins.some((item) => item.id === "feature-lab"), "设置清单应有 feature-lab");

  const builtins = await read("../../../assets/inject/renderer-inject/96-builtin-extensions.js");
  assert.match(builtins, /id: "feature-lab"/);
  assert.match(builtins, /codexPlusFeatureLabExtension/);

  const manifest = JSON.parse(await read("../../../assets/inject/renderer-inject/manifest.json")) as {
    fragments: Array<{ name: string }>;
  };
  const names = manifest.fragments.map((fragment) => fragment.name);
  const lab = names.indexOf("96-feature-lab.js");
  assert.ok(lab >= 0, "manifest 应登记 96-feature-lab.js");
  assert.ok(lab < names.indexOf("96-builtin-extensions.js"), "Feature Lab 分片应在内置拓展清单之前");
});

it("Statsig 页签：选择默认即删掉这一项，其余整份保留且不改入参", async () => {
  const api = await runtime();
  const current = { "983842163": "on", "623263923": "off", "bad": "on" };
  assert.deepEqual(api.labNext(current, "983842163", "default"), { "623263923": "off" });
  assert.deepEqual(api.labNext(current, "1174206456", "native"), { "983842163": "on", "623263923": "off", "1174206456": "native" });
  assert.deepEqual(api.labNext(current, "x.y", "on"), { "983842163": "on", "623263923": "off" }, "非法 ID 不写入");
  assert.deepEqual(current, { "983842163": "on", "623263923": "off", "bad": "on" });

  assert.equal(api.labChoice(current, "983842163"), "on");
  assert.equal(api.labChoice(current, "1"), "default");
  assert.equal(api.labChoice({ "1": "weird" }, "1"), "default");

  assert.equal(api.labDefault({ defaultOverride: true }), "强制打开");
  assert.equal(api.labDefault({ defaultOverride: null }), "不覆盖");
});

it("Statsig 页签：「其他开关」只列不在清单里的，按查询次数排序，含手动覆盖过的", async () => {
  const api = await runtime();
  const rows = api.labUnknown(
    [{ id: "1174206456" }],
    { "999": "on" },
    [
      { id: "1174206456", native: false, count: 9 },
      { id: "222", native: true, count: 1 },
      { id: "333", native: false, count: 5 },
      { id: "not-digits", native: true, count: 3 },
    ],
  );
  assert.deepEqual(rows.map((row) => row.id), ["333", "222", "999"]);
  assert.equal(rows[0].native, false);
  assert.equal(rows[2].native, null, "只有覆盖记录、没被查询过的原值未知");
});
