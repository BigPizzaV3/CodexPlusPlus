import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowRightLeft, Check, Copy, Loader2, Play, RefreshCw, Save, Square } from "lucide-react";
import { PageHeader } from "./ManagementPages";
import { AppSelect } from "./AppSelect";
import { agentQueueState, DEFAULT_MAX_CONCURRENT_TASKS_LIMIT, normalizeConcurrency } from "./agent-collaboration-queue";
import { t, tf } from "./i18n";
import "./AgentCollaborationPage.css";

// maxConcurrentTasks：同一 Agent 最多同时运行的任务数（1–4，默认 1），超出时排队（#2800）。
type Agent = { id: string; name: string; executable: string; enabled: boolean; model: string; reuseSavedAuth: boolean; maxConcurrentTasks: number };
type CollaborationConfig = { enabled: boolean; defaultAgentId: string; agents: Agent[]; maxRuntimeMinutes: number };
const defaultMaxRuntimeMinutes = 10;
const maxRuntimeMinutesLimit = 240;
type Integration = { status: "connected" | "not_connected" | "conflict"; message: string; home?: string; configPath?: string };
type CodexRuntime = {
  status: "ready" | "partial" | "unavailable" | "error";
  message: string;
  reloadSucceeded: boolean;
  toolsReady: boolean;
  toolCount: number;
  threadVerified: boolean;
  cacheRefreshed: boolean;
};
type TaskStatus = "queued" | "running" | "completed" | "failed" | "cancelled" | "interrupted";
type TaskRecord = {
  id: string;
  agentId: string;
  prompt: string;
  cwd: string;
  status: TaskStatus;
  answer: string;
  error: string;
  sessionId: string;
  parentTaskId?: string | null;
  createdAtMs: number;
  updatedAtMs: number;
  cancelRequested: boolean;
  progress: string;
};
type Snapshot = {
  config: CollaborationConfig;
  tasks: TaskRecord[];
  /** configuredMissing：已保存的路径不存在；recovered：已改用自动找到的 CLI（桌面版更新后哈希目录变化，#2751）。 */
  availability?: { id: string; available: boolean; path?: string | null; configuredMissing?: boolean; recovered?: boolean }[];
  integration?: Integration;
  runtime?: CodexRuntime;
  /** maxQueuedPerAgent：每个 Agent 最多排队的任务数（#2772）；maxConcurrentTasksLimit：并发上限的可选上界（#2800）。 */
  capabilities?: { maxQueuedPerAgent?: number; maxConcurrentTasksLimit?: number };
};
type McpConfig = { codexToml: string; claudeJson: string; command: string; args: string[] };

const defaultConfig = (): CollaborationConfig => ({
  enabled: false,
  defaultAgentId: "deepseek",
  agents: [
    { id: "codex", name: "Codex", executable: "", enabled: false, model: "", reuseSavedAuth: false, maxConcurrentTasks: 1 },
    { id: "deepseek", name: "DeepSeek Harness", executable: "", enabled: false, model: "", reuseSavedAuth: false, maxConcurrentTasks: 1 },
    { id: "claude", name: "Claude Code", executable: "", enabled: false, model: "", reuseSavedAuth: false, maxConcurrentTasks: 1 },
  ],
  maxRuntimeMinutes: defaultMaxRuntimeMinutes,
});
const statusLabels: Record<TaskStatus, string> = {
  queued: "排队中", running: "执行中", completed: "已完成", failed: "失败", cancelled: "已取消", interrupted: "已中断",
};
const unfinished = (task: TaskRecord) => task.status === "queued" || task.status === "running";
const absolutePath = (value: string) => /^(?:\/|[a-zA-Z]:[\\/]|\\\\)/.test(value.trim());
const errorText = (error: unknown) => error instanceof Error ? error.message : String(error);
const runtimeReady = (runtime?: CodexRuntime | null) => runtime?.status === "ready"
  && runtime.reloadSucceeded === true && runtime.toolsReady === true && runtime.cacheRefreshed === true;
// 旧版本快照和默认关闭的认证字段可能省略，读取后补齐，保存时发送完整配置。
const copyConfig = (config: CollaborationConfig): CollaborationConfig => ({
  ...config,
  defaultAgentId: config.defaultAgentId || "deepseek",
  maxRuntimeMinutes: Number.isInteger(config.maxRuntimeMinutes) && config.maxRuntimeMinutes > 0
    ? config.maxRuntimeMinutes : defaultMaxRuntimeMinutes,
  agents: config.agents.map((agent) => ({ ...agent, model: agent.model || "", reuseSavedAuth: agent.reuseSavedAuth === true, maxConcurrentTasks: normalizeConcurrency(agent.maxConcurrentTasks) })),
});
const formatTime = (value: number) => Number.isFinite(value) && value > 0
  ? new Date(value).toLocaleString("zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", hour12: false })
  : "—";

