// 管理器主界面里多个页面共用的小组件与辅助函数（从 App.tsx 拆出）。
import { ArrowRightLeft, CheckCircle2, RefreshCw, AppWindow, Bug, Server, type LucideIcon } from "lucide-react";
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { parseContextEntries, splitContextConfigText, type CodexContextEntries } from "./context-config-text";
import { appConfirm } from "./app-dialog";
import { enhancementPlatformFromUserAgent } from "./enhancement-catalog";
import { defaultWhaleBalanceSettings, normalizeWhaleBalanceSettings } from "./whale-settings";
import { defaultDictationSettings, normalizeDictationSettings } from "./dictation-settings";
import { providerAvatarText } from "./provider-name";
import { defaultDreamSkinTheme, normalizeDreamSkinTheme } from "./dream-skin";
import { t, tf } from "@/i18n";
import {
  syncLegacyRelayFields,
  removeRelayProfile,
  relayProfileDeleteIssue,
  isAggregateRelayProfile,
  normalizeAggregateConfig,
  aggregateStrategyLabel,
  ensureTrailingNewline,
  joinTomlSections,
  splitTomlRootAndTables,
  emptyContextSelection,
  hydrateAggregateRelayProfile,
  normalizeRelayProfile,
} from "./relay-profile-editing";
import { runningInstancesForProfile } from "./multi-launch-model";
import { CxBadge } from "./cx-layout";
import type {
  Actions,
  AutoSaveOutcome,
  AutoTakeoverResult,
  BackendSettings,
  ClaudeLocalizationStatus,
  ClaudeLocalStatus,
  CommandResult,
  OverviewResult,
  RelayContextSelection,
  RelayEditorMode,
  RelayMode,
  RelayProfile,
  RelayProtocol,
  RelaySessionProvider,
  RequestLogEntry,
  RuntimeServiceState,
  Status,
  StepwiseGenerationMode,
  StepwiseProtocol,
  TaskProgress,
} from "./app-types";

 // cx-pages:agents：Agent 页面在函数内解构，避免和别的分组的具名导入重名

/// 增强设置项清单按平台过滤（例如桌宠跟随鼠标只在 Windows / macOS 显示）。
export const enhancementPlatform = enhancementPlatformFromUserAgent(navigator.userAgent);

/** Codex++ 自己的 logo：与应用图标（src-tauri/icons/icon.png）同一张图。 */
export const APP_LOGO_URL = new URL("./assets/codex-plus-logo.png", import.meta.url).href;

