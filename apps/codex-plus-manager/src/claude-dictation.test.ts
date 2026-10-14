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
