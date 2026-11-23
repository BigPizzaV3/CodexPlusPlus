// Claude 桌面版页面：完整功能模式与汉化补丁（route claude / claudePatch）。
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { PackageOpen, RefreshCw, Rocket, ShieldCheck, Square } from "lucide-react";
import { createPortal } from "react-dom";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { AppSelect } from "./AppSelect";
import { onTabListKeyDown } from "./tablist-keys";
import { formatTokens } from "./gateway-model";
import { announceBackendSettingsChanged } from "./settings-events";
import { Menu, MenuItem } from "./ManagementPages";
import { t, tf } from "@/i18n";
import * as AgentCx from "./cx-layout"; // cx-pages:agents：Agent 页面在函数内解构，避免和别的分组的具名导入重名
import type {
  ClaudeBashTimeoutSettings,
  ClaudeLocalConfig,
  ClaudeLocalizationStatus,
  ClaudeLocalStatus,
  ClaudeTemplateVersion,
  CommandResult,
  Status,
} from "./app-types";
import { clampNumber, claudeLocalizationMemo, claudeLocalMemo, stringifyError, ToggleVisual } from "./app-shared";
import type { Route } from "./routes";
import { ClaudeAutoTakeoverRow } from "./ClaudeAutoTakeover";
import { patchClaudeEnhance, saveClaudeBashTimeout, setClaudeDelivery, useClaudeEnhance, type ClaudeDelivery } from "./claude-enhance";

/** Desktop 增强页的设置页签。 */
type ClaudeSettingsTab = "providers" | "account" | "features" | "environment";

/** Claude 在供应商没有声明窗口时自己用的默认上下文窗口。 */
const CLAUDE_DEFAULT_CONTEXT_WINDOW = 200_000;

/**
 * 上下文窗口展示：1000000 -> `1M`、1100000 -> `1.1M`、200000 -> `200k`。
 * 复用网关用量的 `formatTokens`（B/M/K），只把这里的 K 换成小写，和模型列表里的 `[200k]` 写法一致。
 */
function formatContextWindow(value: number | null | undefined): string {
  if (value == null || !Number.isFinite(value) || value <= 0) return "";
  return formatTokens(value).replace(/K$/, "k");
}

