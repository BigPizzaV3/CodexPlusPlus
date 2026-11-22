// 设置页：整体设置、关于与更新、Codex 设置与安装维护、网关共用服务设置。
import { invoke } from "@tauri-apps/api/core";
import { Copy, Download, Github, ExternalLink, Play, MessageCircle, RefreshCw, Search, Upload } from "lucide-react";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { ConfigDoctorPanel } from "./ConfigDoctorPanel";
import { Switch } from "./gateway-ui";
import { AppSelect, type AppSelectOption } from "./AppSelect";
import { DisplaySettings } from "./DisplaySettings";
import { diagnosticFacts, filterLogEntries, formatLogTime, parseLogText } from "./log-view-model";
import { TopbarActions } from "./TopbarActions";
import type { AgentFocus } from "./agent-focus-model";
import { ReleaseNotes } from "./ReleaseNotes";
import { Menu, MenuItem } from "./ManagementPages";
import { t, tf } from "@/i18n";
import type { ThemePreference } from "./theme-mode";
import { CxBadge, CxCard, CxEmpty, CxFilters, CxRow, CxSection } from "./cx-layout";
import type {
  Actions,
  AutoTakeoverResult,
  BackendSettings,
  DiagnosticsResult,
  LogsResult,
  OverviewResult,
  SettingsResult,
  TaskProgress,
  Theme,
  UpdateDownloadState,
  UpdateResult,
  WatcherResult,
} from "./app-types";
import {
  APP_LOGO_URL,
  autoTakeoverView,
  clampNumber,
  healthItems,
  normalizeRetryStatusCodes,
  SettingsAutoSave,
  StatusBadge,
  statusLabel,
  stringifyError,
  TaskProgressBox,
  ToggleVisual,
} from "./app-shared";

/** 设置页里「使用模式」一行的说明。 */
function agentFocusDescription(focus: AgentFocus): string {
  if (focus === "codex") return t("只管理 Codex：隐藏其他 Agent 的界面，已有网关连接保持不变。");
  if (focus === "enhance") return t("只用增强功能：隐藏供应商、网关和用量，关闭旧版供应商配置切换。已有网关连接仍会运行并管理对应配置。");
  return t("全部功能：同时管理 Codex、Claude Code、Grok 等多个 Agent。只用 Codex 或只用增强功能的话，可以切换模式隐藏用不到的内容。");
}

/**
 * 新旧两套共用的本地服务设置：服务状态、请求体上限、请求避震。放在「网关 → 网关设置」，
 * 按网关设置的行样式排版；改的是 settings.json（随主窗口的设置一起自动保存）。
 */
