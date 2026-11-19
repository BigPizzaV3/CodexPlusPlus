import assert from "node:assert/strict";
import { test } from "node:test";
import { readFile } from "node:fs/promises";

/**
 * 「远程控制」页：前端只调一条 `remote_control_request`，按 action 分发；
 * 钉死路由登记、命令名与后端注册一致、文案都有英文。
 */
const read = (path: string) => readFile(new URL(path, import.meta.url), "utf8");

test("远程控制页挂在侧栏 Codex 下面、紧跟「微信连接」，⌘K 仍能搜到，并在路由表中渲染", async () => {
  const app = await read("./App.tsx");
  assert.match(app, /\| "remoteControl" \|/, "Route 类型里要有 remoteControl");
  // 属于 Codex：只在 Codex 的子项里出现，切到别的 Agent 时不跟过去。
  assert.match(app, /\{ id: "remoteControl", get label\(\) \{ return t\("远程控制"\); \}, icon: Smartphone, badge: "实验", tool: "codex" \}/);
  // 侧栏 Codex 子项来自 navigationSections：放在「微信连接」后面（两者都是「在手机上用 Codex」）。
  const sections = app.slice(app.indexOf("const navigationSections"), app.indexOf("const routeTool"));
  assert.match(sections, /"weixin", "remoteControl"/);
  // 不再是全局页：打开它时 Codex 那一行保持展开，子项选中态才看得见；也不再在「工具」里重复出现。
  assert.doesNotMatch(app, /const globalNavigationRoutes: Route\[\] = \[[^\]]*"remoteControl"[^\]]*\];/);
  assert.doesNotMatch(app, /const toolNavigationRoutes: Route\[\] = \[[^\]]*"remoteControl"[^\]]*\];/);
  // ⌘K：Codex 的全部子页面（agentRoutes）都会进命令面板。
  assert.match(app, /agentRoutes\(tool\.id\)\.forEach\(\(routeId\) => pushCommand\(routeCommand\(routeId, tool\.name,/);
  assert.match(app, /\{route === "remoteControl" \? <RemoteControlPage \/> : null\}/);
  assert.match(app, /remoteControl: t\("/, "页面要有副标题");
});

test("前端调用的命令名与 Tauri 注册一致，动作与后端分发一致", async () => {
  const page = await read("./RemoteControlPage.tsx");
  const commands = await read("../src-tauri/src/remote_control_commands.rs");
  const core = await read("../../../crates/codex-plus-core/src/remote_control/mod.rs");
  const lib = await read("../src-tauri/src/lib.rs");
  assert.match(page, /invoke<RemoteControlStatus>\("remote_control_request", \{ action, args \}\)/);
  assert.match(commands, /pub async fn remote_control_request\(action: String, args: Option<Value>\)/, "带参数的动作从 args 取");
  assert.match(commands, /rc::dispatch\(&action, &args\.unwrap_or\(Value::Null\)\)/, "与 Codex 页面共用 core 的分发");
  assert.match(lib, /remote_control_commands::remote_control_request,/);
  assert.match(lib, /remote_control_commands::start_from_saved_state_in_background\(\);/, "管理器启动时按保存的开关拉起");
  assert.match(lib, /codex_plus_core::remote_control::stop\(\);/, "管理器退出时停掉服务");
  for (const action of ["status", "enable", "disable", "resetToken", "enableLan", "disableLan", "startPairing", "cancelPairing", "setDeviceRole", "revokeDevice"]) {
    assert.ok(page.includes(`"${action}"`), `前端缺少动作 ${action}`);
    assert.ok(core.includes(`"${action}" =>`), `后端缺少动作 ${action}`);
  }
});

test("远程控制页的文案都有英文", async () => {
  const page = await read("./RemoteControlPage.tsx");
  const en = await read("./i18n-en.ts");
  const keys = [...page.matchAll(/\bt\("([^"]+)"\)/g)].map((match) => match[1]);
  assert.ok(keys.length > 10);
  const missing = [...new Set(keys)].filter((key) => !en.includes(`${JSON.stringify(key)}:`));
  assert.deepEqual(missing, []);
});

test("访问链接显示时把令牌打码", async () => {
  const page = await read("./RemoteControlPage.tsx");
  assert.match(page, /url\.replace\(\/#token=\.\*\/, "#token=…"\)/);
});

test("配对与设备管理：带参数的动作传对参数名，移除设备要确认", async () => {
  const page = await read("./RemoteControlPage.tsx");
  const core = await read("../../../crates/codex-plus-core/src/remote_control/mod.rs");
  assert.match(page, /run\("startPairing", \{ role: pairRole \}\)/);
  assert.match(page, /run\("setDeviceRole", \{ id: device\.id, role \}\)/);
  assert.match(page, /run\("revokeDevice", \{ id: device\.id \}\)/);
  assert.match(core, /"startPairing" => start_pairing\(arg\("role"\)\?\)/);
  assert.match(core, /"setDeviceRole" => set_device_role\(arg\("id"\)\?, arg\("role"\)\?\)/);
  assert.match(core, /"revokeDevice" => revoke_device\(arg\("id"\)\?\)/);
  assert.match(page, /await appConfirm\(t\("移除后这台设备立刻无法访问/, "移除设备前确认");
});

test("Codex 左侧图标栏的「远程控制」入口：登记进所有写死内置 id 的地方，页面经 /remote-control 调同一份分发", async () => {
  const renderer = await read("../../../assets/inject/renderer-inject.js");
  const pageSource = await read("../../../assets/inject/renderer-inject/45-remote-control-page.js");
  const routes = await read("../../../crates/codex-plus-core/src/routes.rs");
  const extensionApi = await read("../../../assets/inject/renderer-inject/91-extension-api.js");
  const core = await read("../../../crates/codex-plus-core/src/remote_control/mod.rs");
  const commands = await read("../src-tauri/src/remote_control_commands.rs");
  assert.match(renderer, /codexPlusRailRemoteId = "codex-plus-rail-remote"/);
  assert.match(renderer, /\{ id: codexPlusRailRemoteId, label: "远程控制", iconMarkup: icons\.remote, withStatus: false, onActivate: openCodexPlusRemoteControlPage \}/);
  assert.match(renderer, /\[codexPlusRailRemoteId, "remote"\],/, "选中态");
  assert.match(renderer, /if \(tab === codexPlusRemoteControlTab\) return "remote";/);
  // 两处图标栏点击监听都要排除新入口，否则点它会把刚打开的页面关掉。
  assert.equal(renderer.split("#${codexPlusRailSponsorId}, #${codexPlusRailRemoteId}, #${codexPlusRailWeixinId}, #${codexPlusRailTasksId}`)) return;").length - 1, 2);
  assert.match(renderer, /#\$\{codexPlusRailRemoteId\} > button/, "扫描调度排除自己的节点");
  assert.match(pageSource, /data-codex-plus-active-tab="\$\{codexPlusRemoteControlTab\}"/);
  assert.match(pageSource, /postJson\("\/remote-control", \{ action, \.\.\.args \}\)/);
  assert.match(pageSource, /window\.__codexPlusExtensionPageCleanup = \(\) => \{/, "关页时停掉轮询");
  assert.match(pageSource, /await codexPlusAsk\(/, "移除设备前确认");
  assert.match(routes, /"\/remote-control" => \{/);
  assert.ok(!extensionApi.includes(`"/remote-control"`), "不开放给拓展");
  assert.match(core, /if !HOSTED\.load/, "bridge 不在管理器进程里时不起服务");
  assert.match(commands, /rc::mark_hosted_by_this_process\(\);/);
  for (const action of ["enable", "disable", "resetToken", "enableLan", "disableLan", "startPairing", "cancelPairing", "revokeDevice"]) {
    assert.ok(pageSource.includes(`"${action}"`), `Codex 页面缺少动作 ${action}`);
  }
  assert.match(pageSource, /codexPlusRemoteRun\("setDeviceRole", \{ id: deviceId, role \}\)/);
});
