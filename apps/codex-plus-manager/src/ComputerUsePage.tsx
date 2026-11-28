/** Computer Use：复用 Codex app 的 Computer Use 与 Chrome 控制，通过 MCP 提供给 Claude Code、DeepSeek Harness、ZCode 等客户端。 */
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { appConfirm } from "./app-dialog";
import { AlertTriangle, AppWindow, Check, CheckCircle2, ChevronRight, FolderOpen, Globe, Loader2, Play, Plus, RefreshCw, X } from "lucide-react";
import { ComputerUsePreview } from "./ComputerUsePreview";
import { AppSelect } from "./AppSelect";
import { Menu, MenuItem } from "./ManagementPages";
import { TopbarActions } from "./TopbarActions";
import { AgentGlyph } from "./hub-shared";
import { Switch } from "./gateway-ui";
import { CxAddRow, CxBadge, CxBanner, CxCard, CxRow, CxSection } from "./cx-layout";
import { t, tf } from "./i18n";
import { enhancementPlatformFromUserAgent } from "./enhancement-catalog";
import { announceBackendSettingsChanged } from "./settings-events";
import { errorText } from "./lib/utils";
import "./ComputerUsePage.css";

type CuTargetId = "claudeCode" | "claudeDesktop" | "codex" | "dsh" | "zcode" | "workBuddy" | "devin";
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
export { BACKEND_SETTINGS_CHANGED_EVENT } from "./settings-events";
const announceSettingsChanged = announceBackendSettingsChanged;
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
const targetIds: CuTargetId[] = ["claudeCode", "claudeDesktop", "dsh", "zcode", "workBuddy", "devin"];
const targetFallback: Record<CuTargetId, { label: string; configPath: string }> = {
  claudeCode: { label: "Claude Code", configPath: "~/.claude.json" },
  claudeDesktop: { label: "Claude Desktop", configPath: "claude_desktop_config.json" },
  codex: { label: "Codex", configPath: "config.toml" },
  dsh: { label: "DeepSeek Harness", configPath: "~/.dsh/profiles/desktop/cordis.patch.yml" },
  zcode: { label: "ZCode", configPath: "~/.zcode/cli/config.json" },
  workBuddy: { label: "WorkBuddy AI", configPath: "~/.workbuddy-ai/mcp.json" },
  devin: { label: "Devin", configPath: "~/.config/devin/mcp_config.json" },
};
const hasValue = (value?: string) => Boolean(value && value.trim());
// 配置路径通常很长，保留首尾两段比整体省略更能看清是哪个客户端。
const elidePath = (value: string, limit = 52) => {
  if (value.length <= limit) return value;
  const head = Math.ceil(limit * 0.45);
  const tail = limit - head - 1;
  return `${value.slice(0, head)}…${value.slice(-tail)}`;
};
/** 列表行的图标：借用「本地服务」里同一个客户端的图标，没有打包图标的显示首字母。 */
const cuAgentIcon: Record<CuTargetId, string> = {
  claudeCode: "claude",
  claudeDesktop: "claude-desktop",
  codex: "codex",
  dsh: "dsh",
  zcode: "zcode",
  workBuddy: "workbuddy-ai",
  devin: "devin",
};
// 自定义目标可选的类型：与上面列出的内置目标一致，但 Devin 不支持自定义目录（后端 CUSTOM_KINDS 也没有它）。
const customKinds: CuTargetId[] = targetIds.filter((id) => id !== "devin");
// 输入框占位说明：告诉用户该填哪一层目录。渲染时再过 t()。
const customDirHint: Record<CuTargetId, string> = {
  claudeCode: "CLAUDE_CONFIG_DIR 指向的目录（里面有 .claude.json）",
  claudeDesktop: "包含 claude_desktop_config.json 的目录",
  codex: "包含 config.toml 的目录",
  dsh: "包含 cordis.patch.yml 的目录",
  zcode: "包含 config.json 的目录",
  workBuddy: "包含 mcp.json 的目录",
  devin: "包含 mcp_config.json 的目录",
};
// 后端的状态说明里，这两句与徽标完全同义，再显示一行只会把列表拉长。
const redundantMessages = new Set(["已接入", "尚未接入"]);
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
/**
 * 句子里要嵌 <code> 等节点时用：整句按 tf 的模板翻译，再把 {0}、{1}… 换成节点，避免把一句话拆成几段分别翻译。
 */
function tfNodes(key: string, nodes: ReactNode[]): ReactNode[] {
  const template = tf(key, nodes.map((_, index) => `{${index}}`));
  return template.split(/(\{\d+\})/).map((part, index) => {
    const slot = /^\{(\d+)\}$/.exec(part);
    return slot ? <span key={index}>{nodes[Number(slot[1])]}</span> : part;
  });
}