export function SharedServiceSettings({
  overview,
  form,
  onFormChange,
  autoSave,
}: {
  overview: OverviewResult | null;
  form: BackendSettings;
  onFormChange: (value: BackendSettings) => void;
  /** 自动保存状态，放在第一个分组标题的右侧。 */
  autoSave?: ReactNode;
}) {
  const [retryStatusInput, setRetryStatusInput] = useState("");
  const setRetryStatusCodes = (codes: number[]) =>
    onFormChange({ ...form, protocolProxyAutoRetryStatusCodes: normalizeRetryStatusCodes(codes) });
  const addRetryStatusCodes = () => {
    const next = retryStatusInput.split(/[,，\s]+/).map(Number);
    if (!normalizeRetryStatusCodes(next).length) return;
    setRetryStatusCodes([...form.protocolProxyAutoRetryStatusCodes, ...next]);
    setRetryStatusInput("");
  };
  // 本地服务的地址与运行状态在上面「监听地址」已经有了，这里只列协议转换与调试连接。
  const health = healthItems(overview).filter((item) => item.title === t("协议转换") || item.title === t("调试连接状态"));
  const tone = (item: (typeof health)[number]) => (["running", "connected", "ok"].includes(item.status) ? "ok" : item.ok ? undefined : "warning");
  const number = (value: number, min: number, max: number, unit: string, apply: (next: number) => void, step = 1) => (
    <label className="gw-unit-input">
      <input
        max={max}
        min={min}
        onChange={(event) => apply(clampNumber(Number(event.currentTarget.value), min, max))}
        step={step}
        type="number"
        value={value}
      />
      <span>{unit}</span>
    </label>
  );
  const retry = form.protocolProxyAutoRetry;
  const retryModeLabel = form.protocolProxyAutoRetryMode === "buffered" ? t("整段缓冲") : form.protocolProxyAutoRetryMode === "streaming-legacy" ? t("流式（旧行为）") : t("流式（推荐）");
  const sharedHint = t("网关和旧版本地协议代理共用这些设置");
  // 本地服务一节、请求避震一节。避震的五个参数默认值够用，收进折叠区，摘要里显示当前值。
  return (
    <div className="gw-shared-settings">
      {autoSave ? <TopbarActions><span className="cx-set-autosave">{autoSave}</span></TopbarActions> : null}
      <CxSection title={<span title={sharedHint}>{t("本地服务")}</span>}>
        <CxCard>
          {health.map((item) => (
            <CxRow detail={item.detail} key={item.title} title={item.title} tooltip={item.detail}>
              <CxBadge tone={tone(item) === "ok" ? "ok" : tone(item) === "warning" ? "warn" : "neutral"}>{statusLabel(item.status)}</CxBadge>
            </CxRow>
          ))}
          <CxRow
            detail={t("长上下文会话超过上限会报 413；调大会增加本地服务的内存占用。")}
            title={t("请求体上限")}
            tooltip={t("默认 128 MiB，可填 32~512。长上下文会话（1M 窗口 + 截图历史）的请求体会超过旧的 32 MiB 硬上限并触发 413；调大后本地代理处理单次请求的内存占用也会增加。保存后立即对本地代理生效；环境变量 CODEX_PLUS_MAX_HTTP_BODY_BYTES（字节）优先于本设置。")}
          >
            {number(form.helperMaxRequestBodyMib, 32, 512, "MiB", (next) => onFormChange({ ...form, helperMaxRequestBodyMib: next }), 16)}
          </CxRow>
        </CxCard>
      </CxSection>

      <CxSection title={<span title={sharedHint}>{t("请求避震")}</span>}>
        <CxCard>
          <CxRow
            detail={retry
              ? t("在本地服务内消化上游过载与限流，尽量不把错误交给 Agent。作用于经过网关和旧版本地协议代理的请求。")
              : t("已关闭：上游过载或限流时，错误直接交给 Agent。")}
            title={t("启用请求避震")}
          >
            <Switch label={t("启用请求避震")} on={retry} onToggle={() => onFormChange({ ...form, protocolProxyAutoRetry: !retry })} />
          </CxRow>
          {retry ? (
            <details className="cx-set-disclosure">
              <summary>
                {t("重试参数")}
                <span className="cx-set-summary-hint">
                  {[tf("最多 {0} 次", [form.protocolProxyAutoRetryMaxAttempts]), tf("间隔 {0} 秒", [form.protocolProxyAutoRetryIntervalSeconds]), retryModeLabel].join(" · ")}
                </span>
              </summary>
              <CxRow detail={t("0–20 次；次数用尽后把错误交给 Agent。")} title={t("最多自动重试次数")}>
                {number(form.protocolProxyAutoRetryMaxAttempts, 0, 20, t("次"), (next) => onFormChange({ ...form, protocolProxyAutoRetryMaxAttempts: next }))}
              </CxRow>
              <CxRow detail={t("之后按 2 倍退避，单次最长 30 秒；有 Retry-After 时优先采用。")} title={t("首次重试间隔")}>
                {number(form.protocolProxyAutoRetryIntervalSeconds, 0, 30, t("秒"), (next) => onFormChange({ ...form, protocolProxyAutoRetryIntervalSeconds: next }))}
              </CxRow>
              <CxRow
                detail={form.protocolProxyAutoRetryMode === "buffered"
                  ? t("始终由代理重试，但要等整段生成完才显示。")
                  : form.protocolProxyAutoRetryMode === "streaming-legacy"
                    ? t("旧版流式行为，仅在新版有问题时使用。")
                    : t("保留实时输出；正文出现前遇到过载由代理重试。")}
                title={t("避震模式")}
                tooltip={t("流式：保留实时输出，正文提交前遇到过载由代理重试，之后交给 Codex；「首字提交延迟」为出现正文后再缓冲的时长，0 表示立即提交。整段缓冲：始终由代理重试，但要等整段生成完才显示。")}
              >
                <AppSelect
                  className="cx-set-select"
                  label={t("避震模式")}
                  onChange={(next) => onFormChange({ ...form, protocolProxyAutoRetryMode: next })}
                  options={[
                    { value: "streaming", label: t("流式（推荐）"), hint: t("保留实时输出；正文出现前遇到过载由代理重试。") },
                    { value: "streaming-legacy", label: t("流式（旧行为）"), hint: t("旧版流式行为，仅在新版有问题时使用。") },
                    { value: "buffered", label: t("整段缓冲"), hint: t("始终由代理重试，但要等整段生成完才显示。") },
                  ]}
                  size="sm"
                  value={form.protocolProxyAutoRetryMode}
                  menuWidth={300}
                />
              </CxRow>
              {form.protocolProxyAutoRetryMode === "streaming" ? (
                <CxRow detail={t("出现正文后再缓冲的时长，0 表示立即提交。")} title={t("首字提交延迟")}>
                  {number(form.protocolProxyAutoRetryCommitDelayMs, 0, 10000, "ms", (next) => onFormChange({ ...form, protocolProxyAutoRetryCommitDelayMs: next }), 100)}
                </CxRow>
              ) : null}
              <CxRow detail={t("命中时退避重发。默认不选；常用 429（限流）和 5xx（服务端故障）。")} title={t("重试的 HTTP 状态码")}>
                <button
                  className="cx-secondary"
                  onClick={() => setRetryStatusCodes([...form.protocolProxyAutoRetryStatusCodes, ...COMMON_RETRY_STATUS_CODES])}
                  type="button"
                >
                  {t("填入常用")}
                </button>
              </CxRow>
              {/* 状态码编辑不能放进 <label>：WebKit 会把点击转发给第一个按钮，一次删掉两个状态码。 */}
              <div className="cx-set-block">
                <div className="gw-retry-codes" role="group" aria-label={t("重试的 HTTP 状态码")}>
                  {form.protocolProxyAutoRetryStatusCodes.map((status) => (
                    <button
                      className="gw-chip gw-retry-code"
                      key={status}
                      onClick={() => setRetryStatusCodes(form.protocolProxyAutoRetryStatusCodes.filter((item) => item !== status))}
                      title={t("点击移除")}
                      type="button"
                    >
                      {status} ×
                    </button>
                  ))}
                  <input
                    aria-label={t("输入状态码后回车")}
                    className="gw-retry-input"
                    inputMode="numeric"
                    onChange={(event) => setRetryStatusInput(event.currentTarget.value)}
                    onKeyDown={(event) => {
                      if (event.key === "Enter") {
                        event.preventDefault();
                        addRetryStatusCodes();
                      }
                    }}
                    placeholder={t("加状态码，回车")}
                    value={retryStatusInput}
                  />
                </div>
              </div>
            </details>
          ) : null}
        </CxCard>
      </CxSection>
    </div>
  );
}

