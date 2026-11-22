import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Plug } from "lucide-react";
import { CxBadge, CxRow, CxStatusIcon } from "./cx-layout";
import { Menu, MenuItem } from "./ManagementPages";
import { t, tf } from "./i18n";
import { announceBackendSettingsChanged } from "./settings-events";
import { enhancementPlatformFromUserAgent } from "./enhancement-catalog";
import {
  CLAUDE_AUTO_TAKEOVER_COMMANDS,
  summarizeClaudeAutoTakeover,
  type ClaudeAutoTakeoverResult,
  type ClaudeTakeoverSummary,
} from "./claude-auto-takeover-model";

/** 和 Codex 的接管状态一样，开着时每 3 秒读一次（只在这一行挂着、也就是页面可见时）。 */
const POLL_MS = 3000;

function ToggleVisual() {
  return (
    <span aria-hidden="true" className="toggle-switch-visual">
      <span className="toggle-switch-thumb" />
    </span>
  );
}

/** Rust 判据的原因码翻成人话（`claude_auto_takeover::decide` 与共用的 `auto_takeover::decide`）。 */
function reasonText(code: string | null): string {
  if (!code) return "";
  const reasons: Record<string, string> = {
    mode_3p: t("Claude 的启动模式是 3p 模式，只在完整功能模式下自动接管。"),
    mode_copy: t("Claude 的启动模式是补丁副本，只在完整功能模式下自动接管。"),
    platform_unsupported: t("当前系统不支持自动接管。"),
    already_attached: t("Claude 已经经完整功能模式运行。"),
    app_not_running: t("官方 Claude 未运行。"),
    enhanced_copy_running: t("补丁副本正在运行，先不接管。"),
    multi_instance: t("有别的 Claude 实例在运行（多开或带参数启动的）；接管要先退出 Claude，会把它一起关掉，所以先不接管。"),
    user_active: t("Claude 在打开开关或 Codex++ 启动前就已开着，可能正在使用，不自动退出；可以点「立即接管」。"),
    probe_unknown: t("无法确认 Claude 的状态，已保守跳过。"),
    backoff: t("上次接管没有成功，稍后自动重试。"),
    restart_budget_exhausted: t("一小时内接管次数已达上限，已停止自动重试。"),
    paused_for_instance: t("本次已跳过自动接管，重开 Claude 后恢复。"),
    takeover: t("正在接管：退出官方 Claude 后经完整功能模式重开。"),
    claude_not_installed: t("未检测到官方 Claude Desktop。"),
    node_missing: t("未找到可用的 Node.js（需要 22 以上），完整功能模式起不来，没有退出 Claude。"),
    ca_untrusted: t("本地证书还没有信任，完整功能模式起不来，没有退出 Claude。"),
    upstream_not_ready: t("还没有可用的上游模型，完整功能模式起不来，没有退出 Claude。"),
    quit_failed: t("官方 Claude 没有完全退出，稍后自动重试。"),
    launch_failed: t("完整功能模式没有启动成功，已把官方 Claude 开回去。"),
  };
  return reasons[code] ?? "";
}

function labelText(summary: ClaudeTakeoverSummary): string {
  switch (summary.label) {
    case "listening": return t("监听中");
    case "taking_over": return t("接管中");
    case "degraded": return t("已降级");
    case "paused": return t("本次跳过");
    case "inactive": return t("未生效");
    default: return t("未运行");
  }
}

function detailText(state: ClaudeAutoTakeoverResult, summary: ClaudeTakeoverSummary): string {
  const reason = reasonText(summary.reason);
  // 后端给的出错原文（中文，英文界面按 EN_BACKEND 翻）接在原因后面。
  if (summary.error) return [reason, t(summary.error)].filter(Boolean).join(" ");
  if (reason) return reason;
  return [
    `${t("官方 Claude")} ${state.appRunning ? t("运行中") : t("未运行")}`,
    state.lastTakeoverAtMs ? tf("上次接管：{0}", [new Date(state.lastTakeoverAtMs).toLocaleString()]) : "",
  ].filter(Boolean).join(" · ");
}

const toneOf = { ok: "ok", warning: "warn", failed: "danger", off: "neutral" } as const;
const iconToneOf = { ok: "ok", warning: "warn", failed: "danger", off: "off" } as const;

/**
 * Claude 的「自动接管官方 App」一行：首页 Claude 那一行下面、Desktop 增强页「启动设置」里各放一份。
 * 只在启动模式是完整功能模式时动作；开关单独写 settings.json，写完广播设置已变，App 只合并这个键。
 * `watcherEnabled`（首页传）：Codex++ 没设登录时打开就提醒——关掉 Codex++ 就没人监听。
 */