export function AgentCollaborationPage({ defaultCwd = "" }: { defaultCwd?: string } = {}) {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [runtime, setRuntime] = useState<CodexRuntime | null>(null);
  const [draft, setDraft] = useState<CollaborationConfig>(defaultConfig);
  // 运行上限输入框保留原始文本：输入过程中的空值或「1」→「15」这类中间态不能立刻钳制，
  // 否则删掉一位就跳回 10 或 1，无法正常改数。合法时才写进草稿，失焦时还原成草稿值。
  const [runtimeText, setRuntimeText] = useState(String(defaultMaxRuntimeMinutes));
  const [dirty, setDirty] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [busy, setBusy] = useState("");
  const [readError, setReadError] = useState("");
  const [actionError, setActionError] = useState("");
  const [notice, setNotice] = useState("");
  const [agentId, setAgentId] = useState("");
  const [cwd, setCwd] = useState(defaultCwd);
  const [prompt, setPrompt] = useState("");
  const [selectedId, setSelectedId] = useState("");
  const [continueTaskId, setContinueTaskId] = useState("");
  const [mcpConfig, setMcpConfig] = useState<McpConfig | null>(null);
  const [mcpLoading, setMcpLoading] = useState(false);
  const [mcpError, setMcpError] = useState("");
  const [copied, setCopied] = useState("");
  const promptRef = useRef<HTMLTextAreaElement>(null);
  const mounted = useRef(false);
  const lifecycle = useRef(0);
  const requestSequence = useRef(0);
  const readInFlight = useRef(false);
  const busyRef = useRef(false);
  const dirtyRef = useRef(false);
  const mcpInFlight = useRef(false);

  const applySnapshot = useCallback((next: Snapshot, saved = false) => {
    const normalized = { ...next, config: copyConfig(next.config) };
    setSnapshot(normalized);
    // 普通快照不触发运行时检查，也不能抹掉用户最近一次显式刷新的结果。
    if (saved) setRuntime(null);
    else if (next.runtime) setRuntime(next.runtime);
    // 后台轮询只更新执行状态，保留用户尚未保存的配置。
    if (saved || !dirtyRef.current) {
      setDraft(copyConfig(normalized.config));
      dirtyRef.current = false;
      setDirty(false);
    }
    setSelectedId((current) => next.tasks.some((task) => task.id === current)
      ? current : [...next.tasks].sort((a, b) => b.createdAtMs - a.createdAtMs)[0]?.id ?? "");
  }, []);

  const refresh = useCallback(async () => {
    if (!mounted.current || readInFlight.current || busyRef.current) return;
    const epoch = lifecycle.current;
    const sequence = ++requestSequence.current;
    readInFlight.current = true;
    setRefreshing(true);
    try {
      const next = await invoke<Snapshot>("agent_collaboration_request", { action: "snapshot", payload: {} });
      if (!mounted.current || lifecycle.current !== epoch || sequence !== requestSequence.current) return;
      applySnapshot(next);
      setReadError("");
    } catch (error) {
      if (mounted.current && lifecycle.current === epoch && sequence === requestSequence.current) setReadError(errorText(error));
    } finally {
      if (mounted.current && lifecycle.current === epoch) {
        readInFlight.current = false;
        setRefreshing(false);
      }
    }
  }, [applySnapshot]);

  useEffect(() => {
    mounted.current = true;
    lifecycle.current += 1;
    readInFlight.current = false;
    busyRef.current = false;
    mcpInFlight.current = false;
    void refresh();
    return () => {
      mounted.current = false;
      lifecycle.current += 1;
      requestSequence.current += 1;
    };
  }, [refresh]);

  const hasUnfinishedTasks = snapshot?.tasks.some(unfinished) ?? false;
  useEffect(() => {
    // 空闲时也接收其他 MCP 客户端提交的任务，发现活跃任务后缩短刷新间隔。
    const timer = window.setInterval(() => { void refresh(); }, hasUnfinishedTasks ? 2000 : 10000);
    const refreshOnFocus = () => { void refresh(); };
    window.addEventListener("focus", refreshOnFocus);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener("focus", refreshOnFocus);
    };
  }, [hasUnfinishedTasks, refresh]);

  const mutate = async (action: string, payload: unknown, onSuccess?: (next: Snapshot) => void) => {
    if (!mounted.current || busyRef.current) return;
    busyRef.current = true;
    const epoch = lifecycle.current;
    // 写操作开始后，之前的只读响应即使较晚到达也不能覆盖新状态。
    const sequence = ++requestSequence.current;
    setBusy(action);
    setActionError("");
    setNotice("");
    const refreshesCodex = action === "connect_codex" || action === "refresh_codex";
    if (refreshesCodex) setRuntime(null);
    try {
      const next = await invoke<Snapshot>("agent_collaboration_request", { action, payload });
      if (!mounted.current || lifecycle.current !== epoch || sequence !== requestSequence.current) return;
      applySnapshot(next, action === "save_config");
      setReadError("");
      onSuccess?.(next);
    } catch (error) {
      if (mounted.current && lifecycle.current === epoch) {
        setActionError(errorText(error));
        if (refreshesCodex) setRuntime({ status: "error", message: errorText(error), reloadSucceeded: false, toolsReady: false, toolCount: 0, threadVerified: false, cacheRefreshed: false });
      }
    } finally {
      if (mounted.current && lifecycle.current === epoch) {
        busyRef.current = false;
        setBusy("");
      }
    }
  };

  const editConfig = (next: CollaborationConfig) => {
    dirtyRef.current = true;
    setDirty(true);
    setDraft(next);
    setNotice("");
  };
  useEffect(() => { setRuntimeText(String(draft.maxRuntimeMinutes)); }, [draft.maxRuntimeMinutes]);
  const parsedRuntimeMinutes = (text: string) => {
    const trimmed = text.trim();
    if (!/^\d+$/.test(trimmed)) return null;
    const value = Number(trimmed);
    return value >= 1 && value <= maxRuntimeMinutesLimit ? value : null;
  };
  const editRuntimeText = (text: string) => {
    setRuntimeText(text);
    const value = parsedRuntimeMinutes(text);
    if (value === null) {
      dirtyRef.current = true;
      setDirty(true);
      setNotice("");
      return;
    }
    if (value !== draft.maxRuntimeMinutes) editConfig({ ...draft, maxRuntimeMinutes: value });
  };
  const editAgent = (id: string, patch: Partial<Agent>) => editConfig({ ...draft, agents: draft.agents.map((agent) => agent.id === id ? { ...agent, ...patch } : agent) });
  const saveConfig = () => {
    const invalid = draft.agents.find((agent) => agent.executable.trim() && !absolutePath(agent.executable));
    if (invalid) { setActionError(`请填写 ${invalid.name} 可执行文件的绝对路径。`); return; }
    if (parsedRuntimeMinutes(runtimeText) === null) {
      setActionError(`单个任务运行上限需填写 1 到 ${maxRuntimeMinutesLimit} 之间的整数分钟。`);
      return;
    }
    void mutate("save_config", { ...draft, agents: draft.agents.map((agent) => ({ ...agent, executable: agent.executable.trim(), model: agent.model.trim() })) }, () => {
      setNotice("子 Agent 设置已保存。");
      setMcpConfig(null);
      setCopied("");
    });
  };

  const tasks = [...(snapshot?.tasks ?? [])].sort((a, b) => b.createdAtMs - a.createdAtMs);
  const selected = tasks.find((task) => task.id === selectedId);
  const continuation = tasks.find((task) => task.id === continueTaskId);
  const enabledAgents = snapshot?.config.agents.filter((agent) => agent.enabled) ?? [];
  const targetId = continuation?.agentId ?? (enabledAgents.some((agent) => agent.id === agentId) ? agentId
    : enabledAgents.find((agent) => agent.id === snapshot?.config.defaultAgentId)?.id ?? enabledAgents[0]?.id ?? "");
  const targetAgent = enabledAgents.find((agent) => agent.id === targetId);
  const targetUnavailable = snapshot?.availability?.find((item) => item.id === targetId)?.available === false;
  // 同一 Agent 运行中的任务未达并发上限时直接启动，达到上限时排队、任一结束后按提交顺序启动；
  // 只有排队已满才禁止提交（#2772、#2800）。
  const reportedConcurrencyMax = snapshot?.capabilities?.maxConcurrentTasksLimit;
  const concurrencyMax = Number.isInteger(reportedConcurrencyMax) && (reportedConcurrencyMax as number) > 0
    ? reportedConcurrencyMax as number : DEFAULT_MAX_CONCURRENT_TASKS_LIMIT;
  const concurrencyLimit = (id: string) => normalizeConcurrency(snapshot?.config.agents.find((agent) => agent.id === id)?.maxConcurrentTasks);
  const targetQueue = agentQueueState(tasks, targetId, snapshot?.capabilities?.maxQueuedPerAgent, concurrencyLimit(targetId));
  const targetCwd = continuation?.cwd ?? cwd;
  // 并发任务共用工作目录时只提示、不阻止（#2800）。
  const trimmedCwd = (value: string) => value.trim().replace(/[\\/]+$/, "");
  const sameCwdRunning = targetCwd.trim() ? tasks.filter((task) => task.status === "running" && trimmedCwd(task.cwd) === trimmedCwd(targetCwd)).length : 0;
  const formDisabledReason = !snapshot ? "正在读取协作配置。"
    : !snapshot.config.enabled ? "请先启用外部子 Agent 并保存设置。"
    : !targetAgent ? "请先启用至少一个 CLI 并保存配置。"
    : targetUnavailable ? "目标 CLI 的可执行文件不可用，请检查路径并保存。"
    : targetQueue.full ? tf("此 Agent 的排队任务已达上限（{0} 个），请等待前面的任务结束或取消部分排队任务。", [targetQueue.limit])
    : continuation && (continuation.status !== "completed" || !continuation.sessionId) ? "该任务目前无法续聊，请创建新任务。"
    : "";
  const agentName = (id: string) => snapshot?.config.agents.find((agent) => agent.id === id)?.name ?? id;
  // 任务列表顶部按 Agent 汇总并发与排队情况（#2800）：只列出有未结束任务的 Agent。
  const agentLoads = (snapshot?.config.agents ?? [])
    .map((agent) => ({ agent, state: agentQueueState(tasks, agent.id, snapshot?.capabilities?.maxQueuedPerAgent, concurrencyLimit(agent.id)) }))
    .filter(({ state }) => state.active > 0);
  const runtimeRefreshing = busy === "connect_codex" || busy === "refresh_codex";
  const runtimeSucceeded = runtimeReady(runtime);
  const connectCodex = () => {
    if (dirtyRef.current) { setActionError("请先保存子 Agent 设置，再接入 Codex。"); return; }
    void mutate("connect_codex", {}, (next) => {
      if (next.integration?.status === "connected") {
        setNotice(runtimeReady(next.runtime)
          ? "已接入配置，Codex 工具已刷新。刷新后的工具从下一轮消息起可用。"
          : "已接入配置；运行中的 Codex 工具是否已刷新，请查看下方运行状态。");
      } else {
        setActionError(next.integration?.message || "尚未完成 Codex 接入，请检查配置后重试。");
      }
    });
  };
  const refreshCodex = () => {
    if (dirtyRef.current) { setActionError("请先保存子 Agent 设置，再刷新 Codex 工具。"); return; }
    void mutate("refresh_codex", {}, (next) => {
      if (runtimeReady(next.runtime)) setNotice("Codex 工具已刷新。刷新后的工具从下一轮消息起可用。");
      else if (!next.runtime) setActionError("未取得 Codex 工具刷新结果，请重试或手动重新加载 MCP。");
    });
  };

  const submitTask = (event: FormEvent) => {
    event.preventDefault();
    if (busyRef.current || formDisabledReason) return;
    if (!absolutePath(targetCwd)) { setActionError("请填写工作目录的绝对路径。"); return; }
    if (!prompt.trim()) { setActionError("请填写要交给 Agent 的任务。"); return; }
    const previousIds = new Set(tasks.map((task) => task.id));
    void mutate("submit", {
      agentId: targetId,
      prompt: prompt.trim(),
      cwd: targetCwd.trim(),
      ...(continuation ? { continueTaskId: continuation.id } : {}),
    }, (next) => {
      const created = [...next.tasks].sort((a, b) => b.createdAtMs - a.createdAtMs).find((task) => !previousIds.has(task.id));
      if (created) setSelectedId(created.id);
      setPrompt("");
      setContinueTaskId("");
      setCwd(targetCwd);
      setNotice(targetQueue.willQueue ? t("任务已提交并进入排队，前面的任务结束后自动启动。") : "任务已提交，可在下方查看执行状态。");
    });
  };
  const continueTask = (task: TaskRecord) => {
    setContinueTaskId(task.id);
    setAgentId(task.agentId);
    setCwd(task.cwd);
    setPrompt("");
    setActionError("");
    setNotice("");
    promptRef.current?.focus();
  };
  const loadMcpConfig = async () => {
    if (mcpInFlight.current) return;
    mcpInFlight.current = true;
    const epoch = lifecycle.current;
    setMcpLoading(true);
    setMcpError("");
    setCopied("");
    try {
      const result = await invoke<McpConfig>("agent_collaboration_request", { action: "mcp_config", payload: {} });
      if (mounted.current && lifecycle.current === epoch) setMcpConfig(result);
    } catch (error) {
      if (mounted.current && lifecycle.current === epoch) setMcpError(errorText(error));
    } finally {
      if (mounted.current && lifecycle.current === epoch) {
        mcpInFlight.current = false;
        setMcpLoading(false);
      }
    }
  };
  const copyText = async (value: string, label: string) => {
    const epoch = lifecycle.current;
    try {
      await navigator.clipboard.writeText(value);
      if (mounted.current && lifecycle.current === epoch) { setCopied(label); setMcpError(""); }
    } catch (error) {
      if (mounted.current && lifecycle.current === epoch) setMcpError(`复制失败：${errorText(error)}`);
    }
  };

  return <div className="mg-page agent-collaboration-page">
    <PageHeader icon={<ArrowRightLeft size={20} strokeWidth={1.5} />} title="子 Agent 设置" actions={
      <button className="mg-button" type="button" onClick={() => void refresh()} disabled={refreshing || Boolean(busy)}>
        <RefreshCw size={15} className={refreshing ? "agent-collaboration-spin" : ""} />{refreshing ? "刷新中…" : "刷新"}
      </button>
    } />
    <div className="agent-collaboration-content" aria-busy={!snapshot && refreshing}>
      <div className="agent-collaboration-intro">
        <p>在 Codex 对话里调用外部子 Agent，结果由 Codex 在原对话中汇总。</p>
        <div className="agent-collaboration-examples" aria-label="在 Codex 对话中的用法"><span>“让 DeepSeek 实现这个功能”</span><span>“让 Claude Code 审核这次改动”</span><span>“交给默认子 Agent 调查这个报错”</span></div>
        <p className="agent-collaboration-hint">在这里选好默认子 Agent，保存设置并接入 Codex，然后回到对话中分派任务。</p>
      </div>

      {readError && <p className="agent-collaboration-error" role="alert">读取协作状态失败：{readError}</p>}
      {actionError && <p className="agent-collaboration-error" role="alert">操作失败：{actionError}</p>}
      {notice && <p className="agent-collaboration-notice" role="status">{notice}</p>}
      {!snapshot && refreshing && <p className="agent-collaboration-loading" role="status"><Loader2 size={15} className="agent-collaboration-spin" />正在读取协作配置…</p>}

      <section className="agent-collaboration-section" aria-labelledby="agent-collaboration-config-title">
        <div className="agent-collaboration-section-head">
          <div><h2 id="agent-collaboration-config-title">子 Agent 配置</h2><p>选择可调用的 CLI 与默认模型。CLI 路径留空时自动查找已安装的程序。</p></div>
          <button type="button" className="mg-button mg-primary" disabled={!snapshot || !dirty || Boolean(busy)} onClick={saveConfig}>
            {busy === "save_config" ? <Loader2 size={15} className="agent-collaboration-spin" /> : <Save size={15} />}{busy === "save_config" ? "保存中…" : "保存配置"}
          </button>
        </div>
        <label className="agent-collaboration-enable">
          <input type="checkbox" checked={draft.enabled} disabled={!snapshot || busy === "save_config"} onChange={(event) => editConfig({ ...draft, enabled: event.target.checked })} />
          <span><strong>启用外部子 Agent</strong><small>默认关闭，保存后生效。</small></span>
          <span className="mg-badge">{snapshot?.config.enabled ? "已启用" : "未启用"}</span>
        </label>
        <div className="agent-collaboration-default">
          <div className="mg-field"><span>默认子 Agent</span><AppSelect ariaLabel="默认子 Agent" disabled={!snapshot || busy === "save_config"} id="agent-collaboration-default" onChange={(next) => editConfig({ ...draft, defaultAgentId: next })} options={draft.agents.map((agent) => ({ value: agent.id, label: `${agent.name}${agent.enabled ? "" : "（未启用）"}` }))} value={draft.defaultAgentId} /></div>
          <label className="mg-field" htmlFor="agent-collaboration-runtime"><span>单个任务运行上限（分钟）</span><input id="agent-collaboration-runtime" type="number" min={1} max={maxRuntimeMinutesLimit} step={1} value={runtimeText} disabled={!snapshot || busy === "save_config"} aria-invalid={parsedRuntimeMinutes(runtimeText) === null} onChange={(event) => editRuntimeText(event.target.value)} onBlur={() => { if (parsedRuntimeMinutes(runtimeText) === null) setRuntimeText(String(draft.maxRuntimeMinutes)); }} /></label>
          <p className="agent-collaboration-hint">说“交给默认子 Agent”时使用此选择；直接说“让 DeepSeek”或“让 Claude Code”时使用指定的子 Agent。子 Agent 任务超过运行上限（1–240 分钟，默认 10）会被终止。{draft.enabled && !draft.agents.some((agent) => agent.id === draft.defaultAgentId && agent.enabled) && " 所选默认子 Agent 尚未启用，默认分派不可用；仍可点名调用其他已启用的子 Agent。"}</p>
        </div>
        <div className="agent-collaboration-agents">
          {draft.agents.map((agent) => {
            const savedAgent = snapshot?.config.agents.find((item) => item.id === agent.id);
            const savedPath = savedAgent?.executable === agent.executable.trim();
            const availability = savedPath ? snapshot?.availability?.find((item) => item.id === agent.id) : undefined;
            const available = availability?.available;
            return <div className="agent-collaboration-agent" key={agent.id}>
              <label className="agent-collaboration-agent-title"><input type="checkbox" checked={agent.enabled} disabled={!snapshot || busy === "save_config"} onChange={(event) => editAgent(agent.id, { enabled: event.target.checked })} /><strong>{agent.name}</strong></label>
              <label className="mg-field" htmlFor={`agent-collaboration-path-${agent.id}`}><span>可执行文件路径（可选）</span><input id={`agent-collaboration-path-${agent.id}`} value={agent.executable} disabled={!snapshot || busy === "save_config"} placeholder={availability?.path || "留空自动查找已安装的 CLI"} spellCheck={false} autoCapitalize="off" autoCorrect="off" onChange={(event) => editAgent(agent.id, { executable: event.target.value })} /></label>
              <label className="mg-field" htmlFor={`agent-collaboration-model-${agent.id}`}><span>{agent.id === "deepseek" ? "模型路由（可选）" : "模型（可选）"}</span><input id={`agent-collaboration-model-${agent.id}`} value={agent.model} disabled={!snapshot || busy === "save_config"} placeholder={agent.id === "deepseek" ? "deepseek-account/deepseek-flash" : "留空沿用该 CLI 默认模型"} spellCheck={false} autoCapitalize="off" autoCorrect="off" aria-describedby={agent.id === "deepseek" ? "agent-collaboration-deepseek-model-hint" : undefined} onChange={(event) => editAgent(agent.id, { model: event.target.value })} />{agent.id === "deepseek" && <small id="agent-collaboration-deepseek-model-hint">使用 provider/model 格式。账户登录用 deepseek-account，API 密钥用 deepseek-official；留空沿用 headless 默认模型。</small>}</label>
              <div className="mg-field"><span>{t("同时执行任务数")}</span><AppSelect ariaLabel={tf("{0} 同时执行任务数", [agent.name])} disabled={!snapshot || busy === "save_config"} id={`agent-collaboration-concurrency-${agent.id}`} onChange={(next) => editAgent(agent.id, { maxConcurrentTasks: normalizeConcurrency(Number(next), concurrencyMax) })} options={Array.from({ length: concurrencyMax }, (_, index) => ({ value: String(index + 1), label: index === 0 ? t("1（逐个执行，其余排队）") : String(index + 1) }))} value={String(agent.maxConcurrentTasks)} /><small className="agent-collaboration-agent-note">{t("同一 Agent 最多同时运行的任务数。超出时排队，任一任务结束后按提交顺序启动下一个；并发任务请使用不同的工作目录，避免互相修改相同文件。")}</small></div>
              <small className="agent-collaboration-agent-state" data-unavailable={available === false}>{!savedPath ? "路径尚未保存" : availability?.recovered ? "已保存的路径已失效（桌面版更新后版本目录会变化），本次自动使用当前 CLI；清空路径保存可一直自动查找" : available === true ? "已找到 CLI 文件" : availability?.configuredMissing ? "已保存的路径不存在；清空路径保存可自动查找，或重新填写" : available === false ? "未找到 CLI 文件" : agent.executable.trim() ? "已配置，执行任务时验证" : "留空自动查找已安装的 CLI"}{availability?.path && <code>{availability.path}</code>}</small>
              {agent.id === "deepseek" && <>
                <label className="agent-collaboration-auth"><input type="checkbox" checked={agent.reuseSavedAuth} disabled={!snapshot || busy === "save_config"} aria-describedby="agent-collaboration-deepseek-auth-hint" onChange={(event) => editAgent(agent.id, { reuseSavedAuth: event.target.checked })} /><span>复用 DSH 已保存的登录与密钥</span></label>
                <small id="agent-collaboration-deepseek-auth-hint" className="agent-collaboration-agent-note">默认关闭。开启后由 dsh 自行读取已保存的认证，Codex++ 不读取或保存登录与密钥；不会自动选择桌面当前使用的模型。</small>
              </>}
            </div>;
          })}
        </div>
        <p className="agent-collaboration-hint">路径检测只确认 CLI 文件存在，登录状态在执行任务时验证。{dirty && " 有未保存的修改；子 Agent 使用已保存的配置。"}</p>
      </section>

      <section className="agent-collaboration-section agent-collaboration-integration" aria-labelledby="agent-collaboration-mcp-title">
        <div className="agent-collaboration-section-head"><div><h2 id="agent-collaboration-mcp-title">在 Codex 对话中使用</h2><p>接入会登记子 Agent 工具，并尝试在本机 Codex 中刷新。已登记配置可单独刷新工具。</p></div><div className="agent-collaboration-integration-actions">
          <button type="button" className="mg-button" disabled={!snapshot || Boolean(busy) || dirty} onClick={refreshCodex}>{busy === "refresh_codex" ? <Loader2 size={15} className="agent-collaboration-spin" /> : <RefreshCw size={15} />}{busy === "refresh_codex" ? "刷新中…" : "刷新 Codex 工具"}</button>
          <button type="button" className="mg-button mg-primary" disabled={!snapshot || Boolean(busy) || dirty} onClick={connectCodex}>{busy === "connect_codex" ? <Loader2 size={15} className="agent-collaboration-spin" /> : <ArrowRightLeft size={15} />}{busy === "connect_codex" ? "接入并刷新中…" : "一键接入并刷新"}</button>
        </div></div>
        <div className="agent-collaboration-integration-state" data-status={snapshot?.integration?.status ?? "not_connected"} role="status"><strong>{!snapshot ? "正在读取接入状态…" : snapshot.integration?.status === "connected" ? "已接入配置" : snapshot.integration?.status === "conflict" ? "接入配置有冲突" : snapshot.integration?.status === "not_connected" ? "尚未接入 Codex" : "尚未检测到接入配置"}</strong><span>{snapshot?.integration?.message || "保存子 Agent 设置后，点击“一键接入并刷新”。"}</span></div>
        {(snapshot?.integration?.home || snapshot?.integration?.configPath) && <p className="agent-collaboration-hint">{snapshot?.integration?.home ? "Codex 配置目录：" : "Codex 配置文件："}<code>{snapshot?.integration?.home || snapshot?.integration?.configPath}</code></p>}
        <div className="agent-collaboration-runtime-state" data-status={runtimeRefreshing ? "refreshing" : runtimeSucceeded ? "ready" : runtime?.status === "ready" ? "partial" : runtime?.status ?? "unchecked"} role="status" aria-busy={runtimeRefreshing}>
          <small>运行中的 Codex · 最近一次工具刷新</small>
          <strong>{runtimeRefreshing ? "正在刷新 Codex 工具…" : runtimeSucceeded ? "Codex 工具已刷新" : runtime?.status === "unavailable" ? "暂未连接到 Codex 运行时" : runtime?.status === "error" ? "工具刷新失败" : runtime?.toolsReady ? "子 Agent 工具已就绪，列表待确认" : runtime ? "工具刷新尚未完成" : "尚未检查运行中的 Codex 工具"}</strong>
          <span>{runtimeRefreshing ? "正在重新加载本机 MCP 配置、检查子 Agent 工具并刷新列表缓存。" : runtime?.message || "点击“刷新 Codex 工具”，检查子 Agent 工具是否已在本机 Codex 中加载。"}</span>
          {runtime && !runtimeRefreshing && <div className="agent-collaboration-runtime-steps"><span data-complete={runtime.reloadSucceeded === true}>{runtime.reloadSucceeded ? "MCP 已重载" : "MCP 重载未确认"}</span><span data-complete={runtime.toolsReady === true}>{runtime.toolsReady ? `已找到子 Agent 工具${runtime.toolCount > 0 ? `（${runtime.toolCount} 个）` : ""}` : "子 Agent 工具未确认"}</span><span data-complete={runtime.cacheRefreshed === true}>{runtime.cacheRefreshed ? "工具列表缓存已刷新" : "工具列表刷新待确认"}</span></div>}
          {runtime?.status === "unavailable" && !runtimeRefreshing && <span>请用 Codex++ 启动或接管 Codex 后重试，也可在 Codex 中手动重新加载 MCP。</span>}
        </div>
        <p className="agent-collaboration-hint">Codex 的 MCP 工具列表可能缓存约 5 分钟，刷新会重新加载配置并更新列表缓存。</p>
        <p className="agent-collaboration-hint">刷新后的工具从下一轮消息起可用，当前正在进行的回合不会中途增加工具。也可手动重新加载 MCP 或开启新对话。{dirty && " 请先保存设置，再接入或刷新。"}</p>
        <details className="agent-collaboration-manual"><summary>手动接入与配置导出</summary><div className="agent-collaboration-manual-body">
          <p className="agent-collaboration-hint">也可以复制配置，合并到对应客户端的 MCP 设置后重新连接。</p>
          <button type="button" className="mg-button" disabled={mcpLoading || !snapshot || Boolean(busy)} onClick={() => void loadMcpConfig()}>{mcpLoading ? <Loader2 size={15} className="agent-collaboration-spin" /> : <Copy size={15} />}{mcpLoading ? "读取中…" : mcpConfig ? "重新生成配置" : "生成手动配置"}</button>
          {mcpError && <p className="agent-collaboration-error" role="alert">{mcpError}</p>}
          {copied && <p className="agent-collaboration-notice" role="status">{copied}已复制。</p>}
          {mcpConfig && <div className="agent-collaboration-mcp-configs">
            {[{ label: "Codex 配置", value: mcpConfig.codexToml }, { label: "Claude 配置", value: mcpConfig.claudeJson }].map((item) => <div className="agent-collaboration-mcp-code" key={item.label}><div><h3>{item.label}</h3><button type="button" className="mg-button" onClick={() => void copyText(item.value, item.label)}>{copied === item.label ? <Check size={14} /> : <Copy size={14} />}复制</button></div><pre tabIndex={0} aria-label={item.label}>{item.value}</pre></div>)}
            <details className="agent-collaboration-mcp-command"><summary>其他 MCP 客户端的启动参数</summary><pre tabIndex={0}>{JSON.stringify({ command: mcpConfig.command, args: mcpConfig.args }, null, 2)}</pre></details>
          </div>}
        </div></details>
      </section>

      <details className="agent-collaboration-debug"><summary>调试与任务记录 <span className="agent-collaboration-count">{tasks.length}</span>{hasUnfinishedTasks && <span className="agent-collaboration-status" data-status="running">有任务执行中</span>}</summary><div className="agent-collaboration-debug-body">
      <p className="agent-collaboration-hint">日常任务直接在 Codex 对话中交给子 Agent。这里用于检查执行情况，或单独发送调试任务。</p>
      <section className="agent-collaboration-section" aria-labelledby="agent-collaboration-create-title">
        <div className="agent-collaboration-section-head"><div><h2 id="agent-collaboration-create-title">{continuation ? "继续调试对话" : "发送调试任务"}</h2><p>任务可以访问所选工作目录，请先完成目标 CLI 的登录。</p></div></div>
        {continuation && <div className="agent-collaboration-continuation"><div><strong>续聊 {agentName(continuation.agentId)} 的任务</strong><span>{continuation.prompt}</span><small>任务 {continuation.id} · 沿用原会话与工作目录</small></div><button type="button" className="mg-button" disabled={busy === "submit"} onClick={() => { setContinueTaskId(""); setPrompt(""); }}>返回新任务</button></div>}
        <form className="agent-collaboration-form" onSubmit={submitTask}>
          <div className="agent-collaboration-form-row">
            <div className="mg-field"><span>执行 Agent</span><AppSelect
              ariaLabel="执行 Agent"
              disabled={Boolean(continuation) || busy === "submit" || !enabledAgents.length}
              id="agent-collaboration-target"
              onChange={(next) => setAgentId(next)}
              options={continuation ? [{ value: continuation.agentId, label: agentName(continuation.agentId) }] : enabledAgents.map((agent) => ({ value: agent.id, label: agent.name }))}
              placeholder="请先启用一个 CLI"
              value={targetId}
            /></div>
            <label className="mg-field" htmlFor="agent-collaboration-cwd"><span>工作目录</span><input id="agent-collaboration-cwd" value={targetCwd} disabled={Boolean(continuation) || busy === "submit"} placeholder="/absolute/path/to/project" spellCheck={false} autoCapitalize="off" autoCorrect="off" onChange={(event) => setCwd(event.target.value)} /></label>
          </div>
          <label className="mg-field" htmlFor="agent-collaboration-prompt"><span>{continuation ? "追加任务或问题" : "任务内容"}</span><textarea id="agent-collaboration-prompt" ref={promptRef} rows={4} value={prompt} disabled={busy === "submit"} placeholder={continuation ? "基于刚才的结果，继续…" : "描述目标、需要检查的文件，以及希望返回的结果…"} onChange={(event) => setPrompt(event.target.value)} /></label>
          <div className="agent-collaboration-form-footer"><p className="agent-collaboration-hint" id="agent-collaboration-submit-hint">{formDisabledReason || (targetQueue.willQueue ? tf("此 Agent 还有 {0} 个任务未结束，新任务会排队，前面的结束后按提交顺序自动启动。", [targetQueue.active]) : targetQueue.running ? tf("此 Agent 正在执行 {0} 个任务（同时最多 {1} 个），新任务会直接并发启动。", [targetQueue.running, targetQueue.concurrency]) : "新任务会创建 CLI 会话。已有任务可在结果区选择“继续该任务”。")}{!formDisabledReason && sameCwdRunning > 0 && tf(" 同一工作目录已有 {0} 个任务在运行，并发执行可能互相修改相同文件，建议使用不同的工作目录。", [sameCwdRunning])}</p><button type="submit" className="mg-button mg-primary" aria-describedby="agent-collaboration-submit-hint" disabled={Boolean(busy) || Boolean(formDisabledReason) || !prompt.trim() || !targetCwd.trim()}>{busy === "submit" ? <Loader2 size={15} className="agent-collaboration-spin" /> : <Play size={15} />}{busy === "submit" ? "提交中…" : continuation ? "发送并续聊" : "提交任务"}</button></div>
        </form>
      </section>

      <section className="agent-collaboration-section" aria-labelledby="agent-collaboration-tasks-title">
        <div className="agent-collaboration-section-head"><div><h2 id="agent-collaboration-tasks-title">任务与结果 <span className="agent-collaboration-count">{tasks.length}</span></h2><p role="status">{hasUnfinishedTasks ? "任务执行期间每 2 秒更新状态。" : "每 10 秒检查新任务，可接收其他客户端派发的任务。"}</p>{agentLoads.length > 0 && <p className="agent-collaboration-hint" aria-label={t("各 Agent 并发情况")}>{agentLoads.map(({ agent, state }) => tf("{0}：执行中 {1}/{2}，排队 {3}", [agent.name, state.running, state.concurrency, state.queued])).join("；")}</p>}</div></div>
        {!tasks.length ? <div className="agent-collaboration-empty">还没有子 Agent 任务。接入后，在 Codex 对话中分派第一个任务。</div> : <div className="agent-collaboration-task-workspace">
          <ul className="agent-collaboration-task-list" aria-label="协作任务">
            {tasks.map((task) => <li key={task.id}><button type="button" className="agent-collaboration-task-select" aria-pressed={task.id === selectedId} onClick={() => setSelectedId(task.id)}><span className="agent-collaboration-task-title"><strong>{agentName(task.agentId)}</strong><span className="agent-collaboration-status" data-status={task.status}>{statusLabels[task.status] ?? task.status}</span></span><span className="agent-collaboration-task-prompt">{task.prompt}</span><small>{formatTime(task.createdAtMs)}{task.parentTaskId ? " · 续聊" : ""}</small></button></li>)}
          </ul>
          {selected && <div className="agent-collaboration-task-detail" key={selected.id}>
            <div className="agent-collaboration-detail-head"><div><h3>{agentName(selected.agentId)}</h3><span className="agent-collaboration-status" data-status={selected.status} role="status">{selected.cancelRequested && unfinished(selected) ? "正在取消…" : statusLabels[selected.status] ?? selected.status}</span></div><div className="agent-collaboration-detail-actions">
              {unfinished(selected) ? <button type="button" className="mg-button" disabled={Boolean(busy) || selected.cancelRequested} onClick={() => void mutate("cancel", { id: selected.id }, () => setNotice("已请求取消任务，状态将在任务停止后更新。"))}><Square size={13} />{busy === "cancel" || selected.cancelRequested ? "正在取消…" : "取消任务"}</button> : selected.status === "completed" && Boolean(selected.sessionId) && <button type="button" className="mg-button" disabled={Boolean(busy)} onClick={() => continueTask(selected)}>继续该任务</button>}
            </div></div>
            <dl className="agent-collaboration-task-meta"><dt>工作目录</dt><dd>{selected.cwd}</dd><dt>任务 ID</dt><dd>{selected.id}</dd><dt>更新时间</dt><dd>{formatTime(selected.updatedAtMs)}</dd>{selected.sessionId && <><dt>会话 ID</dt><dd>{selected.sessionId}</dd></>}{selected.parentTaskId && <><dt>接续任务</dt><dd>{selected.parentTaskId}</dd></>}</dl>
            <div className="agent-collaboration-output"><h4>任务内容</h4><pre tabIndex={0}>{selected.prompt}</pre></div>
            {selected.progress && <div className="agent-collaboration-output"><h4>执行进度</h4><pre tabIndex={0} aria-label="执行进度">{selected.progress}</pre></div>}
            {selected.answer && <div className="agent-collaboration-output"><h4>结果</h4><pre tabIndex={0} aria-label="任务结果">{selected.answer}</pre></div>}
            {selected.error && <div className="agent-collaboration-output agent-collaboration-output-error"><h4>错误信息</h4><pre tabIndex={0} aria-label="任务错误">{selected.error}</pre></div>}
            {!selected.progress && !selected.answer && !selected.error && <p className="agent-collaboration-hint">{unfinished(selected) ? "等待 CLI 返回输出…" : "该任务没有返回文本输出。"}</p>}
            {!unfinished(selected) && (selected.status !== "completed" || !selected.sessionId) && <p className="agent-collaboration-hint">只有已完成且有会话 ID 的任务可以续聊。可创建新任务继续处理。</p>}
          </div>}
        </div>}
      </section>

      </div></details>
      <p className="agent-collaboration-hint agent-collaboration-footnote">外部 CLI 子 Agent，不接管已经打开的桌面会话。</p>
    </div>
  </div>;
}