export function MaintenanceScreen({
  overview,
  launchPending,
  watcher,
  autoTakeover,
  settings,
  launchForm,
  onLaunchFormChange,
  actions,
  advanced = null,
}: {
  overview: OverviewResult | null;
  launchPending: boolean;
  watcher: WatcherResult | null;
  autoTakeover: AutoTakeoverResult | null;
  settings: SettingsResult | null;
  launchForm: { appPath: string; debugPort: string; helperPort: string };
  onLaunchFormChange: (next: { appPath: string; debugPort: string; helperPort: string }) => void;
  actions: Actions;
  /** 「高级」卡片里排在手动启动前面的内容（额外启动参数）。 */
  advanced?: ReactNode;
}) {
  const savedCodexAppPath = settings?.settings.codexAppPath ?? "";
  const takeoverOn = Boolean(settings?.settings.codexAppAutoTakeover);
  const takeover = autoTakeoverView(autoTakeover);
  return (
    <>
      {/* Codex 应用一行：识别到的路径 + 状态；换路径、清除保存的路径都在它的菜单里（原来「检查与修复」「应用路径」两节显示同一个路径三次）。 */}
      <CxSection
        aside={<button className="cx-secondary" onClick={() => void actions.checkHealth()} type="button"><RefreshCw />{t("检查")}</button>}
        id="codex-settings-check"
        title={t("应用与入口")}
      >
        <CxCard>
          <CxRow
            badges={savedCodexAppPath ? <CxBadge title={savedCodexAppPath}>{t("已手动指定")}</CxBadge> : null}
            detail={<code className="cx-set-path">{overview?.codex_app.path || savedCodexAppPath || t("未记录路径")}</code>}
            title={t("Codex 应用")}
            tooltip={t("免安装版或解包版只需要选择一次，之后从界面启动会自动复用")}
          >
            <StatusBadge status={overview?.codex_app.status} />
            <Menu>
              <MenuItem onClick={() => void actions.chooseCodexAppPath("folder")}>{t("选择应用目录")}</MenuItem>
              <MenuItem onClick={() => void actions.chooseCodexAppPath("file")}>{t("选择 Codex.exe")}</MenuItem>
              <MenuItem danger disabled={!savedCodexAppPath} onClick={() => void actions.clearCodexAppPath()}>{t("清除保存路径")}</MenuItem>
            </Menu>
          </CxRow>
          <CxRow detail={<code className="cx-set-path">{overview?.management_shortcut.path || t("未记录路径")}</code>} title={t("Codex++ 应用入口")}>
            <StatusBadge status={overview?.management_shortcut.status} />
            <Menu>
              <MenuItem onClick={() => void actions.repairShortcuts()}>{t("修复快捷方式")}</MenuItem>
            </Menu>
          </CxRow>
        </CxCard>
      </CxSection>
      <CxSection id="codex-settings-startup" title={t("开机启动")}>
        {/* 「登录时」的两个子选项只在开启登录打开后出现；「打开时启动 Codex」对手动打开也生效，单独一行。 */}
        <CxCard className="cx-set-switches">
          <label className="switch-row" title={t("开启后注册系统登录项，关闭时移除；是否自动启动 Codex 由下面的独立开关决定。")}>
            <input
              checked={Boolean(watcher?.enabled)}
              disabled={!watcher}
              onChange={(event) => void actions.setWatcherEnabled(event.currentTarget.checked)}
              type="checkbox"
            />
            <span>
              <strong>{t("登录时自动打开")}</strong>
              <small>{t("登录系统后打开 Codex++")}</small>
            </span>
            <ToggleVisual />
          </label>
          {watcher?.enabled ? (
            <>
              <label className="switch-row cx-set-sub" title={t("仅登录启动 Codex++ 时自动启动 Codex；手动打开 Codex++ 仍不会自动启动。")}>
                <input
                  checked={Boolean(watcher?.start_codex)}
                  disabled={!watcher}
                  onChange={(event) => void actions.setWatcherStartCodex(event.currentTarget.checked)}
                  type="checkbox"
                />
                <span>
                  <strong>{t("登录时自动启动 Codex")}</strong>
                  <small>{t("只在开机登录打开时生效")}</small>
                </span>
                <ToggleVisual />
              </label>
              <label className="switch-row cx-set-sub" title={t("仅登录启动时隐藏到托盘；手动打开 Codex++ 仍显示主窗口。")}>
                <input
                  checked={Boolean(watcher?.start_minimized)}
                  disabled={!watcher}
                  onChange={(event) => void actions.setWatcherStartMinimized(event.currentTarget.checked)}
                  type="checkbox"
                />
                <span>
                  <strong>{t("启动时自动最小化")}</strong>
                  <small>{t("开机登录时只留在托盘")}</small>
                </span>
                <ToggleVisual />
              </label>
            </>
          ) : null}
          <label className="switch-row" title={t("仅手动打开 Codex++ 时生效；已有 Codex 会先恢复，不会重复启动。")}>
            <input
              checked={Boolean(watcher?.start_codex_on_open)}
              disabled={!watcher}
              onChange={(event) => void actions.setWatcherStartCodexOnOpen(event.currentTarget.checked)}
              type="checkbox"
            />
            <span>
              <strong>{t("打开 Codex++ 时自动启动 Codex")}</strong>
              <small>{t("已有 Codex 会先恢复，不会重复启动")}</small>
            </span>
            <ToggleVisual />
          </label>
        </CxCard>
      </CxSection>
      <CxSection id="codex-settings-takeover" title={t("自动接管官方 App")}>
        <CxCard className="cx-set-switches">
          <label
            className="switch-row"
            title={`${t("开启后，你自己启动官方 App 时 Codex++ 会自动接管。官方 App 已在运行且没有调试端口时，会先退出再带参数重开（可能中断正在进行的操作）。")}\n${t("仅在 Codex++ 正在运行时生效；建议同时开启上面的「开机打开 Codex++」。")}`}
          >
            <input
              checked={takeoverOn}
              onChange={(event) => void actions.setAutoTakeover(event.currentTarget.checked)}
              type="checkbox"
            />
            <span>
              <strong>{t("开启自动接管")}</strong>
              <small>{t("你自己双击启动官方 App 时，Codex++ 自动接管并注入增强")}</small>
            </span>
            <ToggleVisual />
          </label>
          {takeoverOn ? (
            <CxRow
              badges={<StatusBadge status={takeover.status} label={takeover.label} />}
              detail={takeover.detail}
              title={t("接管状态")}
              tooltip={t("仅在 Codex++ 正在运行时生效；需要重启官方 App 时会自动重启。")}
            >
              <button className="cx-secondary" onClick={() => void actions.takeOverNow()} type="button">{t("立即接管")}</button>
              <Menu>
                <MenuItem onClick={() => void actions.pauseAutoTakeover()}>{t("本次不接管")}</MenuItem>
                <MenuItem onClick={() => void actions.refreshAutoTakeover()}>{t("刷新状态")}</MenuItem>
              </Menu>
            </CxRow>
          ) : null}
        </CxCard>
      </CxSection>
      <CxSection id="codex-settings-manual-launch" title={t("高级")}>
        <CxCard>
          {advanced}
          {/* 给少数人用：用别的路径或端口启动一次。默认收起。 */}
          <details className={`cx-set-disclosure ${advanced ? "" : "cx-set-disclosure-first"}`.trim()}>
            <summary>{t("手动启动")}<span className="cx-set-summary-hint">{t("应用路径留空时使用已保存路径；没有保存路径时使用自动探测")}</span></summary>
            <CxRow title={t("应用路径覆盖")}>
              <Input
                className="cx-set-input-wide"
                value={launchForm.appPath}
                onChange={(event) => onLaunchFormChange({ ...launchForm, appPath: event.currentTarget.value })}
                placeholder={savedCodexAppPath || t("例如 C:\\Program Files\\WindowsApps\\OpenAI.Codex...\\app")}
              />
            </CxRow>
            <CxRow title={t("Debug 端口")}>
              <Input
                className="cx-set-input-port"
                value={launchForm.debugPort}
                onChange={(event) => onLaunchFormChange({ ...launchForm, debugPort: event.currentTarget.value })}
              />
            </CxRow>
            <CxRow title={t("Helper 端口")}>
              <Input
                className="cx-set-input-port"
                value={launchForm.helperPort}
                onChange={(event) => onLaunchFormChange({ ...launchForm, helperPort: event.currentTarget.value })}
              />
            </CxRow>
            <div className="cx-set-footer">
              <button className="cx-secondary" onClick={() => void actions.saveManualCodexAppPath()} type="button">{t("保存为默认路径")}</button>
              <button className="cx-primary" disabled={launchPending} onClick={() => void actions.launch()} type="button"><Play />{t("启动 Codex")}</button>
            </div>
          </details>
        </CxCard>
      </CxSection>
    </>
  );
}