export const defaultSettings: BackendSettings = {
  ...defaultWhaleBalanceSettings(),
  codexAppPath: "",
  codexAppAutoTakeover: false,
  codexAppAutoTakeoverWhenActive: "defer",
  claudeAppAutoTakeover: false,
  claudeDesktopLocal: { enabled: false, model: "", localization: true, voiceInput: true, computerUse: false, backgroundMode: false },
  codexExtraArgs: [],
  dictation: defaultDictationSettings(),
  providerSyncEnabled: false,
  sessionIndexRepairEnabled: true,
  threadModelFollowSwitchEnabled: false,
  keepProjectsWithoutAccount: false,
  chatgptCompatEnabled: false,
  updateAutoCheck: true,
  updateAutoDownload: true,
  updateSkippedVersion: "",
  ccsDbPath: "",
  providerSyncSavedProviders: [],
  providerSyncManualProviders: [],
  providerSyncLastSelectedProvider: "",
  relayProfilesEnabled: true,
  localConfigProviderImported: false,
  enhancementsEnabled: true,
  codexAppStandaloneQuotaUnlock: false,
  codexAppPluginMarketplaceUnlock: true,
  codexAppModelWhitelistUnlock: true,
  codexAppSessionDelete: true,
  codexAppSessionDeleteSkipConfirm: false,
  codexAppSideChatSave: false,
  codexAppMarkdownExport: true,
  codexAppUsageStatusLine: true,
  usageHistoryEnabled: false,
  codexAppProxyRequestStatus: true,
  codexAppPasteFix: false,
  codexAppPromptArgs: true,
  codexAppForceChineseLocale: false,
  codexAppThreadIdBadge: false,
  codexAppConversationView: false,
  codexAppThreadScrollRestore: true,
  codexAppHistoryTurnRecovery: false,
  codexAppNativeMenuPlacement: true,
  codexAppNativeBrowserRequireIdentification: false,
  codexAppNativeFastUnlock: false,
  codexAppNativePredictions: false,
  codexAppThirdPartyReasoningCollapse: false,
  codexAppOfflineAutoReview: false,
  codexAppStatsigOverrides: false,
  codexAppStatsigGateOverrides: {},
  codexAppReportedUsageCompaction: false,
  codexAppDisableWorkView: false,
  codexAppPetRealMouseLook: false,
  codexAppActivityOverlayRight: false,
  protocolProxyAutoRetry: true,
  protocolProxyAutoRetryMaxAttempts: 5,
  protocolProxyAutoRetryIntervalSeconds: 2,
  protocolProxyAutoRetryStatusCodes: [],
  protocolProxyAutoRetryMode: "streaming",
  protocolProxyAutoRetryCommitDelayMs: 1000,
  protocolProxyMapUnlistedModels: false,
  helperMaxRequestBodyMib: 128,
  upstreamStreamHeaderTimeoutSecs: null,
  codexAppCapacityAutoRetry: false,
  codexAppTurnRewind: false,
  codexAppStepwiseEnabled: false,
  codexAppAnswerOutlineEnabled: false,
  codexAppStepwiseDirectSend: false,
  codexAppStepwiseProtocol: "chat_completions",
  codexAppStepwiseGenerationMode: "auto",
  codexAppStepwiseBaseUrl: "",
  codexAppStepwiseApiKey: "",
  codexAppStepwiseApiKeyEnv: "CODEX_STEPWISE_API_KEY",
  codexAppStepwiseModel: "",
  codexAppStepwiseMaxItems: 4,
  codexAppStepwiseMaxInputChars: 6000,
  codexAppStepwiseMaxOutputTokens: 500,
  codexAppStepwiseTimeoutMs: 8000,
  codexAppDreamSkinEnabled: false,
  codexAppDreamSkinPaused: false,
  codexAppDreamSkinTheme: "pink",
  codexAppDreamSkinThemeConfig: defaultDreamSkinTheme(),
  codexAppDreamSkinImagePath: "",
  codexGoalsEnabled: false,
  weixinConnectEnabled: false,
  weixinConnectBaseUrl: "https://ilinkai.weixin.qq.com",
  weixinConnectToken: "",
  weixinConnectAccountId: "",
  weixinConnectAllowFrom: "",
  weixinConnectAllowOutboundAttachments: false,
  weixinConnectRouteTag: "",
  weixinConnectWorkDir: "",
  weixinConnectModel: "",
  weixinConnectReasoningEffort: "",
  weixinConnectProjectId: "",
  weixinConnectSandbox: "read-only",
  weixinConnectCodexPath: "",
  weixinConnectLinkedUserId: "",
  weixinConnectDefaultMode: "jiajia",
  weixinConnectPushToWeixin: false,
  relayBaseUrl: "",
  relayApiKey: "",
  relayProfiles: [
    {
      id: "default",
      name: t("默认中转"),
      model: "",
      baseUrl: "",
      upstreamBaseUrl: "",
      apiKey: "",
      protocol: "responses",
      relayMode: "official",
      officialMixApiKey: false,
      hideOfficialUsageAlert: false,
      testModel: "",
      configContents: "",
      authContents: "",
      useCommonConfig: true,
      contextSelection: emptyContextSelection(),
      contextSelectionInitialized: true,
      contextWindow: "",
      autoCompactLimit: "",
      modelList: "",
      modelWindows: "",
      modelAutoCompact: "",
      modelMetadata: "",
      modelVlm: "",
      vlmApiKey: "",
      vlmModel: "",
      vlmBaseUrl: "",
      userAgent: "",
      customHeaders: [],
      sub2apiEnabled: false,
      noAuth: false,
      sub2apiMultiplier: "",
      standardOpenaiProtocol: false,
      webSearchHistoryCompat: false,
    },
  ],
  relayCommonConfigContents: "",
  relayPreserveNonproviderSettings: false,
  relayContextConfigContents: "",
  activeRelayId: "default",
  aggregateRelayProfiles: [],
  activeAggregateRelayId: "",
  relayTestModel: "gpt-5.4-mini",
  tools: {},
  activeTool: "codex",
};

/** 请求来自哪个 Agent：网关记录带 agent 字段，Codex 直连代理的记录没有。 */
export function requestLogAgent(entry: RequestLogEntry): string {
  return entry.agent || "codex";
}

/** Codex 接入网关时，网关在直连模式的供应商列表里放的托管供应商（core 的 `CODEX_GATEWAY_RELAY_ID`）。 */
export const CODEX_GATEWAY_RELAY_ID = "codex-plus-gateway";

export function codexGatewayModelCount(profile: RelayProfile | undefined): number {
  return (profile?.modelList ?? "").split("\n").filter((line) => line.trim()).length;
}

export function multiLaunchSupported(profile: RelayProfile): boolean {
  return !isAggregateRelayProfile(profile)
    && (profile.protocol === "responses" || profile.protocol === "chatCompletions")
    && !(profile.modelRoutes ?? []).some((route) => route.model.trim() || route.targetRelayId.trim());
}

