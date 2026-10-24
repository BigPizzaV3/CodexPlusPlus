/** Computer Use：复用 Codex app 的 Computer Use 与 Chrome 控制，通过 MCP 提供给 Claude Code、DeepSeek Harness、ZCode 等客户端。 */
import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AlertTriangle, CheckCircle2, Loader2, MonitorSmartphone, Play, Plug, RefreshCw, Unplug } from "lucide-react";
import { PageHeader } from "./ManagementPages";
import { t } from "./i18n";
import "./ComputerUsePage.css";

type CuTargetId = "claudeCode" | "claudeDesktop" | "codex" | "dsh" | "zcode" | "workBuddy";
type CuTargetState = "registered" | "missing" | "needs_update" | "foreign" | "unsupported";
type CuTarget = { id: string; label: string; configPath: string; registered: boolean; state: CuTargetState; message: string };
type CuTargets = Record<CuTargetId, CuTarget>;
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
};
type CuMutation = { status: string; message: string; changed: boolean; backupPath?: string; targets?: Partial<CuTargets> };
type CuFailureKind = "runtime_missing" | "untrusted_parent" | "permission_required" | "client_unsupported" | "service_unavailable" | "error";
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
type CuApprovalMode = "auto" | "claude";
type CuApprovedApp = { decision: string; mode?: string; displayName?: string; decidedAt?: number };
type CuLastClient = { name?: string; version?: string; elicitation?: boolean; at?: number };
type CuApprovalState = {
  status?: string;
  message?: string;
  mode?: CuApprovalMode;
  approvals?: Array<[string, CuApprovedApp]>;
  lastClient?: CuLastClient | null;
  turnHooks?: { installed?: boolean; events?: string[]; settings?: string } | null;
};

// Codex 桌面版自己会把 [mcp_servers.computer-use] 写进 ~/.codex/config.toml，
// 再挂一份只会让同一个 CU 服务出现两条连接，所以不列 Codex。
const targetIds: CuTargetId[] = ["claudeCode", "claudeDesktop", "dsh", "zcode", "workBuddy"];
const targetFallback: Record<CuTargetId, { label: string; configPath: string }> = {
  claudeCode: { label: "Claude Code", configPath: "~/.claude.json" },
  claudeDesktop: { label: "Claude Desktop", configPath: "claude_desktop_config.json" },
  codex: { label: "Codex", configPath: "config.toml" },
  dsh: { label: "DeepSeek Harness", configPath: "~/.dsh/profiles/desktop/cordis.patch.yml" },
  zcode: { label: "ZCode", configPath: "~/.zcode/cli/config.json" },
  workBuddy: { label: "WorkBuddy AI", configPath: "~/.workbuddy/mcp.json" },
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
    case "runtime_missing": return t("没找到 ChatGPT.app，请先安装 Codex 桌面版");
    case "untrusted_parent": return t("Computer Use 拒绝了本次调用：ChatGPT.app 版本不兼容，请更新后再试");
    case "permission_required": return t("请在系统设置里给 ChatGPT 授予辅助功能/屏幕录制权限");
    case "client_unsupported": return t("当前 Claude Code 版本不支持审批表单（elicitation）");
    case "service_unavailable": return t("Computer Use 服务未就绪，先打开一次 Codex 的 Computer Use");
    case "error": return check.failureDetail || check.message || t("自检失败，请查看详情后重试。");
    default: return check.failureDetail || check.message || t("自检失败，请查看详情后重试。");
  }
}