export function AboutScreen({
  sections,
  overview,
  update,
  updateDownload = null,
  updateInstallProgress,
  logs,
  diagnostics,
  actions,
  form,
  onFormChange,
}: {
  /** 当前显示的设置分类里的卡片 id；其余卡片不渲染。 */
  sections: string[];
  form: BackendSettings;
  onFormChange: (value: BackendSettings) => void;
  overview: OverviewResult | null;
  update: UpdateResult | null;
  /** 后台下载更新的状态（「有可用更新时自动下载」）。 */
  updateDownload?: UpdateDownloadState | null;
  updateInstallProgress: TaskProgress;
  logs: LogsResult | null;
  diagnostics: DiagnosticsResult | null;
  actions: Actions;
}) {
  const version = overview?.current_version ?? update?.currentVersion ?? "-";
  const openUrl = (url: string) => void actions.openExternalUrl(url);
  const shown = (id: string) => sections.includes(id);
  return (
    <>
      {shown("app-settings-update") ? <div className="cx-set-group" id="app-settings-update">
        <CxSection>
          <CxCard>
            {/* 一行摘要 + 一个操作：有新版本时主按钮是「安装」（已在后台下好）或「下载并安装」，检查更新退到旁边。 */}
            {(() => {
              const latest = update?.updateAvailable ? update.latestVersion ?? "" : "";
              const download = latest && updateDownload?.version === latest ? updateDownload : null;
              const ready = Boolean(latest) && (update?.downloaded === true || download?.state === "ready");
              const downloading = !ready && download?.state === "downloading";
              const detail = latest
                ? ready
                  ? t("新版本已下载，点「安装」更新；安装时 Codex++ 会关闭并自动重启")
                  : downloading
                    ? download?.percent != null
                      ? tf("正在后台下载 {0}%，下完不会自动安装", [Math.round(download.percent)])
                      : t("正在后台下载，下完不会自动安装")
                    : download?.state === "failed"
                      ? t("后台下载没成功，点「下载并安装」重试")
                      : t("从 GitHub Release 下载安装包更新")
                : update?.latestVersion
                  ? t("已是最新版本")
                  : t("尚未检查更新");
              return (
                <CxRow
                  badges={latest ? <CxBadge tone="ok">{ready ? tf("{0} 已下载", [latest]) : tf("有新版本 {0}", [latest])}</CxBadge> : null}
                  detail={<>{detail}{latest && update?.assetName ? ` · ${update.assetName}` : ""}</>}
                  icon={<img alt="" src={APP_LOGO_URL} />}
                  title={`Codex++ ${version}`}
                  tooltip={download?.state === "failed" ? download.message : undefined}
                >
                  <button className="cx-secondary" onClick={() => void actions.checkUpdate()} type="button"><RefreshCw />{t("检查更新")}</button>
                  {latest ? (
                    <button className="cx-primary" disabled={updateInstallProgress.active} onClick={() => void actions.performUpdate()} type="button">
                      <Download />
                      {updateInstallProgress.active ? t("正在安装…") : ready ? t("安装") : t("下载并安装")}
                    </button>
                  ) : null}
                </CxRow>
              );
            })()}
            {/* 改的是设置草稿，由同页 GlobalSettingsScreen 的自动保存落盘（#1272）。 */}
            <CxRow
              detail={t("打开时及运行期间每隔几小时检查一次")}
              title={t("启动时自动检查更新")}
              tooltip={t("打开 Codex++ 时及运行期间每隔几小时在后台检查新版本并提示。关闭后不再自动检查，仍可随时点「检查更新」。")}
            >
              <Switch label={t("启动时自动检查更新")} on={form.updateAutoCheck !== false} onToggle={() => onFormChange({ ...form, updateAutoCheck: form.updateAutoCheck === false })} />
            </CxRow>
            <CxRow
              detail={t("在后台把安装包下好，但不安装；点「安装」才更新")}
              title={t("有可用更新时自动下载")}
              tooltip={t("检查到新版本后，在后台下载安装包并校验，下好后这里和侧栏会提示「安装」。不会自动安装，也不会重启 Codex++。")}
            >
              <Switch label={t("有可用更新时自动下载")} on={form.updateAutoDownload !== false} onToggle={() => onFormChange({ ...form, updateAutoDownload: form.updateAutoDownload === false })} />
            </CxRow>
            {update?.updateAvailable && update.releaseSummary ? (
              <details className="app-update-notes cx-set-disclosure">
                <summary>{tf("{0} 更新说明", [update.latestVersion ?? ""])}</summary>
                <div className="cx-set-block"><ReleaseNotes markdown={update.releaseSummary} onOpenUrl={openUrl} /></div>
              </details>
            ) : null}
          </CxCard>
          <TaskProgressBox completedTitle={t("上次更新结果")} progress={updateInstallProgress} title={t("安装包更新进度")} />
        </CxSection>
      </div> : null}
      {shown("app-settings-about") ? <div className="cx-set-group" id="app-settings-about">
        <CxSection>
          <CxCard>
            <CxRow detail={t("本地 Codex 增强、管理工具和安装包维护")} icon={<img alt="" src={APP_LOGO_URL} />} title={`Codex++ ${version}`} />
            <CxRow title={t("Codex 版本")}>
              <span className="cx-set-value">{overview?.codex_version ?? t("未检测到")}</span>
            </CxRow>
            <CxRow detail="github.com/BigPizzaV3/CodexPlusPlus" onClick={() => openUrl("https://github.com/BigPizzaV3/CodexPlusPlus")} title={t("项目地址")}>
              <ExternalLink aria-hidden="true" className="cx-set-chevron" />
            </CxRow>
          </CxCard>
        </CxSection>
        <CxSection title={t("反馈与社区")}>
          <CxCard>
            <CxRow detail={t("在 GitHub Issues 报告问题或提建议")} icon={<Github />} onClick={() => openUrl("https://github.com/BigPizzaV3/CodexPlusPlus/issues")} title={t("反馈问题")}>
              <ExternalLink aria-hidden="true" className="cx-set-chevron" />
            </CxRow>
            <CxRow detail="discord.gg/y96kX7A76v" icon={<MessageCircle />} onClick={() => openUrl("https://discord.gg/y96kX7A76v")} title="Discord">
              <ExternalLink aria-hidden="true" className="cx-set-chevron" />
            </CxRow>
            <CxRow detail="t.me/CodexPlusPlus" icon={<MessageCircle />} onClick={() => openUrl("https://t.me/CodexPlusPlus")} title="Telegram">
              <ExternalLink aria-hidden="true" className="cx-set-chevron" />
            </CxRow>
          </CxCard>
        </CxSection>
      </div> : null}
      {shown("app-settings-logs") ? <div className="cx-set-group" id="app-settings-logs"><LogsPanel logs={logs} actions={actions} /></div> : null}
      {shown("app-settings-diagnostics") ? <div className="cx-set-group" id="app-settings-diagnostics"><DiagnosticsPanel diagnostics={diagnostics} actions={actions} /></div> : null}
    </>
  );
}

