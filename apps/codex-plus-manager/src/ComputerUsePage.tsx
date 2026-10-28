/** Computer Use：复用 Codex app 的 Computer Use 与 Chrome 控制，通过 MCP 提供给 Claude Code、DeepSeek Harness、ZCode 等客户端。 */
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { appConfirm } from "./app-dialog";
import { AlertTriangle, AppWindow, Check, CheckCircle2, FolderOpen, Globe, Loader2, MonitorSmartphone, MousePointerClick, Pencil, Play, Plug, Plus, RefreshCw, ShieldCheck, Trash2, Unplug, X } from "lucide-react";
import { ComputerUsePreview } from "./ComputerUsePreview";
import { AppSelect } from "./AppSelect";
import { PageHeader } from "./ManagementPages";
import { HubTopology } from "./HubOverview";
import { APP_LOGO_URL } from "./gateway-ui";
import type { HubLink } from "./hub-overview-model";
import { t, tf } from "./i18n";
import { enhancementPlatformFromUserAgent } from "./enhancement-catalog";
import "./ComputerUsePage.css";

type CuTargetId = "claudeCode" | "claudeDesktop" | "codex" | "dsh" | "zcode" | "workBuddy";
type CuTargetState = "registered" | "missing" | "needs_update" | "foreign" | "unsupported";
type CuTarget = { id: string; label: string; configPath: string; registered: boolean; state: CuTargetState; message: string };
type CuTargets = Record<CuTargetId, CuTarget>;
/**
 * 自定义目标：同一种客户端换一个配置目录，id 形如 `custom-…`。
 * `name` 是用户起的名字（可能为空）；`label` 已按它取好显示名，没起名时是客户端类型名。
 */
type CuCustomTarget = CuTarget & { kind: CuTargetId; path: string; name: string };

/** 自定义目标名字的长度上限，与后端 `CUSTOM_NAME_MAX_CHARS` 一致。 */
const CUSTOM_NAME_MAX = 40;
type CuStatus = {
  status: string;
  message: string;
  runtimeFound: boolean;
  claudeDesktopRunning?: boolean;
  appPath?: string;
  packageVersion?: string;
  servicePath?: string;
  configHint?: string;
  targets?: Partial<CuTargets>;
  customTargets?: CuCustomTarget[];
};
type CuMutation = { status: string; message: string; changed: boolean; backupPath?: string; targets?: Partial<CuTargets>; customTargets?: CuCustomTarget[] };
type CuFailureKind = "unsupported_platform" | "runtime_missing" | "untrusted_parent" | "permission_required" | "client_unsupported" | "service_unavailable" | "error";
type CuCheck = {
  status: string;
  message: string;
  ok: boolean;
  target: string;
  appCount?: number;
  browserCount?: number;
  sampleApps?: string[];
  durationMs?: number;
  failureKind?: CuFailureKind;
  failureDetail?: string;
};
type CuResult = { status: string; message: string };
type CuTab = "overview" | "preview" | "diagnose";
type CuApprovalMode = "auto" | "claude";
/** 操作方式：cua_repl 的 js（默认，和 Codex 当前主要路径一致），或离散工具（直通原生 CU）。 */
type CuMode = "tools" | "js";

/**
 * 本页的几个开关（操作方式、审批策略、悬浮预览）由后端单独写进 settings.json，文件版本号会变。
 * 通知主界面同步，否则它表单里的旧版本号会让之后的设置保存报「已被其他操作更新」。
 */
export const BACKEND_SETTINGS_CHANGED_EVENT = "codex-plus:backend-settings-changed";
const announceSettingsChanged = () => window.dispatchEvent(new Event(BACKEND_SETTINGS_CHANGED_EVENT));
type CuApprovedApp = { decision: string; mode?: string; displayName?: string; decidedAt?: number; source?: string };
type CuLastClient = { name?: string; version?: string; elicitation?: boolean; at?: number };
type CuApprovalState = {
  status?: string;
  message?: string;
  mode?: CuApprovalMode;
  cuMode?: CuMode;
  pip?: boolean;
  approvals?: Array<[string, CuApprovedApp]>;
  lastClient?: CuLastClient | null;
  turnHooks?: { installed?: boolean; events?: string[]; settings?: string } | null;
};

// Codex 自带 Computer Use，不提供接入；status 里的 codex 只用来发现旧版本留下的条目，
// 那会让 Codex 同时出现两套电脑控制工具，见下方的残留提示。
const targetIds: CuTargetId[] = ["claudeCode", "claudeDesktop", "dsh", "zcode", "workBuddy"];
const targetFallback: Record<CuTargetId, { label: string; configPath: string }> = {
  claudeCode: { label: "Claude Code", configPath: "~/.claude.json" },
  claudeDesktop: { label: "Claude Desktop", configPath: "claude_desktop_config.json" },
  codex: { label: "Codex", configPath: "config.toml" },
  dsh: { label: "DeepSeek Harness", configPath: "~/.dsh/profiles/desktop/cordis.patch.yml" },
  zcode: { label: "ZCode", configPath: "~/.zcode/cli/config.json" },
  workBuddy: { label: "WorkBuddy AI", configPath: "~/.workbuddy-ai/mcp.json" },
};
const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));
const hasValue = (value?: string) => Boolean(value && value.trim());
// 配置路径通常很长，保留首尾两段比整体省略更能看清是哪个客户端。
const elidePath = (value: string, limit = 52) => {
  if (value.length <= limit) return value;
  const head = Math.ceil(limit * 0.45);
  const tail = limit - head - 1;
  return `${value.slice(0, head)}…${value.slice(-tail)}`;
};
// 自定义目标可选的类型，与上面列出的内置目标一致。
const customKinds: CuTargetId[] = targetIds;
// 输入框占位说明：告诉用户该填哪一层目录。渲染时再过 t()。
const customDirHint: Record<CuTargetId, string> = {
  claudeCode: "CLAUDE_CONFIG_DIR 指向的目录（里面有 .claude.json）",
  claudeDesktop: "包含 claude_desktop_config.json 的目录",
  codex: "包含 config.toml 的目录",
  dsh: "包含 cordis.patch.yml 的目录",
  zcode: "包含 config.json 的目录",
  workBuddy: "包含 mcp.json 的目录",
};
// 后端的状态说明里，这两句与徽标完全同义，再显示一行只会把列表拉长。
const redundantMessages = new Set(["已接入", "尚未接入"]);
const removable = (state?: CuTargetState) => state === "registered" || state === "needs_update" || state === "foreign";
const installable = (state?: CuTargetState) => state === "missing" || state === "needs_update" || state === "foreign";