/**
 * 删除直连供应商的统一确认（旧版供应商列表、多开页共用）：
 * 先查占用（当前正在使用、聚合配置的默认 / 最后成员直接说明原因），再提醒运行中的多开实例，最后危险确认。
 * 确认后返回删掉它的新设置，取消或不能删时返回 null。
 */
export async function confirmRemoveRelayProfile(
  settings: BackendSettings,
  profile: RelayProfile,
  actions: Actions,
  from: "relay" | "multiLaunch",
): Promise<BackendSettings | null> {
  const name = profile.name || t("未命名供应商");
  const deleteIssue = relayProfileDeleteIssue(settings, profile.id);
  if (deleteIssue) {
    await actions.showMessage(t("删除供应商"), deleteIssue, "failed");
    return null;
  }
  const running = runningInstancesForProfile(actions.multiLaunchInstances, profile.id);
  const message = running.length
    ? tf("删除供应商「{0}」？它还有 {1} 个多开实例在运行，删除后实例不会被结束，但之后不能再用它启动。", [name, running.length])
    : from === "multiLaunch"
      ? tf("删除供应商「{0}」？删除后不能再用它启动多开实例，旧版供应商列表里也会一并移除。", [name])
      : tf("删除供应商「{0}」？它的 API Key、模型列表和配置文件会一并删除，不能撤销。", [name]);
  if (!(await appConfirm(message, { title: t("删除供应商"), confirmText: t("删除"), danger: true }))) return null;
  return removeRelayProfile(settings, profile.id);
}

const ENHANCE_AUTO_SAVE_DELAY_MS = 500;

type AutoSaveState =
  | { state: "idle" | "pending" | "saving" | "saved" }
  | { state: "invalid" | "failed"; message: string; retry: () => void };

