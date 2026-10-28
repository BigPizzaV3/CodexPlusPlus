import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const read = (name: string) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");


test("总览「最近请求」可展开看这次怎么走的，右上角去请求日志", () => {
  const hub = read("HubOverview.tsx");
  assert.match(hub, /<h3>\{t\("最近请求"\)\}<\/h3>/);
  assert.match(hub, /onNavigate\("requestLog"\)/);
  assert.doesNotMatch(hub, /gatewayRouting/);
  assert.match(hub, /routeNarrative\(item\.agent, record, payload\)/);
});

test("界面不再出现「路由组」，统一叫模型组", () => {
  for (const name of ["GatewayPage.tsx", "GatewayAgents.tsx", "HubOverview.tsx", "gateway-model.ts"]) {
    assert.doesNotMatch(read(name), /"[^"\n]*路由组[^"\n]*"/, name);
  }
});



test("路由页：「我的组」和「自动组」两段卡片，卡片上直接改路由方式、会话固定、隐藏成员，手动模式点成员锁定", () => {
  const page = read("GatewayPage.tsx");
  assert.match(page, /\{ id: "routing", label: t\("路由"\) \}/);
  assert.match(read("App.tsx"), /id: "gatewayRouting", get label\(\) \{ return t\("路由"\); \}/);
  assert.doesNotMatch(page, /RouteDiagram|RouteCard|routeNarrative|DispatchTable|<ModelMatrix/);
  assert.match(page, /\{t\("我的组"\)\}/);
  assert.match(page, /\{t\("自动组"\)\}/);
  assert.match(page, /"save_gateway_auto_group", \{\s*model: group\.name,/);
  assert.match(page, /onPick=\{\(\) => onChange\(\{ pick: member\.model \}\)\}/);
  assert.match(page, /onToggle=\{\(\) => onChange\(\{ hideMembers: !group\.hideMembers \}\)\}/);
  // 编辑器不管隐藏成员与手动选中：保存时沿用卡片上的。
  assert.match(page, /hideMembers: previous\?\.hideMembers \?\? false, pick: previous\?\.pick \?\? ""/);
});

test("侧栏顺序：供应商池 → Agent 接入 → 路由；供应商池里「按模型」是供应商 × 模型矩阵，点列头回到列表展开那一家", () => {
  const app = read("App.tsx");
  assert.match(app, /const gatewayNavigationRoutes: Route\[\] = \["hub", "gatewayProviders", "gateway", "gatewayRouting", "gatewayUsage", "gatewaySettings"\];/);
  const providers = read("GatewayProviders.tsx");
  assert.match(providers, /<ModelMatrix\s/);
  assert.match(providers, /setView\("list"\);\s*setExpanded\(id\);/);
  const matrix = read("GatewayModelMatrix.tsx");
  assert.match(matrix, /onClick=\{\(\) => onOpenProvider\(entry\.id\)\}/);
  assert.match(matrix, /title=\{tf\("在供应商池里打开「\{0\}」", \[maskAccountName\(entry\.name\)\]\)\}/);
  assert.match(read("GatewayAgents.tsx"), /section\.key === "group" && nav\.openModelGroups \?/);
});