/// Codex 的额外启动参数：设置模式「Codex」分类（agentSettings）里，维护页最后「高级」卡片的内容。
/// 应用级选项在整体设置里，见 [GlobalSettingsScreen]。
export function AgentSettingsScreen({
  dirty,
  form,
  onFormChange,
  actions,
}: {
  dirty: boolean;
  form: BackendSettings;
  onFormChange: (value: BackendSettings) => void;
  actions: Actions;
}) {
  const count = form.codexExtraArgs.length;
  return (
    <>
      <TopbarActions><SettingsAutoSave className="cx-set-autosave" dirty={dirty} form={form} actions={actions} /></TopbarActions>
      {/* 放在「高级」卡片里，默认收起；填了参数时标题旁显示条数，不用展开也知道有没有设。 */}
      <details className="cx-set-disclosure cx-set-disclosure-first" open={count > 0 || undefined}>
        <summary>
          {t("额外启动参数")}
          <span className="cx-set-summary-hint">{count ? tf("已设 {0} 个", [count]) : t("启动 Codex App 时追加的命令行参数")}</span>
        </summary>
        <CxRow detail={t("每行一个参数，例如 --force_high_performance_gpu。不需要填写 open 或 --args。")} title={t("额外参数")} tooltip={t("启动 Codex App 时追加到默认 CDP 参数后。留空则保持默认启动行为。")} />
        <div className="cx-set-block">
          <Textarea
              aria-label={t("Codex 启动参数")}
              className="launch-args-input"
              placeholder="--force_high_performance_gpu"
              spellCheck={false}
              value={codexExtraArgsToInput(form.codexExtraArgs)}
              onChange={(event) =>
                onFormChange({
                  ...form,
                  codexExtraArgs: inputToCodexExtraArgs(event.currentTarget.value),
                })
              }
            />
        </div>
      </details>
    </>
  );
}

