import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import {
  CLAUDE_AUTO_TAKEOVER_COMMANDS,
  summarizeClaudeAutoTakeover,
  type ClaudeAutoTakeoverPayload,
} from "./claude-auto-takeover-model.ts";
import { appDeclarationSource, readAppSource } from "./app-source.test-helper.ts";

/**
 * 「Claude Desktop 自动接管」的状态折算与接线契约（照 auto-takeover.test.ts 的思路）。
 * 只做纯逻辑与静态检查，不启动 Tauri、不碰真实进程。
 */

const root = path.resolve(import.meta.dirname, "..");
const read = (file: string) => readFileSync(path.join(root, file), "utf8");
// 主界面拆成了多个文件，按拼起来的全文 / 按声明名取源码。
const appSource = readAppSource();
const componentSource = read("src/ClaudeAutoTakeover.tsx");
const libSource = read("src-tauri/src/lib.rs");
const commandsSource = read("src-tauri/src/commands.rs");
const glueSource = read("src-tauri/src/claude_auto_takeover.rs");
const settingsSource = read("../../crates/codex-plus-core/src/settings.rs");
const dictionary = read("src/i18n-en.ts");

function payload(patch: Partial<ClaudeAutoTakeoverPayload> = {}): ClaudeAutoTakeoverPayload {
  return {
    state: "probing",
    enabled: true,
    platformSupported: true,
    mode: "full",
    lastReason: "app_not_running",
    appRunning: false,
    attached: false,
    copyRunning: false,
    restartsLastHour: 0,
    restartsTotal: 0,
    running: true,
    ...patch,
  };
}

test("off, unsupported or non-full modes never offer a takeover", () => {
  assert.deepEqual(summarizeClaudeAutoTakeover(null), { tone: "off", label: "off", reason: null, error: null, canTakeOver: false });
  assert.equal(summarizeClaudeAutoTakeover(payload({ enabled: false })).label, "off");
  assert.equal(summarizeClaudeAutoTakeover(payload({ enabled: false, mode: "3p" })).reason, "mode_3p");
  assert.equal(summarizeClaudeAutoTakeover(payload({ platformSupported: false })).reason, "platform_unsupported");
  for (const mode of ["3p", "copy"]) {
    const summary = summarizeClaudeAutoTakeover(payload({ mode, appRunning: true }));
    assert.equal(summary.label, "inactive");
    assert.equal(summary.reason, mode === "3p" ? "mode_3p" : "mode_copy");
    assert.equal(summary.canTakeOver, false);
  }
});

test("listening state offers take-over only for an official instance that isn't attached", () => {
  const official = summarizeClaudeAutoTakeover(payload({ appRunning: true, lastReason: "user_active" }));
  assert.equal(official.tone, "ok");
  assert.equal(official.label, "listening");
  assert.equal(official.reason, "user_active");
  assert.equal(official.canTakeOver, true);
  assert.equal(summarizeClaudeAutoTakeover(payload({ appRunning: true, attached: true, lastReason: "already_attached" })).canTakeOver, false);
  assert.equal(summarizeClaudeAutoTakeover(payload({ appRunning: true, copyRunning: true })).canTakeOver, false);
  assert.equal(summarizeClaudeAutoTakeover(payload({ appRunning: false })).canTakeOver, false);
  assert.equal(summarizeClaudeAutoTakeover(payload({ running: false })).label, "off");
});

test("degraded keeps the backend message, but reason codes are translated instead", () => {
  const preflight = summarizeClaudeAutoTakeover(payload({ state: "degraded", lastReason: "backoff", lastError: "本地证书还没有信任，请先在 Desktop 增强页信任证书。", appRunning: true }));
  assert.equal(preflight.tone, "failed");
  assert.equal(preflight.reason, "backoff");
  assert.equal(preflight.error, "本地证书还没有信任，请先在 Desktop 增强页信任证书。");
  const budget = summarizeClaudeAutoTakeover(payload({ state: "degraded", lastError: "restart_budget_exhausted" }));
  assert.equal(budget.reason, "restart_budget_exhausted");
  assert.equal(budget.error, null);
  assert.equal(summarizeClaudeAutoTakeover(payload({ state: "taking_over", appRunning: true })).canTakeOver, false);
  assert.equal(summarizeClaudeAutoTakeover(payload({ state: "paused", pausedInstance: "1|x", appRunning: true })).reason, "paused_for_instance");
});

