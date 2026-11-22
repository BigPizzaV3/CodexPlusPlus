import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { it } from "node:test";
import { readAppSource } from "./app-source.test-helper.ts";

const read = async (path: string) => (await readFile(new URL(path, import.meta.url), "utf8")).replace(/\r\n/g, "\n");

it("网页导入 / 一键换肤待确认项靠事件和窗口唤起读取，不再定时轮询磁盘", async () => {
  const app = readAppSource();
  const lib = await read("../src-tauri/src/lib.rs");
  // 前后端事件名一致。
  const eventName = /const PENDING_IMPORT_CHANGED_EVENT: &str = "([^"]+)";/.exec(lib)?.[1];
  assert.ok(eventName, "后端要定义待确认项事件");
  assert.ok(app.includes(`const PENDING_IMPORT_CHANGED_EVENT = "${eventName}";`));
  // macOS「打开 URL」存好后发事件。
  assert.match(lib, /handle_provider_import_url\(url\.as_str\(\)\) \{\n\s*show_main_window\(app_handle\);\n\s*\/\/[^\n]*\n\s*let _ = app_handle\.emit\(PENDING_IMPORT_CHANGED_EVENT, \(\)\);/);
  assert.match(app, /listen\(PENDING_IMPORT_CHANGED_EVENT,/);
  // 窗口唤起 / 获得焦点（后端发导航事件）时也读一次。
  const navigationListener = app.slice(app.indexOf("void listen(MANAGER_NAVIGATION_EVENT"));
  assert.match(navigationListener.slice(0, 400), /refreshPendingProviderImport\(true\);\n\s*void refreshPendingDreamSkinCommunity\(\);/);
  // 旧的 1.2 秒轮询已删除。
  assert.doesNotMatch(app, /\}, 1200\);/);
});

it("App 级轮询走 usePolling（窗口不可见暂停），数据没变沿用旧引用；聚焦刷新概览有节流", async () => {
  const app = readAppSource();
  assert.match(app, /usePolling\(async \(\) => \{\n\s*const result = await invoke<AutoTakeoverResult>\("load_auto_takeover_state"\);\n\s*if \(autoTakeoverPollingRef\.current\) setAutoTakeoverState\(\(previous\) => keepIfSame\(previous, result\)\);\n\s*\}, 3000, \{ enabled: autoTakeoverPolling \}\);/);
  assert.match(app, /usePolling\(\(\) => refreshMultiLaunchInstances\(true\), 3_000, \{ enabled: route === "multiLaunch" \}\);/);
  assert.match(app, /setMultiLaunchInstances\(\(previous\) => keepIfSame\(previous, instances\)\);/);
  assert.match(app, /usePolling\(\(\) => refreshSessionIndexRepairReport\(\(\) => routeRef\.current === "sessions"\), 15_000, \{ enabled: route === "sessions" \}\);/);
  assert.match(app, /setWeixinStatus\(\(previous\) => keepIfSame\(previous, result\)\);\n\s*\}, 2_000, \{ enabled: route === "weixin", immediate: false \}\);/);
  assert.match(app, /if \(now - lastRefreshAt < FOCUS_REFRESH_MIN_INTERVAL_MS\) return;/);
  // App 里不再有裸 setInterval 轮询这些命令。
  assert.doesNotMatch(app, /window\.setInterval\(load, 3000\)/);
  assert.doesNotMatch(app, /window\.setInterval\(\(\) => void refreshWeixinStatus/);
});

it("顶栏操作按钮在 layout effect 里找挂载点，切页时不晚一帧", async () => {
  const source = await read("./TopbarActions.tsx");
  assert.match(source, /useLayoutEffect\(\(\) => \{\n\s*setSlot\(document\.querySelector\("\[data-topbar-actions\]"\)\);/);
  assert.doesNotMatch(source, /\buseEffect\b/);
});

it("Codex 概览的请求日志只读一次（仪表板读、概览算统计），刷新按钮连仪表板一起重读", async () => {
  const app = readAppSource();
  const dashboard = await read("./AgentDashboard.tsx");
  const overview = app.slice(app.indexOf("function OverviewScreen("), app.indexOf("\nfunction ", app.indexOf("function OverviewScreen(") + 10));
  assert.doesNotMatch(overview, /load_request_logs/);
  assert.match(overview, /onRequestLogs=\{summarizeCodexRequests\}/);
  assert.match(overview, /refreshKey=\{dashboardRefresh\}/);
  assert.match(overview, /onClick=\{\(\) => \{ setDashboardRefresh\(\(value\) => value \+ 1\); void actions\.refreshCurrent\(\); \}\}/);
  assert.equal(dashboard.match(/"load_request_logs"/g)?.length, 1);
  assert.match(dashboard, /\}, \[agent, refreshKey\]\);/);
});
