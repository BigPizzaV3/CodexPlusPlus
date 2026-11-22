// 请求日志页。
import { invoke } from "@tauri-apps/api/core";
import { RefreshCw, X } from "lucide-react";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Switch } from "./gateway-ui";
import { AppSelect, type AppSelectOption } from "./AppSelect";
import { appConfirm } from "./app-dialog";
import { AgentGlyph, useGatewayPayload } from "./hub-shared";
import {
  agentName,
  groupDisplayName,
  formatQueueWait,
  formatRequestCost,
  maskAccountName,
  queueWaitMs,
  QUEUE_REASON_PREFIX,
  foreignCompactionDropped,
  FOREIGN_COMPACTION_DROPPED_REASON,
  compactionNote,
  parseCompactionNote,
  type CompactionNote,
  usageRangeStart,
  relayFilterFromFocus,
  relayFilterArgs,
  type RelayFilter,
  type UsageRange,
} from "./gateway-model";
import type { RequestLogFocus } from "./UsageTabs";
import { usePolling } from "./use-polling";
import { singleAgent } from "./agent-focus-model";
import { useAgentFocus } from "./agent-focus-store";
import { Menu, MenuItem } from "./ManagementPages";
import { errorText } from "@/lib/utils";
import { mergeRequestLogDelta, reuseRequestLogEntries, type RequestLogCursor } from "./request-log-merge";
import { getLanguage, t, tf } from "@/i18n";
import { CxCard, CxEmpty, CxFilters, CxToolbar } from "./cx-layout";
import type { RequestLogEntry, Status } from "./app-types";
import { requestLogAgent } from "./app-shared";

type RequestLogsResult = {
  status: Status;
  message: string;
  entries: RequestLogEntry[];
  /** 符合条件的不止返回的这些。 */
  more?: boolean;
  /** 日志文件里最早一条的开始时间（日志按大小滚动）。 */
  oldestMs?: number | null;
  /** 文件里最后读到的位置：下一轮带上它只读新写入的记录。 */
  cursor?: RequestLogCursor | null;
  /** 带游标读到的增量：entries 只是新完成的 + 进行中的，要并进已有列表。 */
  incremental?: boolean;
};

function formatRequestLogTime(timestampMs: number) {
  if (!Number.isFinite(timestampMs) || timestampMs <= 0) return "-";
  const date = new Date(timestampMs);
  const now = new Date();
  const sameDay =
    date.getFullYear() === now.getFullYear()
    && date.getMonth() === now.getMonth()
    && date.getDate() === now.getDate();
  const time = date.toLocaleTimeString(getLanguage() === "en" ? "en-US" : "zh-CN", { hour12: false });
  return sameDay ? time : `${date.getMonth() + 1}/${date.getDate()} ${time}`;
}

function formatRequestLogTokens(value?: number | null) {
  if (value == null) return "-";
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(2)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return String(value);
}

/** 缓存命中率展示：≥99% 保留一位小数，只有完全命中才显示 100%。 */
function formatRequestLogCacheHitRatio(
  inputTokens?: number | null,
  cachedTokens?: number | null,
  ratio?: number | null,
) {
  const input = inputTokens ?? 0;
  const cached = cachedTokens ?? 0;
  if (input <= 0 || cached <= 0) return "-";
  if (cached >= input) return "100%";
  const percent = (ratio ?? cached / input) * 100;
  if (percent >= 99) return `${percent.toFixed(1)}%`;
  return `${Math.round(percent)}%`;
}

/** 会话 id 展示：UUIDv7 前缀按时间递增、容易重名，这里取末 8 位区分。 */
function formatRequestLogSession(threadId?: string | null): string {
  const value = (threadId ?? "").trim();
  if (!value) return "-";
  return value.length > 12 ? `…${value.slice(-8)}` : value;
}

function formatRequestLogDuration(durationMs: number) {
  if (!Number.isFinite(durationMs) || durationMs < 0) return "-";
  if (durationMs < 1000) return `${durationMs}ms`;
  if (durationMs < 60_000) return `${(durationMs / 1000).toFixed(1)}s`;
  return `${Math.floor(durationMs / 60_000)}m${Math.round((durationMs % 60_000) / 1000)}s`;
}

