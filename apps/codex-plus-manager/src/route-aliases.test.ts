import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { it } from "node:test";
import { isRouteAlias, resolveRouteAlias, ROUTE_ALIASES } from "./route-aliases.ts";

const app = readFileSync(new URL("./App.tsx", import.meta.url), "utf8").replace(/\r\n/g, "\n");

it("旧路由 id 换成现在的页面，其余原样返回", () => {
  assert.equal(resolveRouteAlias("maintenance"), "agentSettings");
  assert.equal(resolveRouteAlias("about"), "settings");
  assert.equal(resolveRouteAlias("relayEnvironment"), "relay");
  assert.equal(resolveRouteAlias("recommendations"), "home");
  assert.equal(resolveRouteAlias("dreamSkin"), "enhance");
  assert.equal(resolveRouteAlias("sessions"), "sessions");
  assert.equal(isRouteAlias("toString"), false, "原型上的键不算旧 id");
});

it("别名目标都是现有路由，旧 id 不再是路由表里的页面（dreamSkin 由增强页兜底保留）", () => {
  const routeIds = new Set([...app.matchAll(/^\s*\{ id: "([A-Za-z]+)",/gm)].map((match) => match[1]));
  for (const [alias, target] of Object.entries(ROUTE_ALIASES)) {
    assert.ok(routeIds.has(target), `${alias} → ${target} 不是路由`);
    if (alias !== "dreamSkin") assert.ok(!routeIds.has(alias), `${alias} 不该再是单独的路由`);
  }
});

it("所有入口都经别名表：navigate、拓展 navigate、?previewRoute；会话深链走 navigate 读列表", () => {
  assert.match(app, /const navigate = async \(target: RouteTarget\) => \{/);
  assert.match(app, /const next = isRouteAlias\(target\) \? target : isKnownRoute\(target\) \? target : null;/);
  assert.match(app, /const previewTarget = previewRoute \? resolveRouteAlias<string>\(previewRoute\) : null;/);
  // 进页面的数据并行读；会话页回来时停在原来那一页；「连接与供应商」不再读微信状态。
  assert.match(app, /await Promise\.all\(loads\);/);
  assert.match(app, /refreshLocalSessions\(true, sessionsOffset\)/);
  const relayLoads = app.slice(app.indexOf('      case "relay":'), app.indexOf('      case "multiLaunch":'));
  assert.doesNotMatch(relayLoads, /refreshWeixinStatus/);
  // 拓展市场和本地清单一起读。
  assert.match(app, /await Promise\.all\(\[refreshScriptMarket\(true\), refreshUserScriptInventory\(\)\]\)/);
});
