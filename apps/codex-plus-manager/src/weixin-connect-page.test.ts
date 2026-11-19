import assert from "node:assert/strict";
import { test } from "node:test";
import { readFile } from "node:fs/promises";

/**
 * Codex 左侧图标栏的「微信连接」页（45-weixin-connect-page.js）：经 bridge 路由 `/weixin-connect`
 * 调管理器登记的宿主路由，动作直接调管理器微信页用的同一组命令函数。
 * 钉死：路由不开放给拓展、分片登记与行数、图标栏入口登记进所有写死内置 id 的地方、
 * 页面关键元素（纯函数渲染）、按键在 window 捕获阶段接、不用原生 select。
 */
const read = (path: string) => readFile(new URL(path, import.meta.url), "utf8");
const shardPath = "../../../assets/inject/renderer-inject/45-weixin-connect-page.js";

type PageState = Record<string, any>;

/** 分片顶层只有常量和函数声明：补上它依赖的工具函数，直接求值拿出渲染函数。 */
async function loadShard() {
  const source = await read(shardPath);
  const stubs = `
    const escapeHtml = (value) => String(value).replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;").replaceAll('"', "&quot;").replaceAll("'", "&#39;");
    const codexPlusMenuIcons = { chevron: "<svg data-chevron></svg>", check: "<svg data-check></svg>" };
    const postJson = async () => ({ status: "failed" });
  `;
  return new Function(`${stubs}\n${source}\nreturn { codexPlusWeixinPageHtml, codexPlusWeixinPageState };`)() as {
    codexPlusWeixinPageHtml: (state: PageState) => string;
    codexPlusWeixinPageState: PageState;
  };
}

const longAccount = "o9cq80-very-long-account-id-that-would-overflow-the-row@im.bot.weixin.qq.com";
const baseState = (patch: PageState = {}): PageState => ({
  busy: "",
  error: "",
  notice: "",
  copied: false,
  qr: null,
  catalog: {
    models: ["gpt-5.5", "deepseek-v4-pro"],
    model: "gpt-5.5",
    modelMetadata: {
      "gpt-5.5": { displayName: "GPT-5.5", supportedReasoningEfforts: [{ reasoningEffort: "low" }, { reasoningEffort: "high" }] },
    },
  },
  catalogError: "",
  catalogLoading: false,
  data: {
    status: "ok",
    connection: { state: "running", message: "微信连接运行中。", accountId: longAccount, processedMessages: 7, lastPeerId: "wxid_peer_with_a_really_long_identifier_value@im.wechat" },
    settings: {
      weixinConnectAllowFrom: "me@im.wechat",
      weixinConnectWorkDir: "/Users/me/code/project",
      weixinConnectModel: "",
      weixinConnectReasoningEffort: "high",
      weixinConnectDefaultMode: "jiajia",
      weixinConnectSandbox: "workspace-write",
      weixinConnectPushToWeixin: true,
      weixinConnectAllowOutboundAttachments: false,
      weixinConnectAccountId: longAccount,
      loggedIn: true,
    },
    remoteControlRunning: false,
  },
  ...patch,
});