function failureText(check: CuCheck): string {
  switch (check.failureKind) {
    case "unsupported_platform": return t("Computer Use 仅支持 macOS");
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

/** 非 macOS 平台只给出明确的「仅支持 macOS」说明，不去探测运行时，避免误报「找不到 ChatGPT」。 */
function ComputerUseUnsupported() {
  return <div className="mg-page cu-page">
    <div className="cu-content">
      <div className="cu-runtime-missing" role="status" data-reason="unsupported-platform">
        <AlertTriangle size={16} />
        <div>
          <strong>{t("Computer Use 仅支持 macOS")}</strong>
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
  /** 「未接入」是否展开；没动过时按有没有已接入的客户端决定。 */
  const [othersOpen, setOthersOpen] = useState<boolean | null>(null);
  const [approvalsOpen, setApprovalsOpen] = useState(false);
  /** 「添加自定义配置目录」那一行是否已点开成输入行。 */
  const [addingCustom, setAddingCustom] = useState(false);
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

  /**
   * 页面上的操作一次只跑一个：上锁、记下 busy、清掉上一条提示，出错写进操作失败提示。
   * `alive()` 判断结果还该不该写进界面（组件已卸载或被更新的检测顶掉时丢弃）。
   */
  const runExclusive = useCallback(async (label: string, task: (alive: () => boolean) => Promise<void>) => {
    if (lock.current) return;
    lock.current = true;
    const stamp = ++epoch.current;
    const alive = () => mounted.current && epoch.current === stamp;
    setBusy(label);
    setActionError("");
    setNotice("");
    try {
      await task(alive);
    } catch (reason) {
      if (alive()) setActionError(errorText(reason));
    } finally {
      if (alive()) {
        lock.current = false;
        setBusy("");
      }
    }
  }, []);

  /** 保存本页的一个开关：成功后通知主界面同步设置版本号并重读审批状态。 */
  const saveSetting = (label: string, command: string, args: Record<string, unknown>, success: string, failure: string) =>
    runExclusive(label, async (alive) => {
      const result = await invoke<CuResult>(command, args);
      if (!alive()) return;
      if (result.status === "ok") {
        announceSettingsChanged();
        setNotice(result.message || success);
        await loadApproval();
      } else {
        setActionError(result.message || failure);
      }
    });

  const changeApprovalMode = (mode: CuApprovalMode) =>
    saveSetting("approval", "computer_use_approval_set", { mode }, t("审批策略已保存。"), t("保存审批策略失败。"));

  const changeCuMode = async (mode: CuMode) => {
    if (approval?.cuMode === mode) return;
    await saveSetting("cuMode", "computer_use_mode_set", { mode }, t("操作方式已保存；对新开的客户端会话生效。"), t("保存操作方式失败。"));
  };

  const changePip = (enabled: boolean) =>
    saveSetting("pip", "computer_use_pip_set", { enabled }, enabled ? t("已开启悬浮预览。") : t("已关闭悬浮预览。"), t("保存失败，请重试。"));

  const forgetApproval = (app: string) => runExclusive(`forget:${app}`, async (alive) => {
    const result = await invoke<CuResult>("computer_use_approval_forget", { app });
    if (alive() && result.status === "ok") {
      setNotice(result.message || t("已撤销。"));
      await loadApproval();
    }
  });

  // 清空会连 Codex 里的「始终允许」一起清掉，先确认。
  const clearApprovals = async () => {
    if (lock.current) return;
    const confirmed = await appConfirm(t("Codex 里的「始终允许」也会一起清掉，之后每个应用都会重新询问。"), {
      title: t("清空全部审批记录？"),
      confirmText: t("全部清空"),
      danger: true,
    });
    if (!confirmed || !mounted.current) return;
    await runExclusive("approval:clear", async (alive) => {
      const result = await invoke<CuResult>("computer_use_approval_clear");
      if (alive() && result.status === "ok") {
        setNotice(result.message || t("已清空。"));
        await loadApproval();
      }
    });
  };

  const loadStatus = useCallback(() => runExclusive("refresh", async (alive) => {
    setReadError("");
    try {
      const next = await invoke<CuStatus>("computer_use_status");
      if (!alive()) return;
      setStatus(next);
      if (next.status !== "ok") setReadError(next.message || t("检测失败，请重试。"));
    } catch (reason) {
      if (alive()) setReadError(errorText(reason));
    }
  }), [runExclusive]);

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

  const openChatGpt = () => runExclusive("open", async (alive) => {
    const result = await invoke<CuResult>("computer_use_open_app");
    if (!alive()) return;
    if (result.status === "ok") setNotice(t("已请求打开 ChatGPT.app。"));
    else setActionError(result.message || t("打开 ChatGPT.app 失败。"));
  });

  /** `target` 是内置目标 id 或自定义目标的 `custom-…` id。 */
  const mutateTarget = (target: string, action: "install" | "uninstall") => {
    const custom = status?.customTargets?.find((item) => item.id === target);
    const label = custom
      ? tf("{0} · 自定义", [custom.label])
      : status?.targets?.[target as CuTargetId]?.label || targetFallback[target as CuTargetId]?.label || target;
    return runExclusive(`${action}:${target}`, async (alive) => {
      const result = await invoke<CuMutation>(action === "install" ? "computer_use_install" : "computer_use_uninstall", { target });
      if (!alive()) return;
      if (result.targets || result.customTargets) {
        setStatus((current) => (current ? {
          ...current,
          targets: { ...current.targets, ...result.targets },
          customTargets: result.customTargets ?? current.customTargets,
        } : current));
      }
      if (result.status === "ok") {
        setNotice(action === "install" ? tf("{0} · 已接入，重启客户端后加载工具", [label]) : tf("{0} · 已移除", [label]));
        setBackups((current) => {
          const next = { ...current };
          if (action === "install" && result.backupPath) next[target] = result.backupPath;
          else delete next[target];
          return next;
        });
      } else {
        setActionError(result.message || t("操作失败，请重试。"));
      }
    });
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
    if (!newPath.trim()) return;
    await runExclusive("custom:add", async (alive) => {
      const result = await invoke<CuStatus>("computer_use_custom_add", { kind: newKind, path: newPath.trim(), name: newName.trim() });
      if (!alive()) return;
      if (result.status === "ok") {
        setStatus(result);
        setNewPath("");
        setNewName("");
        setAddingCustom(false);
        setNotice(result.message || t("已添加自定义目标；点「接入」写入配置。"));
      } else {
        setActionError(result.message || t("添加失败，请重试。"));
      }
    });
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
    await runExclusive(`custom:rename:${id}`, async (alive) => {
      const result = await invoke<CuStatus>("computer_use_custom_rename", { id, name: draft.trim() });
      if (!alive()) return;
      if (result.status === "ok") {
        setStatus(result);
        setRenaming(null);
        setNotice(result.message || t("已更新名称。"));
      } else {
        setActionError(result.message || t("改名失败，请重试。"));
      }
    });
  };

  const removeCustomTarget = async (target: CuCustomTarget) => {
    if (lock.current) return;
    const confirmed = await appConfirm(
      tf("删除后会先移除写入 {0} 的 Codex++ 条目（别人写的不动），再从列表里去掉。", [target.configPath]),
      { title: t("删除自定义目标"), confirmText: t("删除"), danger: true },
    );
    if (!confirmed || !mounted.current) return;
    await runExclusive(`custom:remove:${target.id}`, async (alive) => {
      const result = await invoke<CuStatus>("computer_use_custom_remove", { id: target.id });
      if (!alive()) return;
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
    });
  };

  const runCheck = () => runExclusive("check", async (alive) => {
    const result = await invoke<CuCheck>("computer_use_check");
    if (alive()) setCheck(result);
  });

  const locked = Boolean(busy);
  const loading = busy === "refresh" && !status;
  const found = status?.runtimeFound === true;
  const apps = (check?.sampleApps ?? []).slice(0, 8);
  const codexLeftover = status?.targets?.codex?.registered === true;

  const activeTab = tab;
  const cuTabs: Array<{ id: CuTab; label: string }> = [
    { id: "overview", label: t("总览") },
    { id: "preview", label: t("窗口预览") },
    { id: "diagnose", label: t("诊断") },
  ];
  const approvalMode: CuApprovalMode = approval?.mode ?? "auto";
  const cuMode: CuMode = approval?.cuMode ?? "js";

  /** 一个接入目标（内置客户端或自定义配置目录）在列表里的一行所需的数据。 */
  type ClientEntry = { id: string; kind: CuTargetId; label: string; target?: CuTarget; configPath: string; custom?: CuCustomTarget };
  const clients: ClientEntry[] = [
    ...targetIds.map((id): ClientEntry => ({
      id,
      kind: id,
      label: status?.targets?.[id]?.label || targetFallback[id].label,
      target: status?.targets?.[id],
      configPath: status?.targets?.[id]?.configPath || targetFallback[id].configPath,
    })),
    ...(status?.customTargets ?? []).map((target): ClientEntry => ({ id: target.id, kind: target.kind, label: target.label, target, configPath: target.configPath, custom: target })),
  ];
  // 「需更新」也算已接入：它已经写在客户端配置里，只是条目旧了。
  const connectedClients = clients.filter((client) => client.target?.registered);
  const otherClients = clients.filter((client) => !client.target?.registered);
  // 一个都没接入时「未接入」直接展开，新用户不用再点一下（同「本地服务」的 Agent 列表）。
  const othersShown = othersOpen ?? connectedClients.length === 0;

  /** 图下方的设置：一行一项，右侧分段控件 / 开关；长说明放悬停。改了即保存，对新开的客户端会话生效。 */
  const renderSettings = () => {
    const jsHint = t("推荐 JS：与 Codex 当前的用法一致，桌面和 Chrome 都在同一个 js 工具里；首次使用自带完整说明和确认规则；一次调用能连续做多步操作，更快、更省 token；能用 paste 输入中文和多行文字。");
    const approvalHint = t("弹卡确认同 Codex：由当前连接的客户端（Claude Code、Codex 等）弹卡，拒绝即不放行，勾「始终允许」后这个应用以后不再问；客户端不支持弹卡时退回自动放行。自动放行：不询问，只记审计日志。");
    const noElicitation = approvalMode === "claude" && approval?.lastClient?.elicitation === false;
    const restartNote = cuMode === "js"
      ? t("接入后重启对应客户端。JS 方式下桌面与浏览器都用 mcp__codex-plus-cu__js，codex-plus-browser 不提供工具。")
      : t("接入后重启对应客户端，工具名以 mcp__codex-plus-cu__ 与 mcp__codex-plus-browser__ 开头。");
    // 「接入后重启客户端、工具名」这段注脚收进右侧小字的悬停，卡片下面不再多一段话。
    return <CxSection aside={<span title={restartNote}>{t("对新开的客户端会话生效")}</span>} title={t("codex-plus-cu 设置")}>
      <CxCard>
        <CxRow
          detail={cuMode === "js" ? t("模型可执行任意 JavaScript（能读写文件、联网）。") : t("离散工具每个动作一次调用，返回都带截图；中文需用 set_value。")}
          title={t("操作方式")}
          tooltip={jsHint}
        >
          <div className="mg-segmented" role="radiogroup" aria-label={t("操作方式")}>
            <button type="button" role="radio" aria-checked={cuMode === "js"} data-active={cuMode === "js"} disabled={locked}
              title={t("和 Codex 一样，用 cua_repl 的 js 工具操作桌面与 Chrome；首次使用会自动下发完整说明与确认规则，支持 paste 输入中文。")} onClick={() => void changeCuMode("js")}>{t("JS（同 Codex）")}<span className="cu-flow-recommend">{t("推荐")}</span></button>
            <button type="button" role="radio" aria-checked={cuMode === "tools"} data-active={cuMode === "tools"} disabled={locked}
              title={t("原生 Computer Use 的 10 个离散工具：get_app_state、click、type_text 等。")} onClick={() => void changeCuMode("tools")}>{t("离散工具")}</button>
          </div>
        </CxRow>
        <CxRow
          badges={noElicitation ? <CxBadge tone="warn" title={t("最近连接的客户端不支持审批卡，会退回自动放行")}>{t("会退回自动放行")}</CxBadge> : null}
          detail={approvalMode === "auto" ? t("自动放行所有应用，只记审计日志。") : t("和 Codex 一样，把许可请求转给当前连接的客户端弹卡确认；拒绝即不放行。")}
          title={t("首次接管某个应用时")}
          tooltip={approvalHint}
        >
          <div className="mg-segmented" role="radiogroup" aria-label={t("审批策略")}>
            <button type="button" role="radio" aria-checked={approvalMode === "auto"} data-active={approvalMode === "auto"} disabled={locked}
              title={t("自动放行所有应用，只记审计日志。")} onClick={() => void changeApprovalMode("auto")}>{t("自动放行")}</button>
            <button type="button" role="radio" aria-checked={approvalMode === "claude"} data-active={approvalMode === "claude"} disabled={locked}
              title={t("和 Codex 一样，把许可请求转给当前连接的客户端弹卡确认；拒绝即不放行。")} onClick={() => void changeApprovalMode("claude")}>{t("弹卡确认")}</button>
          </div>
        </CxRow>
        <CxRow
          detail={t("屏幕右上角实时显示目标窗口，需要屏幕录制权限")}
          title={t("操作后台应用时显示悬浮预览")}
          tooltip={t("同 Codex 的画中画：在屏幕右上角实时显示目标窗口；回合结束或空闲 20 秒自动收起。需要屏幕录制权限。")}
        >
          <Switch disabled={locked} label={t("操作后台应用时显示悬浮预览")} on={approval?.pip ?? true} onToggle={() => void changePip(!(approval?.pip ?? true))} />
        </CxRow>
      </CxCard>
    </CxSection>;
  };

  /**
   * 一行接入目标，样子同「本地服务」的 Agent 行：图标、名称、配置路径，右侧开关接入 / 移除。
   * 内置与自定义共用；传了 `custom` 才显示「自定义」标记，改名和删除收在三点菜单里。
   */
  const renderClient = ({ id, kind, label, target, configPath, custom }: ClientEntry) => {
    const state = target?.state ?? "missing";
    const backup = backups[id];
    const working = busy === `install:${id}` || busy === `uninstall:${id}`;
    const deleting = busy === `custom:remove:${id}`;
    const registered = target?.registered === true;
    const message = target?.message && !redundantMessages.has(target.message) ? target.message : "";
    // 分组标题已经说了接没接入，这里只标需要注意的状态。
    const stateBadge = !status
      ? <CxBadge>{t("未检测")}</CxBadge>
      : state === "needs_update" || state === "foreign"
        ? <CxBadge tone="warn">{stateLabel(state)}</CxBadge>
        : state === "unsupported" ? <CxBadge>{stateLabel(state)}</CxBadge> : null;
    const title = custom && renaming?.id === id
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
      : label;
    const blockedReason = status && !found ? t("先安装并打开一次 ChatGPT 后再接入") : undefined;
    return <CxRow
      key={id}
      badges={<>
        {/* 起了名字时标签补上客户端类型，否则只看名字分不出是哪种客户端。 */}
        {custom ? <CxBadge>{custom.name ? tf("{0} · 自定义", [targetFallback[custom.kind]?.label ?? custom.kind]) : t("自定义")}</CxBadge> : null}
        {stateBadge}
      </>}
      detail={<code className="cx-set-path" title={configPath}>{elidePath(configPath, 72)}</code>}
      icon={<AgentGlyph agent={cuAgentIcon[kind]} />}
      title={title}
      tooltip={[message, backup ? tf("已备份到 {0}", [backup]) : ""].filter(Boolean).join("\n") || undefined}
    >
      {state === "needs_update" || (registered && state === "foreign")
        ? <button className="cx-secondary" type="button" disabled={locked || !found} title={blockedReason} onClick={() => void mutateTarget(id, "install")}>
            {busy === `install:${id}` ? <Loader2 size={14} className="cu-spin" /> : null}{t("更新")}
          </button>
        : null}
      {working ? <Loader2 aria-hidden="true" className="cu-spin cu-row-spin" size={14} /> : null}
      {state === "unsupported" ? null : (
        <span title={!registered ? blockedReason : undefined}>
          <Switch
            disabled={locked || !status || (!registered && (!found || !installable(state)))}
            label={registered ? tf("从 {0} 移除", [label]) : tf("接入 {0}", [label])}
            on={registered}
            onToggle={() => void mutateTarget(id, registered ? "uninstall" : "install")}
          />
        </span>
      )}
      {custom ? (
        <Menu>
          <MenuItem disabled={locked} onClick={() => setRenaming({ id, draft: custom.name })}>{t("改名")}</MenuItem>
          <MenuItem danger disabled={locked} onClick={() => void removeCustomTarget(custom)}>{deleting ? t("删除中…") : t("删除")}</MenuItem>
        </Menu>
      ) : null}
    </CxRow>;
  };

  /** 自定义配置目录：列表末尾一行「添加」，点开后就地变成类型、名称、目录一行填完。 */
  const renderCustomAdd = () => addingCustom
    ? <div className="cu-custom-add" title={t("客户端改用了别的配置目录时（例如被包装器设置了 CLAUDE_CONFIG_DIR），默认路径它读不到。在这里登记它实际读取的目录，可添加任意多个；也可以直接填配置文件路径。")}>
        <div className="cu-custom-add-row">
          <AppSelect
            ariaLabel={t("客户端类型")}
            className="cu-custom-kind-select"
            disabled={locked}
            onChange={(next) => setNewKind(next)}
            options={customKinds.map((kind) => ({ value: kind, label: targetFallback[kind].label }))}
            size="sm"
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
            onKeyDown={(event) => { if (event.key === "Enter") void addCustomTarget(); if (event.key === "Escape") setAddingCustom(false); }}
          />
          <input
            autoFocus
            className="cu-custom-path"
            type="text"
            value={newPath}
            disabled={locked}
            spellCheck={false}
            placeholder={t(customDirHint[newKind])}
            aria-label={t("配置目录")}
            onChange={(event) => setNewPath(event.target.value)}
            onKeyDown={(event) => { if (event.key === "Enter") void addCustomTarget(); if (event.key === "Escape") setAddingCustom(false); }}
          />
          <button className="cx-secondary" type="button" disabled={locked} onClick={() => void pickCustomDir()} title={t("选择目录")} aria-label={t("选择目录")}>
            <FolderOpen size={14} />
          </button>
          <button className="cx-secondary" type="button" disabled={locked || !newPath.trim()} onClick={() => void addCustomTarget()}>
            {busy === "custom:add" ? <Loader2 size={14} className="cu-spin" /> : <Plus size={14} />}{t("添加")}
          </button>
          <button className="cu-flow-icon-button" type="button" disabled={locked} title={t("取消")} aria-label={t("取消")} onClick={() => setAddingCustom(false)}>
            <X size={12} />
          </button>
        </div>
      </div>
    : <CxAddRow
        detail={t("客户端改用了别的配置目录（例如设置了 CLAUDE_CONFIG_DIR）时，登记它实际读取的目录")}
        disabled={locked}
        onClick={() => setAddingCustom(true)}
        title={t("添加自定义配置目录")}
      />;

  const lastClientWarn = approval?.mode === "claude" && approval?.lastClient?.elicitation === false;

  return <div className="mg-page cu-page cx-cu-page">
    <TopbarActions>
      <button className="cx-secondary" type="button" onClick={() => void loadStatus()} disabled={locked} title={t("重新检测本机运行时与接入状态")}>
        <RefreshCw size={14} className={busy === "refresh" ? "cu-spin" : ""} />{busy === "refresh" ? t("检测中…") : t("重新检测")}
      </button>
    </TopbarActions>
    <div className="cu-content" aria-busy={loading}>
      {/* 页面顶部最多一条提示：出错 > 运行时缺失 > 刚完成的操作。 */}
      {readError || actionError
        ? <div role="alert"><CxBanner icon={<AlertTriangle />} title={readError ? tf("读取状态失败：{0}", [readError]) : tf("操作失败：{0}", [actionError])} tone="warn" /></div>
        : status && !found
          // 运行时缺失会让所有分页都不可用，放在分页之外，在哪一页都看得到。
          ? <div role="status"><CxBanner icon={<AlertTriangle />} title={t("没找到 ChatGPT.app")} detail={status.configHint || t("请先安装 Codex 桌面版，安装后点「重新检测」。")} tone="warn">
              <button className="cx-secondary" type="button" onClick={() => void openChatGpt()} disabled={locked}>
                {busy === "open" ? <Loader2 size={14} className="cu-spin" /> : <Play size={14} />}{busy === "open" ? t("正在打开…") : t("打开 ChatGPT")}
              </button>
            </CxBanner></div>
          : notice ? <p className="cx-set-notice" role="status"><CheckCircle2 size={14} />{notice}</p> : null}
      {loading && <p className="cu-loading" role="status"><Loader2 size={15} className="cu-spin" />{t("正在检测本机运行时…")}</p>}

      {/* 与会话页等其他页面同一种下划线页签（tp-tabs）。 */}
      <div className="tp-tabs cu-tabs" role="tablist" aria-label={t("Computer Use 分页")}>
        {cuTabs.map((item) => <button key={item.id} type="button" role="tab" aria-selected={activeTab === item.id} data-active={activeTab === item.id} onClick={() => setTab(item.id)}>{item.label}</button>)}
      </div>

      {activeTab === "overview" && <>
        {codexLeftover ? (
          <div role="alert">
            <CxBanner icon={<AlertTriangle />} title={t("Codex 配置里残留了旧版本写入的 Codex++ Computer Use 条目，会和 Codex 自带的重复，可能出现 ambiguous 报错。移除后重启 Codex 生效。")} tone="warn">
              <button className="cx-secondary" type="button" disabled={locked} onClick={() => void mutateTarget("codex", "uninstall")}>{t("移除")}</button>
            </CxBanner>
          </div>
        ) : status?.claudeDesktopRunning ? (
          <CxBanner icon={<AlertTriangle />} title={t("Claude Desktop 正在运行：它退出时会重写自己的配置，接入前建议先完全退出 Claude Desktop，改完再打开。")} tone="warn" />
        ) : null}

        {/* 同「本地服务」的 Agent 列表：「已接入」「未接入」两节，每个客户端一行，开关接入 / 移除；接入后重启客户端加载工具。 */}
        {connectedClients.length ? (
          <CxSection aside={<span>{t("接入后重启对应客户端")}</span>} count={connectedClients.length} title={t("已接入")}>
            <CxCard>{connectedClients.map(renderClient)}</CxCard>
          </CxSection>
        ) : null}
        <CxSection
          className="cu-clients-others"
          title={(
            <button aria-expanded={othersShown} className="cx-section-toggle" onClick={() => setOthersOpen(!othersShown)} type="button">
              <ChevronRight aria-hidden="true" className="h-3.5 w-3.5" />
              {t("未接入")}
              {otherClients.length ? <span className="cx-section-count">{otherClients.length}</span> : null}
            </button>
          )}
        >
          {othersShown ? <CxCard>{otherClients.map(renderClient)}{renderCustomAdd()}</CxCard> : null}
        </CxSection>

        {/* 工具最终操作的是什么：本机桌面应用经 ChatGPT.app，Chrome 经官方扩展。 */}
        <CxSection title={<span title={t("Codex++ 把 ChatGPT.app 里现成的 Computer Use 与 Chrome 扩展，通过 MCP（codex-plus-cu、codex-plus-browser）转给其它 AI 客户端。")}>{t("能操作的对象")}</span>}>
          <CxCard>
            <CxRow
              badges={!status ? null : <CxBadge tone={found ? "ok" : "warn"}>{found ? t("可用") : t("未找到")}</CxBadge>}
              detail={!status ? t("检测中…") : found ? t("经 ChatGPT.app 自带的 Computer Use") : t("未找到 ChatGPT.app，先安装 Codex 桌面版")}
              icon={<AppWindow />}
              title={t("本机桌面应用")}
            />
            <CxRow
              badges={extStatus == null ? null : <CxBadge tone={extStatus.installed ? "ok" : "warn"}>{extStatus.installed ? t("可用") : t("未安装")}</CxBadge>}
              detail={extStatus == null ? t("检测中…") : extStatus.installed ? t("经 ChatGPT for Chrome 扩展，用你已登录的浏览器") : t("需要安装 ChatGPT for Chrome 扩展")}
              icon={<Globe />}
              title={t("Chrome 标签页")}
            >
              {extStatus && !extStatus.installed
                ? <button className="cx-secondary" type="button" disabled={locked} onClick={() => void invoke("computer_use_open_extension_store")}>{t("安装扩展")}</button>
                : null}
            </CxRow>
          </CxCard>
        </CxSection>

        {renderSettings()}

        {/* 审批记录不常看，默认折叠，同「未接入」那种可点的分组标题。 */}
        <CxSection
          title={(
            <button aria-expanded={approvalsOpen} className="cx-section-toggle" onClick={() => setApprovalsOpen(!approvalsOpen)} title={t("客户端能力与始终允许的应用")} type="button">
              <ChevronRight aria-hidden="true" className="h-3.5 w-3.5" />
              {t("审批记录")}
              <span className="cx-section-count">{approval?.approvals?.length ?? 0}</span>
            </button>
          )}
        >
          {approvalsOpen ? <CxCard>
            {approval?.lastClient ? <CxRow
              badges={<CxBadge tone={approval.lastClient.elicitation ? "ok" : lastClientWarn ? "warn" : "neutral"}>{approval.lastClient.elicitation ? t("支持审批卡") : t("不支持审批卡")}</CxBadge>}
              detail={<code className="cx-set-path">{[approval.lastClient.name, approval.lastClient.version].filter(Boolean).join(" ") || t("未知")}</code>}
              title={t("最近连接的客户端")}
              tooltip={(() => {
                const client = approval.lastClient.name || t("这个客户端");
                return approval.lastClient.elicitation
                  ? tf("{0} 支持审批卡。", [client])
                  : approval.mode === "claude"
                    ? tf("{0} 未声明审批卡能力（elicitation）。在这个客户端里「弹卡确认」会退回为自动放行，不会弹卡。", [client])
                    : tf("{0} 未声明审批卡能力（elicitation）；如果切到「弹卡确认」，在这个客户端里会退回为自动放行。", [client]);
              })()}
            /> : null}
            {approval?.turnHooks ? <CxRow
              badges={<CxBadge tone={approval.turnHooks.installed ? "ok" : "warn"}>{approval.turnHooks.installed ? t("已安装") : t("未安装")}</CxBadge>}
              detail={approval.turnHooks.installed
                ? tf("浏览器回合钩子已装好：{0}，回合结束时自动关闭未标记的标签页。", [(approval.turnHooks.events ?? []).join(" / ")])
                : t("未安装浏览器回合钩子。安装 Claude Code 接入后，回合结束时未标记的标签页不会自动关闭，会越堆越多。")}
              title={t("浏览器回合钩子")}
            /> : null}
            {(approval?.approvals ?? []).map(([key, entry]) => (
              <CxRow
                key={key}
                badges={<CxBadge tone={entry.decision !== "allow" ? "danger" : "ok"}
                  title={entry.source === "computer-use" ? t("Computer Use 自己记的，Codex 也读这一份；撤销后 Codex 里也会重新询问。") : t("旧版 Codex++ 记录，只对 Codex++ 生效。")}>
                  {entry.decision !== "allow" ? t("拒绝") : entry.source === "computer-use" ? t("与 Codex 共用") : t("Codex++ 旧记录")}
                </CxBadge>}
                detail={entry.displayName ? <code className="cx-set-path" title={key}>{key}</code> : null}
                title={entry.displayName || key}
              >
                <button className="cx-secondary" type="button" disabled={locked} onClick={() => void forgetApproval(key)}>
                  {busy === `forget:${key}` ? <Loader2 size={13} className="cu-spin" /> : null}{t("撤销")}
                </button>
              </CxRow>
            ))}
            {(approval?.approvals?.length ?? 0) > 0 ? (
              <div className="cx-set-footer">
                <button className="cx-secondary" type="button" disabled={locked} title={t("Codex 里的「始终允许」也会一起清掉。")} onClick={() => void clearApprovals()}>
                  {busy === "approval:clear" ? <Loader2 size={14} className="cu-spin" /> : null}{t("全部清空")}
                </button>
              </div>
            ) : approval ? <p className="cx-set-note cx-set-note-inset">{t("还没有始终允许的应用。在「弹卡确认」的卡片上勾选「始终允许」，或在 Codex 里选「始终允许」后，应用会出现在这里（两边共用）。")}</p> : null}
          </CxCard> : null}
        </CxSection>
      </>}

      {activeTab === "preview" && <>
        <ComputerUsePreview />
      </>}

      {activeTab === "diagnose" && <>
        <CxSection title={<span title={t("Computer Use 的服务由本机 ChatGPT.app 提供，接入前先确认它能被找到。")}>{t("运行时")}</span>}>
          <CxCard>
            <CxRow
              badges={<CxBadge tone={!status ? "neutral" : found ? "ok" : "warn"}>{!status ? t("未检测") : found ? t("已找到") : t("未找到")}</CxBadge>}
              detail={hasValue(status?.appPath) ? <code className="cx-set-path" title={status?.appPath}>{status?.appPath}</code> : "—"}
              icon={<AppWindow />}
              title={t("应用路径")}
            />
            <CxRow detail={hasValue(status?.packageVersion) ? <code className="cx-set-path">{status?.packageVersion}</code> : "—"} title="@oai/cua-repl" />
            <CxRow detail={<code className="cx-set-path">SkyComputerUseClient</code>} title={t("CU server")} />
            <CxRow detail={hasValue(status?.servicePath) ? <code className="cx-set-path" title={status?.servicePath}>{status?.servicePath}</code> : "—"} title={t("CU 服务路径")} />
          </CxCard>
        </CxSection>

        <CxSection title={t("Chrome 浏览器扩展")}>
          <CxCard>
            <CxRow
              badges={extStatus == null ? null : <CxBadge tone={extStatus.installed ? "ok" : "warn"}>{extStatus.installed ? t("扩展已安装") : t("未安装")}</CxBadge>}
              detail={extStatus == null ? t("检测中…") : extStatus.installed ? t("装好后无需额外配置，刷新页面即生效。") : t("未检测到扩展，浏览器工具不可用。")}
              icon={<Globe />}
              title="ChatGPT for Chrome"
              tooltip={t("浏览器控制依赖 OpenAI 官方的 ChatGPT for Chrome 扩展。装好后无需额外配置，刷新页面即生效。")}
            >
              {extStatus && !extStatus.installed
                ? <button className="cx-primary" type="button" disabled={locked} onClick={() => void invoke("computer_use_open_extension_store")}>{t("安装 Chrome 扩展 →")}</button>
                : null}
            </CxRow>
          </CxCard>
        </CxSection>

        <CxSection title={t("连通性检查")}>
          <CxCard>
            <CxRow
              badges={check ? <CxBadge tone={check.ok ? "ok" : "warn"}>{check.ok ? t("自检通过") : t("自检未通过")}</CxBadge> : null}
              detail={!check ? t("实际跑一次 MCP 握手并调用 sky.list_apps()，确认工具链可用。") : !check.ok ? failureText(check) : check.message || t("自检通过")}
              icon={check ? (check.ok ? <CheckCircle2 /> : <AlertTriangle />) : <Play />}
              title={t("运行自检")}
              tooltip={check?.message || undefined}
            >
              <button className="cx-secondary" type="button" onClick={() => void runCheck()} disabled={locked}>
                {busy === "check" ? <Loader2 size={14} className="cu-spin" /> : <Play size={14} />}{busy === "check" ? t("正在自检…") : check ? t("重新自检") : t("运行自检")}
              </button>
            </CxRow>
            {check?.ok ? <>
              <CxRow title={t("目标平台")}><span className="cx-set-value">{check.target || "—"}</span></CxRow>
              <CxRow detail={apps.length > 0 ? apps.join(" · ") : null} title={t("应用数量")}><span className="cx-set-value">{typeof check.appCount === "number" ? check.appCount : "—"}</span></CxRow>
              <CxRow title={t("浏览器")}><span className="cx-set-value">{typeof check.browserCount === "number" ? check.browserCount : "—"}</span></CxRow>
              <CxRow title={t("耗时")}><span className="cx-set-value">{typeof check.durationMs === "number" ? `${check.durationMs} ms` : "—"}</span></CxRow>
            </> : null}
          </CxCard>
          <p className="cx-set-note">{tfNodes("接入后需要重启对应客户端（Claude Code 会话、DeepSeek Harness、ZCode、WorkBuddy、Devin）才会加载工具，工具名以 {0} 与 {1} 前缀出现在工具列表中。", [<code>mcp__codex-plus-cu__</code>, <code>mcp__codex-plus-browser__</code>])}</p>
        </CxSection>
      </>}
    </div>
  </div>;
}