function requestLogRetryLabel(entry: RequestLogEntry) {
  const base = entry.autoRetries > 0
    ? tf("重试 ×{0}", [String(entry.autoRetries)])
    : entry.attempts > 1
      ? tf("切换 ×{0}", [String(entry.attempts)])
      : "";
  // 网关因 Codex++ 本地限速排过队：标出排了多久。
  const queued = queueWaitMs(entry.retryReasons);
  const queue = queued === null ? "" : tf("限速排队 {0}", [formatQueueWait(queued)]);
  // 换上游后去掉了别家签发的压缩上下文：较早的对话内容上游已看不到，必须让用户看得见。
  const compaction = foreignCompactionDropped(entry.retryReasons) ? t("已去掉别家压缩上下文") : "";
  // 压缩请求改发到了压缩模型，或回退到了原模型（#3142）。
  const note = compactionNote(entry.retryReasons);
  const compactModel = note ? (note.kind === "model" ? t("压缩模型") : t("压缩回退原模型")) : "";
  return [base, queue, compaction, compactModel].filter(Boolean).join(" · ") || "-";
}

/** 压缩模型说明的展示文字（#3142）。 */
function compactionNoteText(note: CompactionNote): string {
  if (note.kind === "model") return tf("压缩改用 {0}", [note.model]);
  const reason = note.reason === "unavailable" ? t("压缩模型不可用")
    : note.reason === "incompatible" ? t("压缩形态与压缩模型的上游不兼容")
    : note.reason === "window" ? t("压缩模型的窗口装不下待压缩内容")
    : note.reason === "failed" ? t("压缩模型的上游拒绝或出错")
    : note.reason;
  return tf("压缩模型 {0} 未使用（{1}），已用原模型", [note.model, reason]);
}

/** 重试原因的展示：网关记的限速排队换成「限速排队 Xs」，去掉别家压缩上下文换成说明，其它原样。 */
function requestLogRetryReasons(entry: RequestLogEntry): string {
  return (entry.retryReasons ?? [])
    .map((reason) => {
      if (reason === FOREIGN_COMPACTION_DROPPED_REASON) {
        return t("已去掉别家签发的压缩上下文，较早的对话内容不可用");
      }
      const note = parseCompactionNote(reason);
      if (note) return compactionNoteText(note);
      if (!reason.startsWith(QUEUE_REASON_PREFIX)) return reason;
      const queued = queueWaitMs([reason]);
      return queued === null ? reason : tf("限速排队 {0}", [formatQueueWait(queued)]);
    })
    .join("、");
}

/** 生图请求（网关记成 imageGenerations / imageEdits）：列表里标「生图」，运行中显示「生成中」而不是「流式中」。 */
function isImageRequest(entry: { wireApi?: string }): boolean {
  return (entry.wireApi ?? "").startsWith("image");
}

const REQUEST_LOG_LIMIT = 300;
const REQUEST_LOG_POLL_MS = 2000;

/** 失败请求在列表里直接显示的错误摘要：上游返回 JSON 时取里面的 message，否则取第一行。 */
function requestLogErrorSummary(error: string): string {
  const text = error.trim();
  try {
    const value = JSON.parse(text) as { error?: { message?: unknown } | string; message?: unknown };
    const message = typeof value.error === "string" ? value.error : value.error?.message ?? value.message;
    if (typeof message === "string" && message.trim()) return message.trim();
  } catch {
    // 不是 JSON，按原文处理。
  }
  return text.split("\n")[0] ?? text;
}

type RequestLogRowProps = {
  entry: RequestLogEntry;
  /** 模型名、上游模型名的展示名（分组别名），父组件按 entries 缓存好传进来。 */
  model: string;
  upstream: string;
  expanded: boolean;
  /** 只有进行中的行用得到（算已用时）；其它行传 0，免得每秒跟着重渲染。 */
  nowMs: number;
  onToggle: (id: string) => void;
};

