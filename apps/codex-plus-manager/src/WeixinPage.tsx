// 微信连接页。
import { invoke } from "@tauri-apps/api/core";
import { ChevronDown, Copy, ExternalLink, Play, MessageCircle, FolderOpen, PowerOff, RefreshCw, ScanLine, Search } from "lucide-react";
import { createPortal } from "react-dom";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { Input } from "@/components/ui/input";
import { Switch } from "./gateway-ui";
import { AppSelect } from "./AppSelect";
import { useGatewayPayload } from "./hub-shared";
import { groupDisplayName } from "./gateway-model";
import { remoteControlRequest, type RemoteControlStatus } from "./RemoteControlPage";
import { Menu, MenuItem } from "./ManagementPages";
import { usePopoverLayer } from "./use-popover-layer";
import { t, tf } from "@/i18n";
import * as AgentCx from "./cx-layout"; // cx-pages:agents：Agent 页面在函数内解构，避免和别的分组的具名导入重名
import type { BackendSettings, LocalSession, WeixinConnectStatusResult, WeixinQrResult } from "./app-types";
import { formatTime, SettingRow, statusLabel } from "./app-shared";

type WeixinConnectCatalog = {
  status: string;
  message?: string;
  model?: string;
  default_model?: string;
  models: string[];
  modelMetadata?: Record<string, {
    displayName?: string;
    supportedReasoningEfforts?: { reasoningEffort: string; description?: string }[];
  }>;
};

type WeixinConnectProjects = {
  projects: { id: string; name: string; roots?: { path: string }[] }[];
};

function SearchablePathPicker({
  value,
  options,
  placeholder,
  onChange,
}: {
  value: string;
  options: string[];
  placeholder: string;
  onChange: (value: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const filteredOptions = useMemo(() => {
    const query = value.trim().toLowerCase();
    return options.filter((option) => !query || option.toLowerCase().includes(query)).slice(0, 30);
  }, [options, value]);
  // 外点关闭/滚动跟随/多开约束统一交给 usePopoverLayer；菜单 portal 出去，不被滚动区裁掉。
  const { rootRef, menuRef, menuStyle } = usePopoverLayer({ open, onOpenChange: setOpen, maxHeight: 280 });

  return (
    <div className="weixin-search-picker" ref={rootRef}>
      <div className="weixin-search-input-wrap">
        <Search className="weixin-search-input-icon h-4 w-4" />
        <Input
          aria-expanded={open}
          aria-label={placeholder}
          className="h-10"
          onChange={(event) => {
            onChange(event.target.value);
            setOpen(true);
          }}
          onFocus={() => setOpen(true)}
          onKeyDown={(event) => {
            if (event.key === "Escape") setOpen(false);
          }}
          placeholder={placeholder}
          value={value}
        />
        <ChevronDown className={`weixin-search-input-chevron h-4 w-4${open ? " is-open" : ""}`} />
      </div>
      {open ? createPortal(
        <div className="weixin-search-menu weixin-search-menu-portal" ref={menuRef} role="listbox" style={menuStyle}>
          {filteredOptions.length ? filteredOptions.map((option) => (
            <button
              className="weixin-search-option"
              key={option}
              onClick={() => {
                onChange(option);
                setOpen(false);
              }}
              type="button"
            >
              <span>{option}</span>
            </button>
          )) : (
            <div className="weixin-search-empty">{t("没有匹配的已有目录，可继续直接输入。")}</div>
          )}
        </div>,
        document.body,
      ) : null}
    </div>
  );
}

function SearchableSessionPicker({
  sessions,
  selectedId,
  onSelect,
}: {
  sessions: LocalSession[];
  selectedId: string;
  onSelect: (session: LocalSession | null) => void;
}) {
  const selected = sessions.find((session) => session.id === selectedId) ?? null;
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const filteredSessions = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    return sessions
      .filter((session) => !normalized || [session.title, session.cwd, session.id, session.modelProvider].some((value) => value.toLowerCase().includes(normalized)))
      .slice(0, 30);
  }, [query, sessions]);
  const { rootRef, menuRef, menuStyle } = usePopoverLayer({ open, onOpenChange: setOpen, maxHeight: 280 });

  useEffect(() => {
    if (selected) setQuery(selected.title || selected.id);
  }, [selected]);

  return (
    <div className="weixin-search-picker" ref={rootRef}>
      <div className="weixin-search-input-wrap">
        <Search className="weixin-search-input-icon h-4 w-4" />
        <Input
          aria-expanded={open}
          aria-label={t("已有会话")}
          className="h-10"
          onChange={(event) => {
            setQuery(event.target.value);
            if (selectedId) onSelect(null);
            setOpen(true);
          }}
          onFocus={() => setOpen(true)}
          onKeyDown={(event) => {
            if (event.key === "Escape") setOpen(false);
          }}
          placeholder={sessions.length ? t("搜索已有会话") : t("暂无可用的本地会话")}
          value={query}
        />
        <ChevronDown className={`weixin-search-input-chevron h-4 w-4${open ? " is-open" : ""}`} />
      </div>
      {open ? createPortal(
        <div className="weixin-search-menu weixin-search-menu-portal weixin-session-menu" ref={menuRef} role="listbox" style={menuStyle}>
          {filteredSessions.length ? filteredSessions.map((session) => (
            <button
              aria-selected={session.id === selectedId}
              className="weixin-search-option weixin-session-option"
              key={session.id}
              onClick={() => {
                onSelect(session);
                setQuery(session.title || session.id);
                setOpen(false);
              }}
              type="button"
            >
              <strong>{session.title || t("未命名会话")}</strong>
              <span>{session.cwd || t("未记录项目路径")}</span>
              <small>{formatTime(session.updatedAtMs ?? 0)} · {session.modelProvider || t("provider 未记录")}</small>
            </button>
          )) : (
            <div className="weixin-search-empty">{t("没有匹配的本地会话。")}</div>
          )}
        </div>,
        document.body,
      ) : null}
    </div>
  );
}