/// 增强页的自动保存。
///
/// - 防抖：同一轮连续改动只存一次；
/// - 同一时间只有一个保存在途，期间的改动等它回来后再存（App 会同步版本号并触发下一轮）；
/// - 离开页面时还有没存的改动就立即补存一次，不丢草稿。
export function useEnhanceAutoSave(
  dirty: boolean,
  form: BackendSettings,
  save: (settings: BackendSettings) => Promise<AutoSaveOutcome>,
): AutoSaveState {
  const [status, setStatus] = useState<AutoSaveState>({ state: "idle" });
  const savingRef = useRef(false);
  const latest = useRef({ dirty, form, save });
  latest.current = { dirty, form, save };

  const flush = useCallback(async () => {
    if (savingRef.current) return;
    const { dirty: pending, form: snapshot, save: persist } = latest.current;
    if (!pending) return;
    savingRef.current = true;
    setStatus({ state: "saving" });
    const outcome = await persist(snapshot);
    savingRef.current = false;
    setStatus(outcome.status === "saved"
      ? { state: "saved" }
      : { state: outcome.status, message: outcome.message, retry: () => void flush() });
  }, []);

  useEffect(() => {
    if (!dirty) return;
    setStatus((current) => (current.state === "saving" || current.state === "invalid" ? current : { state: "pending" }));
    const timer = window.setTimeout(() => void flush(), ENHANCE_AUTO_SAVE_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [dirty, form, flush]);

  useEffect(() => () => {
    if (latest.current.dirty && !savingRef.current) void latest.current.save(latest.current.form);
  }, []);

  return status;
}

export function AutoSaveStatus({ state }: { state: AutoSaveState }) {
  if (state.state === "invalid" || state.state === "failed") {
    return (
      <>
        <span className="is-error" role="alert" title={state.message}>
          {state.state === "invalid" ? tf("未保存：{0}", [t(state.message)]) : tf("自动保存失败：{0}", [t(state.message)])}
        </span>
        {state.state === "failed" ? (
          <Button onClick={state.retry} size="sm" variant="secondary">
            <RefreshCw className="h-4 w-4" />
            {t("重试")}
          </Button>
        ) : null}
      </>
    );
  }
  const label = state.state === "saving" ? t("正在保存…")
    : state.state === "pending" ? t("即将自动保存…")
      : t("改动会自动保存");
  return (
    <span className={state.state === "saved" || state.state === "idle" ? "" : "is-dirty"} role="status">
      {state.state === "saved" ? <CheckCircle2 className="h-3.5 w-3.5" /> : null}
      {state.state === "saved" ? t("已自动保存") : label}
    </span>
  );
}

/// 设置表单的自动保存：改动后稍等片刻写入，替代原来的「保存设置」按钮。
/// 做成组件而不是在各页面里直接调用 hook，方便在列表、面板等位置按需插入。
export function SettingsAutoSave({ dirty, form, actions, className = "" }: {
  dirty: boolean;
  form: BackendSettings;
  actions: Actions;
  className?: string;
}) {
  const state = useEnhanceAutoSave(dirty, form, actions.autoSaveSettings);
  return (
    <div className={`enhance-save-actions settings-autosave ${className}`}>
      <AutoSaveStatus state={state} />
    </div>
  );
}

/**
 * 自动接管的状态标签与一行说明（首页和设置页共用）。
 * 说明里需要注意的情况优先：出错、本次跳过、平台不支持、为什么还没接管；都没有才显示官方 App、端口、上次接管。
 * `lastReason` / 降级时的 `lastError` 是 Rust 判据的英文原因码（`auto_takeover::decide`），这里翻成人话。
 */
export function autoTakeoverView(state: AutoTakeoverResult | null): { status: string; label: string; detail: string } {
  const reasons: Record<string, string> = {
    multi_instance: t("有另一个 Codex 实例在运行（多开或带调试端口启动的）；接管要先退出 Codex，会把它一起关掉，所以先不接管。"),
    user_active: t("Codex 正在执行任务，等它空闲后再接管；也可以点「立即接管」。"),
    probe_unknown: t("无法确认官方 App 状态，已保守跳过。"),
    backoff: t("上次接管没有成功，稍后自动重试。"),
    restart_budget_exhausted: t("一小时内接管次数已达上限，已停止自动重试。"),
    injection_ready_failed: t("上次注入没有成功，重启 Codex 可以重新注入。"),
    takeover: t("正在接管：退出官方 App 后带参数重开。"),
  };
  const reasonText = (code?: string | null) => (code ? reasons[code] ?? "" : "");
  if (!state) return { status: "off", label: t("未运行"), detail: "" };
  const attached = state.lastReason === "already_attached";
  const detail = reasonText(state.lastError) || state.lastError
    || (state.pausedInstance ? t("本次已跳过自动接管，重启官方 App 后恢复。") : "")
    || (!state.platformSupported ? t("当前系统不支持自动接管。") : "")
    || reasonText(state.lastReason)
    || [
      attached ? t("官方 App 已接管") : `${t("官方 App")} ${state.appRunning ? t("运行中") : t("未运行")}`,
      state.actualDebugPort ? `${t("调试端口")} ${state.actualDebugPort}` : "",
      state.lastTakeoverAtMs ? tf("上次接管：{0}", [new Date(state.lastTakeoverAtMs).toLocaleString()]) : "",
    ].filter(Boolean).join(" · ");
  if (state.state === "degraded") return { status: "failed", label: t("已降级"), detail };
  if (state.state === "taking_over") return { status: "warning", label: t("接管中"), detail };
  if (state.state === "paused") return { status: "warning", label: t("本次跳过"), detail };
  return state.running ? { status: "ok", label: t("监听中"), detail } : { status: "off", label: t("未运行"), detail };
}

/// 状态小标签：沿用 statusLabel / statusClass 的文案与分级，换成 CxBadge 的四种色调。
export function StatusBadge({ status = "unknown", label }: { status?: string; label?: string }) {
  const level = statusClass(status);
  const tone = level === "good" ? "ok" : level === "bad" ? "danger" : level === "idle" ? "neutral" : "warn";
  return <CxBadge tone={tone}>{label ?? statusLabel(status)}</CxBadge>;
}

/** 开关图形。包着它的 `<label>` 行只有点这个图形才切换，点文字不切换（switch-only.ts 统一处理）。 */
export function ToggleVisual() {
  return (
    <span aria-hidden="true" className="toggle-switch-visual">
      <span className="toggle-switch-thumb" />
    </span>
  );
}

export function formatProgressPercent(value: number): string {
  if (!Number.isFinite(value)) return "0.00";
  return Math.min(100, Math.max(0, value)).toFixed(2);
}

export function TaskProgressBox({ progress, title, completedTitle = t("上次修复结果") }: { progress: TaskProgress; title: string; completedTitle?: string }) {
  if (!progress.active && progress.percent <= 0 && !progress.failed) return null;
  return (
    <div className="provider-sync-progress task-progress" data-active={progress.active}>
      <div className="provider-sync-progress-head">
        <strong>{progress.active ? title : completedTitle}</strong>
        <span>{progress.indeterminate || progress.failed ? null : `${formatProgressPercent(progress.percent)}%`}</span>
      </div>
      {!progress.failed ? <div
        aria-valuemax={100}
        aria-valuemin={0}
        aria-valuenow={progress.indeterminate ? undefined : progress.percent}
        className="provider-sync-progress-bar"
        data-indeterminate={progress.indeterminate ? "true" : undefined}
        role="progressbar"
      >
        <div
          className="provider-sync-progress-fill"
          style={progress.indeterminate ? undefined : { width: `${progress.percent}%` }}
        />
      </div> : null}
      <small>{progress.message}</small>
    </div>
  );
}

/** 隐藏模型列表最多列 5 个，其余用「等」收尾。 */
export const claudeLocalMemo: {
  status: CommandResult<ClaudeLocalStatus> | null;
  autoDetected: boolean;
} = { status: null, autoDetected: false };

/**
 * Claude 增强页跨挂载的记忆：Codex++ 运行期间只在第一次进入时自动检测一次，
 * 之后再进来直接显示上次结果，需要时手动点「检测 Claude」刷新。
 *
 * 检测已在后台线程跑、带 15 秒超时，未安装 Claude 时直接早退（#2539 的死机不会再现），
 * 所以首次进入自动检测是安全的；但每次切回来都重新探测进程和运行时没有必要。
 */
export const claudeLocalizationMemo: {
  status: CommandResult<ClaudeLocalizationStatus> | null;
  autoDetected: boolean;
} = { status: null, autoDetected: false };

/// 设置行：左边标题 + 一句短说明，右边控件；长说明放进悬停提示（`detail`）。
/// `stacked` 时控件放到说明下方，给状态码编辑、多行输入这类宽控件用。
export function SettingRow({ title, description, detail, stacked = false, children }: {
  title: ReactNode;
  description?: ReactNode;
  detail?: string;
  stacked?: boolean;
  children?: ReactNode;
}) {
  return (
    <div className={`setting-row${stacked ? " setting-row-stacked" : ""}`} title={detail}>
      <div className="setting-row-copy">
        <strong>{title}</strong>
        {description ? <span title={detail ?? (typeof description === "string" ? description : undefined)}>{description}</span> : null}
      </div>
      {children ? <div className="setting-row-control">{children}</div> : null}
    </div>
  );
}

export function Toolbar({ children, className = "" }: { children: React.ReactNode; className?: string }) {
  return <div className={`toolbar ${className}`.trim()}>{children}</div>;
}

export function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

export function contextEntriesFromSettings(settings: BackendSettings): CodexContextEntries {
  const commonConfig = normalizeDuplicateTomlTables(settings.relayContextConfigContents || "");
  return {
    mcpServers: parseContextEntries(commonConfig, "mcp", "mcp_servers"),
    skills: parseContextEntries(commonConfig, "skill", "skills"),
    plugins: parseContextEntries(commonConfig, "plugin", "plugins"),
  };
}

export function tomlRootKeyFromLine(line: string): string | null {
  if (!line || line.startsWith("#")) return null;
  const index = line.indexOf("=");
  if (index < 0) return null;
  const key = line.slice(0, index).trim();
  return key || null;
}

export function joinTomlSectionsRootFirst(sections: string[]): string {
  const rootParts: string[] = [];
  const tableParts: string[] = [];

  for (const section of sections) {
    const { root, tables } = splitTomlRootAndTables(section);
    if (root.trim()) rootParts.push(root.trim());
    if (tables.trim()) tableParts.push(tables.trim());
  }

  return normalizeDuplicateTomlTables(joinTomlSections([...dedupeTomlRootLines(rootParts), ...tableParts]));
}

function normalizeDuplicateTomlTables(contents: string): string {
  const seenHeaders = new Set<string>();
  const kept: string[] = [];
  let skipping = false;

  for (const line of contents.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (/^\[[^\]]+\]$/.test(trimmed)) {
      skipping = seenHeaders.has(trimmed);
      seenHeaders.add(trimmed);
      if (skipping) continue;
    }
    if (!skipping) kept.push(line);
  }

  return ensureTrailingNewline(kept.join("\n").trimEnd());
}

function dedupeTomlRootLines(rootParts: string[]): string[] {
  const rootLines = rootParts
    .join("\n")
    .split(/\r?\n/)
    .map((line) => line.trimEnd());
  const rootSeen = new Set<string>();
  const kept: string[] = [];

  for (let index = rootLines.length - 1; index >= 0; index -= 1) {
    const line = rootLines[index];
    const key = tomlRootKeyFromLine(line.trim());
    if (key) {
      if (rootSeen.has(key)) continue;
      rootSeen.add(key);
    }
    kept.push(line);
  }

  const normalized = kept.reverse().join("\n").trim();
  return normalized ? [normalized] : [];
}

export function contextSelectionForAllEntries(settings: BackendSettings): RelayContextSelection {
  const entries = contextEntriesFromSettings(settings);
  return {
    mcpServers: entries.mcpServers.map((entry) => entry.id),
    skills: entries.skills.map((entry) => entry.id),
    plugins: entries.plugins.map((entry) => entry.id),
  };
}

/// 供应商头像上的两个字，规则见 provider-name.ts 的 providerAvatarText。
export function providerInitial(name: string) {
  return providerAvatarText(name, t("供"));
}

export function statusLabel(status: string) {
  const labels: Record<string, string> = {
    found: t("已找到"),
    missing: t("缺失"),
    installed: t("已安装"),
    ok: t("正常"),
    running: t("运行中"),
    running_degraded: t("运行中（增强等待中）"),
    degraded: t("已降级"),
    starting: t("启动中"),
    failed: t("失败"),
    archived: t("已归档"),
    accepted: t("已受理"),
    not_checked: t("未检查"),
    not_implemented: t("未实现"),
    disabled: t("已禁用"),
    stopped: t("未运行"),
    waiting: t("等待启动"),
    off: t("未启用"),
    connected: t("已连接"),
    disconnected: t("未连接"),
    unattached: t("未挂载"),
    unknown: t("未知"),
  };
  return labels[status] ?? status;
}

export function statusClass(status: string) {
  if (["found", "installed", "ok", "running", "running_degraded", "connected"].includes(status)) return "good";
  if (status === "waiting" || status === "off") return "idle";
  if (["failed", "missing"].includes(status)) return "bad";
  return "warn";
}

export function isSuccessStatus(status?: Status) {
  return status === "ok" || status === "accepted";
}

/// 概览健康检查的各项。每项带自己的图标；本地服务器与协议转换拆成两项：
/// 协议转换跑在本地服务器里，但两者状态独立（服务器在跑、转换没开是常态）。
export function healthItems(overview: OverviewResult | null): Array<{ title: string; status: string; ok: boolean; detail: string; icon: LucideIcon; action?: "restart" }> {
  const conversionEnabled = overview?.runtime_health?.protocol_conversion_enabled;
  const serviceItem = (title: string, state: RuntimeServiceState | undefined, icon: LucideIcon) => ({
    title,
    status: state?.status ?? "not_checked",
    ok: ["running", "connected", "disabled"].includes(state?.status ?? ""),
    detail: state ? [state.address, t(state.message)].filter(Boolean).join(" · ") : t("等待状态检查"),
    icon,
  });
  // 协议转换没开不是问题：直连 Responses 的供应商本来就不需要，显示为中性的「未启用」。
  const conversionHealth = {
    title: t("协议转换"),
    status: typeof conversionEnabled !== "boolean" ? "not_checked" : conversionEnabled ? "ok" : "off",
    ok: typeof conversionEnabled === "boolean",
    detail: typeof conversionEnabled !== "boolean"
      ? t("等待状态检查")
      : conversionEnabled
        ? t("Chat Completions / Anthropic 请求经本地服务器转成 Responses")
        : t("当前供应商直连 Responses，不需要转换"),
    icon: ArrowRightLeft,
  };
  // 调试连接依赖 Codex 在运行：Codex 没启动时连不上是预期内的，显示为中性的「等待启动」而不是警告。
  const debuggerState = overview?.runtime_health?.debugger;
  const codexRunning = overview?.runtime_health?.codex_app?.status === "running";
  // Codex 从 Dock / Finder 等外部启动时没带调试端口，增强脚本挂不上：提示原因，并给「重启并挂载」。
  const debuggerHealth = overview?.runtime_health?.codex_unattached
    ? {
      title: t("调试连接状态"),
      status: "unattached",
      ok: false,
      detail: t("Codex 不是经 Codex++ 启动的，增强脚本没有挂载；重启 Codex 即可挂载"),
      icon: Bug,
      action: "restart" as const,
    }
    : debuggerState && debuggerState.status !== "connected" && !codexRunning
    ? {
      title: t("调试连接状态"),
      status: "waiting",
      ok: true,
      detail: [debuggerState.address, t("Codex 未运行，启动后自动连接")].filter(Boolean).join(" · "),
      icon: Bug,
    }
    : serviceItem(t("调试连接状态"), debuggerState, Bug);
  return [
    {
      title: t("Codex 应用"),
      status: overview?.codex_app.status ?? "not_checked",
      ok: overview?.codex_app.status === "found",
      detail: overview?.codex_app.path || t("尚未检查 Codex 应用路径。"),
      icon: AppWindow,
    },
    serviceItem(t("本地服务器状态"), overview?.runtime_health?.local_server, Server),
    conversionHealth,
    debuggerHealth,
  ];
}


export function normalizeSettings(settings: BackendSettings): BackendSettings {
  const backendAggregates = new Map(
    (settings.aggregateRelayProfiles ?? []).map((aggregate) => [aggregate.id, aggregate] as const),
  );
  const splitCommon = splitContextConfigText(settings.relayCommonConfigContents || "");
  const relayCommonConfigContents = splitCommon.common;
  const relayContextConfigContents = joinTomlSectionsRootFirst([
    settings.relayContextConfigContents || "",
    splitCommon.context,
  ]);
  const defaultContextSelection = contextSelectionForAllEntries({
    ...settings,
    relayCommonConfigContents,
    relayContextConfigContents,
  });
  const profiles =
    settings.relayProfiles?.length
      ? settings.relayProfiles.map((profile) =>
          normalizeRelayProfile(hydrateAggregateRelayProfile(profile, backendAggregates.get(profile.id)), defaultContextSelection),
        )
      : [
          {
            id: settings.activeRelayId || "default",
            name: t("默认中转"),
            model: "",
            baseUrl: settings.relayBaseUrl || defaultSettings.relayBaseUrl,
            upstreamBaseUrl: settings.relayBaseUrl || defaultSettings.relayBaseUrl,
            apiKey: settings.relayApiKey || "",
            protocol: "responses" as RelayProtocol,
            relayMode: "official" as RelayMode,
            sessionProvider: "custom" as RelaySessionProvider,
            officialMixApiKey: false,
            hideOfficialUsageAlert: false,
            testModel: "",
            configContents: "",
            authContents: "",
            useCommonConfig: true,
            contextSelection: defaultContextSelection,
            contextSelectionInitialized: true,
            contextWindow: "",
            autoCompactLimit: "",
            modelList: "",
            modelWindows: "",
            modelAutoCompact: "",
            modelMetadata: "",
            modelVlm: "",
            vlmApiKey: "",
            vlmModel: "",
            vlmBaseUrl: "",
            userAgent: "",
            customHeaders: [],
            sub2apiEnabled: false,
            noAuth: false,
            sub2apiMultiplier: "",
            standardOpenaiProtocol: false,
            webSearchHistoryCompat: false,
          },
        ];
  const activeRelayId = profiles.some((profile) => profile.id === settings.activeRelayId)
    ? settings.activeRelayId
    : profiles[0]?.id || "default";
  return syncLegacyRelayFields({
    ...defaultSettings,
    ...settings,
    usageHistoryEnabled: settings.usageHistoryEnabled === true,
    ...normalizeWhaleBalanceSettings(settings),
    ccsDbPath: (settings.ccsDbPath || "").trim(),
    weixinConnectReasoningEffort: (settings.weixinConnectReasoningEffort || "").trim(),
    weixinConnectProjectId: (settings.weixinConnectProjectId || "").trim(),
    dictation: normalizeDictationSettings(settings.dictation),
    relayProfilesEnabled: settings.relayProfilesEnabled !== false,
    protocolProxyAutoRetry: settings.protocolProxyAutoRetry !== false,
    // 默认关闭（opt-in），只有显式开启才生效。
    protocolProxyMapUnlistedModels: settings.protocolProxyMapUnlistedModels === true,
    protocolProxyAutoRetryMaxAttempts: clampNumber(settings.protocolProxyAutoRetryMaxAttempts ?? 5, 0, 20),
    protocolProxyAutoRetryMode:
      settings.protocolProxyAutoRetryMode === "buffered"
        ? "buffered"
        : settings.protocolProxyAutoRetryMode === "streaming-legacy"
          ? "streaming-legacy"
          : "streaming",
    protocolProxyAutoRetryCommitDelayMs: clampNumber(settings.protocolProxyAutoRetryCommitDelayMs ?? 1000, 0, 10000),
    protocolProxyAutoRetryIntervalSeconds: clampNumber(settings.protocolProxyAutoRetryIntervalSeconds ?? 2, 0, 30),
    protocolProxyAutoRetryStatusCodes: normalizeRetryStatusCodes(settings.protocolProxyAutoRetryStatusCodes),
    helperMaxRequestBodyMib: clampNumber(settings.helperMaxRequestBodyMib ?? 128, 32, 512),
    // 未设置（null / 缺失）表示用后端默认 120；设置了就按后端同样的 120~3600 区间收口。
    upstreamStreamHeaderTimeoutSecs:
      typeof settings.upstreamStreamHeaderTimeoutSecs === "number"
        ? clampNumber(settings.upstreamStreamHeaderTimeoutSecs, 120, 3600)
        : null,
    codexAppDreamSkinPaused: settings.codexAppDreamSkinPaused === true,
    codexAppDreamSkinThemeConfig: normalizeDreamSkinTheme(settings.codexAppDreamSkinThemeConfig),
    codexAppDreamSkinImagePath: (settings.codexAppDreamSkinImagePath || "").trim(),
    codexAppStepwiseProtocol: normalizeStepwiseProtocol(settings.codexAppStepwiseProtocol),
    weixinConnectAllowOutboundAttachments: settings.weixinConnectAllowOutboundAttachments === true,
    codexAppStepwiseGenerationMode: normalizeStepwiseGenerationMode(settings.codexAppStepwiseGenerationMode),
    codexAppStepwiseMaxItems: clampNumber(settings.codexAppStepwiseMaxItems ?? 4, 0, 6),
    codexAppStepwiseMaxInputChars: clampNumber(settings.codexAppStepwiseMaxInputChars || 6000, 1000, 24000),
    codexAppStepwiseMaxOutputTokens: clampNumber(settings.codexAppStepwiseMaxOutputTokens || 500, 100, 4000),
    codexAppStepwiseTimeoutMs: clampNumber(settings.codexAppStepwiseTimeoutMs || 8000, 1000, 60000),
    relayCommonConfigContents,
    relayContextConfigContents,
    relayProfiles: profiles,
    activeRelayId,
  });
}

function normalizeStepwiseProtocol(value: StepwiseProtocol | undefined): StepwiseProtocol {
  return value === "auto"
    || value === "responses"
    || value === "anthropic_messages"
    ? value
    : "chat_completions";
}

/** 与 Rust `normalize_protocol_proxy_auto_retry_status_codes` 保持一致：400~599、去重升序、最多 20 个。 */
export function normalizeRetryStatusCodes(value: unknown): number[] {
  if (!Array.isArray(value)) return [];
  const codes = value
    .map(Number)
    .filter((status) => Number.isInteger(status) && status >= 400 && status <= 599);
  return Array.from(new Set(codes)).sort((a, b) => a - b).slice(0, 20);
}

export function clampNumber(value: number, min: number, max: number): number {
  if (!Number.isFinite(value)) return min;
  return Math.min(max, Math.max(min, Math.round(value)));
}

function normalizeStepwiseGenerationMode(value: StepwiseGenerationMode | undefined): StepwiseGenerationMode {
  return value === "manual" ? "manual" : "auto";
}

/// 列表标签用的短协议名：卡片空间小，「转 Responses」留在编辑页里说明。
export function relayProtocolShortLabel(protocol: RelayProtocol): string {
  if (protocol === "chatCompletions") return "Chat Completions";
  if (protocol === "anthropicMessages") return "Anthropic";
  return "Responses";
}

/// 接入方式标签的配色分组。
export function relayProfileTagTone(profile: RelayProfile): string {
  if (isAggregateRelayProfile(profile)) return "aggregate";
  if (profile.relayMode === "official") return profile.officialMixApiKey ? "mixed" : "official";
  return "api";
}

export function relayProtocolLabel(protocol: RelayProtocol): string {
  if (protocol === "chatCompletions") return t("Chat Completions 转 Responses");
  if (protocol === "anthropicMessages") return t("Anthropic Messages 转 Responses");
  return "Responses API";
}

function relayModeLabel(mode: RelayMode): string {
  if (mode === "aggregate") return t("聚合供应商");
  if (mode === "pureApi") return t("纯 API");
  return t("官方登录");
}

export function relayEditorMode(profile: Pick<RelayProfile, "relayMode" | "officialMixApiKey">): RelayEditorMode {
  if (profile.relayMode === "pureApi") return "pureApi";
  if (profile.relayMode === "mixedApi" || profile.officialMixApiKey) return "mixedApi";
  return "official";
}

export function relayProfileModeLabel(profile: Pick<RelayProfile, "relayMode" | "officialMixApiKey" | "aggregate">): string {
  if (profile.relayMode === "aggregate") {
    return profile.aggregate?.modelAggregationEnabled === true ? t("模型聚合") : t("故障转移");
  }
  return relayEditorMode(profile) === "mixedApi" ? t("官方登录 + API") : relayModeLabel(profile.relayMode);
}

export function relayProfileConfigBrief(profile: RelayProfile): string {
  if (isAggregateRelayProfile(profile)) {
    const aggregate = normalizeAggregateConfig(profile.aggregate, []);
    // 模型聚合与轮转策略无关，卡片上不显示「失败切换」这类策略文案。
    if (aggregate.modelAggregationEnabled === true) {
      return tf("模型固定绑定 · {0} 个成员", [aggregate.members.length]);
    }
    return tf("{0} · {1} 个成员", [aggregateStrategyLabel(aggregate.strategy), aggregate.members.length]);
  }
  if (profile.relayMode === "official") return profile.officialMixApiKey ? t("混入 API Key") : t("不写 API 文件");
  return profile.baseUrl || t("未填写 URL");
}

export function formatTime(value: number) {
  if (!value) return "-";
  return new Date(value).toLocaleString("zh-CN");
}

export function stringifyError(error: unknown) {
  if (error instanceof Error) return error.message;
  return String(error);
}