/// 托盘快捷面板开关。值仍存在 gateway.json 的 `trayPanel`（老版本在网关设置里改过的选择照样有效），
/// 由后端命令读写，后端同时更新托盘行为和内存缓存。
/** 设置页里直接读写后端的一个开关（托盘快捷面板、轻量模式）：读不到时按开着显示。 */
function useBackendToggle(readCommand: string, writeCommand: string) {
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    let cancelled = false;
    void invoke<boolean>(readCommand)
      .then((value) => { if (!cancelled) setEnabled(value !== false); })
      .catch(() => { if (!cancelled) setEnabled(true); });
    return () => { cancelled = true; };
  }, [readCommand]);
  const toggle = async () => {
    if (enabled === null || saving) return;
    setSaving(true);
    setError("");
    try {
      setEnabled(await invoke<boolean>(writeCommand, { enabled: !enabled }));
    } catch (failure) {
      setError(stringifyError(failure));
    } finally {
      setSaving(false);
    }
  };
  return { enabled, saving, error, toggle };
}


/// 整体设置页（最左侧齿轮）。
///
/// 只放与当前 Agent 无关的应用级选项，内容不随 Agent 切换而变。
/// Agent 专属选项放在各 Agent 菜单的「设置」里，见 [AgentSettingsScreen]。
export function GlobalSettingsScreen({
  sections,
  dirty,
  form,
  settings,
  theme,
  themePreference,
  actions,
  onOpenTour,
  agentFocus,
  onOpenAgentFocus,
}: {
  /** 当前显示的设置分类里的卡片 id；其余卡片不渲染。 */
  sections: string[];
  onOpenTour: () => void;
  agentFocus: AgentFocus;
  onOpenAgentFocus: () => void;
  dirty: boolean;
  form: BackendSettings;
  settings: SettingsResult | null;
  theme: Theme;
  themePreference: ThemePreference;
  actions: Actions;
  onOpenAgentSettings: () => void;
}) {
  const settingsPath = settings?.settings_path ?? "";
  const shown = (id: string) => sections.includes(id);
  const trayPanel = useBackendToggle("tray_panel_enabled", "set_tray_panel_enabled");
  const lightweight = useBackendToggle("lightweight_mode_enabled", "set_lightweight_mode_enabled");
  const themeOptions: AppSelectOption<ThemePreference>[] = [
    { value: "system", label: t("跟随系统") },
    { value: "light", label: t("浅色") },
    { value: "dark", label: t("深色") },
  ];
  // 参照 Cursor 设置页：页面标题就是分类名，每节一个卡片外的小标题 + 一张行卡片；
  // 说明只写一行，长说明放悬停提示。自动保存状态挂到标题栏，只在保存中 / 出错时出现。
  return (
    <>
      <TopbarActions><SettingsAutoSave className="cx-set-autosave" dirty={dirty} form={form} actions={actions} /></TopbarActions>
      {shown("app-settings-general") ? <div className="cx-set-group" id="app-settings-general">
        <CxSection title={t("外观")}>
          <CxCard>
            <CxRow
              title={t("界面主题")}
              detail={themePreference === "system"
                ? `${t("跟随系统外观，当前为")}${theme === "dark" ? t("深色") : t("浅色")}${t("模式。")}`
                : `${t("当前为")}${theme === "dark" ? t("深色") : t("浅色")}${t("模式。")}`}
            >
              <AppSelect ariaLabel={t("界面主题")} className="cx-set-select" onChange={(value) => actions.setThemePreference(value)} options={themeOptions} size="sm" value={themePreference} />
            </CxRow>
            <DisplaySettings />
          </CxCard>
        </CxSection>
        <CxSection title={t("窗口与托盘")}>
          <CxCard>
            <CxRow
              detail={trayPanel.error || t("单击托盘图标查看状态、启动 Codex、切换供应商")}
              title={t("点击托盘图标弹出快捷面板")}
              tooltip={t("面板里能看 Codex 状态、一键启动或重启、切换供应商、看用量，网关用户还能给各 Agent 换模型。右键仍是原来的菜单，双击打开主窗口；关掉恢复原来的点击行为。")}
            >
              <Switch disabled={trayPanel.enabled === null || trayPanel.saving} label={t("点击托盘图标弹出快捷面板")} on={trayPanel.enabled ?? true} onToggle={() => void trayPanel.toggle()} />
            </CxRow>
            <CxRow
              detail={lightweight.error || t("后台服务照常运行，再次打开时重新加载界面")}
              title={t("关闭窗口后释放界面内存")}
              tooltip={t("关闭主窗口时连同界面一起释放，后台服务、网关和托盘照常运行，从托盘再打开时重新加载界面。未保存的编辑内容会丢失；关掉则关闭窗口时只隐藏。")}
            >
              <Switch disabled={lightweight.enabled === null || lightweight.saving} label={t("关闭窗口后释放界面内存")} on={lightweight.enabled ?? false} onToggle={() => void lightweight.toggle()} />
            </CxRow>
          </CxCard>
        </CxSection>
        <CxSection title={t("入门")}>
          <CxCard>
            <CxRow detail={agentFocus === "codex" ? t("只管理 Codex") : agentFocus === "enhance" ? t("只用增强功能") : t("全部功能")} title={t("使用模式")} tooltip={agentFocusDescription(agentFocus)}>
              <button className="cx-secondary" onClick={onOpenAgentFocus} type="button">{t("修改")}</button>
            </CxRow>
            <CxRow detail={t("完成配置、验证连接并开始使用，也可以查看各个入口的导览。")} title={t("新手引导")}>
              <button className="cx-secondary" onClick={onOpenTour} type="button">{t("重新打开引导")}</button>
            </CxRow>
          </CxCard>
        </CxSection>
      </div> : null}
      {shown("app-settings-doctor") ? <ConfigDoctorPanel /> : null}
      {shown("app-settings-config") ? <div className="cx-set-group" id="app-settings-config">
        <CxSection title={t("配置备份与迁移")}>
          <CxCard>
            <CxRow detail={t("包含供应商 API Key 等敏感信息，请妥善保管")} title={t("导出完整配置")}>
              <button className="cx-secondary" onClick={() => void actions.exportSettings()} type="button"><Upload />{t("导出")}</button>
            </CxRow>
            <CxRow detail={t("读入为草稿，检查无误后自动保存")} title={t("导入配置草稿")} tooltip={t("导出文件包含供应商 API Key 等敏感信息；导入只会读入草稿，检查后仍需点击保存设置。")}>
              <button className="cx-secondary" onClick={() => void actions.importSettings()} type="button"><Download />{t("导入")}</button>
            </CxRow>
          </CxCard>
        </CxSection>
        <CxSection title={t("存放位置")}>
          <CxCard>
            <CxRow
              detail={<code className="cx-set-path" title={settingsPath}>{settingsPath || "—"}</code>}
              title={t("配置文件路径")}
              tooltip={t("只迁移 settings.json；日志、备份、Skills 和运行状态仍留在原状态目录。更换后建议重新打开管理器。")}
            >
              <button className="cx-secondary" onClick={() => void actions.chooseSettingsPath()} type="button">{t("更改…")}</button>
              <Menu>
                <MenuItem onClick={() => void actions.resetSettingsPath()}>{t("恢复默认路径")}</MenuItem>
              </Menu>
            </CxRow>
          </CxCard>
        </CxSection>
      </div> : null}
    </>
  );
}