test("路由不开放给拓展；routes.rs 有专门的分支，管理器没跑时照实提示", async () => {
  const extensionApi = await read("../../../assets/inject/renderer-inject/91-extension-api.js");
  const routes = await read("../../../crates/codex-plus-core/src/routes.rs");
  const page = await read("../../../crates/codex-plus-core/src/connect/page.rs");
  assert.ok(!extensionApi.includes(`"/weixin-connect"`), "不开放给拓展");
  assert.match(page, /pub const BRIDGE_ROUTE: &str = "\/weixin-connect";/);
  assert.match(routes, /crate::connect::page::BRIDGE_ROUTE => match host_route\(path\) \{/);
  assert.match(routes, /Codex\+\+ 管理器未运行，暂时不能在 Codex 里操作微信连接/);
});

test("宿主路由在管理器登记，每个动作调管理器微信页同一组命令函数，不另写一份", async () => {
  const lib = await read("../src-tauri/src/lib.rs");
  const bridge = await read("../src-tauri/src/weixin_connect_bridge.rs");
  assert.match(lib, /register_host_route\(codex_plus_core::connect::page::BRIDGE_ROUTE, weixin\);/);
  for (const call of [
    "commands::weixin_connect_qr_start(",
    "commands::weixin_connect_qr_status()",
    "commands::weixin_connect_start()",
    "commands::weixin_connect_stop()",
    "commands::weixin_connect_status()",
    "commands::weixin_connect_catalog()",
    "commands::save_settings(settings, None)",
  ]) {
    assert.ok(bridge.includes(call), `应复用 ${call}`);
  }
  // 设置只合并白名单字段；改完通知管理器界面重新读取。
  assert.match(bridge, /page::apply_page_settings\(&mut settings, patch\)\?;/);
  assert.match(bridge, /notify_external_settings_change\(\)/);
  // 页面拿不到登录凭据。
  assert.doesNotMatch(await read("../../../crates/codex-plus-core/src/connect/page.rs"), /"weixinConnectToken":/);
});

test("分片登记进清单，行数与文件一致；页面的动作都在后端分发里", async () => {
  const manifest = JSON.parse(await read("../../../assets/inject/renderer-inject/manifest.json"));
  const fragments: Array<{ name: string; lines: number }> = manifest.fragments;
  const names = fragments.map((fragment) => fragment.name);
  const index = names.indexOf("45-weixin-connect-page.js");
  assert.ok(index > 0, "分片登记进清单");
  assert.equal(names[index - 1], "45-remote-control-page.js", "紧跟远程控制页");
  const source = await read(shardPath);
  assert.equal(fragments[index].lines, source.split("\n").length - 1);
  const bridge = await read("../src-tauri/src/weixin_connect_bridge.rs");
  for (const action of ["state", "save", "qrStart", "qrStatus", "start", "stop", "catalog"]) {
    assert.ok(source.includes(`"${action}"`), `页面缺少动作 ${action}`);
    assert.ok(bridge.includes(`"${action}"`), `后端缺少动作 ${action}`);
  }
  assert.match(source, /postJson\("\/weixin-connect", \{ action, \.\.\.args \}\)/);
});

test("Codex 左侧图标栏的「微信连接」入口：紧跟远程控制，登记进所有写死内置 id 的地方", async () => {
  const renderer = await read("../../../assets/inject/renderer-inject.js");
  const source = await read(shardPath);
  assert.match(renderer, /codexPlusRailWeixinId = "codex-plus-rail-weixin"/);
  assert.match(renderer, /codexPlusWeixinConnectTab = "weixin-connect"/);
  assert.match(renderer, /onActivate: openCodexPlusRemoteControlPage \},\n(?:\s*\/\/[^\n]*\n)*\s*\{ id: codexPlusRailWeixinId, label: "微信连接", iconMarkup: icons\.weixin, withStatus: false, onActivate: openCodexPlusWeixinConnectPage \}/);
  assert.match(renderer, /\[codexPlusRailWeixinId, "weixin"\],/, "选中态");
  assert.match(renderer, /if \(tab === codexPlusWeixinConnectTab\) return "weixin";/);
  // 两处图标栏点击监听都要排除新入口，否则点它会把刚打开的页面关掉。
  assert.equal(renderer.split("#${codexPlusRailRemoteId}, #${codexPlusRailWeixinId}, #${codexPlusRailTasksId}`)) return;").length - 1, 2);
  assert.match(renderer, /#\$\{codexPlusRailWeixinId\} > button/, "扫描调度排除自己的节点");
  assert.match(renderer, /codexPlusRailRemoteId, codexPlusRailWeixinId, codexPlusRailTasksId, codexPlusRailExtensionsId, codexPlusRailSponsorId\]\.forEach\(\(id\) => \{/, "第三方入口排在内置之后");
  assert.match(source, /data-codex-plus-active-tab="\$\{codexPlusWeixinConnectTab\}"/);
  assert.match(source, /markCodexPlusExtensionNode\(overlay, "builtin-weixin-connect"\)/);
  assert.match(source, /setCodexPlusSidebarNavActive\(true, "weixin"\)/);
});

test("按键在 window 捕获阶段接，关页时移除并停掉轮询；不用原生 select", async () => {
  const source = await read(shardPath);
  assert.match(source, /window\.addEventListener\("keydown", codexPlusWeixinCaptureKeydown, true\);/);
  assert.match(source, /window\.removeEventListener\("keydown", codexPlusWeixinCaptureKeydown, true\);/);
  assert.match(source, /window\.__codexPlusExtensionPageCleanup = \(\) => \{\n\s*clearInterval\(state\.pollTimer\);/);
  assert.match(source, /state\.qrGeneration \+= 1;/, "关页时停掉扫码轮询");
  assert.doesNotMatch(source, /<select/);
  assert.match(source, /class="codex-plus-menu-trigger"/);
  assert.match(source, /class="codex-plus-menu-popover codex-plus-weixin-menu-popover"/);
});

test("样式：引用 Codex 变量、长文本省略号截断、行内控件可收缩", async () => {
  const style = await read("../../../assets/inject/renderer-inject/10-style.js");
  assert.match(style, /\.codex-plus-weixin-ellipsis \{ display: block; min-width: 0; max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; \}/);
  assert.match(style, /\.codex-plus-weixin-page \.codex-plus-row > :not\(:first-child\) \{ max-width: 60%; min-width: 0; \}/);
  assert.match(style, /\.codex-plus-weixin-qr-link \{[^}]*text-overflow: ellipsis; white-space: nowrap; \}/);
  assert.match(style, /data-codex-plus-active-tab="weixin-connect"\] \{\n\s*background: var\(--codex-plus-settings-page-bg, var\(--codex-plus-bg-primary\)\) !important;/);
});

test("已登录、运行中：状态、停止 / 重新登录、对话、加加、权限、更多设置都在；长 ID 省略号截断", async () => {
  const { codexPlusWeixinPageHtml } = await loadShard();
  const html = codexPlusWeixinPageHtml(baseState());
  assert.match(html, /data-codex-weixin-state="running"[^>]*>运行中</);
  assert.match(html, /data-codex-weixin-action="stop"/);
  assert.match(html, /data-codex-weixin-action="qrStart"[^>]*>重新登录</);
  assert.doesNotMatch(html, /data-codex-weixin-action="start"/);
  // 账号、联系人 ID 截断，完整值放在 title 里。
  assert.match(html, new RegExp(`<code class="codex-plus-weixin-ellipsis" title="${longAccount.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}">`));
  assert.match(html, /已处理消息 <b>7<\/b>/);
  assert.match(html, /data-codex-weixin-field="weixinConnectAllowFrom" value="me@im\.wechat"/);
  assert.match(html, /data-codex-weixin-field="weixinConnectWorkDir" value="\/Users\/me\/code\/project"/);
  // 模型：继承默认（带显示名）；推理强度跟着继承的模型走。
  assert.match(html, /data-codex-weixin-menu="weixinConnectModel" data-value=""/);
  assert.match(html, /继承 Codex 默认模型 \(GPT-5\.5\)/);
  assert.match(html, /data-codex-weixin-menu="weixinConnectReasoningEffort" data-value="high"/);
  assert.match(html, /data-codex-weixin-menu="weixinConnectDefaultMode" data-value="jiajia"[^>]*aria-label="微信默认模式：加加"/);
  assert.match(html, /data-codex-weixin-remote="false">远程控制未开</);
  assert.match(html, /data-codex-weixin-action="openRemote"/, "远程控制没开时给个直达入口");
  assert.match(html, /aria-checked="true" data-enabled="true"[^>]*data-codex-weixin-toggle="weixinConnectPushToWeixin"/);
  assert.match(html, /data-codex-weixin-menu="weixinConnectSandbox" data-value="workspace-write"[^>]*aria-label="沙箱权限：允许修改工作目录"/);
  assert.match(html, /data-codex-weixin-toggle="weixinConnectAllowOutboundAttachments" >/, "填了白名单和目录时能打开附件回传");
  assert.match(html, /data-codex-weixin-action="openManager">在 Codex\+\+ 管理工具里打开更多设置</);
  for (const title of ["连接", "对话", "加加", "权限", "更多设置"]) assert.match(html, new RegExp(`aria-label="${title}"`));
  assert.doesNotMatch(html, /<select/);
  assert.doesNotMatch(html, /weixinConnectToken/, "页面里没有登录凭据");
});

test("没登录：主操作是扫码登录；二维码在页内显示，链接截断；附件回传缺条件时置灰", async () => {
  const { codexPlusWeixinPageHtml } = await loadShard();
  const state = baseState();
  state.data.settings = { ...state.data.settings, loggedIn: false, weixinConnectAllowFrom: "*", weixinConnectAccountId: "" };
  state.data.connection = { state: "stopped", message: "微信连接未启动。", accountId: "", processedMessages: 0, lastPeerId: "" };
  state.data.remoteControlRunning = true;
  state.qr = { qrStatus: "scaned", qrContent: "https://liteapp.weixin.qq.com/q/abc?qrcode=1234567890", qrSvg: "<svg data-qr></svg>" };
  const html = codexPlusWeixinPageHtml(state);
  assert.match(html, /data-primary="true" data-codex-weixin-action="qrStart"[^>]*>扫码登录</);
  assert.doesNotMatch(html, /重新登录|data-codex-weixin-action="stop"/);
  assert.match(html, /账号 <code class="codex-plus-weixin-ellipsis" title="未登录">未登录<\/code>/);
  assert.match(html, /已扫码，请在手机上确认/);
  assert.match(html, /<div class="codex-plus-remote-qr codex-plus-weixin-qr" data-codex-weixin-qr="true"><svg data-qr><\/svg><\/div>/);
  assert.match(html, /<code class="codex-plus-weixin-qr-link" title="https:\/\/liteapp/);
  for (const action of ["openQr", "copyQr", "cancelQr"]) assert.match(html, new RegExp(`data-codex-weixin-action="${action}"`));
  assert.match(html, /data-codex-weixin-remote="true">远程控制已开</);
  assert.doesNotMatch(html, /data-codex-weixin-action="openRemote"/);
  assert.match(html, /data-codex-weixin-toggle="weixinConnectAllowOutboundAttachments" disabled>/, "白名单是 * 时不能打开附件回传");
});

test("还没读到状态：读取中 / 失败时给重试；出错信息进横幅", async () => {
  const { codexPlusWeixinPageHtml } = await loadShard();
  assert.match(codexPlusWeixinPageHtml(baseState({ data: null, busy: "load" })), /正在读取…/);
  const failed = codexPlusWeixinPageHtml(baseState({ data: null, error: "Codex++ 管理器未运行，暂时不能在 Codex 里操作微信连接；请先打开 Codex++。" }));
  assert.match(failed, /role="alert">Codex\+\+ 管理器未运行/);
  assert.match(failed, /data-codex-weixin-action="reload">重试</);
});

test("模型不在目录里时标「不可用」并提示；目录读失败时显示原因", async () => {
  const { codexPlusWeixinPageHtml } = await loadShard();
  const state = baseState();
  state.data.settings = { ...state.data.settings, weixinConnectModel: "retired-model" };
  const html = codexPlusWeixinPageHtml(state);
  assert.match(html, /retired-model \(不可用\)/);
  assert.match(html, /当前模型已不在可用模型目录中，请重新选择。/);
  const broken = codexPlusWeixinPageHtml(baseState({ catalog: null, catalogError: "读取模型目录失败。" }));
  assert.match(broken, /读取模型目录失败。/);
});
