// Claude Desktop 自动接管的纯逻辑：状态快照怎么折算成标签 / 说明 / 能不能「立即接管」。
// 不依赖 i18n / React，便于 node --test 直接测；文案由 ClaudeAutoTakeover.tsx 翻译。

/** 与 Rust 侧 `claude_auto_takeover::ClaudeAutoTakeoverPayload`（rename_all = "camelCase"）逐字段对齐。 */
export type ClaudeAutoTakeoverPayload = {
  /** disabled | probing | taking_over | degraded | paused */
  state: string;
  enabled: boolean;
  platformSupported: boolean;
  /** Claude 的启动模式：full | 3p | copy，只有 full 才接管。 */
  mode: string;
  lastTakeoverAtMs?: number | null;
  lastReason?: string | null;
  lastError?: string | null;
  appRunning?: boolean | null;
  attached: boolean;
  copyRunning: boolean;
  restartsLastHour: number;
  restartsTotal: number;
  pausedInstance?: string | null;
  running: boolean;
};

export type ClaudeAutoTakeoverResult = { status: string; message: string } & ClaudeAutoTakeoverPayload;

/** 四个命令都要在 lib.rs 的 generate_handler! 里注册（契约测试守着）。 */
export const CLAUDE_AUTO_TAKEOVER_COMMANDS = {
  load: "load_claude_auto_takeover_state",
  set: "set_claude_auto_takeover",
  takeOverNow: "take_over_claude_now",
  pause: "pause_claude_auto_takeover_for_instance",
} as const;

export type ClaudeTakeoverLabel = "off" | "inactive" | "listening" | "taking_over" | "degraded" | "paused";

export type ClaudeTakeoverSummary = {
  /** StatusBadge / 图标的色调。 */
  tone: "ok" | "warning" | "failed" | "off";
  label: ClaudeTakeoverLabel;
  /** 说明用的原因码（Rust 判据的英文码，组件翻成人话）；没有就是 null。 */
  reason: string | null;
  /** 后端给的出错原文（体检、退出、启动失败），优先于原因码显示。 */
  error: string | null;
  /** 现在点「立即接管」有没有意义（官方 Claude 在跑、还没接上、没有副本在跑）。 */
  canTakeOver: boolean;
};

/** 不是英文原因码的 lastError 是后端的中文原文，原样显示。 */
const REASON_CODE = /^[a-z][a-z0-9_]*$/;

export function summarizeClaudeAutoTakeover(state: ClaudeAutoTakeoverPayload | null): ClaudeTakeoverSummary {
  const none: ClaudeTakeoverSummary = { tone: "off", label: "off", reason: null, error: null, canTakeOver: false };
  if (!state) return none;
  const modeReason = state.mode === "3p" ? "mode_3p" : state.mode === "copy" ? "mode_copy" : null;
  if (!state.enabled) return { ...none, reason: modeReason };
  if (!state.platformSupported) return { ...none, reason: "platform_unsupported" };
  // 开着但启动模式不是完整功能模式：不接管，说清楚为什么。
  if (modeReason) return { ...none, label: "inactive", reason: modeReason };
  const lastError = state.lastError?.trim() || null;
  const errorIsCode = Boolean(lastError && REASON_CODE.test(lastError));
  const canTakeOver = Boolean(state.appRunning) && !state.attached && !state.copyRunning && state.state !== "taking_over";
  if (state.state === "degraded") {
    return {
      tone: "failed",
      label: "degraded",
      reason: errorIsCode ? lastError : state.lastReason ?? null,
      error: errorIsCode ? null : lastError,
      canTakeOver,
    };
  }
  if (state.state === "taking_over") return { tone: "warning", label: "taking_over", reason: "takeover", error: null, canTakeOver: false };
  if (state.state === "paused" || state.pausedInstance) {
    return { tone: "warning", label: "paused", reason: "paused_for_instance", error: null, canTakeOver };
  }
  if (!state.running) return { ...none, reason: state.lastReason ?? null };
  return { tone: "ok", label: "listening", reason: state.lastReason ?? null, error: null, canTakeOver };
}