export function ClaudeAutoTakeoverRow({
  showIcon = false,
  loginReminder = false,
  onEnableLogin,
}: {
  showIcon?: boolean;
  /** 首页：没开登录自启时提醒并给开关。 */
  loginReminder?: boolean;
  onEnableLogin?: () => Promise<unknown> | void;
}) {
  const [state, setState] = useState<ClaudeAutoTakeoverResult | null>(null);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState("");
  const [loginEnabled, setLoginEnabled] = useState<boolean | null>(null);
  const enabled = state?.enabled === true;

  useEffect(() => {
    let cancelled = false;
    const load = () => void invoke<ClaudeAutoTakeoverResult>(CLAUDE_AUTO_TAKEOVER_COMMANDS.load)
      .then((result) => { if (!cancelled) setState(result); })
      .catch(() => {});
    load();
    if (!enabled) return () => { cancelled = true; };
    const timer = window.setInterval(load, POLL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [enabled]);

  useEffect(() => {
    if (!loginReminder || !enabled) return;
    let cancelled = false;
    void invoke<{ enabled: boolean }>("load_watcher_state")
      .then((result) => { if (!cancelled) setLoginEnabled(Boolean(result.enabled)); })
      .catch(() => {});
    return () => { cancelled = true; };
  }, [loginReminder, enabled]);

  // 命令失败的提示顶替说明显示一会儿，之后回到轮询到的状态。
  useEffect(() => {
    if (!notice) return;
    const timer = window.setTimeout(() => setNotice(""), 8000);
    return () => window.clearTimeout(timer);
  }, [notice]);

  const run = async (command: string, args?: Record<string, unknown>) => {
    setBusy(true);
    try {
      const result = await invoke<ClaudeAutoTakeoverResult>(command, args);
      setState(result);
      setNotice(result.status === "ok" ? "" : result.message);
      return result;
    } catch (error) {
      setNotice(String(error));
      return null;
    } finally {
      setBusy(false);
    }
  };

  const toggle = async (next: boolean) => {
    const result = await run(CLAUDE_AUTO_TAKEOVER_COMMANDS.set, { enabled: next });
    if (result?.status === "ok") announceBackendSettingsChanged();
  };

  const enableLogin = async () => {
    await onEnableLogin?.();
    const result = await invoke<{ enabled: boolean }>("load_watcher_state").catch(() => null);
    if (result) setLoginEnabled(Boolean(result.enabled));
  };

  const summary = summarizeClaudeAutoTakeover(state);
  const needsLogin = loginReminder && enabled && loginEnabled === false;
  const platform = enhancementPlatformFromUserAgent(navigator.userAgent);
  const offDetail = platform === "windows"
    ? t("仅在启动模式为完整功能模式时生效：打开后，你自己打开官方 Claude（开始菜单、任务栏）也会先退出再经完整功能模式重开，汉化、听写、页面扩展照常生效。")
    : t("仅在启动模式为完整功能模式时生效：打开后，你自己打开官方 Claude（Dock、启动台）也会先退出再经完整功能模式重开，汉化、听写、页面扩展照常生效。");
  const detail = (notice && t(notice)) || (enabled && state
    ? [detailText(state, summary), needsLogin ? t("Codex++ 没有设为登录时打开，退出 Codex++ 后不会接管。") : ""].filter(Boolean).join(" ")
    : offDetail);

  return (
    <CxRow
      badges={enabled ? <CxBadge tone={toneOf[summary.tone]}>{labelText(summary)}</CxBadge> : null}
      detail={detail}
      icon={showIcon ? <CxStatusIcon tone={enabled ? iconToneOf[summary.tone] : "off"}><Plug /></CxStatusIcon> : undefined}
      title={t("自动接管官方 App")}
      tooltip={t("只在完整功能模式下生效：官方 Claude 不是经本地服务启动的，会先退出再重开（可能中断正在进行的对话）。3p 模式和补丁副本不接管。")}
    >
      {needsLogin ? (
        <button className="cx-secondary" onClick={() => void enableLogin()} type="button">{t("登录时打开 Codex++")}</button>
      ) : null}
      {enabled && summary.canTakeOver ? (
        <button className="cx-secondary" disabled={busy} onClick={() => void run(CLAUDE_AUTO_TAKEOVER_COMMANDS.takeOverNow)} type="button">{t("立即接管")}</button>
      ) : null}
      {enabled ? (
        <Menu>
          <MenuItem disabled={busy || !state?.appRunning || state.attached} onClick={() => void run(CLAUDE_AUTO_TAKEOVER_COMMANDS.pause)}>{t("本次不接管")}</MenuItem>
          <MenuItem disabled={busy} onClick={() => void run(CLAUDE_AUTO_TAKEOVER_COMMANDS.load)}>{t("刷新状态")}</MenuItem>
        </Menu>
      ) : null}
      <label className="switch-row compact relay-master-switch relay-master-switch-compact">
        <input
          aria-label={t("开启 Claude 自动接管")}
          checked={enabled}
          disabled={!state || busy}
          onChange={(event) => void toggle(event.currentTarget.checked)}
          type="checkbox"
        />
        <span>{t("自动接管")}</span>
        <ToggleVisual />
      </label>
    </CxRow>
  );
}
