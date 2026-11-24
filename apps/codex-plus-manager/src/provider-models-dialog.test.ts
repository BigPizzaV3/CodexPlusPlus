import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const read = (name: string) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");

test("模型列表可以手动编辑、从上游同步，后台发现的差异直接带进弹窗；「+N 新模型」和「从上游同步」打开同一个弹窗", () => {
  const dialog = read("ProviderModelsDialog.tsx");
  assert.match(dialog, /const added = entry\.upstreamNew \?\? \[\];/);
  assert.match(dialog, /if \(syncOnOpen\) void load\(\);/);
  const providers = read("GatewayProviders.tsx");
  assert.match(providers, /"save_gateway_provider_models", \{ id: entry\.id, models, defaultModel \}/);
  assert.match(providers, /onClick=\{\(\) => setModelsOpen\("edit"\)\}/);
  // 「+N 新模型」和详情里的「从上游同步」都打开模型弹窗并立刻拉上游（#2785）；没有单独的同步弹窗了。
  assert.equal(providers.match(/onClick=\{\(\) => setModelsOpen\("sync"\)\}/g)?.length, 2);
  assert.doesNotMatch(providers, /ProviderSyncDialog/);
  // 勾选的同步改动先按同步写（清掉被删模型的价格、窗口、思考等级），再写最终列表。
  assert.match(providers, /"apply_gateway_provider_models_sync", \{ id: entry\.id, remove: sync\.remove, add: sync\.add \}, t\("从上游同步"\), true\)/);
  assert.match(providers, /\{t\("从上游同步"\)\}/);
  assert.doesNotMatch(providers, /fetch_gateway_provider_models/);
  assert.doesNotMatch(providers, /只属于网关，直连模式里看不到/);
});

test("共享供应商顶栏的搜索框和按钮同高、不带按钮自己的上边距", () => {
  assert.match(read("GatewayPage.css"), /\.shell \.gw-provider-search-row \.gw-primary-button \{\s*margin-top: 0;/);
});

test("Agent 接入的开关在模型下拉之后", () => {
  const agents = read("GatewayAgents.tsx");
  const row = agents.slice(agents.indexOf('className="gw-agent-row cx-row"'));
  assert.ok(row.indexOf("<ModelPicker") < row.indexOf("<Switch"));
});