// 语言可在运行中切换，状态文案必须渲染时取，不能缓存在模块级常量里。
function stateLabel(state: CuTargetState): string {
  switch (state) {
    case "registered": return t("已接入");
    case "needs_update": return t("需更新");
    case "foreign": return t("被占用");
    case "unsupported": return t("不支持");
    default: return t("未接入");
  }
}
function failureText(check: CuCheck): string {
  switch (check.failureKind) {
    case "unsupported_platform": return t("Computer Use 暂不支持 Windows");
    case "runtime_missing": return t("没找到 ChatGPT.app，请先安装 Codex 桌面版");
    case "untrusted_parent": return t("Computer Use 拒绝了本次调用：ChatGPT.app 版本不兼容，请更新后再试");
    case "permission_required": return t("请在系统设置里给 ChatGPT 授予辅助功能/屏幕录制权限");
    case "client_unsupported": return t("当前客户端不支持审批卡（elicitation）");
    case "service_unavailable": return t("Computer Use 服务未就绪，先打开一次 Codex 的 Computer Use");
    case "error": return check.failureDetail || check.message || t("自检失败，请查看详情后重试。");
    default: return check.failureDetail || check.message || t("自检失败，请查看详情后重试。");
  }
}

/** Computer Use 依赖 macOS 版 ChatGPT.app 自带的运行时，其它平台暂无可接入的运行时（#2718）。 */
const computerUseSupported = enhancementPlatformFromUserAgent(navigator.userAgent) === "macos";

export function ComputerUsePage() {
  return computerUseSupported ? <ComputerUseMacPage /> : <ComputerUseUnsupported />;
}

/** 非 macOS 平台只给出明确的「暂不支持」说明，不去探测运行时，避免误报「找不到 ChatGPT」。 */
function ComputerUseUnsupported() {
  return <div className="mg-page cu-page">
    <PageHeader icon={<MonitorSmartphone size={20} strokeWidth={1.5} />} title="Computer Use" />
    <div className="cu-content">
      <div className="cu-runtime-missing" role="status" data-reason="unsupported-platform">
        <AlertTriangle size={16} />
        <div>
          <strong>{t("Computer Use 暂不支持 Windows")}</strong>
          <p>{t("Computer Use 接入目前只支持 macOS：它依赖 macOS 版 ChatGPT.app 自带的 Computer Use 服务，当前平台还没有可接入的运行时。")}</p>
        </div>
      </div>
    </div>
  </div>;
}

