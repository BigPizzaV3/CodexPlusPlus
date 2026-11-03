import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const read = (name: string) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");


test("总览「最近请求」是真实请求列表，对得上路由记录的那条可展开看这次怎么走的，右上角去请求日志", () => {
  const hub = read("HubOverview.tsx");
  assert.match(hub, /title=\{t\("最近请求"\)\}/);
  assert.match(hub, /onNavigate\("requestLog"\)/);
  assert.doesNotMatch(hub, /gatewayRouting/);
  assert.match(hub, /useRequestFlow\(FLOW_REFRESH_MS\)/);
  assert.match(hub, /<FlowRequestRow entry=\{entry\} \/>/);
  assert.match(hub, /routeNarrative\(agent, record, payload\)/);
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
  // 隐藏成员在组卡片标题行的三点菜单里（打勾项），卡片不再有底栏。
  assert.match(page, /<MenuItem checked=\{group\.hideMembers === true\} disabled=\{disabled\} onClick=\{\(\) => onChange\(\{ hideMembers: !group\.hideMembers \}\)\}>/);
  assert.doesNotMatch(page, /gw-route-group-foot/);
  // 编辑器不管隐藏成员与手动选中：保存时沿用卡片上的。
  assert.match(page, /hideMembers: previous\?\.hideMembers \?\? false, pick: previous\?\.pick \?\? ""/);
});

test("侧栏顺序：供应商池 → Agent 接入 → 路由；供应商池只有按供应商的列表（「按模型」矩阵已去掉）", () => {
  const app = read("App.tsx");
  assert.match(app, /const gatewayNavigationRoutes: Route\[\] = \["hub", "gatewayProviders", "gateway", "gatewayRouting", "gatewaySettings"\];/);
  const providers = read("GatewayProviders.tsx");
  assert.doesNotMatch(providers, /<ModelMatrix\s|t\("按模型"\)/);
  assert.match(providers, /placeholder=\{t\("查找供应商"\)\}/);
  assert.match(read("GatewayAgents.tsx"), /section\.key === "group" && nav\.openModelGroups \?/);
});
