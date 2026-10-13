import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";

/**
 * 「自动接管官方 App」的接线契约。
 *
 * 这条功能最容易坏在两个静默失败上：
 * 1. Rust 命令没注册到 `generate_handler!` —— 前端 `invoke` 报 command not found，
 *    但没有任何编译期检查兜底（全仓只有一处 `generate_handler!`）。
 * 2. 新文案没进 i18n 词典 —— 英文界面回落中文，`tools/i18n-verify.mjs` 又不是门禁。
 *
 * 这里只做静态检查，不启动 Tauri、不碰真实进程。
 */

const root = path.resolve(import.meta.dirname, "..");
const appSource = readFileSync(path.join(root, "src/App.tsx"), "utf8");
const libSource = readFileSync(
  path.join(root, "src-tauri/src/lib.rs"),
  "utf8",
);
const commandsSource = readFileSync(
  path.join(root, "src-tauri/src/commands.rs"),
  "utf8",
);
const dictionary = readFileSync(path.join(root, "src/i18n-en.ts"), "utf8");

const REQUIRED_COMMANDS = [
  "load_auto_takeover_state",
  "take_over_codex_now",
  "pause_auto_takeover_for_instance",
];

test("auto-takeover commands are registered in the tauri handler", () => {
  for (const command of REQUIRED_COMMANDS) {
    assert.ok(
      commandsSource.includes(`pub fn ${command}(`),
      `commands.rs 缺少 #[tauri::command] ${command}`,
    );
    assert.ok(
      libSource.includes(`commands::${command},`),
      `lib.rs 的 generate_handler! 没有注册 ${command}`,
    );
  }
});

test("auto-takeover switch persists through save_settings, not a side channel", () => {
  // 开关必须走既有设置保存链路，才能在设置文件里持久化并被守护线程读到。
  assert.ok(
    appSource.includes("setAutoTakeover") &&
      appSource.includes("codexAppAutoTakeover: enabled"),
    "维护页开关必须通过 saveSettingsValue 写入 codexAppAutoTakeover",
  );
  assert.ok(
    appSource.includes('"load_auto_takeover_state"') &&
      appSource.includes('"take_over_codex_now"') &&
      appSource.includes('"pause_auto_takeover_for_instance"'),
    "App.tsx 必须调用三个自动接管命令",
  );
});

test("auto-takeover switch is visible in the maintenance screen", () => {
  // 放在维护页而不是增强页：EnhanceScreen 有「恰好 3 个 tabpanel」的契约。
  assert.ok(
    appSource.includes('t("自动接管官方 App")'),
    "维护页必须展示自动接管开关卡片",
  );
  assert.ok(
    appSource.includes("autoTakeover={autoTakeover}"),
    "MaintenanceScreen 必须拿到自动接管状态",
  );
});

const payloadSource = () =>
  readFileSync(path.join(root, "src-tauri/src/auto_takeover.rs"), "utf8");

test("auto-takeover status fields stay aligned with the Rust payload", () => {
  // 前端按 camelCase 读，Rust 侧靠 rename_all 转换——两者必须同时存在，
  // 少一个就会出现「状态行永远空白」这类静默失效。
  assert.ok(
    payloadSource().includes('#[serde(rename_all = "camelCase")]'),
    "AutoTakeoverPayload 必须序列化成 camelCase（前端按 camelCase 读）",
  );
  for (const field of [
    "platform_supported",
    "last_takeover_at_ms",
    "restarts_last_hour",
    "paused_instance",
    "session_status",
  ]) {
    assert.ok(
      payloadSource().includes(`pub ${field}:`),
      `AutoTakeoverPayload 缺少 ${field}（前端类型依赖它）`,
    );
  }
  for (const camelField of ["platformSupported", "lastTakeoverAtMs", "pausedInstance"]) {
    assert.ok(
      appSource.includes(camelField),
      `App.tsx 的 AutoTakeoverResult 缺少 ${camelField}`,
    );
  }
});

test("auto-takeover copy is translated in the English dictionary", () => {
  // 关掉自动接管现在是开关的反向操作，不再有「关闭自动接管」按钮文案。
  for (const key of [
    "自动接管官方 App",
    "开启自动接管",
    "立即接管",
    "本次不接管",
    "接管状态",
    "上次接管：{0}",
  ]) {
    assert.ok(
      dictionary.includes(`"${key}":`),
      `i18n-en.ts 缺少「${key}」的英文条目`,
    );
  }
});

test("daemon waits for the manager startup sequence before probing", () => {
  // 管理器启动时会先 resume 已有 Codex 的 helper/注入；守护线程若立刻开探，
  // 会看到「App 在跑 + 会话未就绪」而误判成需要接管（实测撞过一次）。
  assert.ok(
    payloadSource().includes("STARTUP_QUIET"),
    "守护线程必须有启动静默期，否则会和 resume 流程抢跑",
  );
  assert.ok(
    payloadSource().includes("state.started_at = Some(Instant::now())"),
    "启动静默期必须在 start_supervisor 里打点",
  );
});

test("supervisor lifecycle hooks survive refactors", () => {
  // 这两个钩子被并发编辑覆盖过一次：功能编译通过、命令也在，但守护线程从不启动，
  // 表现为「开关打开了却什么都不发生」。静态断言是最便宜的兜底。
  assert.ok(
    libSource.includes("auto_takeover::start_supervisor()"),
    "lib.rs setup 必须调用 start_supervisor()，否则开关打开也不会启动守护线程",
  );
  assert.ok(
    libSource.includes("auto_takeover::stop_supervisor()"),
    "退出流程必须调用 stop_supervisor()",
  );
  assert.ok(
    commandsSource.includes("crate::auto_takeover::apply_enabled("),
    "save_settings 成功后必须让开关立即生效（不必重启管理器）",
  );
});

test("auto-takeover is off by default and defaults to the defer policy", () => {
  // 产品要求：默认关闭。Rust 与前端两侧都要是 false，否则新用户会被自动重启官方 App。
  const settingsSource = readFileSync(
    path.join(root, "../../crates/codex-plus-core/src/settings.rs"),
    "utf8",
  );
  assert.ok(
    settingsSource.includes("codex_app_auto_takeover: false,"),
    "Rust 默认值必须是 false（默认关闭）",
  );
  assert.ok(
    settingsSource.includes("codex_app_auto_takeover: bool,") &&
      /rename = "codexAppAutoTakeover", default\)/.test(settingsSource),
    "codexAppAutoTakeover 必须带 serde default：老配置缺键时按关闭处理",
  );
  assert.ok(
    /codexAppAutoTakeover: false,/.test(appSource),
    "前端 defaultSettings 必须是 false",
  );
  // 策略默认必须是 defer：留空串会让「没设置」和「显式 defer」无法区分。
  assert.ok(
    settingsSource.includes("default_auto_takeover_when_active"),
    "策略默认值必须是显式的 defer（default_auto_takeover_when_active）",
  );
  assert.ok(
    /codexAppAutoTakeoverWhenActive: "defer",/.test(appSource),
    "前端策略默认值必须是 defer",
  );
});