/** 录制版本的显示：前端构建日期 + build id，例如「2026-11-14 · a4ced7f85b」；没有构建时间时只显示 build id。 */
function claudeTemplateLabel(template: ClaudeTemplateVersion): string {
  if (!template.builtAt) return template.build;
  const date = new Date(template.builtAt * 1000);
  const day = `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
  return `${day} · ${template.build}`;
}

/**
 * Claude Desktop 页。
 *
 * - `mode="local"`（默认）：完整功能模式——页头的原版启动、本地状态、汉化/听写/电脑控制/后台模式；
 *   增强副本那一块搬到「补丁 Desktop」页，这一页不再提它，也不放跳转。
 * - `mode="patch"`：「补丁 Desktop」页——只渲染改过代码的 Desktop 副本（制作/更新/检查/启动、副本开关、
 *   Bash 时限）；不渲染本地状态那部分，副本块常开。
 */
export function ClaudeLocalizationScreen(
  { mode = "local", onLocalStatus, onNavigate, notify }: {
    mode?: "local" | "patch";
    /** 供应商那一块的操作结果提示（App 的 showMessage）；不传就不显示供应商那一块。 */
    notify?: (title: string, message: string, status?: Status) => Promise<void>;
    /** 每次读到新的原版本地状态（检测、启动、停止、配置）时交给 App，侧栏的播放键要跟着变。 */
    onLocalStatus?: (status: CommandResult<ClaudeLocalStatus>) => void;
    /** 两种生效方式之间互相跳转（Desktop 增强 ↔ 补丁 Desktop）。 */
    onNavigate?: (route: Route) => void;
  } = {},
) {
  const patchOnly = mode === "patch";
  const enhance = useClaudeEnhance();
  const delivery = enhance?.delivery ?? null;
  const [deliveryBusy, setDeliveryBusy] = useState(false);
  const chooseDelivery = async (next: ClaudeDelivery) => {
    if (deliveryBusy || next === delivery) return true;
    setDeliveryBusy(true);
    try { await setClaudeDelivery(next); return true; }
    catch (cause) { if (mounted.current) setLocalError(stringifyError(cause)); return false; }
    finally { if (mounted.current) setDeliveryBusy(false); }
  };
  const [status, setStatus] = useState<CommandResult<ClaudeLocalizationStatus> | null>(claudeLocalizationMemo.status);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [settingsTab, setSettingsTab] = useState<ClaudeSettingsTab>("providers");
  /** 补丁页里副本块恒开；启动模式选了补丁副本时，Desktop 增强页直接把副本块搬进来；其余情况不轮询副本状态，避免白跑检测。 */
  const copyInline = !patchOnly && delivery === "copy";
  const advancedShown = patchOnly || advancedOpen || copyInline;
  const [localStatus, setLocalStatus] = useState<CommandResult<ClaudeLocalStatus> | null>(claudeLocalMemo.status);
  const [localBusy, setLocalBusy] = useState<string | null>(claudeLocalMemo.autoDetected ? null : "detect");
  const [localError, setLocalError] = useState<string | null>(null);
  const localPending = useRef(false);
  const localGeneration = useRef(0);
  /** 制作增强版时宿主推送的阶段进度（0-100）。 */
  const [progress, setProgress] = useState<{ percent: number; message: string } | null>(null);
  const mounted = useRef(true);
  const pending = useRef(false);
  const statusGeneration = useRef(0);
  const [bashTimeoutDraft, setBashTimeoutDraft] = useState<ClaudeBashTimeoutSettings | null>(
    claudeLocalizationMemo.status?.status === "ok"
      ? claudeLocalizationMemo.status.bashTimeout
      : null,
  );
  // 编辑时保留空字符串，避免清空输入框立刻补 1 后把新数字拼在后面。
  const [bashTimeoutDefaultText, setBashTimeoutDefaultText] = useState<string | null>(null);
  const [bashTimeoutMaxText, setBashTimeoutMaxText] = useState<string | null>(null);
  const bashTimeoutSaving = useRef(false);
  const bashTimeoutQueued = useRef<ClaudeBashTimeoutSettings | null>(null);
  /**
   * 页头右上角的操作槽：原版启动放在和 Codex 页「启动 Codex」相同的位置。
   * 页头与本页都按路由重建、同一次提交挂载，所以在 layout effect 里取，首帧绘制前就到位。
   */
  const [topbarSlot, setTopbarSlot] = useState<HTMLElement | null>(null);
  useLayoutEffect(() => {
    setTopbarSlot(document.querySelector<HTMLElement>("[data-topbar-actions]"));
  }, []);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<{ percent: number; message: string }>("claude-localization-progress", (event) => {
      if (mounted.current) setProgress(event.payload);
    }).then((fn) => { unlisten = fn; });
    return () => { unlisten?.(); };
  }, []);
  const accept = (result: CommandResult<ClaudeLocalizationStatus>) => {
    // 先记下有效结果再判断挂载：检测中切走页面，回来时仍能看到这次的结果。
    if (result.status === "ok") claudeLocalizationMemo.status = result;
    if (!mounted.current) return;
    if (result.status === "ok") {
      setStatus(result);
      setBashTimeoutDraft(result.bashTimeout);
      setBashTimeoutDefaultText(null);
      setBashTimeoutMaxText(null);
    }
    else setError(result.message);
  };
  const acceptLocal = (result: CommandResult<ClaudeLocalStatus>) => {
    if (result.status === "ok") {
      claudeLocalMemo.status = result;
      // 通知 App：侧栏 Claude 行和这里共用一份状态（回调是普通函数，别塞进 effect 依赖）。
      onLocalStatus?.(result);
    }
    if (!mounted.current) return;
    if (result.status === "ok") setLocalStatus(result);
    else setLocalError(result.message);
  };
  const refreshLocal = async () => {
    if (localPending.current) return;
    localPending.current = true;
    localGeneration.current++;
    setLocalBusy("detect"); setLocalError(null);
    try { acceptLocal(await invoke<CommandResult<ClaudeLocalStatus>>("claude_local_status")); }
    catch (cause) { if (mounted.current) setLocalError(stringifyError(cause)); }
    finally {
      localPending.current = false;
      if (mounted.current) setLocalBusy(null);
    }
  };
  useEffect(() => {
    if (claudeLocalMemo.autoDetected) return;
    claudeLocalMemo.autoDetected = true;
    void refreshLocal();
  }, []);
  const localAction = async (action: "launch" | "stop" | "trust" | "untrust" | "trust_machine" | "prepare" | "sync_sessions") => {
    if (localPending.current || pending.current) return;
    localPending.current = true;
    localGeneration.current++;
    setLocalBusy(action); setLocalError(null);
    try {
      acceptLocal(await invoke<CommandResult<ClaudeLocalStatus>>("claude_local_action", { action }));
    } catch (cause) { if (mounted.current) setLocalError(stringifyError(cause)); }
    finally {
      localPending.current = false;
      if (mounted.current) setLocalBusy(null);
    }
  };
  const configureLocal = async (config: ClaudeLocalConfig) => {
    if (localPending.current || pending.current) return;
    localPending.current = true;
    localGeneration.current++;
    setLocalBusy("configure"); setLocalError(null);
    try {
      const result = await invoke<CommandResult<ClaudeLocalStatus>>("claude_local_configure", { config });
      acceptLocal(result);
      if (result.status === "ok") announceBackendSettingsChanged();
    } catch (cause) { if (mounted.current) setLocalError(stringifyError(cause)); }
    finally {
      localPending.current = false;
      if (mounted.current) setLocalBusy(null);
    }
  };
  useEffect(() => {
    if ((!localStatus?.running && !localStatus?.claudeRunning) || localBusy !== null) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    const generation = localGeneration.current;
    const poll = async () => {
      try {
        const result = await invoke<CommandResult<ClaudeLocalStatus>>("claude_local_status");
        if (cancelled || !mounted.current || localPending.current || generation !== localGeneration.current) return;
        if (result.status === "ok") acceptLocal(result);
      } catch { /* 状态探测失败保留上次结果，手动检测仍可显示具体错误。 */ }
      if (!cancelled && mounted.current && generation === localGeneration.current) {
        timer = setTimeout(() => void poll(), 3000);
      }
    };
    timer = setTimeout(() => void poll(), 3000);
    return () => { cancelled = true; clearTimeout(timer); };
  }, [localStatus?.running, localStatus?.claudeRunning, localBusy]);
  const refresh = async () => {
    if (pending.current) return;
    pending.current = true;
    statusGeneration.current++;
    setBusy("detect"); setError(null);
    try { accept(await invoke<CommandResult<ClaudeLocalizationStatus>>("claude_localization_status")); }
    catch (cause) { if (mounted.current) setError(stringifyError(cause)); }
    finally {
      pending.current = false;
      if (mounted.current) setBusy(null);
    }
  };
  useEffect(() => {
    if (!advancedShown || claudeLocalizationMemo.autoDetected) return;
    claudeLocalizationMemo.autoDetected = true;
    void refresh();
  }, [advancedShown]);
  /**
   * Claude Desktop 是否已在 Computer Use 页接入 codex-plus-cu（含自定义目录）。
   * 和这里的电脑控制同时开会让同一个对话里出现两套工具，用来提示二选一；读不到时按未接入处理。
   */
  const [claudeDesktopCuConnected, setClaudeDesktopCuConnected] = useState(false);
  useEffect(() => {
    const connected = (state?: string) => state === "registered" || state === "needs_update";
    void invoke<{ targets?: Record<string, { state?: string }>; customTargets?: Array<{ kind?: string; state?: string }> }>("computer_use_status")
      .then((result) => {
        if (!mounted.current) return;
        setClaudeDesktopCuConnected(connected(result.targets?.claudeDesktop?.state)
          || Boolean(result.customTargets?.some((target) => target.kind === "claudeDesktop" && connected(target.state))));
      })
      .catch(() => undefined);
  }, []);
  const act = async (action: "prepare" | "launch" | "restore" | "check") => {
    if (pending.current || localPending.current) return false;
    pending.current = true;
    statusGeneration.current++;
    setBusy(action); setError(null);
    const launching = action === "launch" || action === "restore";
    const clearLaunchNotice = () => {
      setStatus((current) => current?.status === "ok"
        ? { ...current, bashTimeoutWarning: null, bashTimeoutPending: false } : current);
    };
    if (launching) {
      const timeout = bashTimeoutDraft ?? (status?.status === "ok" ? status.bashTimeout : null);
      const awaitingConfirmation = Boolean(timeout?.enabled && (timeout.defaultEnabled || timeout.maxEnabled));
      setStatus((current) => current?.status === "ok" ? {
        ...current,
        bashTimeoutWarning: awaitingConfirmation ? "已准备，暂未确认。将在补丁版启动后自动确认。" : null,
        bashTimeoutPending: awaitingConfirmation,
      } : current);
    }
    if (action === "prepare") setProgress({ percent: 0, message: t("准备开始…") });
    let succeeded = false;
    try {
      const result = await invoke<CommandResult<ClaudeLocalizationStatus>>("claude_localization_action", { action });
      // 失败响应里的 data 是空对象：直接覆盖状态会让下面的渲染读到不存在的字段
      // （曾经就是这样整页白屏的），所以只有成功时才更新状态。
      if (launching && result.status !== "ok") clearLaunchNotice();
      accept(result);
      succeeded = result.status === "ok";
    } catch (cause) { if (mounted.current) { if (launching) clearLaunchNotice(); setError(stringifyError(cause)); } }
    finally {
      pending.current = false;
      if (mounted.current) setBusy(null);
      if (action === "prepare") setTimeout(() => { if (mounted.current) setProgress(null); }, 1500);
    }
    return succeeded;
  };
  /// 补丁版的三个开关（汉化、语音输入、Computer Use）共用：一次只跑一个，成功时才更新状态。
  const setClaudeFlag = async (
    key: "localization" | "voiceInput" | "computerUse",
    command: "claude_localization_set_localization" | "claude_localization_set_voice_input" | "claude_localization_set_computer_use",
    enabled: boolean,
  ) => {
    if (pending.current || localPending.current) return;
    pending.current = true;
    statusGeneration.current++;
    setBusy(key); setError(null);
    try {
      const result = await invoke<CommandResult<ClaudeLocalizationStatus>>(command, { enabled });
      // 失败响应里的 data 是空对象：直接覆盖状态会让下面的渲染读到不存在的字段
      // （曾经就是这样整页白屏的），所以只有成功时才更新状态。
      accept(result);
    } catch (cause) { if (mounted.current) setError(stringifyError(cause)); }
    finally {
      pending.current = false;
      if (mounted.current) setBusy(null);
    }
  };
  const toggleVoiceInput = (enabled: boolean) => setClaudeFlag("voiceInput", "claude_localization_set_voice_input", enabled);
  const toggleComputerUse = (enabled: boolean) => setClaudeFlag("computerUse", "claude_localization_set_computer_use", enabled);
  const saveBashTimeout = async (next: ClaudeBashTimeoutSettings) => {
    if (localPending.current || (pending.current && !bashTimeoutSaving.current)) return;
    bashTimeoutQueued.current = next;
    setBashTimeoutDraft(next);
    if (bashTimeoutSaving.current) return;
    let confirmed = status?.status === "ok" ? status.bashTimeout : null;
    pending.current = true;
    statusGeneration.current++;
    bashTimeoutSaving.current = true;
    setBusy("bashTimeout"); setError(null);
    try {
      // 失焦保存先于开关点击；串行处理后续草稿，避免吞掉这次点击。
      while (bashTimeoutQueued.current) {
        const timeout = bashTimeoutQueued.current;
        bashTimeoutQueued.current = null;
        const result = await invoke<CommandResult<ClaudeLocalizationStatus>>(
          "claude_localization_set_bash_timeout",
          { timeout },
        );
        if (result.status !== "ok") throw new Error(result.message);
        confirmed = result.bashTimeout;
        patchClaudeEnhance({ bashTimeout: result.bashTimeout });
        claudeLocalizationMemo.status = result;
        if (mounted.current) setStatus(result);
        if (!bashTimeoutQueued.current) accept(result);
      }
    } catch (cause) {
      bashTimeoutQueued.current = null;
      if (mounted.current) {
        setBashTimeoutDraft(confirmed);
        setError(stringifyError(cause));
      }
    }
    finally {
      bashTimeoutSaving.current = false;
      pending.current = false;
      if (mounted.current) setBusy(null);
    }
  };
  const toggleLocalization = (enabled: boolean) => setClaudeFlag("localization", "claude_localization_set_localization", enabled);
  const s = status;
  const bashTimeout = bashTimeoutDraft ?? s?.bashTimeout;
  useEffect(() => {
    if (!advancedShown || !s?.bashTimeoutPending || !s.running || busy !== null) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    const generation = statusGeneration.current;
    const poll = async () => {
      try {
        const result = await invoke<CommandResult<{ warning: string | null; pending: boolean }>>(
          "claude_localization_bash_timeout_status",
        );
        if (cancelled || !mounted.current || pending.current || statusGeneration.current !== generation) return;
        if (result.status === "ok") {
          // 只同步提示，不覆盖正在编辑的秒数或其他设置草稿。
          const update = { bashTimeoutWarning: result.warning, bashTimeoutPending: result.pending };
          setStatus((current) => current?.status === "ok" ? { ...current, ...update } : current);
          if (claudeLocalizationMemo.status?.status === "ok") {
            claudeLocalizationMemo.status = { ...claudeLocalizationMemo.status, ...update };
          }
          if (!result.pending) return;
        }
      } catch { /* 临时读取失败保留提示，下一轮继续确认。 */ }
      if (!cancelled && mounted.current && statusGeneration.current === generation) {
        timer = setTimeout(() => void poll(), 1000);
      }
    };
    timer = setTimeout(() => void poll(), 1000);
    return () => { cancelled = true; clearTimeout(timer); };
  }, [advancedShown, s?.bashTimeoutPending, s?.running, busy]);
  // 只有明确点击可选的增强版启动，才允许制作副本。
  const needsPrepare = Boolean(s && (!s.prepared || s.needsRebuild));
  const canLaunch = Boolean(s?.supported && s.nodeAvailable && (s.prepared || needsPrepare));
  const launchEnhanced = async () => {
    if (needsPrepare && !(await act("prepare"))) return;
    await act("launch");
  };
  const { CxBadge, CxBanner, CxCard, CxRow, CxSection, CxStatusIcon } = AgentCx;
  const statusTone = !s ? "pending" : s.prepared ? (s.needsRebuild ? "warn" : "ok") : "idle";
  const statusText = busy === "detect" ? t("检测中…") : !s ? t("未检测") : !s.supported ? t("未检测到 Claude") : s.prepared ? (s.needsRebuild ? t("需要更新") : t("已准备")) : t("未准备");
  const meta = [
    tf("官方版本 {0}", [!s ? t("未检测") : s.sourceVersion || t("未检测到")]),
    tf("副本版本 {0}", [s?.copyVersion || t("未准备")]),
    tf("外壳语言 {0}", [s?.shellLocale || t("未设置")]),
    s?.catalogEntries != null ? tf("官方词条 {0} 条", [s.catalogEntries.toLocaleString()]) : tf("官方词条 {0}", [t("未检测")]),
  ].join(" · ");
  const prepareLabel = busy === "prepare"
    ? t("制作中…")
    : s?.needsRebuild
      ? t("更新副本")
      : s?.prepared
        ? t("重新制作补丁版")
        : t("制作补丁版");
  const prepareHint = t("首次制作会创建独立副本并执行完整性校验；官方应用、登录态和 app.asar 不会被改写。macOS 可能需要钥匙串授权，Windows 会使用独立副本启动。");
  const local = localStatus;
  const claudeLaunch: "3p" | "full" | "copy" = delivery === "copy" ? "copy" : local?.config.enabled ? "full" : "3p";
  const chooseClaudeLaunch = async (next: string) => {
    if (next === "copy") { await chooseDelivery("copy"); return; }
    // 从补丁副本换回原版启动：先把生效方式改回完整功能模式（3p 不注入，生效方式的值不影响它）。
    if (delivery === "copy" && !(await chooseDelivery("full"))) return;
    if (local && local.config.enabled !== (next === "full")) await configureLocal({ ...local.config, enabled: next === "full" });
  };
  const localState = !local ? t("未检测")
    : local.connected ? t("本地模式已连接")
      : local.claudeRunning ? (local.config.enabled ? t("原版运行，未连接本地服务") : t("原版已运行"))
        : local.running ? t("本地服务已就绪") : t("未启动");
  const localTone = local?.connected ? "ok" : local?.config.enabled && local?.claudeRunning ? "warn" : "neutral";
  const canLaunchOriginal = Boolean(local?.supported && (!local.config.enabled || local.nodeAvailable));
  const localDisabled = localBusy !== null || busy !== null;
  const localTemplates = local?.templates ?? [];
  const localDetail = local
    ? (local.supported ? tf("原版版本 {0}", [local.version || t("未检测到")]) : t("未检测到支持的 Claude Desktop"))
      + ` · ${local.contextWindow != null && local.contextWindow > 0
        ? tf("上下文窗口 {0}（供应商声明）", [formatContextWindow(local.contextWindow)])
        : t("上下文窗口 未声明（Claude 按 200k）")}`
    : t("正在读取原版 Claude 状态");
  const defaultModelWindow = local?.contextWindow != null && local.contextWindow > 0
    ? formatContextWindow(local.contextWindow)
    : formatContextWindow(CLAUDE_DEFAULT_CONTEXT_WINDOW);
  // 设置页签：补丁副本没有这些设置；证书与环境、功能只在完整功能模式下有。证书没信任或缺 Node 时那个页签带个点。
  const fullLocal = claudeLaunch === "full" && Boolean(local);
  const settingsTabs: Array<{ id: ClaudeSettingsTab; label: string; attention?: boolean }> = claudeLaunch === "copy" ? [] : [
    { id: "providers", label: t("模型") },
    { id: "account", label: t("账号与会话") },
    ...(fullLocal ? [
      { id: "features" as const, label: t("功能") },
      { id: "environment" as const, label: t("证书与环境"), attention: Boolean(local && (!local.caTrusted || !local.nodeAvailable)) },
    ] : []),
  ];
  const activeTab = settingsTabs.some((tab) => tab.id === settingsTab) ? settingsTab : settingsTabs[0]?.id ?? "providers";
  const settingsPanel = (id: ClaudeSettingsTab) => ({
    "aria-labelledby": `claude-local-tab-${id}`,
    className: "claude-local-panel",
    hidden: activeTab !== id,
    id: `claude-local-panel-${id}`,
    role: "tabpanel",
  });
  return (
    <div className="claude-localization-page cx-page">
      {!patchOnly && topbarSlot
        ? createPortal(
            claudeLaunch === "copy" ? (
              <Button
                disabled={busy !== null || localBusy !== null || !canLaunch}
                onClick={() => void launchEnhanced()}
                title={needsPrepare ? t("制作副本并启动补丁版") : t("启动补丁版")}
              >
                <Rocket className="h-4 w-4" />
                {busy === "launch" ? t("启动中…") : busy === "prepare" ? t("制作中…") : t("启动补丁版")}
              </Button>
            ) : (
              <Button
                disabled={localDisabled || !canLaunchOriginal}
                onClick={() => void localAction("launch")}
                title={t("启动原版 Claude Desktop")}
              >
                <Rocket className="h-4 w-4" />
                {localBusy === "launch" ? t("启动中…") : t("启动 Claude")}
              </Button>
            ),
            topbarSlot,
          )
        : null}
      {!patchOnly && localError ? <div className="claude-l10n-error" role="alert">{localError}</div> : null}
      {/* 本地服务的提示（没有可用模型、待重启）只和原版启动有关，补丁副本时不显示。 */}
      {!patchOnly && claudeLaunch !== "copy" && local?.warning ? <div className="claude-l10n-warning" role="status">{local.warning}</div> : null}
      {!patchOnly && claudeLaunch !== "copy" && local?.needsRestart ? <p className="claude-l10n-hint" role="status">{t("设置已保存，完全退出 Claude 后重新启动。")}</p> : null}
      {!patchOnly && !copyInline ? <CxCard className="claude-l10n-hero">
        <CxRow
          badges={<CxBadge tone={localTone}>{localBusy === "detect" ? t("检测中…") : localState}</CxBadge>}
          detail={localDetail}
          icon={<CxStatusIcon tone={local?.connected ? "ok" : "off"}><Rocket /></CxStatusIcon>}
          title="Claude Desktop"
        >
          <div className="claude-l10n-actions">
            {local?.running ? (
              <button className="cx-secondary" disabled={localDisabled} onClick={() => void localAction("stop")} type="button">
                <Square />{t("停止本地服务")}
              </button>
            ) : null}
            <button className="cx-secondary" disabled={localDisabled} onClick={() => void refreshLocal()} type="button">
              <RefreshCw />{t("检测原版 Claude")}
            </button>
          </div>
        </CxRow>
      </CxCard> : null}
      {!patchOnly ? <CxCard className="claude-local-launch">
          {/* 一个下拉管启动方式，也就定了增强怎么生效：3p 不注入，完整功能模式经本地服务注入官方 Claude，补丁副本在副本里注入。 */}
          <CxRow
            detail={claudeLaunch === "copy"
              ? t("启动改过代码的副本，汉化、听写、页面扩展在副本里注入；官方 Claude 不动。")
              : claudeLaunch === "full"
                ? t("经本地服务启动官方 Claude，汉化、听写、页面扩展都由它注入。已打开的 Claude 需完全退出重开。")
                : t("用安装包自带的界面直连本地服务，不注入汉化、听写和页面扩展。已打开的 Claude 需完全退出重开。")}
            title={t("启动模式")}
          >
            <AppSelect
              ariaLabel={t("启动模式")}
              disabled={localDisabled || deliveryBusy || !local?.supported}
              onChange={(next) => void chooseClaudeLaunch(next)}
              options={[
                { value: "3p", label: t("3p 模式") },
                { value: "full", label: t("完整功能模式") },
                { value: "copy", label: t("补丁副本") },
              ]}
              value={claudeLaunch}
            />
          </CxRow>
      </CxCard> : null}
      {/* 其余设置按用途分页签。没选中的页签只是藏起来，不卸载：切页签不丢输入框里的草稿，供应商列表也不用重读。补丁副本没有这些设置。 */}
      {!patchOnly && settingsTabs.length > 0 ? <div className="claude-local-settings">
        <nav aria-label={t("Desktop 增强设置")} className="tp-tabs claude-local-tabs" onKeyDown={onTabListKeyDown} role="tablist">
          {settingsTabs.map((tab) => (
            <button
              aria-controls={`claude-local-panel-${tab.id}`}
              aria-selected={activeTab === tab.id}
              data-active={activeTab === tab.id}
              id={`claude-local-tab-${tab.id}`}
              key={tab.id}
              onClick={() => setSettingsTab(tab.id)}
              role="tab"
              tabIndex={activeTab === tab.id ? 0 : -1}
              type="button"
            >
              {tab.label}
              {tab.attention ? <span aria-label={t("需要处理")} className="claude-local-tab-dot" role="img" /> : null}
            </button>
          ))}
        </nav>
        {/* 模型：模型选择器里列哪些模型。供应商和默认模型在 Claude 的「供应商」页（补丁副本走 Claude 自己的登录，没有这一页）。 */}
        <div {...settingsPanel("providers")}>
          <CxCard>
          {onNavigate ? (
            <CxRow detail={t("Claude 用哪些供应商、默认哪个模型，在 Claude 的「供应商」页里设置。")} title={t("供应商与默认模型")}>
              <button className="cx-secondary" onClick={() => onNavigate("claudeProviders")} type="button">{t("打开供应商")}</button>
            </CxRow>
          ) : null}
          {/* 窗口比默认模型小的模型不会列给 Claude（Claude 按默认模型的窗口压所有模型）；说清楚它们去哪了。 */}
          <CxRow
            detail={t("默认只列窗口不小于默认模型的模型（Claude 的窗口是启动时一份全局值，切到更小的模型不会跟着变小）；打开后池里的聊天模型全列出来，代价是切到小窗口模型可能超上限。")}
            title={t("列出全部模型")}
          >
            <label className="switch-row compact">
              <input
                aria-label={t("列出全部模型")}
                checked={Boolean(local?.config.listAllModels)}
                disabled={localDisabled || !local?.supported}
                onChange={(event) => {
                  if (local) void configureLocal({ ...local.config, listAllModels: event.currentTarget.checked });
                }}
                type="checkbox"
              />
              <span>{local?.config.listAllModels ? t("已开启") : t("已关闭")}</span>
              <ToggleVisual />
            </label>
          </CxRow>
          <CxRow
            detail={t("Claude 自带的 Opus、Sonnet、Haiku 等条目不对应任何供应商，选了也会被换成默认模型。打开后选择器里只留共享供应商里的模型；改完需要重新启动本地服务。")}
            title={t("隐藏无供应商的模型")}
          >
            <label className="switch-row compact">
              <input
                aria-label={t("隐藏无供应商的模型")}
                checked={local?.config.hideUnroutedModels ?? true}
                disabled={localDisabled || !local?.supported}
                onChange={(event) => {
                  if (local) void configureLocal({ ...local.config, hideUnroutedModels: event.currentTarget.checked });
                }}
                type="checkbox"
              />
              <span>{local?.config.hideUnroutedModels ?? true ? t("已开启") : t("已关闭")}</span>
              <ToggleVisual />
            </label>
          </CxRow>
          </CxCard>
        </div>
        {/* 账号与会话：Claude 里显示的账号信息，以及和命令行 Claude Code 怎么共用会话。 */}
        <div {...settingsPanel("account")}>
          <CxCard>
          <CxRow detail={t("只改 Claude 里显示的账号信息，从这里启动后生效。")} title={t("用户名")}>
            <Input
              aria-label={t("用户名")}
              defaultValue={local?.config.displayName ?? ""}
              disabled={localDisabled || !local?.supported}
              key={local?.config.displayName ?? ""}
              onBlur={(event) => {
                if (local && event.currentTarget.value !== (local.config.displayName ?? "")) void configureLocal({ ...local.config, displayName: event.currentTarget.value });
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter") event.currentTarget.blur();
              }}
              placeholder={t("Local User")}
            />
          </CxRow>
          <CxRow detail={t("只改 Claude 里显示的账号邮箱，从这里启动后生效。")} title={t("邮箱")}>
            <Input
              aria-label={t("邮箱")}
              defaultValue={local?.config.email ?? ""}
              disabled={localDisabled || !local?.supported}
              key={local?.config.email ?? ""}
              onBlur={(event) => {
                if (local && event.currentTarget.value !== (local.config.email ?? "")) void configureLocal({ ...local.config, email: event.currentTarget.value });
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter") event.currentTarget.blur();
              }}
              placeholder="local@claude.local"
              type="email"
            />
          </CxRow>
          <CxRow title={t("套餐")}>
            <AppSelect
              ariaLabel={t("套餐")}
              disabled={localDisabled || !local?.supported}
              onChange={(next) => {
                if (local) void configureLocal({ ...local.config, plan: next });
              }}
              options={[
                { value: "free", label: "Free" },
                { value: "pro", label: "Pro" },
                { value: "max", label: "Max" },
                { value: "ultra", label: "Ultra" },
              ]}
              value={local?.config.plan || "ultra"}
            />
          </CxRow>
          </CxCard>
          <CxCard>
          <CxRow
            detail={(local?.config.codeShareHome ?? true)
              ? t("Code 页签和命令行的 Claude Code 共用 ~/.claude：会话、MCP、插件是同一份；~/.claude/settings.json 里的接口地址（env）会优先于本地服务。从这里启动后生效。")
              : t("Code 页签用独立目录，请求走 Codex++ 本地服务；平时在命令行 / 3p 模式建的会话要靠会话同步才能打开。")}
            title={t("和命令行共用 ~/.claude")}
          >
            <label className="switch-row compact">
              <input
                aria-label={t("和命令行共用 ~/.claude")}
                checked={local?.config.codeShareHome ?? true}
                disabled={localDisabled || !local?.supported}
                onChange={(event) => {
                  if (local) void configureLocal({ ...local.config, codeShareHome: event.currentTarget.checked });
                }}
                type="checkbox"
              />
              <span>{(local?.config.codeShareHome ?? true) ? t("已开启") : t("已关闭")}</span>
              <ToggleVisual />
            </label>
          </CxRow>
          <CxRow
            detail={t("启动 Claude 前，把终端、BaoCode 等命令行里建的会话加进左侧列表（已在列表里的、删过的不会重复加）。")}
            title={t("显示命令行会话")}
          >
            <label className="switch-row compact">
              <input
                aria-label={t("显示命令行会话")}
                checked={local?.config.importCliSessions ?? true}
                disabled={localDisabled || !local?.supported}
                onChange={(event) => {
                  if (local) void configureLocal({ ...local.config, importCliSessions: event.currentTarget.checked });
                }}
                type="checkbox"
              />
              <span>{(local?.config.importCliSessions ?? true) ? t("已开启") : t("已关闭")}</span>
              <ToggleVisual />
            </label>
          </CxRow>
          <CxRow detail={t("启动 Claude 前同步 3p 模式与完整功能模式的 Code 会话")} title={t("会话同步")}>
            <div className="claude-l10n-actions">
              <label className="switch-row compact">
                <input
                  aria-label={t("会话同步")}
                  checked={local?.config.sessionSync === true}
                  disabled={localDisabled || !local?.supported}
                  onChange={(event) => {
                    if (local) void configureLocal({ ...local.config, sessionSync: event.currentTarget.checked });
                  }}
                  type="checkbox"
                />
                <ToggleVisual />
              </label>
              <button className="cx-secondary" disabled={localDisabled || !local?.supported || local?.claudeRunning} onClick={() => void localAction("sync_sessions")} title={t("立即同步")} type="button">{t("立即同步")}</button>
            </div>
          </CxRow>
          {/* 自动接管只在完整功能模式下动作，所以只在这个模式下显示（首页 Claude 行下面也有一份）。 */}
          {claudeLaunch === "full" ? <ClaudeAutoTakeoverRow /> : null}
          </CxCard>
        </div>
        {claudeLaunch === "full" && local ? (
          <>
            {/* 证书与环境：本地服务要用的证书、听写、Node 与界面版本。 */}
            <div {...settingsPanel("environment")}>
            <CxCard>
              <CxRow badges={<CxBadge tone={local.caTrusted ? "ok" : "neutral"}>{local.caTrusted ? t("已信任") : local.caReady ? t("未信任") : t("未准备")}</CxBadge>} title={t("本地证书")}>
                <div className="claude-l10n-actions">
                  {!local.caReady ? (
                    <button className="cx-secondary" disabled={localDisabled} onClick={() => void localAction("prepare")} type="button"><PackageOpen />{t("准备证书")}</button>
                  ) : null}
                  {!local.caTrusted ? (
                    <button className="cx-secondary" disabled={localDisabled || !local.caReady} onClick={() => void localAction("trust")} type="button"><ShieldCheck />{t("信任证书")}</button>
                  ) : (
                    <button className="cx-secondary" disabled={localDisabled} onClick={() => void localAction("untrust")} type="button"><ShieldCheck />{t("取消信任证书")}</button>
                  )}
                </div>
              </CxRow>
              {/* Cowork 虚拟机以系统服务运行，只读系统级根证书：只有 Windows 需要，Cowork 关着时用不上。 */}
              {(local.config.cowork ?? true) && local.caMachineSupported ? (
                <CxRow
                  badges={<CxBadge tone={local.caMachineTrusted ? "ok" : "neutral"}>{local.caMachineTrusted ? t("已安装") : t("未安装")}</CxBadge>}
                  detail={t("Cowork 虚拟机以系统服务运行，只读取系统级根证书；安装需要管理员授权。")}
                  title={t("系统证书")}
                >
                  {!local.caMachineTrusted ? (
                    <div className="claude-l10n-actions">
                      <button className="cx-secondary" disabled={localDisabled} onClick={() => void localAction("trust_machine")} type="button"><ShieldCheck />{t("安装系统证书")}</button>
                    </div>
                  ) : null}
                </CxRow>
              ) : null}
              <CxRow badges={<CxBadge tone={local.dictationReady ? "ok" : "neutral"}>{local.dictationReady ? t("已就绪") : t("未就绪")}</CxBadge>} title={t("语音识别")} />
              <CxRow badges={<CxBadge tone={local.nodeAvailable ? "ok" : "warn"}>{local.nodeAvailable ? t("已就绪") : t("缺少 Node.js")}</CxBadge>} title={t("运行环境")} />
              {localTemplates.length > 1 ? (
                <CxRow
                  detail={t("录制 claude.ai 界面时的版本。新版本有问题时可以换回旧的；前端代码按所选版本从 Anthropic 下载。")}
                  title={t("界面版本")}
                >
                  <AppSelect
                    ariaLabel={t("界面版本")}
                    disabled={localDisabled}
                    menuWidth={300}
                    onChange={(next) => void configureLocal({ ...local.config, template: next })}
                    options={[
                      { value: "", label: tf("最新（{0}）", [claudeTemplateLabel(localTemplates[0])]) },
                      ...localTemplates.map((template) => ({ value: template.build, label: claudeTemplateLabel(template) })),
                    ]}
                    value={localTemplates.some((template) => template.build === local.config.template) ? local.config.template ?? "" : ""}
                  />
                </CxRow>
              ) : null}
            </CxCard>
            </div>
            {/* 功能：完整功能模式往官方 Claude 里注入的各项能力，以及 Bash 时限。 */}
            <div {...settingsPanel("features")}>
            <CxCard className="claude-l10n-feature-grid">
              {([
                ["localization", "界面汉化", "本地界面汉化", ""],
                ["voiceInput", "原生听写", "本地原生听写", ""],
                ["computerUse", "电脑控制", "本地电脑控制", ""],
                ["backgroundMode", "后台模式", "本地后台模式", ""],
                ["forceBackgroundMode", "新系统也开后台模式", "本地后台模式不限系统版本", "Claude 在较新的 macOS 上会提示「暂不支持后台模式」；打开后不再限制系统版本，未经 Claude 验证，可能不稳定"],
                ["cowork", "Cowork", "本地 Cowork", "关掉后按 Claude 自己的开关，新版 Claude 通常只剩 Chat / Code。系统条件（Windows 需商店版、虚拟化可用）仍由 Claude 检查"],
                ["hubHome", "合并式主页（实验）", "本地合并式主页", "Claude 正在小范围试验的主页布局：对话和 Cowork 合在一个输入框里，没有「对话 | Cowork」切换。关着时用有切换的经典主页"],
              ] as const).map(([key, label, ariaLabel, hint]) => (
                <label
                  className="switch-row compact"
                  key={key}
                  title={hint ? t(hint) : undefined}
                >
                  <input
                    aria-label={t(ariaLabel)}
                    // 只有 Cowork 默认是开的。
                    checked={local.config[key] ?? key === "cowork"}
                    disabled={localDisabled}
                    onChange={(event) => {
                      const checked = event.currentTarget.checked;
                      // 关后台模式时一并关掉「新系统也开」，否则保存时会被它重新打开。
                      const extra = key === "backgroundMode" && !checked ? { forceBackgroundMode: false } : {};
                      void configureLocal({ ...local.config, [key]: checked, ...extra });
                    }}
                    type="checkbox"
                  />
                  <span><strong>{t(label)}</strong></span>
                  <ToggleVisual />
                </label>
              ))}
            </CxCard>
            {enhance ? <FullModeBashTimeout timeout={enhance.bashTimeout} /> : null}
            </div>
          </>
        ) : null}
      </div> : null}
      {advancedShown ? <section className={`claude-local-advanced${patchOnly || copyInline ? " claude-local-advanced-page" : ""}`}>
        <fieldset className="claude-local-advanced-body" disabled={localBusy !== null}>
      {patchOnly && delivery === "full" ? (
        <CxBanner
          detail={t("现在的生效方式是完整功能模式：汉化、听写、页面扩展由本地服务注入官方 Claude，用不到补丁版。要改用补丁版，先把生效方式换过来。")}
          icon={<PackageOpen />}
          title={t("补丁副本没在用")}
          tone="warn"
        >
          <button className="cx-secondary" disabled={deliveryBusy} onClick={() => void chooseDelivery("copy")} type="button">{t("改用补丁副本")}</button>
        </CxBanner>
      ) : null}
      {/* 顶部最多几条一次性的提示：检测中 / 未安装、错误、Bash 时限确认、制作进度。 */}
      {!s ? (
        <p className="claude-l10n-hint">
          {busy === "detect"
            ? t("正在读取 Claude 安装状态，只读检测，不会修改应用。")
            : t("没有读到安装状态，点击“检测 Claude”重试；检测不会修改应用。")}
        </p>
      ) : null}
      {error ? <div className="claude-l10n-error" role="alert">{error}</div> : null}
      {bashTimeout?.enabled && s?.bashTimeoutWarning ? (
        <div className="claude-l10n-warning" role="status">{t("Bash 时限补丁：")} {t(s.bashTimeoutWarning)}</div>
      ) : null}
      {progress ? (
        <div
          aria-valuemax={100}
          aria-valuemin={0}
          aria-valuenow={progress.percent}
          className="claude-l10n-progress"
          role="progressbar"
        >
          <div className="claude-l10n-progress-head">
            <span>{progress.message || t("处理中…")}</span>
            <span className="claude-l10n-progress-pct">{progress.percent}%</span>
          </div>
          <div className="claude-l10n-progress-track">
            <div
              className="claude-l10n-progress-fill"
              style={{ width: `${Math.min(100, Math.max(2, progress.percent))}%` }}
            />
          </div>
        </div>
      ) : null}

      {/* 增强版状态一行：版本与词表收成一行小字；制作 / 检测 / 检查在右侧，启动在页头右上角。 */}
      <CxCard className="claude-l10n-hero">
        <CxRow
          badges={<CxBadge tone={statusTone === "ok" ? "ok" : statusTone === "warn" ? "warn" : "neutral"}>{statusText}</CxBadge>}
          detail={<span title={t("所有补丁的前置条件：官方 Claude.app 与 app.asar 保持原样，补丁通过副本和启动时注入生效。")}>{s && !s.supported
            ? t("未检测到 Claude，请先安装官方应用。你可以继续使用 Codex++ 的其他功能。")
            : s?.supported && !s.prepared ? t("首次启动会先制作独立副本；官方 Claude.app 与 app.asar 不改动") : meta}</span>}
          icon={<CxStatusIcon tone={statusTone === "ok" ? "ok" : statusTone === "warn" ? "warn" : "off"}><PackageOpen /></CxStatusIcon>}
          title={t("补丁版 Claude")}
        >
          {/* 增强副本仅在这里手动制作和启动；检查与重新检测收进菜单。 */}
          <div className="claude-l10n-actions">
            {patchOnly ? <button className="cx-secondary" disabled={busy !== null || localBusy !== null || !canLaunch} onClick={() => void launchEnhanced()} title={needsPrepare ? t("制作副本并启动补丁版") : t("启动补丁版")} type="button">
              <Rocket />{busy === "launch" ? t("启动中…") : busy === "prepare" ? t("制作中…") : t("启动补丁版")}
            </button> : null}
            {needsPrepare || !s ? (
              <button className="cx-secondary" disabled={busy !== null || !s?.supported} onClick={() => void act("prepare")} title={prepareHint} type="button">
                <PackageOpen />
                {prepareLabel}
              </button>
            ) : null}
            <Menu>
              {needsPrepare || !s ? null : (
                <MenuItem disabled={busy !== null || !s?.supported} onClick={() => void act("prepare")}>{prepareLabel}</MenuItem>
              )}
              <MenuItem disabled={busy !== null || !s?.prepared} onClick={() => void act("check")}>{t("检查补丁版")}</MenuItem>
              <MenuItem disabled={busy !== null} onClick={() => void refresh()}>{busy === "detect" ? t("检测中…") : t("检测 Claude")}</MenuItem>
            </Menu>
          </div>
        </CxRow>
      </CxCard>
      {/* 增强开关一节：每项一行，各自独立、即点即存，重启增强版后应用。 */}
      <CxSection
        aside={s?.supported && !s.prepared ? (
          <span className="claude-l10n-section-note">{t("补丁版还没制作：开关会先存下来，制作并启动补丁版后生效。")}</span>
        ) : (
          <span className="claude-l10n-section-note" title={t("各自独立开关，互不影响；副本只是它们共同的前置条件，改动在重启副本后应用。")}>{t("重启补丁版后应用")}</span>
        )}
        className="claude-l10n-features"
        title={t("补丁项")}
      >
        <CxCard className="claude-l10n-feature-grid">
          <label className="switch-row compact">
            <input
              checked={Boolean(s?.localization)}
              disabled={busy !== null || !s?.supported}
              onChange={(event) => void toggleLocalization(event.currentTarget.checked)}
              type="checkbox"
            />
            <span>
              <strong>{t("界面汉化")}</strong>
              <small>{t("启动时注入官方中文词表，界面语言切到简体中文；关掉后副本按原语言启动。")}</small>
            </span>
            <ToggleVisual />
          </label>
          <label className="switch-row compact">
            <input
              checked={Boolean(s?.voiceInput)}
              disabled={busy !== null || !s?.supported}
              onChange={(event) => void toggleVoiceInput(event.currentTarget.checked)}
              type="checkbox"
            />
            <span>
              <strong>{t("语音输入")}</strong>
              <small>{t("在 Claude 输入框右下角加一个麦克风入口，识别结果只写进输入框、不自动发送。")}</small>
            </span>
            <ToggleVisual />
          </label>
          <label className="switch-row compact">
            <input
              checked={Boolean(s?.computerUse)}
              disabled={busy !== null || !s?.supported || s?.computerUseSupported === false}
              onChange={(event) => void toggleComputerUse(event.currentTarget.checked)}
              type="checkbox"
            />
            <span>
              <strong>{t("Computer Use")}</strong>
              <small title={s?.computerUseSupported === false
                ? t("暂仅支持 macOS：Windows 的副本构建流程不会改写 app.asar，打开这个开关不会有任何效果。")
                : `${t("解锁桌面控制：订阅与灰度校验已解除，API Key（3p）模式同样可用；会改写副本的 app.asar 并关闭 Electron 的 asar 完整性校验，原件不动。首次启动补丁版会提示授权辅助功能与屏幕录制。")}\n${t("这里的电脑控制和「Computer Use」页给 Claude Desktop 接入的 codex-plus-cu 是两套独立实现，同一个 Claude Desktop 里二选一即可，不要同时开。")}`}
              >{s?.computerUseSupported === false
                ? t("暂仅支持 macOS：Windows 的副本构建流程不会改写 app.asar，打开这个开关不会有任何效果。")
                : t("解锁桌面控制：订阅与灰度校验已解除，API Key（3p）模式同样可用；会改写副本的 app.asar 并关闭 Electron 的 asar 完整性校验，原件不动。首次启动补丁版会提示授权辅助功能与屏幕录制。")}</small>
            </span>
            <ToggleVisual />
          </label>
          {s?.computerUseSupported !== false ? (
            s?.computerUse && claudeDesktopCuConnected ? (
              <p className="claude-l10n-hint is-warning" role="alert">
                {t("Claude Desktop 同时接入了「Computer Use」页的 codex-plus-cu：同一个对话里会有两套电脑控制，Claude 可能混着调用、互相抢前台，同一个应用也要确认两次。建议二选一：保留这里的开关，就到「Computer Use」页撤销 Claude Desktop 的接入；或者关掉这里的开关。")}
              </p>
            ) : null
          ) : null}
          <label className="switch-row compact claude-bash-timeout-master">
            <input
              aria-label={t("解锁 Bash 时间限制")}
              checked={Boolean(bashTimeout?.enabled)}
              disabled={(busy !== null && busy !== "bashTimeout") || !bashTimeout}
              onChange={(event) => {
                if (bashTimeout) void saveBashTimeout({ ...bashTimeout, enabled: event.currentTarget.checked });
              }}
              type="checkbox"
            />
            <span>
              <strong>{t("解锁 Bash 时间限制")}</strong>
              <small title={t("配置 Claude Code Bash 的默认值与最大值；Cowork 还需启动注入成功且模块版本兼容。适配失败时提示并保持该项原生时限，不影响其他补丁。重启补丁版后应用。")}>{t("配置 Claude Code Bash 的默认值与最大值；Cowork 还需启动注入成功且模块版本兼容。适配失败时提示并保持该项原生时限，不影响其他补丁。重启补丁版后应用。")}</small>
            </span>
            <ToggleVisual />
          </label>
          {bashTimeout?.enabled ? (
            <div className="claude-bash-timeout-editor">
              <p>{t("两项可分别启停，单位均为秒。修改数字后离开输入框即保存。")}</p>
              <label className="switch-row compact claude-bash-timeout-toggle">
                <input
                  aria-label={t("修改默认等待时间")}
                  checked={bashTimeout.defaultEnabled}
                  disabled={busy !== null && busy !== "bashTimeout"}
                  onChange={(event) => void saveBashTimeout({
                    ...bashTimeout,
                    defaultEnabled: event.currentTarget.checked,
                  })}
                  type="checkbox"
                />
                <span>
                  <strong>{t("修改默认等待时间")}</strong>
                  <small>{t("未显式传 timeout / timeout_ms 时使用；Claude Code 与本地 Cowork Bash 原生默认 120 秒。")}</small>
                </span>
                <ToggleVisual />
              </label>
              <div className="claude-bash-timeout-value">
                <span>{t("默认等待时间")}</span>
                <Input
                  aria-label={t("默认等待时间（秒）")}
                  disabled={busy !== null || !bashTimeout.defaultEnabled}
                  max={2147423}
                  min={1}
                  onChange={(event) => {
                    setBashTimeoutDefaultText(event.currentTarget.value);
                    const seconds = clampNumber(Number(event.currentTarget.value), 1, 2147423);
                    setBashTimeoutDraft((current) => current ? { ...current, defaultSeconds: seconds } : current);
                  }}
                  onBlur={() => {
                    setBashTimeoutDefaultText(null);
                    void saveBashTimeout(bashTimeout);
                  }}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") event.currentTarget.blur();
                  }}
                  type="number"
                  value={bashTimeoutDefaultText ?? bashTimeout.defaultSeconds}
                />
                <span>{t("秒")}</span>
              </div>
              <label className="switch-row compact claude-bash-timeout-toggle">
                <input
                  aria-label={t("修改最大等待时间")}
                  checked={bashTimeout.maxEnabled}
                  disabled={busy !== null && busy !== "bashTimeout"}
                  onChange={(event) => void saveBashTimeout({
                    ...bashTimeout,
                    maxEnabled: event.currentTarget.checked,
                  })}
                  type="checkbox"
                />
                <span>
                  <strong>{t("修改最大等待时间")}</strong>
                  <small>{t("显式传 timeout / timeout_ms 时的上限；原生参数上限 600 秒，Cowork 还受外层等待时限约束。")}</small>
                </span>
                <ToggleVisual />
              </label>
              <div className="claude-bash-timeout-value">
                <span>{t("最大等待时间")}</span>
                <Input
                  aria-label={t("最大等待时间（秒）")}
                  disabled={busy !== null || !bashTimeout.maxEnabled}
                  max={2147423}
                  min={1}
                  onChange={(event) => {
                    setBashTimeoutMaxText(event.currentTarget.value);
                    const seconds = clampNumber(Number(event.currentTarget.value), 1, 2147423);
                    setBashTimeoutDraft((current) => current ? { ...current, maxSeconds: seconds } : current);
                  }}
                  onBlur={() => {
                    setBashTimeoutMaxText(null);
                    void saveBashTimeout(bashTimeout);
                  }}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") event.currentTarget.blur();
                  }}
                  type="number"
                  value={bashTimeoutMaxText ?? bashTimeout.maxSeconds}
                />
                <span>{t("秒")}</span>
              </div>
            </div>
          ) : null}
        </CxCard>
      </CxSection>
        </fieldset>
      </section> : null}
    </div>
  );
}

/** 完整功能模式下的 Bash 时限：Claude Code 那部分经环境变量生效；Cowork 那部分置灰，只有补丁副本能改。 */
function FullModeBashTimeout({ timeout }: { timeout: ClaudeBashTimeoutSettings }) {
  const { CxBadge, CxCard } = AgentCx;
  const [draft, setDraft] = useState(timeout);
  const [text, setText] = useState<{ defaultSeconds?: string; maxSeconds?: string }>({});
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => { setDraft(timeout); }, [timeout]);
  const save = async (next: ClaudeBashTimeoutSettings) => {
    setDraft(next); setSaving(true); setError(null);
    try {
      const state = await saveClaudeBashTimeout(next);
      if (state && claudeLocalizationMemo.status?.status === "ok") {
        claudeLocalizationMemo.status = { ...claudeLocalizationMemo.status, bashTimeout: state.bashTimeout };
      }
    } catch (cause) { setDraft(timeout); setError(stringifyError(cause)); }
    finally { setSaving(false); }
  };
  const seconds = (key: "defaultSeconds" | "maxSeconds", enabledKey: "defaultEnabled" | "maxEnabled", label: string) => (
    <div className="claude-bash-timeout-value">
      <label className="switch-row compact claude-bash-timeout-toggle">
        <input aria-label={label} checked={draft[enabledKey]} disabled={saving} onChange={(event) => void save({ ...draft, [enabledKey]: event.currentTarget.checked })} type="checkbox" />
        <span><strong>{label}</strong></span>
        <ToggleVisual />
      </label>
      <Input
        aria-label={tf("{0}（秒）", [label])}
        disabled={saving || !draft[enabledKey]}
        max={2147423}
        min={1}
        onBlur={() => { setText((current) => ({ ...current, [key]: undefined })); void save(draft); }}
        onChange={(event) => {
          const value = event.currentTarget.value;
          setText((current) => ({ ...current, [key]: value }));
          setDraft((current) => ({ ...current, [key]: clampNumber(Number(value), 1, 2147423) }));
        }}
        onKeyDown={(event) => { if (event.key === "Enter") event.currentTarget.blur(); }}
        type="number"
        value={text[key] ?? draft[key]}
      />
      <span>{t("秒")}</span>
    </div>
  );
  return (
    <CxCard className="claude-l10n-feature-grid">
      {error ? <div className="claude-l10n-error" role="alert">{error}</div> : null}
      <label className="switch-row compact claude-bash-timeout-master">
        <input aria-label={t("解锁 Bash 时间限制")} checked={draft.enabled} disabled={saving} onChange={(event) => void save({ ...draft, enabled: event.currentTarget.checked })} type="checkbox" />
        <span>
          <strong>{t("解锁 Bash 时间限制")}</strong>
          <small>{t("改 Claude Code 里 Bash 的默认等待和最长等待时间，重新启动 Claude 后生效。")}</small>
        </span>
        <ToggleVisual />
      </label>
      {draft.enabled ? (
        <div className="claude-bash-timeout-editor">
          {seconds("defaultSeconds", "defaultEnabled", t("默认等待时间"))}
          {seconds("maxSeconds", "maxEnabled", t("最大等待时间"))}
        </div>
      ) : null}
      <label className="switch-row compact" title={t("Cowork 的 Bash 时限写在 Claude 主进程代码里，只有补丁副本能在启动时改；完整功能模式下保持 Claude 原生时限。")}>
        <input aria-label={t("Cowork 的 Bash 时限")} checked={false} disabled type="checkbox" />
        <span>
          <strong>{t("Cowork 的 Bash 时限")}</strong>
          <small><CxBadge>{t("仅副本")}</CxBadge> {t("要改主进程代码，完整功能模式做不到")}</small>
        </span>
        <ToggleVisual />
      </label>
    </CxCard>
  );
}