function LogsPanel({ logs, actions }: { logs: LogsResult | null; actions: Actions }) {
  // 每行一条结构化日志：时间、事件名、一行摘要；点开看完整明细。最新的在前，可搜索、可只看出错。
  const entries = useMemo(() => parseLogText(logs?.text ?? "", logs?.truncated === true), [logs?.text, logs?.truncated]);
  const [query, setQuery] = useState("");
  const [errorsOnly, setErrorsOnly] = useState(false);
  const [limit, setLimit] = useState(200);
  const shown = useMemo(() => filterLogEntries(entries, query, errorsOnly), [entries, query, errorsOnly]);
  const errorCount = useMemo(() => entries.filter((entry) => entry.tone === "error").length, [entries]);
  const logDetail = logs
    ? logs.truncated
      ? tf("日志大小 {0}，仅显示末尾 {1} 行", [formatBytes(logs.fileSize), logs.lines])
      : tf("日志大小 {0}", [formatBytes(logs.fileSize)])
    : "";
  const now = Date.now();
  return (
    <CxSection
      aside={
        <>
          {logDetail ? <span title={logs?.path ?? ""}>{logDetail}</span> : null}
          <button className="cx-secondary" onClick={() => void actions.copyLogs()} type="button"><Copy />{t("复制")}</button>
          <Menu>
            <MenuItem onClick={() => void actions.refreshLogs()}>{t("刷新")}</MenuItem>
            <MenuItem danger onClick={() => void actions.clearLogs()}>{t("清理日志")}</MenuItem>
          </Menu>
        </>
      }
      title={t("最近日志")}
    >
      <div className="cx-log-toolbar">
        <CxFilters
          items={[
            { id: "all", label: t("全部"), count: entries.length },
            { id: "errors", label: t("出错"), count: errorCount },
          ]}
          label={t("日志筛选")}
          onChange={(value) => setErrorsOnly(value === "errors")}
          value={errorsOnly ? "errors" : "all"}
        />
        <label className="cx-search cx-log-search">
          <Search aria-hidden="true" />
          <input aria-label={t("搜索日志")} onChange={(event) => setQuery(event.currentTarget.value)} placeholder={t("搜索事件名或内容")} value={query} />
        </label>
      </div>
      <CxCard className="cx-log-card">
        {shown.slice(0, limit).map((entry) => (
          <details className="cx-log-row" data-tone={entry.tone} key={entry.index}>
            <summary title={entry.pid ? tf("第 {0} 行 · 进程 {1}", [entry.index, entry.pid]) : tf("第 {0} 行", [entry.index])}>
              <i aria-hidden="true" className="cx-log-dot" />
              <span className="cx-log-time">{formatLogTime(entry.timestampMs, now)}</span>
              {entry.event ? <span className="cx-log-event">{entry.event}</span> : null}
              <span className="cx-log-summary">{entry.summary}</span>
            </summary>
            <pre className="cx-log-body">{entry.body}</pre>
          </details>
        ))}
        {shown.length > limit ? (
          <button className="cx-log-more" onClick={() => setLimit((current) => current + 300)} type="button">
            {tf("再显示 {0} 条", [Math.min(300, shown.length - limit)])}
          </button>
        ) : null}
        {!shown.length ? <CxEmpty title={entries.length ? t("没有匹配的日志") : t("暂无日志。")} /> : null}
      </CxCard>
      {logs?.path ? <code className="cx-set-caption">{logs.path}</code> : null}
    </CxSection>
  );
}

