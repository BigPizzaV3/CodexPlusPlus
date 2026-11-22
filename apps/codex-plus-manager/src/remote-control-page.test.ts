import assert from "node:assert/strict";
import { test } from "node:test";
import { readFile } from "node:fs/promises";
import { readAppSource } from "./app-source.test-helper.ts";

/**
 * 「远程控制」页：前端只调一条 `remote_control_request`，按 action 分发；
 * 钉死路由登记、命令名与后端注册一致、文案都有英文。
 */
const read = (path: string) => readFile(new URL(path, import.meta.url), "utf8");

test("远程控制页挂在侧栏 Codex 下面、紧跟「微信连接」，⌘K 仍能搜到，并在路由表中渲染", async () => {
  const app = readAppSource();
  assert.match(app, /\| "remoteControl" \|/, "Route 类型里要有 remoteControl");
  // 属于 Codex：只在 Codex 的子项里出现，切到别的 Agent 时不跟过去。
  assert.match(app, /\{ id: "remoteControl", get label\(\) \{ return t\("远程控制"\); \}, icon: Smartphone, tool: "codex" \}/);
  // 侧栏 Codex 子项来自 navigationSections：放在「微信连接」后面（两者都是「在手机上用 Codex」）。
  const sections = app.slice(app.indexOf("const navigationSections"), app.indexOf("const routeTool"));
  assert.match(sections, /"weixin", "remoteControl"/);
  // 不再是全局页：打开它时 Codex 那一行保持展开，子项选中态才看得见；也不再在「工具」里重复出现。
  assert.doesNotMatch(app, /const globalNavigationRoutes: Route\[\] = \[[^\]]*"remoteControl"[^\]]*\];/);
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
  assert.match(lib, /codex_plus_core::remote_control::shutdown\(\);/, "管理器退出时停掉服务并禁止重建");
  for (const action of ["status", "enable", "disable", "setWeixinFocus", "resetToken", "enableLan", "disableLan", "startPairing", "cancelPairing", "setDeviceRole", "revokeDevice"]) {
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
  assert.match(page, /run\("setWeixinFocus", \{ enabled: !status\?\.weixinFocusEnabled \}\)/);
  assert.match(page, /run\("setDeviceRole", \{ id: device\.id, role \}\)/);
  assert.match(page, /run\("revokeDevice", \{ id: device\.id \}\)/);
  assert.match(core, /"startPairing" => start_pairing\(arg\("role"\)\?\)/);
  assert.match(core, /"setDeviceRole" => set_device_role\(arg\("id"\)\?, arg\("role"\)\?\)/);
  assert.match(core, /"revokeDevice" => revoke_device\(arg\("id"\)\?\)/);
  assert.match(page, /await appConfirm\(t\("移除后这台设备立刻无法访问/, "移除设备前确认");
  // 更换令牌会让已打开的网页全部失效：先确认（danger），确认后才发 resetToken。
  assert.match(page, /const resetToken = async \(\) => \{\n\s*const confirmed = await appConfirm\([\s\S]*?danger: true,[\s\S]*?if \(confirmed\) void run\("resetToken"\);/);
  assert.match(page, /onClick=\{\(\) => void resetToken\(\)\}/);
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
  for (const action of ["enable", "disable", "setWeixinFocus", "resetToken", "enableLan", "disableLan", "startPairing", "cancelPairing", "revokeDevice"]) {
    assert.ok(pageSource.includes(`"${action}"`), `Codex 页面缺少动作 ${action}`);
  }
  assert.match(pageSource, /codexPlusRemoteRun\("setDeviceRole", \{ id: deviceId, role \}\)/);
  assert.match(pageSource, /codexPlusRemoteRun\("setWeixinFocus", \{ enabled: !state\.data\?\.weixinFocusEnabled \}\)/);
});

/** 分片顶层只有常量和函数声明：补上它依赖的工具函数，直接求值拿出渲染函数。 */
async function loadRemoteShard() {
  const source = await read("../../../assets/inject/renderer-inject/45-remote-control-page.js");
  const stubs = `
    const escapeHtml = (value) => String(value).replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;").replaceAll('"', "&quot;");
    const postJson = async () => ({ status: "failed" });
  `;
  return new Function(`${stubs}\n${source}\nreturn { codexPlusRemoteRobotSection, codexPlusRemotePageState };`)() as {
    codexPlusRemoteRobotSection: (data: Record<string, any>) => string;
    codexPlusRemotePageState: Record<string, any>;
  };
}

test("Codex 远程控制页的钉钉与飞书通知：经 /remote-control/robots 调与管理器同一份分发，不开放给拓展", async () => {
  const pageSource = await read("../../../assets/inject/renderer-inject/45-remote-control-page.js");
  const routes = await read("../../../crates/codex-plus-core/src/routes.rs");
  const extensionApi = await read("../../../assets/inject/renderer-inject/91-extension-api.js");
  const core = await read("../../../crates/codex-plus-core/src/remote_control/mod.rs");
  const robots = await read("../../../crates/codex-plus-core/src/remote_control/chat_notifications.rs");
  const commands = await read("../src-tauri/src/remote_control_commands.rs");
  assert.match(pageSource, /postJson\("\/remote-control\/robots", \{ action, provider, update \}\)/);
  assert.match(routes, /"\/remote-control\/robots" => crate::remote_control::robots_bridge_route\(&payload\)\.await,/);
  assert.ok(!extensionApi.includes(`"/remote-control/robots"`), "不开放给拓展");
  assert.match(core, /pub async fn robots_bridge_route[\s\S]*?if !HOSTED\.load/, "bridge 不在管理器进程里时不放行");
  assert.match(core, /chat_notifications::dispatch\(action, provider, update\)\.await/);
  assert.match(commands, /rc::chat_notifications::dispatch\(&action, provider, update\)\.await/, "管理器命令也走同一份分发");
  for (const action of ["status", "save", "clear", "test"]) {
    assert.ok(robots.includes(`("${action}", `), `后端缺少动作 ${action}`);
    assert.ok(pageSource.includes(`"${action}"`), `Codex 页面缺少动作 ${action}`);
  }
  assert.match(pageSource, /window\.addEventListener\("keydown", codexPlusRemoteCaptureKeydown, true\);/, "按键在 window 捕获阶段接");
  assert.match(pageSource, /window\.removeEventListener\("keydown", codexPlusRemoteCaptureKeydown, true\);/);
  assert.doesNotMatch(pageSource, /<select|type="checkbox"/, "不用原生 select / checkbox");
});

test("机器人设置的渲染：没配置时只有「配置机器人」；配置中展开 Webhook、密钥、会话链接；已保存的值不回显", async () => {
  const { codexPlusRemoteRobotSection, codexPlusRemotePageState: state } = await loadRemoteShard();
  const data = { running: true, lan: { running: false } };
  state.robots = {
    dingtalk: { enabled: false, hasWebhook: false, hasSecret: false, linkNetwork: "lan", lastResult: null },
    feishu: { enabled: true, hasWebhook: true, hasSecret: true, linkNetwork: "tailscale", lastResult: "发送成功" },
  };
  let html = codexPlusRemoteRobotSection(data);
  assert.match(html, /aria-label="钉钉与飞书通知"/);
  assert.match(html, /要在手机上打开会话，请启用局域网访问并配对设备。/);
  assert.match(html, /尚未配置机器人/);
  assert.match(html, /发送成功/);
  assert.match(html, /data-codex-remote-action="robotToggle" data-codex-remote-robot="dingtalk" disabled>/, "没 Webhook 不能打开");
  assert.match(html, /aria-checked="true"[^>]*data-codex-remote-action="robotToggle" data-codex-remote-robot="feishu" >/);
  assert.match(html, /data-codex-remote-action="robotTest" data-codex-remote-robot="feishu"/);
  assert.doesNotMatch(html, /data-codex-remote-action="robotTest" data-codex-remote-robot="dingtalk"/);
  assert.doesNotMatch(html, /data-codex-remote-robot-field/);

  state.robotEdit = { provider: "feishu", webhook: "", secret: "", clearSecret: false, network: "tailscale" };
  html = codexPlusRemoteRobotSection(data);
  assert.match(html, /data-codex-remote-action="robotEdit" data-codex-remote-robot="feishu" >取消</);
  assert.match(html, /type="password" data-codex-remote-robot-field="webhook" value="" placeholder="已保存，留空保留"/);
  assert.match(html, /type="password" data-codex-remote-robot-field="secret" value="" placeholder="已保存，留空保留"/);
  assert.match(html, /data-codex-remote-action="robotClearSecret"/);
  assert.match(html, /aria-checked="true" data-codex-remote-robot-network="tailscale"/);
  assert.match(html, /data-codex-remote-action="robotClear"/);
  assert.match(html, /data-codex-remote-action="robotSave"/);
  assert.match(html, /data-codex-remote-action="robotToggle" data-codex-remote-robot="feishu" disabled>/, "配置中不能开关");

  state.robots = null;
  state.robotEdit = null;
  state.robotError = "Codex++ 管理器未运行，暂时不能在 Codex 里设置钉钉与飞书通知；请先打开 Codex++。";
  html = codexPlusRemoteRobotSection({ running: false, lan: {} });
  assert.match(html, /打开远程控制后开始监听任务通知。/);
  assert.match(html, /role="alert">Codex\+\+ 管理器未运行/);
  assert.match(html, /data-codex-remote-action="robotReload">重试</);
});