function ComputerUseMacPage() {
  const [status, setStatus] = useState<CuStatus | null>(null);
  const [check, setCheck] = useState<CuCheck | null>(null);
  const [backups, setBackups] = useState<Record<string, string>>({});
  const [newKind, setNewKind] = useState<CuTargetId>("claudeCode");
  const [newPath, setNewPath] = useState("");
  const [newName, setNewName] = useState("");
  /** 正在改名的自定义目标与草稿。 */
  const [renaming, setRenaming] = useState<{ id: string; draft: string } | null>(null);
  const [tab, setTab] = useState<CuTab>("overview");
  // 图下方「配置路径与自定义目录」折叠区：图里的「添加自定义配置目录」会展开并滚到这里。
  const [showTargets, setShowTargets] = useState(false);
  const targetsDetails = useRef<HTMLDetailsElement>(null);
  const [busy, setBusy] = useState("");
  const [readError, setReadError] = useState("");
  const [actionError, setActionError] = useState("");
  const [notice, setNotice] = useState("");
  const [approval, setApproval] = useState<CuApprovalState | null>(null);
  const [extStatus, setExtStatus] = useState<{ installed: boolean; storeUrl: string } | null>(null);
  const mounted = useRef(false);
  const epoch = useRef(0);
  const lock = useRef(false);

  const loadApproval = useCallback(async () => {
    try {
      const next = await invoke<CuApprovalState>("computer_use_approval_get");
      if (mounted.current) setApproval(next);
    } catch (reason) {
      if (mounted.current) setActionError(errorText(reason));
    }
  }, []);

  const changeApprovalMode = async (mode: CuApprovalMode) => {
    if (lock.current) return;
    lock.current = true;
    setBusy("approval");
    setActionError("");
    setNotice("");
    try {
      const result = await invoke<CuResult>("computer_use_approval_set", { mode });
      if (!mounted.current) return;
      if (result.status === "ok") {
        announceSettingsChanged();
        setNotice(result.message || t("审批策略已保存。"));
        await loadApproval();
      } else {
        setActionError(result.message || t("保存审批策略失败。"));
      }
    } catch (reason) {
      if (mounted.current) setActionError(errorText(reason));
    } finally {
      if (mounted.current) {
        lock.current = false;
        setBusy("");
      }
    }
  };

  const changeCuMode = async (mode: CuMode) => {
    if (lock.current || approval?.cuMode === mode) return;
    lock.current = true;
    setBusy("cuMode");
    setActionError("");
    try {
      const result = await invoke<CuResult>("computer_use_mode_set", { mode });
      if (!mounted.current) return;
      if (result.status === "ok") {
        announceSettingsChanged();
        setNotice(result.message || t("操作方式已保存；对新开的客户端会话生效。"));
        await loadApproval();
      } else {
        setActionError(result.message || t("保存操作方式失败。"));
      }
    } catch (reason) {
      if (mounted.current) setActionError(errorText(reason));
    } finally {
      if (mounted.current) {
        lock.current = false;
        setBusy("");
      }
    }
  };

  const changePip = async (enabled: boolean) => {
    if (lock.current) return;
    lock.current = true;
    setBusy("pip");
    setActionError("");
    try {
      const result = await invoke<CuResult>("computer_use_pip_set", { enabled });
      if (!mounted.current) return;
      if (result.status === "ok") {
        announceSettingsChanged();
        setNotice(result.message || (enabled ? t("已开启悬浮预览。") : t("已关闭悬浮预览。")));
        await loadApproval();
      } else {
        setActionError(result.message || t("保存失败，请重试。"));
      }
    } catch (reason) {
      if (mounted.current) setActionError(errorText(reason));
    } finally {
      if (mounted.current) {
        lock.current = false;
        setBusy("");
      }
    }
  };

  const forgetApproval = async (app: string) => {
    if (lock.current) return;
    lock.current = true;
    setBusy(`forget:${app}`);
    try {
      const result = await invoke<CuResult>("computer_use_approval_forget", { app });
      if (mounted.current && result.status === "ok") {
        setNotice(result.message || t("已撤销。"));
        await loadApproval();
      }
    } catch (reason) {
      if (mounted.current) setActionError(errorText(reason));
    } finally {
      if (mounted.current) {
        lock.current = false;
        setBusy("");
      }
    }
  };

  const clearApprovals = async () => {
    if (lock.current) return;
    lock.current = true;
    setBusy("approval:clear");
    try {
      const result = await invoke<CuResult>("computer_use_approval_clear");
      if (mounted.current && result.status === "ok") {
        setNotice(result.message || t("已清空。"));
        await loadApproval();
      }
    } catch (reason) {
      if (mounted.current) setActionError(errorText(reason));
    } finally {
      if (mounted.current) {
        lock.current = false;
        setBusy("");
      }
    }
  };

  const loadStatus = useCallback(async () => {
    if (lock.current) return;
    lock.current = true;
    const stamp = ++epoch.current;
    setBusy("refresh");
    setReadError("");
    setActionError("");
    setNotice("");
    try {
      const next = await invoke<CuStatus>("computer_use_status");
      if (!mounted.current || epoch.current !== stamp) return;
      setStatus(next);
      if (next.status !== "ok") setReadError(next.message || t("检测失败，请重试。"));
    } catch (reason) {
      if (mounted.current && epoch.current === stamp) setReadError(errorText(reason));
    } finally {
      if (mounted.current && epoch.current === stamp) {
        lock.current = false;
        setBusy("");
      }
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    void loadStatus();
    void loadApproval();
    void invoke<{ installed: boolean; storeUrl: string }>("computer_use_browser_extension_status")
      .then((r) => { if (mounted.current) setExtStatus(r); })
      .catch(() => { /* 非致命，静默忽略 */ });
    return () => {
      mounted.current = false;
      epoch.current += 1;
      lock.current = false;
    };
  }, [loadStatus]);

  const openChatGpt = async () => {
    if (lock.current) return;
    lock.current = true;
    const stamp = ++epoch.current;
    setBusy("open");
    setActionError("");
    setNotice("");
    try {
      const result = await invoke<CuResult>("computer_use_open_app");
      if (!mounted.current || epoch.current !== stamp) return;
      if (result.status === "ok") setNotice(t("已请求打开 ChatGPT.app。"));
      else setActionError(result.message || t("打开 ChatGPT.app 失败。"));
    } catch (reason) {
      if (mounted.current && epoch.current === stamp) setActionError(errorText(reason));
    } finally {
      if (mounted.current && epoch.current === stamp) {
        lock.current = false;
        setBusy("");
      }
    }
  };

  /** `target` 是内置目标 id 或自定义目标的 `custom-…` id。 */
  const mutateTarget = async (target: string, action: "install" | "uninstall") => {
    if (lock.current) return;
    lock.current = true;
    const stamp = ++epoch.current;
    const custom = status?.customTargets?.find((item) => item.id === target);
    const label = custom
      ? `${custom.label} · ${t("自定义")}`
      : status?.targets?.[target as CuTargetId]?.label || targetFallback[target as CuTargetId]?.label || target;
    setBusy(`${action}:${target}`);
    setActionError("");
    setNotice("");
    try {
      const result = await invoke<CuMutation>(action === "install" ? "computer_use_install" : "computer_use_uninstall", { target });
      if (!mounted.current || epoch.current !== stamp) return;
      if (result.targets || result.customTargets) {
        setStatus((current) => (current ? {
          ...current,
          targets: { ...current.targets, ...result.targets },
          customTargets: result.customTargets ?? current.customTargets,
        } : current));
      }
      if (result.status === "ok") {
        setNotice(action === "install" ? `${label} · ${t("已接入，重启客户端后加载工具")}` : `${label} · ${t("已移除")}`);
        setBackups((current) => {
          const next = { ...current };
          if (action === "install" && result.backupPath) next[target] = result.backupPath;
          else delete next[target];
          return next;
        });
      } else {
        setActionError(result.message || t("操作失败，请重试。"));
      }
    } catch (reason) {
      if (mounted.current && epoch.current === stamp) setActionError(errorText(reason));
    } finally {
      if (mounted.current && epoch.current === stamp) {
        lock.current = false;
        setBusy("");
      }
    }
  };

  const pickCustomDir = async () => {
    try {
      const picked = await open({ directory: true, multiple: false, title: t("选择客户端实际读取的配置目录") });
      if (typeof picked === "string" && mounted.current) setNewPath(picked);
    } catch (reason) {
      if (mounted.current) setActionError(errorText(reason));
    }
  };

  const addCustomTarget = async () => {
    if (lock.current || !newPath.trim()) return;
    lock.current = true;
    const stamp = ++epoch.current;
    setBusy("custom:add");
    setActionError("");
    setNotice("");
    try {
      const result = await invoke<CuStatus>("computer_use_custom_add", { kind: newKind, path: newPath.trim(), name: newName.trim() });
      if (!mounted.current || epoch.current !== stamp) return;
      if (result.status === "ok") {
        setStatus(result);
        setNewPath("");
        setNewName("");
        setNotice(result.message || t("已添加自定义目标；点「接入」写入配置。"));
      } else {
        setActionError(result.message || t("添加失败，请重试。"));
      }
    } catch (reason) {
      if (mounted.current && epoch.current === stamp) setActionError(errorText(reason));
    } finally {
      if (mounted.current && epoch.current === stamp) {
        lock.current = false;
        setBusy("");
      }
    }
  };

  /** 改名只改显示名，不动客户端配置；名字清空表示恢复默认显示（客户端类型名）。 */
  const renameCustomTarget = async () => {
    if (lock.current || !renaming) return;
    const { id, draft } = renaming;
    const current = status?.customTargets?.find((item) => item.id === id);
    if (current && current.name === draft.trim()) {
      setRenaming(null);
      return;
    }
    lock.current = true;
    const stamp = ++epoch.current;
    setBusy(`custom:rename:${id}`);
    setActionError("");
    setNotice("");
    try {
      const result = await invoke<CuStatus>("computer_use_custom_rename", { id, name: draft.trim() });
      if (!mounted.current || epoch.current !== stamp) return;
      if (result.status === "ok") {
        setStatus(result);
        setRenaming(null);
        setNotice(result.message || t("已更新名称。"));
      } else {
        setActionError(result.message || t("改名失败，请重试。"));
      }
    } catch (reason) {
      if (mounted.current && epoch.current === stamp) setActionError(errorText(reason));
    } finally {
      if (mounted.current && epoch.current === stamp) {
        lock.current = false;
        setBusy("");
      }
    }
  };

  const removeCustomTarget = async (target: CuCustomTarget) => {
    if (lock.current) return;
    const confirmed = await appConfirm(
      `${t("删除后会先移除写入")} ${target.configPath} ${t("的 Codex++ 条目（别人写的不动），再从列表里去掉。")}`,
      { title: t("删除自定义目标"), confirmText: t("删除"), danger: true },
    );
    if (!confirmed || !mounted.current) return;
    lock.current = true;
    const stamp = ++epoch.current;
    setBusy(`custom:remove:${target.id}`);
    setActionError("");
    setNotice("");
    try {
      const result = await invoke<CuStatus>("computer_use_custom_remove", { id: target.id });
      if (!mounted.current || epoch.current !== stamp) return;
      if (result.status === "ok") {
        setStatus(result);
        setBackups((current) => {
          const next = { ...current };
          delete next[target.id];
          return next;
        });
        setNotice(result.message || t("已删除自定义目标。"));
      } else {
        setActionError(result.message || t("删除失败，请重试。"));
      }
    } catch (reason) {
      if (mounted.current && epoch.current === stamp) setActionError(errorText(reason));
    } finally {
      if (mounted.current && epoch.current === stamp) {
        lock.current = false;
        setBusy("");
      }
    }
  };

  const runCheck = async () => {
    if (lock.current) return;
    lock.current = true;
    const stamp = ++epoch.current;
    setBusy("check");
    setActionError("");
    setNotice("");
    try {
      const result = await invoke<CuCheck>("computer_use_check");
      if (!mounted.current || epoch.current !== stamp) return;
      setCheck(result);
    } catch (reason) {
      if (mounted.current && epoch.current === stamp) setActionError(errorText(reason));
    } finally {
      if (mounted.current && epoch.current === stamp) {
        lock.current = false;
        setBusy("");
      }
    }
  };

  const locked = Boolean(busy);
  const loading = busy === "refresh" && !status;
  const found = status?.runtimeFound === true;
  const apps = (check?.sampleApps ?? []).slice(0, 8);
  const codexLeftover = status?.targets?.codex?.registered === true;

  // 已接入的客户端数：内置目标 + 自定义目标，「需更新」也算已接入。
  const connectedCount = [
    ...targetIds.map((id) => status?.targets?.[id]),
    ...(status?.customTargets ?? []),
  ].filter((target) => target?.registered).length;
  const activeTab = tab;
  const cuTabs: Array<{ id: CuTab; label: string }> = [
    { id: "overview", label: t("总览") },
    { id: "preview", label: t("窗口预览") },
    { id: "diagnose", label: t("诊断") },
  ];
  const approvalMode: CuApprovalMode = approval?.mode ?? "auto";
  const cuMode: CuMode = approval?.cuMode ?? "js";

  /** 左列的一个客户端：状态点 + 名称 + 一键接入/移除。路径等细节放在悬停提示与下方折叠区。 */
  const flowClient = (id: string, label: string, target: CuTarget | undefined, custom?: CuCustomTarget) => {
    const state = target?.state ?? "missing";
    const working = busy === `install:${id}` || busy === `uninstall:${id}`;
    const tip = [target?.configPath, target?.message].filter(Boolean).join("\n");
    const tone = !status ? "off" : state === "registered" ? "ok" : state === "needs_update" ? "warn" : "off";
    return <div className="hub-node cu-hub-client" data-node={`cu-client:${id}`} data-state={status ? state : "unknown"} key={id} title={tip || undefined}>
      <span className="hub-node-text">
        <strong>{label}<i className="hub-dot" data-tone={tone} />{custom && <em>{t("自定义")}</em>}</strong>
        <small>{!status ? t("未检测") : state === "registered" ? t("已接入") : stateLabel(state)}</small>
      </span>
      {!status ? null
        : state === "registered"
          ? <button className="cu-hub-icon-button" type="button" disabled={locked} title={tf("从 {0} 移除", [label])} aria-label={tf("从 {0} 移除", [label])} onClick={() => void mutateTarget(id, "uninstall")}>
              {working ? <Loader2 size={12} className="cu-spin" /> : <Unplug size={12} />}
            </button>
          : state === "unsupported" ? null
            : <button className="cu-hub-action" type="button" disabled={locked || !found} title={!found ? t("先安装并打开一次 ChatGPT 后再接入") : target?.message} onClick={() => void mutateTarget(id, "install")}>
                {working ? <Loader2 size={12} className="cu-spin" /> : null}{state === "needs_update" ? t("更新") : t("接入")}
              </button>}
    </div>;
  };

  /** 右列「被操作的对象」：图标 + 名称 + 经由哪个运行时；运行时缺失时显示原因与补救按钮。 */
  const flowTarget = (node: string, icon: ReactNode, title: string, detail: ReactNode, warn: boolean, extra?: ReactNode) => (
    <div className="hub-node cu-hub-target" data-node={node} data-warn={warn || undefined}>
      <span className="hub-node-icon" aria-hidden="true">{icon}</span>
      <span className="hub-node-text">
        <strong>{title}{warn ? <i className="hub-dot" data-tone="warn" /> : null}</strong>
        <small>{detail}</small>
      </span>
      {extra}
    </div>
  );

  const openTargetDetails = () => {
    setShowTargets(true);
    // 等 <details> 展开后再定位：直接滚到添加表单并聚焦输入框，而不是停在列表开头。
    requestAnimationFrame(() => {
      const form = targetsDetails.current?.querySelector<HTMLElement>(".cu-custom-add");
      form?.scrollIntoView({ behavior: "smooth", block: "center" });
      form?.querySelector<HTMLInputElement>(".cu-custom-path")?.focus({ preventScroll: true });
    });
  };

  /**
   * 工作原理图，布局与网关总览一致：左列 AI 客户端 → 中间 Codex++ MCP 服务 → 右列被操作的对象，
   * 用同一个 HubTopology 按节点真实位置画曲线。已接入的客户端实线连到中间，未接入的虚线；
   * 运行时（ChatGPT.app、Chrome 扩展）缺失时右侧连线变成告警色。
   */
  const renderFlow = () => {
    const clients: Array<{ id: string; label: string; target: CuTarget | undefined; custom?: CuCustomTarget }> = [
      ...targetIds.map((id) => ({ id, label: status?.targets?.[id]?.label || targetFallback[id].label, target: status?.targets?.[id] })),
      ...(status?.customTargets ?? []).map((target) => ({ id: target.id, label: target.label, target, custom: target })),
    ];
    const chromeReady = extStatus?.installed === true;
    const links: HubLink[] = [
      ...clients.map((client): HubLink => {
        const on = client.target?.registered === true;
        return { from: `cu-client:${client.id}`, to: "cu-core", tone: on ? "idle" : "off", weight: on ? 0.25 : 0 };
      }),
      { from: "cu-core", to: "cu-target:desktop", tone: !status || found ? "idle" : "warn", weight: 0.25 },
      { from: "cu-core", to: "cu-target:chrome", tone: extStatus == null || chromeReady ? "idle" : "warn", weight: 0.25 },
    ];
    return <section className="cu-card cu-flow-card" aria-labelledby="cu-flow-title">
      <div className="cu-card-head">
        <div>
          <h2 id="cu-flow-title">{t("工作原理")}</h2>
          <p>{t("Codex++ 把 ChatGPT.app 里现成的 Computer Use 与 Chrome 扩展，通过 MCP 转给其它 AI 客户端。左边点「接入」；操作方式、审批与悬浮预览在图下方设置。")}</p>
        </div>
      </div>
      <HubTopology links={links}>
        <div className="hub-col">
          <div className="hub-col-head"><span>{t("你的 AI 客户端")}</span><span>{status ? tf("已接入 {0} 个", [connectedCount]) : ""}</span></div>
          {clients.map((client) => flowClient(client.id, client.label, client.target, client.custom))}
          <button className="hub-node hub-node-ghost" type="button" onClick={openTargetDetails}><Plus size={14} aria-hidden="true" />{t("添加自定义配置目录")}</button>
        </div>

        <div className="hub-core cu-hub-core" data-node="cu-core" data-running={found}>
          <img alt="" src={APP_LOGO_URL} />
          <strong>{t("Codex++ MCP 服务")}</strong>
          <div className="cu-hub-services">
            <div><MousePointerClick size={14} aria-hidden="true" /><b>codex-plus-cu</b><small>{t("桌面操控")}</small></div>
            <div><Globe size={14} aria-hidden="true" /><b>codex-plus-browser</b><small>{cuMode === "js" ? t("JS 方式下并入 codex-plus-cu") : t("浏览器控制")}</small></div>
          </div>
          <div className="hub-caps" title={t("在下方「codex-plus-cu 设置」里修改")}>
            <span data-accent={cuMode === "js" ? "true" : undefined}>{cuMode === "js" ? t("JS（同 Codex）") : t("离散工具")}</span>
            <span><ShieldCheck size={11} aria-hidden="true" />{approvalMode === "auto" ? t("自动放行") : t("弹卡确认")}</span>
            {approval?.pip ?? true ? <span>{t("悬浮预览")}</span> : null}
          </div>
        </div>

        <div className="hub-col">
          <div className="hub-col-head"><span>{t("被操作的对象")}</span><span /></div>
          {flowTarget("cu-target:desktop", <AppWindow size={16} />, t("本机桌面应用"),
            !status ? t("检测中…") : found ? t("经 ChatGPT.app 原生 Computer Use") : t("未找到 ChatGPT.app，先安装 Codex 桌面版"),
            Boolean(status) && !found)}
          {flowTarget("cu-target:chrome", <Globe size={16} />, t("Chrome 标签页"),
            extStatus == null ? t("检测中…") : chromeReady ? t("经 ChatGPT for Chrome 扩展，用你已登录的浏览器") : t("未安装 ChatGPT for Chrome 扩展"),
            extStatus != null && !chromeReady,
            extStatus && !chromeReady
              ? <button className="cu-hub-action" type="button" disabled={locked} onClick={() => void invoke("computer_use_open_extension_store")}>{t("安装扩展")}</button>
              : null)}
        </div>
      </HubTopology>
    <div className="cu-settings" aria-labelledby="cu-settings-title">
      <div className="cu-settings-head" id="cu-settings-title">{t("codex-plus-cu 设置")}<small>{t("对新开的客户端会话生效")}</small></div>
      <div className="cu-setting">
        <div className="cu-setting-copy">
          <strong>{t("操作方式")}</strong>
          <small>{t("推荐 JS：与 Codex 当前的用法一致，桌面和 Chrome 都在同一个 js 工具里；首次使用自带完整说明和确认规则；一次调用能连续做多步操作，更快、更省 token；能用 paste 输入中文和多行文字。")}</small>
          {cuMode === "js"
            ? <small className="cu-flow-warn">{t("模型可执行任意 JavaScript（能读写文件、联网）。")}</small>
            : <small className="cu-flow-warn">{t("离散工具每个动作一次调用，返回都带截图；中文需用 set_value。")}</small>}
        </div>
        <div className="mg-segmented" role="radiogroup" aria-label={t("操作方式")}>
          <button type="button" role="radio" aria-checked={cuMode === "js"} data-active={cuMode === "js"} disabled={locked}
            title={t("和 Codex 一样，用 cua_repl 的 js 工具操作桌面与 Chrome；首次使用会自动下发完整说明与确认规则，支持 paste 输入中文。")} onClick={() => void changeCuMode("js")}>{t("JS（同 Codex）")}<span className="cu-flow-recommend">{t("推荐")}</span></button>
          <button type="button" role="radio" aria-checked={cuMode === "tools"} data-active={cuMode === "tools"} disabled={locked}
            title={t("原生 Computer Use 的 10 个离散工具：get_app_state、click、type_text 等。")} onClick={() => void changeCuMode("tools")}>{t("离散工具")}</button>
        </div>
      </div>
      <div className="cu-setting">
        <div className="cu-setting-copy">
          <strong>{t("首次接管某个应用时")}</strong>
          <small>{t("弹卡确认同 Codex：由当前连接的客户端（Claude Code、Codex 等）弹卡，拒绝即不放行，勾「始终允许」后这个应用以后不再问；客户端不支持弹卡时退回自动放行。自动放行：不询问，只记审计日志。")}</small>
          {approvalMode === "claude" && approval?.lastClient?.elicitation === false
            ? <small className="cu-flow-warn">{t("最近连接的客户端不支持审批卡，会退回自动放行")}</small>
            : null}
        </div>
        <div className="mg-segmented" role="radiogroup" aria-label={t("审批策略")}>
          <button type="button" role="radio" aria-checked={approvalMode === "auto"} data-active={approvalMode === "auto"} disabled={locked}
            title={t("自动放行所有应用，只记审计日志。")} onClick={() => void changeApprovalMode("auto")}>{t("自动放行")}</button>
          <button type="button" role="radio" aria-checked={approvalMode === "claude"} data-active={approvalMode === "claude"} disabled={locked}
            title={t("和 Codex 一样，把许可请求转给当前连接的客户端弹卡确认；拒绝即不放行。")} onClick={() => void changeApprovalMode("claude")}>{t("弹卡确认")}</button>
        </div>
      </div>
      <label className="cu-setting">
        <span className="cu-setting-copy">
          <strong>{t("操作后台应用时显示悬浮预览")}</strong>
          <small>{t("同 Codex 的画中画：在屏幕右上角实时显示目标窗口；回合结束或空闲 20 秒自动收起。需要屏幕录制权限。")}</small>
        </span>
        <input className="cu-setting-check" type="checkbox" checked={approval?.pip ?? true} disabled={locked} onChange={(event) => void changePip(event.target.checked)} />
      </label>
    </div>
    <p className="cu-flow-foot">{cuMode === "js"
      ? t("接入后重启对应客户端。JS 方式下桌面与浏览器都用 mcp__codex-plus-cu__js，codex-plus-browser 不提供工具。")
      : t("接入后重启对应客户端，工具名以 mcp__codex-plus-cu__ 与 mcp__codex-plus-browser__ 开头。")}</p>
    </section>;
  };

  /** 一行接入目标。内置与自定义共用；传了 `custom` 才显示「自定义」标记和删除按钮。 */
  const renderTarget = (id: string, target: CuTarget | undefined, label: string, configPath: string, custom?: CuCustomTarget) => {
    const state = target?.state ?? "missing";
    const backup = backups[id];
    const installing = busy === `install:${id}`;
    const removing = busy === `uninstall:${id}`;
    const deleting = busy === `custom:remove:${id}`;
    return <div className="cu-target" key={id} data-custom={custom ? "true" : undefined}>
      <div className="cu-target-main">
        <div className="cu-target-title">
          {custom && renaming?.id === id
            ? <span className="cu-rename">
                <input
                  aria-label={t("名称")}
                  autoFocus
                  className="cu-rename-input"
                  disabled={locked}
                  maxLength={CUSTOM_NAME_MAX}
                  onChange={(event) => setRenaming({ id, draft: event.target.value })}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") void renameCustomTarget();
                    if (event.key === "Escape") setRenaming(null);
                  }}
                  placeholder={targetFallback[custom.kind]?.label}
                  value={renaming.draft}
                />
                <button className="cu-flow-icon-button" type="button" disabled={locked} title={t("保存名称")} aria-label={t("保存名称")} onClick={() => void renameCustomTarget()}>
                  {busy === `custom:rename:${id}` ? <Loader2 size={12} className="cu-spin" /> : <Check size={12} />}
                </button>
                <button className="cu-flow-icon-button" type="button" disabled={locked} title={t("取消")} aria-label={t("取消")} onClick={() => setRenaming(null)}>
                  <X size={12} />
                </button>
              </span>
            : <>
                <strong>{label}</strong>
                {custom && <button className="cu-flow-icon-button" type="button" disabled={locked} title={t("改名")} aria-label={tf("给 {0} 改名", [label])} onClick={() => setRenaming({ id, draft: custom.name })}>
                  <Pencil size={12} />
                </button>}
              </>}
          {/* 起了名字时标签补上客户端类型，否则只看名字分不出是哪种客户端。 */}
          {custom && <span className="cu-badge cu-badge-custom">{custom.name ? `${targetFallback[custom.kind]?.label ?? custom.kind} · ${t("自定义")}` : t("自定义")}</span>}
          <span className="cu-badge" data-state={status ? state : "unknown"}>{status ? stateLabel(state) : t("未检测")}</span>
        </div>
        <code className="cu-target-path" title={configPath}>{elidePath(configPath)}</code>
        {target?.message && !redundantMessages.has(target.message) && <small className="cu-target-message">{target.message}</small>}
        {backup && <small className="cu-target-backup">{t("已备份到")}<code title={backup}>{elidePath(backup)}</code></small>}
      </div>
      <div className="cu-target-actions">
        {custom && <button className="mg-button" type="button" disabled={locked} title={t("移除写入的条目并删除这个自定义目标")} onClick={() => void removeCustomTarget(custom)}>
          {deleting ? <Loader2 size={14} className="cu-spin" /> : <Trash2 size={14} />}{deleting ? t("删除中…") : t("删除")}
        </button>}
        <button className="mg-button" type="button" disabled={locked || !status || !removable(state)} onClick={() => void mutateTarget(id, "uninstall")}>
          {removing ? <Loader2 size={14} className="cu-spin" /> : <Unplug size={14} />}{removing ? t("移除中…") : t("移除")}
        </button>
        <button className="mg-button mg-primary" type="button" disabled={locked || !status || !found || !installable(state)} title={status && !found ? t("先安装并打开一次 ChatGPT 后再接入") : state === "unsupported" ? t("该客户端不支持接入") : undefined} onClick={() => void mutateTarget(id, "install")}>
          {installing ? <Loader2 size={14} className="cu-spin" /> : <Plug size={14} />}{installing ? t("接入中…") : t("接入")}
        </button>
      </div>
    </div>;
  };

  return <div className="mg-page cu-page">
    <PageHeader icon={<MonitorSmartphone size={20} strokeWidth={1.5} />} title="Computer Use" actions={
      <button className="mg-button" type="button" onClick={() => void loadStatus()} disabled={locked} title={t("重新检测本机运行时与接入状态")}>
        <RefreshCw size={15} className={busy === "refresh" ? "cu-spin" : ""} />{busy === "refresh" ? t("检测中…") : t("重新检测")}
      </button>
    } />
    <div className="cu-content" aria-busy={loading}>
      {readError && <p className="cu-error" role="alert">{t("读取状态失败：")}{readError}</p>}
      {actionError && <p className="cu-error" role="alert">{t("操作失败：")}{actionError}</p>}
      {notice && <p className="cu-notice" role="status">{notice}</p>}
      {loading && <p className="cu-loading" role="status"><Loader2 size={15} className="cu-spin" />{t("正在检测本机运行时…")}</p>}

      {/* 运行时缺失会让所有分页都不可用，放在分页之外，在哪一页都看得到。 */}
      {status && !found && <div className="cu-runtime-missing" role="status">
        <AlertTriangle size={16} />
        <div>
          <strong>{t("没找到 ChatGPT.app")}</strong>
          <p>{status.configHint || t("请先安装 Codex 桌面版，安装后点「重新检测」。")}</p>
        </div>
        <button className="mg-button" type="button" onClick={() => void openChatGpt()} disabled={locked}>
          {busy === "open" ? <Loader2 size={14} className="cu-spin" /> : <Play size={14} />}{busy === "open" ? t("正在打开…") : t("打开 ChatGPT")}
        </button>
      </div>}

      {/* 与会话页等其他页面同一种下划线页签（tp-tabs）。 */}
      <div className="tp-tabs cu-tabs" role="tablist" aria-label={t("Computer Use 分页")}>
        {cuTabs.map((item) => <button key={item.id} type="button" role="tab" aria-selected={activeTab === item.id} data-active={activeTab === item.id} onClick={() => setTab(item.id)}>{item.label}</button>)}
      </div>

      {activeTab === "overview" && <>
        {renderFlow()}
        {status?.claudeDesktopRunning ? (
          <p className="cu-note cu-note-warn">{t("Claude Desktop 正在运行：它退出时会重写自己的配置，接入前建议先完全退出 Claude Desktop，改完再打开。")}</p>
        ) : null}
        {codexLeftover ? (
          <div className="cu-note cu-note-warn cu-leftover" role="alert">
            <span>{t("Codex 配置里残留了旧版本写入的 Codex++ Computer Use 条目，会和 Codex 自带的重复，可能出现 ambiguous 报错。移除后重启 Codex 生效。")}</span>
            <button className="mg-button" type="button" disabled={locked} onClick={() => void mutateTarget("codex", "uninstall")}>{t("移除")}</button>
          </div>
        ) : null}

        <details className="cu-more" ref={targetsDetails} open={showTargets} onToggle={(event) => setShowTargets(event.currentTarget.open)}>
          <summary><strong>{t("配置路径与自定义目录")}</strong><small>{t("每个客户端写入的配置文件与备份位置；客户端换了配置目录时在这里添加。")}</small></summary>
          <section className="cu-card" aria-labelledby="cu-targets-title">
            <div className="cu-card-head">
              <div>
                <h2 id="cu-targets-title">{t("接入目标")}</h2>
                <p>{t("登记两个 MCP 服务：桌面操控（直通 Codex 原生 CU）与 Chrome 浏览器控制。写入前会自动备份原配置，只改自己写入的条目。")}</p>
              </div>
            </div>
            <div className="cu-targets">
              {targetIds.map((id) => {
                const fallback = targetFallback[id];
                const target = status?.targets?.[id];
                return renderTarget(id, target, target?.label || fallback.label, target?.configPath || fallback.configPath);
              })}
              {(status?.customTargets ?? []).map((target) => renderTarget(target.id, target, target.label, target.configPath, target))}
            </div>

            <div className="cu-custom-add">
              <div className="cu-custom-add-head">
                <strong>{t("添加自定义配置目录")}</strong>
                <p className="cu-hint">{t("客户端改用了别的配置目录时（例如被包装器设置了 CLAUDE_CONFIG_DIR），上面的默认路径它读不到。在这里登记它实际读取的目录，可添加任意多个；也可以直接填配置文件路径。")}</p>
              </div>
              <div className="cu-custom-add-row">
                <AppSelect
                  ariaLabel={t("客户端类型")}
                  className="cu-custom-kind-select"
                  disabled={locked}
                  onChange={(next) => setNewKind(next)}
                  options={customKinds.map((kind) => ({ value: kind, label: targetFallback[kind].label }))}
                  value={newKind}
                />
                <input
                  className="cu-custom-name"
                  type="text"
                  value={newName}
                  disabled={locked}
                  maxLength={CUSTOM_NAME_MAX}
                  placeholder={t("名称（可选）")}
                  aria-label={t("名称")}
                  onChange={(event) => setNewName(event.target.value)}
                  onKeyDown={(event) => { if (event.key === "Enter") void addCustomTarget(); }}
                />
                <input
                  className="cu-custom-path"
                  type="text"
                  value={newPath}
                  disabled={locked}
                  spellCheck={false}
                  placeholder={t(customDirHint[newKind])}
                  aria-label={t("配置目录")}
                  onChange={(event) => setNewPath(event.target.value)}
                  onKeyDown={(event) => { if (event.key === "Enter") void addCustomTarget(); }}
                />
                <button className="mg-button" type="button" disabled={locked} onClick={() => void pickCustomDir()}>
                  <FolderOpen size={14} />{t("选择目录")}
                </button>
                <button className="mg-button mg-primary" type="button" disabled={locked || !newPath.trim()} onClick={() => void addCustomTarget()}>
                  {busy === "custom:add" ? <Loader2 size={14} className="cu-spin" /> : <Plus size={14} />}{t("添加")}
                </button>
              </div>
            </div>
          </section>
        </details>

        <details className="cu-more">
          <summary><strong>{t("审批记录")}</strong><small>{t("最近连接的客户端是否支持审批卡、浏览器回合钩子、始终允许的应用。")}</small></summary>
          <section className="cu-card" aria-labelledby="cu-approval-title">
            <div className="cu-card-head">
              <div>
                <h2 id="cu-approval-title">{t("审批记录")}</h2>
                <p>{t("审批方式在上方「codex-plus-cu 设置」里切换；这里是客户端能力与始终允许的应用。")}</p>
              </div>
            </div>
            {approval?.lastClient && <p
              className={approval.mode === "claude" && approval.lastClient.elicitation === false ? "cu-note cu-note-warn" : "cu-hint"}
              role={approval.mode === "claude" && approval.lastClient.elicitation === false ? "alert" : undefined}
            >
              {t("最近连接的客户端：")}<code>{[approval.lastClient.name, approval.lastClient.version].filter(Boolean).join(" ") || t("未知")}</code>
              {approval.lastClient.elicitation
                ? t("，支持审批卡。")
                : approval.mode === "claude"
                  ? t("，未声明审批卡能力（elicitation）。在这个客户端里「弹卡确认」会退回为自动放行，不会弹卡。")
                  : t("，未声明审批卡能力（elicitation）；如果切到「弹卡确认」，在这个客户端里会退回为自动放行。")}
            </p>}
            {approval?.turnHooks && <p className={approval.turnHooks.installed ? "cu-hint" : "cu-note cu-note-warn"}>
              {approval.turnHooks.installed
                ? t("浏览器回合钩子已装好：") + (approval.turnHooks.events ?? []).join(" / ") + t("，回合结束时自动关闭未标记的标签页。")
                : t("未安装浏览器回合钩子。安装 Claude Code 接入后，回合结束时未标记的标签页不会自动关闭，会越堆越多。")}
            </p>}
            {(approval?.approvals?.length ?? 0) > 0 && <div className="cu-approvals">
              <div className="cu-approvals-head">
                <strong>{t("始终允许的应用")}</strong>
                <button className="mg-button" type="button" disabled={locked} title={t("Codex 里的「始终允许」也会一起清掉。")} onClick={() => void clearApprovals()}>
                  {busy === "approval:clear" ? <Loader2 size={14} className="cu-spin" /> : null}{t("全部清空")}
                </button>
              </div>
              <ul className="cu-approval-list">
                {approval!.approvals!.map(([key, entry]) => (
                  <li key={key}>
                    <span className="cu-approval-name">{entry.displayName || key}</span>
                    <code className="cu-approval-key" title={key}>{entry.displayName ? key : ""}</code>
                    <span className="cu-approval-decision" data-decision={entry.decision}
                      title={entry.source === "computer-use" ? t("Computer Use 自己记的，Codex 也读这一份；撤销后 Codex 里也会重新询问。") : t("旧版 Codex++ 记录，只对 Codex++ 生效。")}>
                      {entry.decision !== "allow" ? t("拒绝") : entry.source === "computer-use" ? t("与 Codex 共用") : t("Codex++ 旧记录")}
                    </span>
                    <button className="mg-button" type="button" disabled={locked} onClick={() => void forgetApproval(key)}>
                      {busy === `forget:${key}` ? <Loader2 size={13} className="cu-spin" /> : null}{t("撤销")}
                    </button>
                  </li>
                ))}
              </ul>
            </div>}
            {approval && (approval.approvals?.length ?? 0) === 0 && <p className="cu-hint">{t("还没有始终允许的应用。在「弹卡确认」的卡片上勾选「始终允许」，或在 Codex 里选「始终允许」后，应用会出现在这里（两边共用）。")}</p>}
          </section>
        </details>
      </>}

      {activeTab === "preview" && <>
        <ComputerUsePreview />
      </>}

      {activeTab === "diagnose" && <>
        <section className="cu-card cu-runtime" data-found={found} aria-labelledby="cu-runtime-title">
          <div className="cu-card-head">
            <div>
              <h2 id="cu-runtime-title">{t("运行时")}</h2>
              <p>{t("Computer Use 的服务由本机 ChatGPT.app 提供，接入前先确认它能被找到。")}</p>
            </div>
            <span className="cu-badge" data-state={!status ? "unknown" : found ? "ok" : "missing"}>{!status ? t("未检测") : found ? t("已找到") : t("未找到")}</span>
          </div>
          <dl className="cu-meta">
            <dt>{t("应用路径")}</dt>
            <dd>{hasValue(status?.appPath) ? <code title={status?.appPath}>{status?.appPath}</code> : "—"}</dd>
            <dt>@oai/cua-repl</dt>
            <dd>{hasValue(status?.packageVersion) ? <code>{status?.packageVersion}</code> : "—"}</dd>
            <dt>{t("CU server")}</dt>
            <dd><code>SkyComputerUseClient</code></dd>
            <dt>{t("CU 服务路径")}</dt>
            <dd>{hasValue(status?.servicePath) ? <code title={status?.servicePath}>{status?.servicePath}</code> : "—"}</dd>
          </dl>
        </section>

        <section className="cu-card" aria-labelledby="cu-ext-title">
          <div className="cu-card-head">
            <div>
              <h2 id="cu-ext-title">{t("Chrome 浏览器扩展")}</h2>
              <p>{t("浏览器控制依赖 OpenAI 官方的 ChatGPT for Chrome 扩展。装好后无需额外配置，刷新页面即生效。")}</p>
            </div>
          </div>
          <div className="cu-ext-row">
            {extStatus == null
              ? <span className="cu-hint">{t("检测中…")}</span>
              : extStatus.installed
                ? <span className="cu-ext-ok"><CheckCircle2 size={14} />{t("扩展已安装")}</span>
                : <>
                    <span className="cu-note cu-note-warn">{t("未检测到扩展，浏览器工具不可用。")}</span>
                    <button
                      className="mg-button mg-primary"
                      type="button"
                      disabled={locked}
                      onClick={() => void invoke("computer_use_open_extension_store")}
                    >
                      {t("安装 Chrome 扩展 →")}
                    </button>
                  </>
            }
          </div>
        </section>

        <section className="cu-card" aria-labelledby="cu-check-title">
          <div className="cu-card-head">
            <div>
              <h2 id="cu-check-title">{t("连通性检查")}</h2>
              <p>{t("实际跑一次 MCP 握手并调用 sky.list_apps()，确认工具链可用。")}</p>
            </div>
            <button className="mg-button" type="button" onClick={() => void runCheck()} disabled={locked}>
              {busy === "check" ? <Loader2 size={15} className="cu-spin" /> : <Play size={15} />}{busy === "check" ? t("正在自检…") : t("运行自检")}
            </button>
          </div>
          {!check
            ? <p className="cu-hint">{t("还没有运行自检。")}</p>
            : <div className="cu-check" data-ok={check.ok} role="status">
              <div className="cu-check-head">
                {check.ok ? <CheckCircle2 size={16} /> : <AlertTriangle size={16} />}
                <strong>{check.ok ? t("自检通过") : t("自检未通过")}</strong>
              </div>
              <dl className="cu-meta">
                <dt>{t("目标平台")}</dt>
                <dd>{check.target || "—"}</dd>
                <dt>{t("应用数量")}</dt>
                <dd>{typeof check.appCount === "number" ? check.appCount : "—"}</dd>
                <dt>{t("浏览器")}</dt>
                <dd>{typeof check.browserCount === "number" ? check.browserCount : "—"}</dd>
                <dt>{t("耗时")}</dt>
                <dd>{typeof check.durationMs === "number" ? `${check.durationMs} ms` : "—"}</dd>
              </dl>
              {apps.length > 0 && <ul className="cu-app-list" aria-label={t("示例应用")}>{apps.map((app) => <li key={app}>{app}</li>)}</ul>}
              {!check.ok && <p className="cu-failure">{failureText(check)}</p>}
              {check.message && <p className="cu-hint">{check.message}</p>}
            </div>}
        </section>
        <p className="cu-note">{t("接入后需要重启对应客户端（Claude Code 会话、DeepSeek Harness、ZCode、WorkBuddy）才会加载工具，工具名以")}<code>mcp__codex-plus-cu__</code>{t("与")}<code>mcp__codex-plus-browser__</code>{t("前缀出现在工具列表中。")}</p>
      </>}
    </div>
  </div>;
}