function DiagnosticsPanel({ diagnostics, actions }: { diagnostics: DiagnosticsResult | null; actions: Actions }) {
  // 先列关键信息，完整报告（反馈问题时复制给开发者）收在下面。
  const report = diagnostics?.report ?? "";
  const facts = useMemo(() => diagnosticFacts(report), [report]);
  return (
    <CxSection
      aside={
        <>
          <button className="cx-secondary" onClick={() => void actions.copyDiagnostics()} type="button"><Copy />{t("复制报告")}</button>
          <Menu>
            <MenuItem onClick={() => void actions.refreshDiagnostics()}>{t("重新生成")}</MenuItem>
          </Menu>
        </>
      }
      title={<span title={t("包含版本、路径、设置和平台信息；反馈问题时点「复制报告」附上即可")}>{t("诊断报告")}</span>}
    >
      <CxCard className="cx-set-mono-card cx-diagnostic-card">
        {facts.map((fact) => (
          <CxRow key={fact.label} title={t(fact.label)}>
            <span className="cx-diagnostic-value" data-tone={fact.tone} title={fact.value}>
              {[fact.status ? t(fact.status) : "", fact.value ?? ""].filter(Boolean).join(" · ")}
            </span>
          </CxRow>
        ))}
        <details className="cx-set-disclosure" open={facts.length ? undefined : true}>
          <summary>{t("完整报告")}</summary>
          <Textarea aria-label={t("诊断报告")} className="log-view" readOnly title={t("包含版本、路径、设置和平台信息")} value={report || t("尚未生成诊断报告。")} />
        </details>
      </CxCard>
    </CxSection>
  );
}

/** 打开设置页时滚到指定分组（例如启动时提示更新 → 版本与更新），滚完清掉。 */
export function AppSettingsFocus({ focus, onDone }: { focus: string | null; onDone: () => void }) {
  useEffect(() => {
    if (!focus) return;
    const frame = window.requestAnimationFrame(() => {
      document.getElementById(focus)?.scrollIntoView({ block: "start" });
      onDone();
    });
    return () => window.cancelAnimationFrame(frame);
  }, [focus, onDone]);
  return null;
}

function formatBytes(bytes: number) {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let index = 0;
  while (value >= 1024 && index < units.length - 1) {
    value /= 1024;
    index += 1;
  }
  return `${value >= 10 || index === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[index]}`;
}

/** 「填入常用」：限流与常见的服务端/网关瞬时错误。 */
const COMMON_RETRY_STATUS_CODES = [429, 500, 502, 503, 504];

function codexExtraArgsToInput(args: string[] | undefined) {
  return (args ?? []).join("\n");
}

function inputToCodexExtraArgs(value: string) {
  return value === "" ? [] : value.split(/\r?\n/);
}