/** 请求日志的一行（含展开的明细）。按 props 记忆：自动刷新时只有变了的行（进行中、新来的）才重渲染。 */
const RequestLogRow = memo(function RequestLogRow({ entry, model, upstream, expanded, nowMs, onToggle }: RequestLogRowProps) {
  const running = entry.state === "running";
  const detailId = `request-log-detail-${entry.id}`;
  return (
    <>
      <div
        className={`request-log-row request-log-entry ${running ? "running" : entry.success ? "ok" : "bad"}`}
        data-expanded={expanded || undefined}
        onClick={() => onToggle(entry.id)}
        role="row"
      >
        <span role="cell">
          {/* 展开 / 收起靠第一格的按钮（能用键盘聚焦、带 aria-expanded）；点整行也一样。 */}
          <button
            aria-controls={expanded ? detailId : undefined}
            aria-expanded={expanded}
            className="request-log-toggle"
            onClick={(event) => {
              event.stopPropagation();
              onToggle(entry.id);
            }}
            type="button"
          >
            {formatRequestLogTime(entry.startedAtMs || entry.timestampMs)}
          </button>
        </span>
        {running ? (
          <span className="request-log-status running" role="cell">
            {t("运行中")}
          </span>
        ) : (
          <span
            className={`request-log-status ${entry.success ? "ok" : "bad"}`}
            role="cell"
            title={entry.error ?? ""}
          >
            {entry.statusCode || (entry.success ? "OK" : "ERR")}
            {!entry.success ? (
              <small className="request-log-stream-failed">
                {entry.stream ? t("流失败") : t("失败")}
              </small>
            ) : null}
          </span>
        )}
        <span className="request-log-ellipsis request-log-source" role="cell" title={`${agentName(requestLogAgent(entry))} · ${maskAccountName(entry.relayName)}`}>
          <AgentGlyph agent={requestLogAgent(entry)} />
          {maskAccountName(entry.relayName) || "-"}
        </span>
        <span className="request-log-ellipsis" role="cell" title={entry.upstreamModel ? `${model} → ${upstream}` : model}>
          {isImageRequest(entry) ? <em className="request-log-kind">{t("生图")}</em> : null}
          {model || "-"}
          {entry.upstreamModel && entry.upstreamModel !== entry.model ? (
            <small> → {upstream}</small>
          ) : null}
        </span>
        <span
          className={entry.autoRetries > 0 || queueWaitMs(entry.retryReasons) !== null || foreignCompactionDropped(entry.retryReasons) || compactionNote(entry.retryReasons)?.kind === "fallback" ? "request-log-retry" : "request-log-muted"}
          role="cell"
          title={requestLogRetryReasons(entry)}
        >
          {requestLogRetryLabel(entry)}
        </span>
        {!running && !entry.success && entry.error ? (
          // 失败的请求没有 token 和费用：这几列直接显示错误原因。
          <span className="request-log-error-cell" role="cell" title={entry.error}>{requestLogErrorSummary(entry.error)}</span>
        ) : (
          <>
            <span className="request-log-muted" role="cell" title={running ? t("token 用量由上游在请求完成时返回，流式过程中无法准确统计") : ""}>
              {running ? (isImageRequest(entry) ? t("生成中") : t("流式中")) : formatRequestLogTokens(entry.usage?.inputTokens)}
            </span>
            <span
              className="request-log-muted"
              role="cell"
              title={running
                ? t("按流式增量估算，完成后会更新为准确值")
                : ""}
            >
              {running
                ? entry.estimatedOutputTokens
                  ? `~${formatRequestLogTokens(entry.estimatedOutputTokens)}`
                  : (isImageRequest(entry) ? t("生成中") : t("流式中"))
                : formatRequestLogTokens(entry.usage?.outputTokens)}
            </span>
            <span role="cell">
              {running ? (
                <span className="request-log-muted">{(isImageRequest(entry) ? t("生成中") : t("流式中"))}</span>
              ) : entry.usage?.cachedTokens ? (
                <>
                  {formatRequestLogTokens(entry.usage.cachedTokens)}
                  <small>
                    {" · "}
                    {formatRequestLogCacheHitRatio(
                      entry.usage.inputTokens,
                      entry.usage.cachedTokens,
                      entry.cacheHitRatio,
                    )}
                  </small>
                </>
              ) : "-"}
            </span>
            <span className="request-log-muted" role="cell">{entry.cost ? formatRequestCost(entry.cost) : "-"}</span>
          </>
        )}
        <span className="request-log-duration" role="cell">
          {formatRequestLogDuration(
            running
              ? Math.max(0, nowMs - entry.startedAtMs)
              : entry.durationMs,
          )}
          {running ? (
            <small>{t("进行中")}</small>
          ) : entry.firstTokenMs != null ? (
            <small>{tf("首字 {0}", [formatRequestLogDuration(entry.firstTokenMs)])}</small>
          ) : null}
        </span>
      </div>
      {expanded ? (
        <div role="row">
          <div className="request-log-detail" id={detailId} role="cell">
            <dl>
              <div><dt>{t("请求 ID")}</dt><dd><code>{entry.id}</code></dd></div>
              <div>
                <dt>{t("会话 ID")}</dt>
                <dd>
                  {entry.threadTitle ? <strong className="request-log-session-title">{entry.threadTitle}</strong> : null}
                  <code>{entry.threadId || "-"}</code>
                </dd>
              </div>
              <div><dt>{t("路径")}</dt><dd><code>{entry.method} {entry.path}</code></dd></div>
              <div><dt>{t("上游协议")}</dt><dd>{entry.wireApi || "-"}</dd></div>
              <div><dt>{t("上游模型")}</dt><dd>{upstream || "-"}</dd></div>
              <div><dt>{t("首字耗时")}</dt><dd>{entry.firstTokenMs == null ? (running ? t("进行中") : "-") : formatRequestLogDuration(entry.firstTokenMs)}</dd></div>
              <div><dt>{t("尝试次数")}</dt><dd>{entry.attempts || "-"}</dd></div>
              <div><dt>{t("自动重试")}</dt><dd>{entry.autoRetries}</dd></div>
              <div><dt>{t("重试原因")}</dt><dd>{requestLogRetryReasons(entry) || "-"}</dd></div>
              {entry.attemptDetails?.length ? (
                <div className="request-log-attempts">
                  <dt>{t("尝试明细")}</dt>
                  <dd>
                    <ul>
                      {entry.attemptDetails.map((detail) => (
                        <li key={`${detail.attempt}-${detail.code ?? ""}`}>
                          <strong>{tf("第 {0} 次", [detail.attempt])}</strong>
                          <code>{detail.code || "-"}</code>
                          {detail.delayMs != null ? (
                            <small>{tf("退避 {0}", [formatRequestLogDuration(detail.delayMs)])}</small>
                          ) : null}
                          {detail.message ? <small title={detail.message}>{detail.message}</small> : null}
                        </li>
                      ))}
                    </ul>
                  </dd>
                </div>
              ) : null}
              <div><dt>{t("流式")}</dt><dd>{entry.stream ? "SSE" : "JSON"}</dd></div>
              <div><dt>{t("流结果")}</dt><dd>{entry.streamOutcome || "-"}</dd></div>
              <div><dt>{t("推理 tokens")}</dt><dd>{formatRequestLogTokens(entry.usage?.reasoningTokens)}</dd></div>
              <div><dt>{t("错误")}</dt><dd>{entry.error || "-"}</dd></div>
            </dl>
          </div>
        </div>
      ) : null}
    </>
  );
});