test("every reason code the Rust side can report has copy", () => {
  // Rust 侧可能写进 lastReason / lastError 的码：Claude 自己的 + 共用判据的。
  const coreSource = read("../../crates/codex-plus-core/src/claude_auto_takeover.rs");
  for (const code of ["mode_3p", "mode_copy", "enhanced_copy_running", "claude_not_installed", "node_missing", "ca_untrusted", "upstream_not_ready"]) {
    assert.ok(coreSource.includes(`"${code}"`), `core 里找不到 ${code}`);
  }
  for (const code of ["quit_failed", "launch_failed"]) {
    assert.ok(glueSource.includes(`"${code}"`), `管理器里找不到 ${code}`);
  }
  for (const code of [
    "mode_3p", "mode_copy", "platform_unsupported", "already_attached", "app_not_running", "enhanced_copy_running",
    "multi_instance", "user_active", "probe_unknown", "backoff", "restart_budget_exhausted", "paused_for_instance",
    "takeover", "claude_not_installed", "node_missing", "ca_untrusted", "upstream_not_ready", "quit_failed", "launch_failed",
  ]) {
    assert.match(componentSource, new RegExp(`\\b${code}: t\\(`), `ClaudeAutoTakeover.tsx 没有翻译 ${code}`);
  }
});

test("claude auto-takeover commands are registered in the tauri handler", () => {
  for (const command of Object.values(CLAUDE_AUTO_TAKEOVER_COMMANDS)) {
    assert.ok(commandsSource.includes(`pub async fn ${command}(`), `commands.rs 缺少 #[tauri::command] ${command}`);
    assert.ok(libSource.includes(`commands::${command},`), `lib.rs 的 generate_handler! 没有注册 ${command}`);
  }
  assert.ok(componentSource.includes("CLAUDE_AUTO_TAKEOVER_COMMANDS.set") && componentSource.includes("CLAUDE_AUTO_TAKEOVER_COMMANDS.takeOverNow")
    && componentSource.includes("CLAUDE_AUTO_TAKEOVER_COMMANDS.pause") && componentSource.includes("CLAUDE_AUTO_TAKEOVER_COMMANDS.load"));
});

test("switch is written by the backend and merged back into the settings form", () => {
  // 开关走 set_claude_auto_takeover 单独写 settings.json；App 的表单必须合并这个键，否则下一次整份保存会写回旧值。
  assert.ok(componentSource.includes("announceBackendSettingsChanged()"), "开关保存后要广播设置已变");
  assert.match(appSource, /for \(const key of \[[\s\S]*?"claudeAppAutoTakeover",[\s\S]*?\]\) merged\[key\] = fresh\[key\];/);
  assert.ok(glueSource.includes('"claudeAppAutoTakeover": enabled'), "set_enabled 要经 SettingsStore::update 写 claudeAppAutoTakeover");
  assert.ok(settingsSource.includes('merge_bool_setting(target, source, "claudeAppAutoTakeover");'), "SettingsStore::update 要认这个键");
});

