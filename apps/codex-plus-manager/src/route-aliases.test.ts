import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { it } from "node:test";
import { isRouteAlias, resolveRouteAlias, ROUTE_ALIASES } from "./route-aliases.ts";
import { APP_SOURCE_FILES, readAppSource } from "./app-source.test-helper.ts";

const app = readAppSource().replace(/\r\n/g, "\n");

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

it("⌘K 能搜到设置的各个分类（更新、日志、配置体检…），搜索框有无障碍名称；页面名称一页一个", () => {
  assert.match(app, /for \(const entry of settingsNavEntries\(toolEntries\.some\(/);
  assert.match(app, /keywords: `settings \$\{entry\.route\} \$\{entry\.key\} \$\{entry\.keywords \?\? ""\}`,/);
  assert.match(app, /if \(entry\.sections\) setSettingsSections\(entry\.sections\);\n\s*void navigate\(entry\.route\);/);
  const palette = readFileSync(new URL("./CommandPalette.tsx", import.meta.url), "utf8");
  assert.match(palette, /<input\n\s*aria-label=\{t\("跳到页面、Agent 或执行操作…"\)\}/);
  assert.match(app, /\{ id: "settings", get label\(\) \{ return t\("设置"\); \}/);
  assert.match(app, /\{ id: "hub", get label\(\) \{ return t\("本地服务总览"\); \}/);
  assert.doesNotMatch(app, /t\("整体设置"\)/);
  const en = readFileSync(new URL("./i18n-en.ts", import.meta.url), "utf8");
  assert.match(en, /"本地服务总览": "Local service overview"/);
  assert.match(en, /"概览": "Overview"/);
});

it("各窗口按需加载自己的界面；英文词典只在用英文时加载，渲染前等它到", () => {
  const main = readFileSync(new URL("./main.tsx", import.meta.url), "utf8");
  assert.match(main, /if \(hash === "#tray-panel"\) return import\("\.\/TrayPanel"\)/);
  assert.match(main, /Promise\.all\(\[loadWindowRoot\(hash\), loadLanguagePack\(\)\]\)/);
  const i18n = readFileSync(new URL("./i18n.ts", import.meta.url), "utf8");
  assert.match(i18n, /^import type \* as EnglishPack from "\.\/i18n-en";$/m);
  assert.match(i18n, /if \(language === "en" && !EN\) EN = await import\("\.\/i18n-en"\);/);
  // 其他源码不能再静态引入英文词典，否则又打进主包。
  for (const name of [...APP_SOURCE_FILES, "TrayPanel.tsx", "CuPipWindow.tsx", "main.tsx"]) {
    assert.doesNotMatch(readFileSync(new URL(`./${name}`, import.meta.url), "utf8"), /from "(@|\.)\/i18n-en"/, name);
  }
  // 样式仍在入口一次引入，保持原来的层叠顺序：组件样式在 styles.css、cursor-ui.css 之前。
  const order = ["./TrayPanel.css", "./styles.css", "./cursor-ui.css"].map((name) => main.indexOf(`import "${name}";`));
  assert.ok(order.every((index, position) => index > 0 && (position === 0 || index > order[position - 1])), String(order));
});