export function RequestLogScreen({ initialAgent = "all", initialFocus = {}, onConsumeInitialAgent }: { initialAgent?: "all" | "codex" | "claude" | "grok"; initialFocus?: RequestLogFocus; onConsumeInitialAgent?: () => void }) {
  const payload = useGatewayPayload(10_000);
  const [entries, setEntries] = useState<RequestLogEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [query, setQuery] = useState("");
  // 时间段、供应商、模型在后端筛：日志只读最近 REQUEST_LOG_LIMIT 条，先截再筛会漏。
  const [range, setRange] = useState<UsageRange>(initialFocus.range ?? "all");
  const [relayFilter, setRelayFilter] = useState<RelayFilter | null>(() => relayFilterFromFocus(initialFocus));
  const [modelFilter, setModelFilter] = useState(initialFocus.model ?? "");
  const [more, setMore] = useState(false);
  const [oldestMs, setOldestMs] = useState<number | null>(null);
  const [filter, setFilter] = useState<"all" | "running" | "success" | "failed" | "retried">("all");
  const [sessionFilter, setSessionFilter] = useState("all");
  const [agentFilter, setAgentFilter] = useState<"all" | "codex" | "claude" | "grok">(initialAgent);
  // 只管理 Codex 时固定看 Codex 的请求，不出现 Agent 筛选。
  const codexOnly = singleAgent(useAgentFocus()) === "codex";
  const agentScope = codexOnly ? "codex" : agentFilter;
  // 初始过滤只用一次：之后从侧栏进来仍是「全部」。
  useEffect(() => { if (initialAgent !== "all" || Object.keys(initialFocus).length) onConsumeInitialAgent?.(); }, []); // eslint-disable-line react-hooks/exhaustive-deps
  const [autoRefresh, setAutoRefresh] = useState(true);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [clearing, setClearing] = useState(false);
  const [nowMs, setNowMs] = useState(() => Date.now());

  const hasRunning = entries.some((entry) => entry.state === "running");
  useEffect(() => {
    if (!hasRunning) return;
    const timer = window.setInterval(() => setNowMs(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [hasRunning]);

  const sinceMs = usageRangeStart(range);
  const filterArgs = {
    limit: REQUEST_LOG_LIMIT,
    sinceMs: sinceMs ?? null,
    agent: agentScope === "all" ? null : agentScope,
    ...relayFilterArgs(relayFilter),
    model: modelFilter || null,
  };
  // 自动刷新的定时器拿最新的筛选条件；序号保证只认最后一次读取。
  const filterRef = useRef(filterArgs);
  filterRef.current = filterArgs;
  const latestLoad = useRef(0);
  // 自动刷新只读上次之后新写入的记录（后端按游标增量读），不再每轮把整个日志读一遍、整表重渲染。
  const cursorRef = useRef<RequestLogCursor | null>(null);
  const entriesRef = useRef(entries);
  entriesRef.current = entries;
  const load = async (silent = false) => {
    const id = ++latestLoad.current;
    // 手动刷新、换筛选条件时整页重读：旧游标对应的是旧条件下的列表。
    if (!silent) {
      setLoading(true);
      cursorRef.current = null;
    }
    const args = silent && cursorRef.current ? { ...filterRef.current, cursor: cursorRef.current } : filterRef.current;
    try {
      const result = await invoke<RequestLogsResult>("load_request_logs", args);
      if (id !== latestLoad.current) return;
      if (result.status === "ok") {
        cursorRef.current = result.cursor ?? null;
        if (result.incremental) {
          const merged = mergeRequestLogDelta(entriesRef.current, result.entries ?? [], REQUEST_LOG_LIMIT, filterRef.current.sinceMs);
          if (merged.entries !== entriesRef.current) setEntries(merged.entries);
          if (merged.truncated) setMore(true);
        } else {
          setEntries((prev) => reuseRequestLogEntries(prev, result.entries ?? []));
          setMore(Boolean(result.more));
          setOldestMs(result.oldestMs ?? null);
        }
        setError("");
      } else {
        setError(result.message ? t(result.message) : t("读取请求日志失败。"));
      }
    } catch (cause) {
      setError(t(errorText(cause)));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [range, agentScope, filterArgs.relayId, filterArgs.relayUnknown, modelFilter]);

  // 每 2 秒增量读一次；窗口不可见时暂停，回到前台立刻补一轮。
  usePolling(() => load(true), REQUEST_LOG_POLL_MS, { enabled: autoRefresh, immediate: false });

  const clearLogs = async () => {
    if (!(await appConfirm(t("清空全部请求日志？此操作不可撤销。"), { title: t("清空请求日志"), confirmText: t("清空"), danger: true }))) return;
    setClearing(true);
    try {
      const result = await invoke<{ status: Status; message: string }>("clear_request_logs");
      if (result.status !== "ok") setError(t(result.message));
      await load(true);
    } catch (cause) {
      setError(t(errorText(cause)));
    } finally {
      setClearing(false);
    }
  };

  const sessions = useMemo(() => {
    const found = new Map<string, { id: string; title: string; count: number }>();
    for (const entry of entries) {
      const id = (entry.threadId ?? "").trim();
      if (!id) continue;
      const title = (entry.threadTitle ?? "").trim();
      const session = found.get(id);
      if (session) {
        session.count += 1;
        if (!session.title && title) session.title = title;
      } else {
        found.set(id, { id, title, count: 1 });
      }
    }
    // entries 本身按时间倒序，Map 的插入顺序即「最近活跃的会话优先」。
    return [...found.values()];
  }, [entries]);

  const hasUnknownSession = useMemo(
    () => entries.some((entry) => !(entry.threadId ?? "").trim()),
    [entries],
  );

  // 日志被清空/切换后，避免筛选项停留在已不存在的会话上。
  useEffect(() => {
    if (sessionFilter === "all" || sessionFilter === "__none__") return;
    if (!sessions.some((session) => session.id === sessionFilter)) {
      setSessionFilter("all");
    }
  }, [sessionFilter, sessions]);

  // 筛的时间段（或点进来的那一项）比日志里最早的一条还早：日志里看到的比统计少。
  const partial = oldestMs !== null && (sinceMs ?? 0) < oldestMs && (range !== "all" || relayFilter !== null || modelFilter !== "");

  // 模型展示名（分组别名）按 entries 和网关数据算一次，行组件和搜索都用它，不再每行每次渲染线性查找。
  const displayNames = useMemo(() => {
    const names = new Map<string, { model: string; upstream: string }>();
    for (const entry of entries) {
      names.set(entry.id, {
        model: groupDisplayName(payload, entry.model) ?? entry.model,
        upstream: groupDisplayName(payload, entry.upstreamModel) ?? entry.upstreamModel,
      });
    }
    return names;
  }, [entries, payload]);
  const toggleExpanded = useCallback((id: string) => setExpanded((current) => (current === id ? null : id)), []);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return entries.filter((entry) => {
      const running = entry.state === "running";
      if (filter === "running" && !running) return false;
      if (filter === "success" && (running || !entry.success)) return false;
      if (filter === "failed" && (running || entry.success)) return false;
      if (filter === "retried" && entry.autoRetries === 0 && entry.attempts <= 1) return false;
      if (agentScope !== "all" && requestLogAgent(entry) !== agentScope) return false;
      const threadId = (entry.threadId ?? "").trim();
      if (sessionFilter === "__none__") {
        if (threadId) return false;
      } else if (sessionFilter !== "all" && threadId !== sessionFilter) {
        return false;
      }
      if (!needle) return true;
      return [
        entry.model,
        entry.upstreamModel,
        displayNames.get(entry.id)?.model,
        displayNames.get(entry.id)?.upstream,
        entry.relayName,
        entry.path,
        entry.threadId ?? "",
        entry.error ?? "",
        entry.wireApi,
        entry.statusCode || "",
      ]
        .join(" ")
        .toLowerCase()
        .includes(needle);
    });
  }, [entries, filter, query, sessionFilter, agentScope, displayNames]);

  // 工具条：搜索 + 自动刷新开关（关掉时旁边出现手动刷新）+ 三点菜单（清空日志，确认框保留）；
  // 下面一行：状态胶囊在左，Agent / 时间 / 会话三个下拉在右；表格一张卡，占满剩下的高度。
  const statusItems: Array<{ id: typeof filter; label: string }> = [
    { id: "all", label: t("全部") },
    { id: "running", label: t("进行中") },
    { id: "success", label: t("成功") },
    { id: "failed", label: t("失败") },
    { id: "retried", label: t("含重试") },
  ];
  return (
    <div className="request-log-panel request-log-page cx-page">
      <CxToolbar onQuery={setQuery} placeholder={t("搜索模型 / 供应商 / 路径 / 错误 / 状态码")} query={query}>
        <span className="request-log-auto" title={t("每 2 秒读取一次新请求")}>
          <Switch label={t("自动刷新")} on={autoRefresh} onToggle={() => setAutoRefresh(!autoRefresh)} />
          <span>{t("自动刷新")}</span>
        </span>
        {autoRefresh ? null : (
          <button aria-label={t("刷新")} className="cx-secondary request-log-refresh" onClick={() => void load()} title={t("刷新")} type="button">
            <RefreshCw className={loading ? "animate-spin" : undefined} />
          </button>
        )}
        <Menu>
          <MenuItem danger disabled={clearing || entries.length === 0} onClick={() => void clearLogs()}>{t("清空日志")}</MenuItem>
        </Menu>
      </CxToolbar>

      <CxFilters
        items={statusItems}
        label={t("状态筛选")}
        onChange={setFilter}
        trailing={
          <>
            {codexOnly ? null : <AppSelect
              ariaLabel={t("按 Agent 筛选")}
              className="request-log-select"
              menuWidth={160}
              onChange={(next) => setAgentFilter(next)}
              options={[
                { value: "all", label: t("全部 Agent") },
                { value: "codex", label: "Codex" },
                { value: "claude", label: "Claude Code" },
                { value: "grok", label: "Grok" },
              ] as AppSelectOption<typeof agentFilter>[]}
              size="sm"
              value={agentFilter}
            />}
            <AppSelect
              ariaLabel={t("时间段")}
              className="request-log-select"
              menuWidth={140}
              onChange={(next) => setRange(next)}
              options={[
                { value: "all", label: t("全部时间") },
                { value: "today", label: t("今天") },
                { value: "7d", label: t("近 7 天") },
                { value: "30d", label: t("近 30 天") },
              ] as AppSelectOption<UsageRange>[]}
              size="sm"
              value={range}
            />
            <AppSelect
              ariaLabel={t("按会话筛选")}
              className="request-log-select request-log-session-select"
              menuWidth={280}
              onChange={(next) => setSessionFilter(next)}
              options={[
                { value: "all", label: t("全部会话") },
                ...sessions.map((session) => ({
                  value: session.id,
                  label: tf("{0}（{1} 条）", [
                    session.title || tf("会话 {0}", [formatRequestLogSession(session.id)]),
                    String(session.count),
                  ]),
                })),
              ]}
              size="sm"
              title={t("按会话筛选")}
              value={sessionFilter}
            />
          </>
        }
        value={filter}
      />

      {relayFilter || modelFilter ? (
        <div className="request-log-chips">
          {relayFilter ? (
            <button className="request-log-chip" onClick={() => setRelayFilter(null)} title={t("去掉这个筛选")} type="button">
              {tf("供应商：{0}", [relayFilter.unknown ? t("未知") : relayFilter.label])}
              <X aria-hidden="true" className="h-3 w-3" />
            </button>
          ) : null}
          {modelFilter ? (
            <button className="request-log-chip" onClick={() => setModelFilter("")} title={t("去掉这个筛选")} type="button">
              {tf("模型：{0}", [groupDisplayName(payload, modelFilter) ?? modelFilter])}
              <X aria-hidden="true" className="h-3 w-3" />
            </button>
          ) : null}
        </div>
      ) : null}

      {error ? (
        <p className="field-hint request-log-error" role="alert">{error}</p>
      ) : null}

      <CxCard className="request-log-card">
        {loading && entries.length === 0 ? (
          <CxEmpty title={t("正在读取请求日志…")} />
        ) : filtered.length ? (
            <div className="request-log-table" role="table" aria-label={t("请求日志")}>
              <div className="request-log-row request-log-head" role="row">
                <span role="columnheader">{t("时间")}</span>
                <span role="columnheader">{t("状态")}</span>
                <span role="columnheader">{t("来源")}</span>
                <span role="columnheader">{t("模型")}</span>
                <span role="columnheader">{t("重试")}</span>
                <span role="columnheader">{t("输入")}</span>
                <span role="columnheader">{t("输出")}</span>
                <span role="columnheader">{t("缓存")}</span>
                <span role="columnheader">{t("费用")}</span>
                <span role="columnheader">{t("耗时")}</span>
              </div>
              {filtered.map((entry) => {
                const names = displayNames.get(entry.id);
                return (
                  <RequestLogRow
                    entry={entry}
                    expanded={expanded === entry.id}
                    key={entry.id}
                    model={names?.model ?? entry.model}
                    nowMs={entry.state === "running" ? nowMs : 0}
                    onToggle={toggleExpanded}
                    upstream={names?.upstream ?? entry.upstreamModel}
                  />
                );
              })}
            </div>
        ) : entries.length ? (
          <CxEmpty title={t("没有匹配当前筛选条件的请求。")} />
        ) : (
          <CxEmpty title={t("还没有请求日志")} detail={t("在 Codex 或已接入网关的 Agent 里发起一次对话后即可看到。")} />
        )}
      </CxCard>
      {more || partial ? (
        <p className="field-hint request-log-note">
          {more ? tf("只显示最近 {0} 条。", [REQUEST_LOG_LIMIT]) : ""}
          {partial && oldestMs ? tf("请求日志按大小滚动，最早只到 {0}；更早的请求只计入统计。", [new Date(oldestMs).toLocaleString(getLanguage() === "en" ? "en-US" : "zh-CN", { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" })]) : ""}
        </p>
      ) : null}
    </div>
  );
}