test("payload fields stay aligned with the Rust struct", () => {
  assert.ok(glueSource.includes('#[serde(rename_all = "camelCase")]'));
  for (const field of ["platform_supported", "mode", "last_reason", "last_error", "app_running", "attached", "copy_running", "paused_instance", "running", "last_takeover_at_ms"]) {
    assert.ok(glueSource.includes(`pub ${field}:`), `ClaudeAutoTakeoverPayload 缺少 ${field}`);
  }
  const model = read("src/claude-auto-takeover-model.ts");
  for (const field of ["platformSupported", "mode", "lastReason", "lastError", "appRunning", "attached", "copyRunning", "pausedInstance", "running", "lastTakeoverAtMs"]) {
    assert.match(model, new RegExp(`\\b${field}\\??:`), `前端类型缺少 ${field}`);
  }
});

test("the row sits under the Claude row on home and in the full-mode launch settings", () => {
  const home = appDeclarationSource("HomeScreen");
  assert.match(home, /tool\.id === "claude"\s*\? \[row, <ClaudeAutoTakeoverRow key="claude-auto-takeover" loginReminder [\s\S]*?showIcon \/>\]/, "首页 Claude 行后面要紧跟自动接管行");
  assert.match(home, /onEnableLogin=\{\(\) => actions\.setWatcherEnabled\(true\)\}/, "没开登录自启时首页要能直接打开");
  const screen = appDeclarationSource("ClaudeLocalizationScreen");
  const launchSettings = screen.slice(screen.indexOf('title={t("启动设置")}'));
  assert.match(launchSettings, /\{claudeLaunch === "full" \? <ClaudeAutoTakeoverRow \/> : null\}/, "Desktop 增强页只在完整功能模式下显示");
  // 只在可见时轮询：组件挂着才起定时器，开关开着才轮询。
  assert.match(componentSource, /if \(!enabled\) return[\s\S]*?window\.setInterval\(load, POLL_MS\)/);
});

test("supervisor lifecycle hooks are wired like the Codex one", () => {
  assert.ok(libSource.includes("claude_auto_takeover::start_supervisor()"), "lib.rs setup 必须启动 Claude 守护线程");
  assert.ok(libSource.includes("claude_auto_takeover::stop_supervisor()"), "退出流程必须停掉 Claude 守护线程");
  assert.ok(commandsSource.includes("crate::claude_auto_takeover::apply_enabled(settings.claude_app_auto_takeover)"), "整份保存设置后也要立即生效");
  assert.ok(glueSource.includes("STARTUP_QUIET") && glueSource.includes("state.started_at = Some(Instant::now())"), "守护线程要有启动静默期");
  // 退出用户的 Claude 之前先体检，起不来就不退。
  assert.ok(glueSource.indexOf("core::check_launch_ready()") < glueSource.indexOf("quiesce(instance, &command_line)"));
});

test("claude auto-takeover is off by default on both sides", () => {
  assert.ok(settingsSource.includes("claude_app_auto_takeover: false,"), "Rust 默认值必须是 false");
  assert.match(settingsSource, /rename = "claudeAppAutoTakeover", default\)\]\s*pub claude_app_auto_takeover: bool,/, "必须带 serde default：老配置缺键按关闭");
  assert.match(appSource, /claudeAppAutoTakeover: false,/, "前端 defaultSettings 必须是 false");
});

test("every string in the component has an English entry", () => {
  const keys = new Set<string>();
  for (const match of componentSource.matchAll(/\bt\("([^"]+)"\)|\btf\("([^"]+)"/g)) keys.add(match[1] ?? match[2]);
  assert.ok(keys.size > 20);
  for (const key of keys) assert.ok(dictionary.includes(`"${key}":`), `i18n-en.ts 缺少「${key}」的英文条目`);
  // 后端的固定提示也要能翻（组件把命令失败的 message 和出错原文过一遍 t）。
  for (const source of [glueSource, read("../../crates/codex-plus-core/src/claude_auto_takeover.rs")]) {
    for (const match of source.matchAll(/(?:Err|return Err)\("([^"]*[一-龥][^"]*)"\.to_string\(\)\)/g)) {
      assert.ok(dictionary.includes(`"${match[1]}":`), `i18n-en.ts 缺少后端提示「${match[1]}」`);
    }
  }
});