export function WeixinConnectScreen({
  form,
  status,
  qr,
  sessions,
  onFormChange,
  autoSave,
  onQrLogin,
  onStart,
  onStop,
  onChooseWorkDir,
  onChooseCodexPath,
  onUseDesktopCodexCli,
  onOpenQr,
  onCopyQr,
}: {
  form: BackendSettings;
  status: WeixinConnectStatusResult | null;
  qr: WeixinQrResult | null;
  sessions: LocalSession[];
  onFormChange: (value: BackendSettings) => void;
  autoSave: ReactNode;
  onQrLogin: () => void;
  onStart: () => void;
  onStop: () => void;
  onChooseWorkDir: () => void;
  onChooseCodexPath: () => void;
  onUseDesktopCodexCli: () => void;
  onOpenQr: (url: string) => void;
  onCopyQr: (url: string) => void;
}) {
  const [selectedSessionId, setSelectedSessionId] = useState("");
  const gatewayPayload = useGatewayPayload(10_000);
  const [catalog, setCatalog] = useState<WeixinConnectCatalog | null>(null);
  const [catalogError, setCatalogError] = useState("");
  const [catalogLoading, setCatalogLoading] = useState(true);
  const [catalogRevision, setCatalogRevision] = useState(0);
  const [projects, setProjects] = useState<WeixinConnectProjects | null>(null);
  const [projectsError, setProjectsError] = useState("");
  const [projectsLoading, setProjectsLoading] = useState(false);
  const [projectsRevision, setProjectsRevision] = useState(0);
  const [resetPeerId, setResetPeerId] = useState("");
  const [resetPending, setResetPending] = useState(false);
  const [resetMessage, setResetMessage] = useState("");
  // 加加跑在远程控制里：没开时微信里的加加不可用，这里提示一下（null = 还没读到）。
  const [remoteControlStatus, setRemoteControlStatus] = useState<RemoteControlStatus | null>(null);
  const remoteControlRunning = remoteControlStatus?.running ?? null;
  const [focusPending, setFocusPending] = useState(false);
  const [focusError, setFocusError] = useState("");

  useEffect(() => {
    let disposed = false;
    remoteControlRequest("status")
      .then((value) => { if (!disposed) setRemoteControlStatus(value); })
      .catch((error) => { if (!disposed) setFocusError(String(error)); });
    return () => {
      disposed = true;
    };
  }, []);

  const toggleFocus = async () => {
    if (!remoteControlStatus) return;
    setFocusPending(true);
    setFocusError("");
    try {
      setRemoteControlStatus(await remoteControlRequest("setWeixinFocus", { enabled: !remoteControlStatus.weixinFocusEnabled }));
    } catch (error) {
      setFocusError(String(error));
    } finally {
      setFocusPending(false);
    }
  };

  useEffect(() => {
    let disposed = false;
    let inFlight = false;
    const refresh = async () => {
      if (inFlight) return;
      inFlight = true;
      setCatalogLoading(true);
      try {
        const result = await invoke<WeixinConnectCatalog>("weixin_connect_catalog");
        if (disposed) return;
        if (result.status === "failed" || !Array.isArray(result.models)) {
          throw new Error(result.message || t("读取模型目录失败。"));
        }
        setCatalog(result);
        setCatalogError("");
      } catch (error) {
        if (!disposed) setCatalogError(String(error));
      } finally {
        inFlight = false;
        if (!disposed) setCatalogLoading(false);
      }
    };
    void refresh();
    return () => {
      disposed = true;
    };
  }, [catalogRevision]);

  const projectCodexPath = form.weixinConnectCodexPath.trim();
  const projectWorkDir = form.weixinConnectWorkDir.trim();
  useEffect(() => {
    let disposed = false;
    let inFlight = false;
    setProjects(null);
    setProjectsError("");
    if (!projectsRevision && !form.weixinConnectProjectId) {
      setProjectsLoading(false);
      return;
    }
    const refresh = async () => {
      if (inFlight) return;
      inFlight = true;
      setProjectsLoading(true);
      try {
        const result = await invoke<WeixinConnectProjects>("weixin_connect_projects", {
          codexPath: projectCodexPath,
          workDir: projectWorkDir,
        });
        if (disposed) return;
        if (!Array.isArray(result.projects)) throw new Error(t("读取项目列表失败。"));
        setProjects(result);
        setProjectsError("");
      } catch (error) {
        if (!disposed) setProjectsError(String(error));
      } finally {
        inFlight = false;
        if (!disposed) setProjectsLoading(false);
      }
    };
    // 输入目录或 CLI 路径时稍等，避免每个按键都启动一次项目查询。
    const initialTimer = window.setTimeout(() => void refresh(), 350);
    return () => {
      disposed = true;
      window.clearTimeout(initialTimer);
    };
  }, [projectCodexPath, projectWorkDir, projectsRevision]);

  const models = catalog?.models ?? [];
  const inheritedModel = catalog?.model || catalog?.default_model || "";
  const effectiveModel = form.weixinConnectModel || inheritedModel;
  const reasoningEfforts = Array.from(new Set(
    (catalog?.modelMetadata?.[effectiveModel]?.supportedReasoningEfforts ?? [])
      .map((entry) => entry.reasoningEffort)
      .filter(Boolean),
  ));
  const modelUnavailable = Boolean(catalog && !catalogError && form.weixinConnectModel
    && !models.includes(form.weixinConnectModel));
  const reasoningUnavailable = Boolean(catalog && !catalogError && form.weixinConnectReasoningEffort
    && !reasoningEfforts.includes(form.weixinConnectReasoningEffort));
  const projectOptions = projects?.projects ?? [];
  const projectUnavailable = Boolean(projects && !projectsError && form.weixinConnectProjectId
    && !projectOptions.some((project) => project.id === form.weixinConnectProjectId));
  const workDirOptions = useMemo(
    () => Array.from(new Set(sessions.map((session) => session.cwd.trim()).filter(Boolean))).sort(),
    [sessions],
  );
  const runtimeState = status?.state ?? "stopped";
  const running = ["starting", "running", "retrying"].includes(runtimeState);
  const stopping = runtimeState === "stopping";
  const runtimeLabel = {
    starting: t("正在启动"),
    running: t("运行中"),
    retrying: t("正在重试"),
    stopping: t("正在停止"),
    error: t("异常"),
    stopped: t("已停止"),
  }[runtimeState] ?? statusLabel(runtimeState);

  const { CxBadge, CxCard, CxRow, CxSection, CxStatusIcon } = AgentCx;
  const statusTone = runtimeState === "running" ? "ok" : runtimeState === "error" ? "danger" : running ? "warn" : "off";
  const accountLabel = status?.accountId || form.weixinConnectAccountId || t("未登录");
  const loggedIn = Boolean(form.weixinConnectToken);
  const outboundBlocked = !form.weixinConnectAllowOutboundAttachments && (
    !form.weixinConnectAllowFrom.split(",").some((peer) => peer.trim())
    || form.weixinConnectAllowFrom.split(",").some((peer) => peer.trim() === "*")
    || !form.weixinConnectWorkDir.trim()
  );
  const resetSession = async () => {
    setResetPending(true);
    setResetMessage("");
    try {
      await invoke("weixin_connect_reset_session", { peerId: resetPeerId.trim(), accountId: form.weixinConnectAccountId.trim() });
      setResetMessage(t("已保存重置请求，下一条消息开启新会话，原历史保留。"));
    } catch (error) {
      setResetMessage(String(error));
    } finally {
      setResetPending(false);
    }
  };

  return (
    <div className="weixin-connect-page sectioned-page cx-page">
      {/* 状态一行：没登录时唯一的主操作是扫码登录；登录后是启动 / 停止，重新登录收进三点菜单。 */}
      <CxCard className={`weixin-status-panel is-${runtimeState}`}>
        <div id="weixin-section-status">
          <CxRow
            badges={<CxBadge tone={statusTone === "ok" ? "ok" : statusTone === "danger" ? "danger" : "neutral"}>{runtimeLabel}</CxBadge>}
            detail={(
              <span className="weixin-status-detail">
                <span title={status?.message ?? t("微信连接未启动。")}>{status?.message ?? t("微信连接未启动。")}</span>
                <span title={accountLabel}>{t("账号")} <code>{accountLabel}</code></span>
                <span>{t("已处理消息")} <b>{status?.processedMessages ?? 0}</b></span>
                <span title={status?.lastPeerId || t("暂无")}>{t("最近联系人")} <code>{status?.lastPeerId || t("暂无")}</code></span>
              </span>
            )}
            icon={<CxStatusIcon tone={statusTone}><MessageCircle /></CxStatusIcon>}
            title={t("个人微信连接")}
          >
            <span className="weixin-autosave">{autoSave}</span>
            {!loggedIn ? (
              <button className="cx-primary" onClick={onQrLogin} type="button">
                <ScanLine />
                {t("扫码登录")}
              </button>
            ) : running || stopping ? (
              <button className="cx-secondary" disabled={stopping} onClick={onStop} type="button">
                <PowerOff />
                {stopping ? t("正在停止") : t("停止")}
              </button>
            ) : (
              <button className="cx-primary" onClick={onStart} type="button">
                <Play />
                {t("启动")}
              </button>
            )}
            {loggedIn ? (
              <Menu>
                <MenuItem onClick={onQrLogin}>{t("重新登录")}</MenuItem>
              </Menu>
            ) : null}
          </CxRow>
        </div>
      </CxCard>

      {qr?.qrContent ? (
        <CxSection title={qr.qrStatus === "scaned" ? t("已扫码，请在手机上确认") : t("微信扫码登录")}>
          <CxCard className="weixin-qr-card">
            {qr.qrSvg ? (
              <div className="weixin-qr-image" dangerouslySetInnerHTML={{ __html: qr.qrSvg }} />
            ) : null}
            <div className="weixin-qr-side">
              <p>{t("在手机微信中打开登录链接，或复制到可生成二维码的设备完成确认。")}</p>
              <div className="weixin-qr-content">{qr.qrContent}</div>
              <div className="weixin-qr-actions">
                <button className="cx-primary" onClick={() => onOpenQr(qr.qrContent)} type="button">
                  <ExternalLink />
                  {t("打开登录链接")}
                </button>
                <button className="cx-secondary" onClick={() => onCopyQr(qr.qrContent)} type="button">
                  <Copy />
                  {t("复制链接")}
                </button>
              </div>
            </div>
          </CxCard>
        </CxSection>
      ) : null}

      {/* 设置只留每天会碰的两节：对话（谁能用、在哪个目录、用哪个模型）和权限；改动即自动保存。
          扫码后自动填好的凭据、偶尔才用的项目归属与重置会话、连接参数都收进「高级」。 */}
      <CxSection id="weixin-section-session" title={t("对话")}>
        <CxCard className="weixin-settings-card">
          <SettingRow description={t("每个微信联系人会映射到独立的 Codex 会话。")} title={t("允许的微信用户 ID")}>
            <Input
              aria-label={t("允许的微信用户 ID")}
              onChange={(event) => onFormChange({ ...form, weixinConnectAllowFrom: event.target.value })}
              placeholder="user@im.wechat"
              value={form.weixinConnectAllowFrom}
            />
          </SettingRow>
          <SettingRow description={t("新会话在这个目录里工作；也可以从已有会话带入它的目录。")} title={t("工作目录")}>
            <div className="weixin-path-row">
              <SearchablePathPicker
                onChange={(value) => {
                  setSelectedSessionId("");
                  onFormChange({ ...form, weixinConnectWorkDir: value });
                }}
                options={workDirOptions}
                placeholder={t("搜索或输入工作目录")}
                value={form.weixinConnectWorkDir}
              />
              <button aria-label={t("选择工作目录")} className="cx-icon-button" onClick={onChooseWorkDir} title={t("选择工作目录")} type="button">
                <FolderOpen />
              </button>
            </div>
          </SettingRow>
          <SettingRow title={t("从已有会话带入")}>
            <SearchableSessionPicker
              onSelect={(session) => {
                setSelectedSessionId(session?.id ?? "");
                if (session?.cwd) onFormChange({ ...form, weixinConnectWorkDir: session.cwd });
              }}
              selectedId={selectedSessionId}
              sessions={sessions}
            />
          </SettingRow>
          {/* 模型和推理强度并成一行：两个下拉并排，推理强度跟着模型走。 */}
          <SettingRow
            description={catalogError
              || (modelUnavailable ? t("当前模型已不在可用模型目录中，请重新选择。") : undefined)
              || (reasoningUnavailable ? t("当前模型不支持已选推理强度，请重新选择或继承。") : undefined)
              || (catalog && !models.length ? t("当前没有可用模型，请检查供应商配置。") : undefined)}
            title={t("模型")}
          >
            <div className="weixin-path-row weixin-model-row">
              <div aria-invalid={modelUnavailable || Boolean(catalogError)} className="weixin-select">
                <AppSelect
                  ariaLabel={t("模型")}
                  onChange={(next) => onFormChange({ ...form, weixinConnectModel: next })}
                  options={[
                    { value: "", label: inheritedModel ? tf("继承 Codex 默认模型（{0}）", [groupDisplayName(gatewayPayload, inheritedModel) ?? catalog?.modelMetadata?.[inheritedModel]?.displayName ?? inheritedModel]) : t("继承 Codex 默认模型") },
                    ...(form.weixinConnectModel && !models.includes(form.weixinConnectModel) ? [{
                      value: form.weixinConnectModel,
                      label: `${groupDisplayName(gatewayPayload, form.weixinConnectModel) ?? form.weixinConnectModel} (${modelUnavailable ? t("不可用") : t("当前值")})`,
                    }] : []),
                    ...models.map((model) => ({ value: model, label: groupDisplayName(gatewayPayload, model) ?? (catalog?.modelMetadata?.[model]?.displayName || model) })),
                  ]}
                  value={form.weixinConnectModel}
                />
              </div>
              <div aria-invalid={reasoningUnavailable} className="weixin-select weixin-effort-select">
                <AppSelect
                  ariaLabel={t("推理强度")}
                  onChange={(next) => onFormChange({ ...form, weixinConnectReasoningEffort: next })}
                  options={[
                    { value: "", label: t("继承推理强度") },
                    ...(form.weixinConnectReasoningEffort && !reasoningEfforts.includes(form.weixinConnectReasoningEffort) ? [{
                      value: form.weixinConnectReasoningEffort,
                      label: `${form.weixinConnectReasoningEffort} (${reasoningUnavailable ? t("不可用") : t("当前值")})`,
                    }] : []),
                    ...reasoningEfforts.map((effort) => ({ value: effort, label: effort })),
                  ]}
                  value={form.weixinConnectReasoningEffort}
                />
              </div>
              <button aria-label={t("刷新模型目录")} className="cx-icon-button" disabled={catalogLoading} onClick={() => setCatalogRevision((value) => value + 1)} title={t("刷新模型目录")} type="button">
                <RefreshCw className={catalogLoading ? "animate-spin" : ""} />
              </button>
            </div>
          </SettingRow>
        </CxCard>
      </CxSection>

      {/* 加加：没自己选过模式的联系人默认交给加加（远程控制没开时自动退回直连）；推送默认关。 */}
      <CxSection
        aside={remoteControlRunning === null ? null : (
          <CxBadge tone={remoteControlRunning ? "ok" : "neutral"}>{remoteControlRunning ? t("远程控制已开") : t("远程控制未开")}</CxBadge>
        )}
        id="weixin-section-jiajia"
        title={t("加加")}
      >
        <CxCard className="weixin-settings-card">
          <SettingRow
            description={remoteControlRunning === false
              ? t("加加需要先打开远程控制；没开时消息先按直连处理")
              : t("没单独选过的联系人用这个模式；在微信里发 /加加 或 /直连 可单独切换")}
            title={t("微信默认模式")}
          >
            <AppSelect
              ariaLabel={t("微信默认模式")}
              onChange={(next) => onFormChange({ ...form, weixinConnectDefaultMode: next })}
              options={[
                { value: "jiajia", label: t("加加") },
                { value: "direct", label: t("直连") },
              ]}
              value={form.weixinConnectDefaultMode}
            />
          </SettingRow>
          <SettingRow description={t("没在对话时，加加的审批卡片和提醒也发到微信；回复数字即可审批")} title={t("推送到微信")}>
            <Switch
              label={t("推送到微信")}
              on={form.weixinConnectPushToWeixin}
              onToggle={() => onFormChange({ ...form, weixinConnectPushToWeixin: !form.weixinConnectPushToWeixin })}
            />
          </SettingRow>
        </CxCard>
      </CxSection>

      <CxSection id="weixin-section-account" title={t("权限")}>
        <CxCard className="weixin-settings-card">
          <SettingRow
            title={t("允许微信专注操作桌面会话")}
            description={focusError || t("远程控制关闭时，仅在此项开启后，已扫码绑定的微信用户仍可进入桌面会话、发消息和处理审批。不会开启网页访问。")}
          >
            <Switch
              label={t("允许微信专注操作桌面会话")}
              on={Boolean(remoteControlStatus?.weixinFocusEnabled)}
              disabled={focusPending || !remoteControlStatus}
              pending={focusPending}
              onToggle={() => void toggleFocus()}
            />
          </SettingRow>
          <SettingRow description={t("Codex 在工作目录里能做什么")} title={t("沙箱权限")}>
            <AppSelect
              ariaLabel={t("沙箱权限")}
              onChange={(next) => onFormChange({ ...form, weixinConnectSandbox: next })}
              options={[
                { value: "read-only", label: t("只读") },
                { value: "workspace-write", label: t("允许修改工作目录") },
                { value: "danger-full-access", label: t("完全访问") },
              ]}
              value={form.weixinConnectSandbox}
            />
          </SettingRow>
          <SettingRow
            description={outboundBlocked ? t("先填允许的微信用户 ID（不能是 *）并选好工作目录，才能打开") : t("允许向指定微信用户回传工作区文件")}
            title={t("工作区附件回传")}
          >
            <Switch
              label={t("允许向指定微信用户回传工作区文件")}
              on={form.weixinConnectAllowOutboundAttachments}
              disabled={outboundBlocked}
              onToggle={() => onFormChange({ ...form, weixinConnectAllowOutboundAttachments: !form.weixinConnectAllowOutboundAttachments })}
            />
          </SettingRow>
        </CxCard>
      </CxSection>

      <details className="weixin-advanced" id="weixin-section-cli">
        <summary>
          <span>{t("高级")}</span>
          <small>{t("新会话项目")} · {t("重置联系人会话")} · {t("登录凭据")} · {t("连接参数")}</small>
        </summary>
        <CxCard className="weixin-settings-card">
          <SettingRow
            description={projectsError || (projectUnavailable ? t("当前项目已不在项目列表中，请重新选择。") : t("新会话归到 Codex 里的哪个项目"))}
            title={t("新会话项目")}
          >
            <div aria-invalid={projectUnavailable || Boolean(projectsError)} className="weixin-path-row">
              <AppSelect
                ariaLabel={t("新会话项目")}
                onChange={(id) => {
                  const selected = projectOptions.find((project) => project.id === id);
                  onFormChange({
                    ...form,
                    weixinConnectProjectId: id,
                    weixinConnectWorkDir: selected?.roots?.[0]?.path || form.weixinConnectWorkDir,
                  });
                }}
                options={[
                  { value: "", label: t("不归属项目") },
                  ...(form.weixinConnectProjectId && !projectOptions.some((project) => project.id === form.weixinConnectProjectId) ? [{
                    value: form.weixinConnectProjectId,
                    label: `${form.weixinConnectProjectId} (${projectUnavailable ? t("不可用") : t("当前值")})`,
                  }] : []),
                  ...projectOptions.map((project) => ({ value: project.id, label: project.name || project.id })),
                ]}
                value={form.weixinConnectProjectId}
              />
              <button aria-label={t("刷新项目")} className="cx-icon-button" disabled={projectsLoading} onClick={() => setProjectsRevision((value) => value + 1)} title={t("刷新项目")} type="button">
                <RefreshCw className={projectsLoading ? "animate-spin" : ""} />
              </button>
            </div>
          </SettingRow>
          <SettingRow description={resetMessage || t("重置该联系人会话，下一条消息生效并保留原历史")} title={t("重置联系人会话")}>
            <div className="weixin-path-row">
              <Input aria-label={t("需要重置的联系人 ID")} placeholder={status?.lastPeerId || t("联系人 ID")} value={resetPeerId} onChange={(event) => setResetPeerId(event.target.value)} />
              <button aria-label={t("重置该联系人会话")} className="cx-secondary" disabled={resetPending || !resetPeerId.trim()} onClick={() => void resetSession()} title={t("重置该联系人会话，下一条消息生效并保留原历史")} type="button">
                <RefreshCw className={resetPending ? "animate-spin" : ""} />
                {t("重置")}
              </button>
            </div>
          </SettingRow>
          <SettingRow description={t("扫码后自动保存，也可粘贴已有 Bearer token")} title={t("登录凭据")}>
            <Input
              aria-label={t("登录凭据")}
              autoComplete="off"
              onChange={(event) => onFormChange({ ...form, weixinConnectToken: event.target.value })}
              placeholder={t("扫码后自动保存，也可粘贴已有 Bearer token")}
              type="password"
              value={form.weixinConnectToken}
            />
          </SettingRow>
          <SettingRow title={t("iLink API 地址")}>
            <Input
              aria-label={t("iLink API 地址")}
              onChange={(event) => onFormChange({ ...form, weixinConnectBaseUrl: event.target.value })}
              value={form.weixinConnectBaseUrl}
            />
          </SettingRow>
          <SettingRow description={t("扫码后自动填写")} title={t("账号标识")}>
            <Input
              aria-label={t("账号标识")}
              onChange={(event) => onFormChange({ ...form, weixinConnectAccountId: event.target.value })}
              placeholder={t("扫码后自动填写")}
              value={form.weixinConnectAccountId}
            />
          </SettingRow>
          <SettingRow description={t("仅在网关要求时填写")} title={t("SKRouteTag")}>
            <Input
              aria-label={t("SKRouteTag")}
              onChange={(event) => onFormChange({ ...form, weixinConnectRouteTag: event.target.value })}
              placeholder={t("仅在网关要求时填写")}
              value={form.weixinConnectRouteTag}
            />
          </SettingRow>
          <SettingRow description={t("留空时从 PATH 查找 codex")} title={t("Codex CLI 路径")}>
            <div className="weixin-path-row weixin-cli-path-row">
              <Input
                aria-label={t("Codex CLI 路径")}
                onChange={(event) => onFormChange({ ...form, weixinConnectCodexPath: event.target.value })}
                placeholder={t("留空时从 PATH 查找 codex")}
                value={form.weixinConnectCodexPath}
              />
              <button aria-label={t("选择 Codex CLI")} className="cx-icon-button" onClick={onChooseCodexPath} title={t("选择 Codex CLI")} type="button">
                <FolderOpen />
              </button>
              <Menu>
                <MenuItem onClick={onUseDesktopCodexCli}>{t("使用桌面版内置 CLI")}</MenuItem>
              </Menu>
            </div>
          </SettingRow>
        </CxCard>
      </details>
    </div>
  );
}