export function ComputerUsePage() {
  const [status, setStatus] = useState<CuStatus | null>(null);
  const [check, setCheck] = useState<CuCheck | null>(null);
  const [backups, setBackups] = useState<Partial<Record<CuTargetId, string>>>({});
  const [busy, setBusy] = useState("");
  const [readError, setReadError] = useState("");
  const [actionError, setActionError] = useState("");
  const [notice, setNotice] = useState("");
  const [approval, setApproval] = useState<CuApprovalState | null>(null);
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

  const mutateTarget = async (target: CuTargetId, action: "install" | "uninstall") => {
    if (lock.current) return;
    lock.current = true;
    const stamp = ++epoch.current;
    const label = status?.targets?.[target]?.label || targetFallback[target].label;
    setBusy(`${action}:${target}`);
    setActionError("");
    setNotice("");
    try {
      const result = await invoke<CuMutation>(action === "install" ? "computer_use_install" : "computer_use_uninstall", { target });
      if (!mounted.current || epoch.current !== stamp) return;
      if (result.targets) setStatus((current) => (current ? { ...current, targets: { ...current.targets, ...result.targets } } : current));
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

  return <div className="mg-page cu-page">
    <PageHeader icon={<MonitorSmartphone size={20} strokeWidth={1.5} />} title="Computer Use" actions={
      <button className="mg-button" type="button" onClick={() => void loadStatus()} disabled={locked} title={t("重新检测本机运行时与接入状态")}>
        <RefreshCw size={15} className={busy === "refresh" ? "cu-spin" : ""} />{busy === "refresh" ? t("检测中…") : t("重新检测")}
      </button>
    } />
    <div className="cu-content" aria-busy={loading}>
      <p className="cu-intro">{t("复用 Codex app 的 Computer Use 与 Chrome 控制，通过 MCP 提供给 Claude Code、Claude Desktop、DeepSeek Harness、ZCode、WorkBuddy 等客户端。")}</p>

      {readError && <p className="cu-error" role="alert">{t("读取状态失败：")}{readError}</p>}
      {actionError && <p className="cu-error" role="alert">{t("操作失败：")}{actionError}</p>}
      {notice && <p className="cu-notice" role="status">{notice}</p>}
      {loading && <p className="cu-loading" role="status"><Loader2 size={15} className="cu-spin" />{t("正在检测本机运行时…")}</p>}

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
      </section>

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
            const state = target?.state ?? "missing";
            const label = target?.label || fallback.label;
            const configPath = target?.configPath || fallback.configPath;
            const backup = backups[id];
            const installing = busy === `install:${id}`;
            const removing = busy === `uninstall:${id}`;
            return <div className="cu-target" key={id}>
              <div className="cu-target-main">
                <div className="cu-target-title">
                  <strong>{label}</strong>
                  <span className="cu-badge" data-state={status ? state : "unknown"}>{status ? stateLabel(state) : t("未检测")}</span>
                </div>
                <code className="cu-target-path" title={configPath}>{elidePath(configPath)}</code>
                {target?.message && <small className="cu-target-message">{target.message}</small>}
                {backup && <small className="cu-target-backup">{t("已备份到")}<code title={backup}>{elidePath(backup)}</code></small>}
              </div>
              <div className="cu-target-actions">
                <button className="mg-button" type="button" disabled={locked || !status || !removable(state)} onClick={() => void mutateTarget(id, "uninstall")}>
                  {removing ? <Loader2 size={14} className="cu-spin" /> : <Unplug size={14} />}{removing ? t("移除中…") : t("移除")}
                </button>
                <button className="mg-button mg-primary" type="button" disabled={locked || !status || !found || !installable(state)} title={status && !found ? t("先安装并打开一次 ChatGPT 后再接入") : state === "unsupported" ? t("该客户端不支持接入") : undefined} onClick={() => void mutateTarget(id, "install")}>
                  {installing ? <Loader2 size={14} className="cu-spin" /> : <Plug size={14} />}{installing ? t("接入中…") : t("接入")}
                </button>
              </div>
            </div>;
          })}
        </div>
      </section>

      <section className="cu-card" aria-labelledby="cu-approval-title">
        <div className="cu-card-head">
          <div>
            <h2 id="cu-approval-title">{t("审批")}</h2>
            <p>{t("Codex 在首次接管某个应用时会请求许可。这里决定这条请求由谁回答。")}</p>
          </div>
        </div>
        <div className="cu-approval-modes" role="radiogroup" aria-label={t("审批策略")}>
          <button
            className="cu-mode"
            type="button"
            role="radio"
            aria-checked={(approval?.mode ?? "auto") === "auto"}
            data-active={(approval?.mode ?? "auto") === "auto"}
            disabled={locked}
            onClick={() => void changeApprovalMode("auto")}
          >
            <strong>{t("完全放开")}</strong>
            <small>{t("自动放行所有应用，只记审计日志。适合完全信任调用方、不想被打断的场景。")}</small>
          </button>
          <button
            className="cu-mode"
            type="button"
            role="radio"
            aria-checked={approval?.mode === "claude"}
            data-active={approval?.mode === "claude"}
            disabled={locked}
            onClick={() => void changeApprovalMode("claude")}
          >
            <strong>{t("交给 Claude 审批")}</strong>
            <small>{t("把 Codex 原生的许可请求转给 MCP 客户端，由 Claude 侧弹卡确认；拒绝即不放行。")}</small>
          </button>
        </div>
        {approval?.lastClient && <p
          className={approval.mode === "claude" && approval.lastClient.elicitation === false ? "cu-note cu-note-warn" : "cu-hint"}
          role={approval.mode === "claude" && approval.lastClient.elicitation === false ? "alert" : undefined}
        >
          {t("最近连接的客户端：")}<code>{[approval.lastClient.name, approval.lastClient.version].filter(Boolean).join(" ") || t("未知")}</code>
          {approval.lastClient.elicitation
            ? t("，支持审批卡。")
            : approval.mode === "claude"
              ? t("，未声明审批卡能力（elicitation）。在这个客户端里「交给 Claude 审批」会退回为自动放行，不会弹卡。")
              : t("，未声明审批卡能力（elicitation）；如果切到「交给 Claude 审批」，在这个客户端里会退回为自动放行。")}
        </p>}
        {approval?.turnHooks && <p className={approval.turnHooks.installed ? "cu-hint" : "cu-note cu-note-warn"}>
          {approval.turnHooks.installed
            ? t("浏览器回合钩子已装好：") + (approval.turnHooks.events ?? []).join(" / ") + t("，回合结束时自动关闭未标记的标签页。")
            : t("未安装浏览器回合钩子。安装 Claude Code 接入后，回合结束时未标记的标签页不会自动关闭，会越堆越多。")}
        </p>}
        {(approval?.approvals?.length ?? 0) > 0 && <div className="cu-approvals">
          <div className="cu-approvals-head">
            <strong>{t("已记忆的应用")}</strong>
            <button className="mg-button" type="button" disabled={locked} onClick={() => void clearApprovals()}>
              {busy === "approval:clear" ? <Loader2 size={14} className="cu-spin" /> : null}{t("全部清空")}
            </button>
          </div>
          <ul className="cu-approval-list">
            {approval!.approvals!.map(([key, entry]) => (
              <li key={key}>
                <span className="cu-approval-name">{entry.displayName || key}</span>
                <code className="cu-approval-key" title={key}>{key}</code>
                <span className="cu-approval-decision" data-decision={entry.decision}>{entry.decision === "allow" ? t("允许") : t("拒绝")}</span>
                <button className="mg-button" type="button" disabled={locked} onClick={() => void forgetApproval(key)}>
                  {busy === `forget:${key}` ? <Loader2 size={13} className="cu-spin" /> : null}{t("撤销")}
                </button>
              </li>
            ))}
          </ul>
        </div>}
        {approval && (approval.approvals?.length ?? 0) === 0 && <p className="cu-hint">{t("还没有记住任何应用。选择「交给 Claude 审批」后，Claude 批准过的应用会出现在这里。")}</p>}
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

      {status?.claudeDesktopRunning ? (
        <p className="cu-note cu-note-warn">{t("Claude Desktop 正在运行：它退出时会重写自己的配置，接入前建议先完全退出 Claude Desktop，改完再打开。")}</p>
      ) : null}
      <p className="cu-note">{t("接入后需要重启对应客户端（Claude Code 会话、DeepSeek Harness、ZCode、WorkBuddy）才会加载工具，工具名以")}<code>mcp__codex-plus-cu__</code>{t("与")}<code>mcp__codex-plus-browser__</code>{t("前缀出现在工具列表中。")}</p>
      <p className="cu-note">{t("Codex 桌面版与终端 CLI 共用")}<code>~/.codex/config.toml</code>{t("，其中已由 Codex 自己写入")}<code>[mcp_servers.computer-use]</code>{t("，无需重复接入；仅当你要在没有该条目的环境里单独启用时，才使用")}<code>--codex-plus-cu-cli install codex</code>{t("。")}</p>
    </div>
  </div>;
}
