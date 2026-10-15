// Claude 语音输入：跨语言契约回归。
//
// 这条链路横跨四个文件，任何一处改名都会让功能"静默失效"（按钮在、点了没反应）：
//   页面 page-dictation.js ↔ 主进程 hook.js（binding 名、回投函数名）
//   hook.js ↔ launcher.rs（听写凭据文件路径与状态目录名）
//   claude_localization.rs 的 ASSETS（脚本没登记就不会被装进副本）
// 所以这里逐条钉住，而不是只测有没有这段代码。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";

const root = path.resolve(import.meta.dirname, "../../..");
const read = (relative: string) => readFileSync(path.join(root, relative), "utf8");

const hook = read("assets/claude-localization/hook.js");
const page = read("assets/claude-localization/page-dictation.js");
const launcher = read("crates/codex-plus-core/src/launcher.rs");
const paths = read("crates/codex-plus-core/src/paths.rs");
const localization = read("crates/codex-plus-core/src/claude_localization.rs");

function literal(source: string, pattern: RegExp, label: string): string {
  const match = source.match(pattern);
  assert.ok(match, `找不到${label}`);
  return match[1];
}

test("页面脚本调用的 binding 名与主进程注册的一致", () => {
  const binding = literal(hook, /const BINDING = '([^']+)'/, "hook.js 的 BINDING");
  assert.ok(
    page.includes(`window.${binding}`),
    `page-dictation.js 必须调用 window.${binding}`,
  );
  assert.ok(
    hook.includes(`{ name: BINDING }`),
    "hook.js 必须用同一个名字 addBinding，否则 bindingCalled 不会派发",
  );
});

test("结果回投函数名一致", () => {
  const deliver = literal(hook, /globalThis\.(__claudeCnDictationDeliver)/, "hook.js 的回投函数");
  assert.ok(
    new RegExp(`window\\.${deliver}\\s*=`).test(page),
    `page-dictation.js 必须定义 window.${deliver}`,
  );
});

test("听写凭据文件名与状态目录跨语言一致", () => {
  const receipt = literal(
    launcher,
    /const DICTATION_HELPER_RECEIPT: &str = "([^"]+)"/,
    "launcher.rs 的凭据文件名",
  );
  const stateDir = literal(paths, /const APP_STATE_DIR: &str = "([^"]+)"/, "paths.rs 的状态目录");
  assert.ok(
    hook.includes(`'${receipt}'`),
    `hook.js 必须读同一个凭据文件 ${receipt}`,
  );
  assert.ok(
    hook.includes(`'${stateDir}'`),
    `hook.js 的凭据目录必须与 paths.rs 的 ${stateDir} 一致`,
  );
});

test("语音脚本被登记进受管副本的资产清单", () => {
  assert.ok(
    localization.includes('"page-dictation.js"'),
    "claude_localization.rs 的 ASSETS 缺少 page-dictation.js，副本里不会有这个文件",
  );
  assert.ok(
    hook.includes("'page-dictation.js'"),
    "hook.js 必须注入 page-dictation.js",
  );
});

test("页面脚本保持幂等，并能自我识别宿主重建", () => {
  assert.ok(
    page.includes("window.__claudeCnDictationInstalled"),
    "重复注入必须直接返回，否则每次补注入都会叠一个入口",
  );
  assert.ok(
    page.includes("new MutationObserver"),
    "宿主是 React 应用，入口被重建后必须自动重挂",
  );
});

test("音频上传字段按白名单收敛，不把页面内容塞进请求头", () => {
  assert.ok(hook.includes("safeFilename"), "文件名必须过滤");
  assert.ok(hook.includes("safeMime"), "MIME 必须过滤");
  assert.ok(
    hook.includes("'X-Codex-Plus-Dictation-Token': receipt.token"),
    "helper 转写需要能力令牌，缺了会 403",
  );
});

test("Code 与 Cowork 两套发送键都要认得", () => {
  // 两套界面的发送键 testid 与文案完全不同：Code 是 code-prompt-send /「发送」，
  // Cowork 是 chat-input-send /「开始任务」。少认一套，入口就会掉到编辑器容器的末尾。
  assert.ok(page.includes("code-prompt-send"), "缺少 Code 模式的发送键 testid");
  assert.ok(page.includes("chat-input-send"), "缺少 Cowork 模式的发送键 testid");
  assert.ok(page.includes("开始任务"), "Cowork 的发送键标签是「开始任务」，兜底匹配必须覆盖");
  assert.ok(
    page.includes("setInterval(schedule"),
    "模式切换后 composer 会被整块重建，需要低频兜底重挂",
  );
});

test("语音输入开关关闭时不注入入口、也不注册调用通道", () => {
  // 管理器的「Claude 增强 → 语音输入」把开关写进环境变量，注入器再交给 hook。
  assert.ok(
    hook.includes("__claudeCnVoiceInput === false"),
    "hook 必须按开关决定是否注入 page-dictation.js",
  );
  assert.ok(
    hook.includes("if (DICTATION)") && hook.includes("Runtime.addBinding"),
    "关掉时不应注册 CDP binding，页面里不该出现可调用的全局函数",
  );
  const injector = read("assets/claude-localization/inject-cn.js");
  assert.ok(
    injector.includes("CODEX_PLUS_CLAUDE_VOICE_INPUT"),
    "注入器必须读取管理器传来的开关",
  );
  const localization = read("crates/codex-plus-core/src/claude_localization.rs");
  assert.ok(
    localization.includes("CODEX_PLUS_CLAUDE_VOICE_INPUT"),
    "启动脚本的环境变量名两端必须一致",
  );
});

test("界面汉化是独立开关，不跟副本准备绑定", () => {
  // 汉化与语音输入一样是可独立关闭的增强项；副本只是它们共同的前置条件。
  assert.ok(
    hook.includes("__claudeCnLocalization === false") && hook.includes("SHIM = ''"),
    "hook 必须能按开关整体跳过中文界面注入",
  );
  const injector = read("assets/claude-localization/inject-cn.js");
  assert.ok(
    injector.includes("CODEX_PLUS_CLAUDE_LOCALIZATION"),
    "注入器必须读取汉化开关",
  );
  const localization = read("crates/codex-plus-core/src/claude_localization.rs");
  assert.ok(
    localization.includes("CODEX_PLUS_CLAUDE_LOCALIZATION"),
    "启动脚本的环境变量名两端必须一致",
  );
  assert.ok(
    localization.includes("pub fn set_localization"),
    "必须有独立写汉化开关的入口",
  );
  const settings = read("crates/codex-plus-core/src/settings.rs");
  assert.ok(
    settings.includes('"claudeAppLocalization"'),
    "汉化开关必须持久化在自己的设置字段里",
  );
});
