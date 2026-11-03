/**
 * 「日志与诊断」页的数据整理：把诊断日志（每行一条 JSON：timestamp_ms / pid / event / detail）
 * 变成可读的列表行，把诊断报告（一整段 JSON）挑出几项关键信息。纯函数，页面和测试共用。
 */

export type LogTone = "error" | "warn" | "info";

export type LogEntry = {
  /** 原始行在文件里的序号（从 1 开始），用作 key。 */
  index: number;
  timestampMs: number | null;
  pid: number | null;
  /** 事件名，如 manager.save_settings；解析不了的行为空。 */
  event: string;
  tone: LogTone;
  /** 一行摘要：detail 里的简单字段 key=value，长的截断。 */
  summary: string;
  /** 展开后显示的内容：detail 的格式化 JSON；解析不了的行就是原文。 */
  body: string;
  raw: string;
};

const ERROR_PATTERN = /fail|error|panic|crash|denied|refused|exception|崩溃|失败|出错/i;
const WARN_PATTERN = /warn|timeout|timed_out|retry|degrad|fallback|stale|超时|重试|降级/i;
const SUMMARY_LIMIT = 180;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function scalarText(value: unknown): string | null {
  if (value === null || value === undefined) return null;
  if (typeof value === "string") return value.length > 60 ? `${value.slice(0, 57)}…` : value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  if (Array.isArray(value)) return `[${value.length}]`;
  if (isRecord(value)) return "{…}";
  return null;
}

/** detail 的一行摘要：先列简单字段，对象和数组只给个占位，整体截断。 */
export function summarizeDetail(detail: unknown): string {
  if (detail === null || detail === undefined) return "";
  if (!isRecord(detail)) return scalarText(detail) ?? "";
  const parts: string[] = [];
  // 简单字段（字符串、数字、布尔）排前面，结构排后面。
  const entries = Object.entries(detail).sort(([, left], [, right]) => Number(isRecord(left) || Array.isArray(left)) - Number(isRecord(right) || Array.isArray(right)));
  for (const [key, value] of entries) {
    const text = scalarText(value);
    if (text === null) continue;
    parts.push(`${key}=${text}`);
    if (parts.join(" · ").length > SUMMARY_LIMIT) break;
  }
  const joined = parts.join(" · ");
  return joined.length > SUMMARY_LIMIT ? `${joined.slice(0, SUMMARY_LIMIT - 1)}…` : joined;
}

/** 事件名或 detail 里的状态、错误字段像出错就标红，像重试、超时就标黄。 */
export function logTone(event: string, detail: unknown): LogTone {
  const status = isRecord(detail) ? [detail.status, detail.level, detail.result].filter((value) => typeof value === "string").join(" ") : "";
  const hasError = isRecord(detail) && (typeof detail.error === "string" && detail.error.length > 0);
  if (ERROR_PATTERN.test(event) || hasError || /^(failed|error)$/i.test(status.trim())) return "error";
  if (WARN_PATTERN.test(event) || /warn/i.test(status)) return "warn";
  return "info";
}

export function parseLogLine(raw: string, index: number): LogEntry {
  const line = raw.trim();
  if (line.startsWith("{")) {
    try {
      const record = JSON.parse(line) as unknown;
      if (isRecord(record) && typeof record.event === "string") {
        const detail = record.detail;
        return {
          index,
          timestampMs: typeof record.timestamp_ms === "number" ? record.timestamp_ms : null,
          pid: typeof record.pid === "number" ? record.pid : null,
          event: record.event,
          tone: logTone(record.event, detail),
          summary: summarizeDetail(detail),
          body: detail === undefined ? "" : JSON.stringify(detail, null, 2),
          raw,
        };
      }
    } catch {
      // 半行（截断时的第一行）或别的格式：按原文显示。
    }
  }
  return { index, timestampMs: null, pid: null, event: "", tone: ERROR_PATTERN.test(line) ? "error" : "info", summary: line, body: line, raw };
}

/**
 * 整个日志文本 → 列表，最新的在前。空行跳过。
 * `truncated`：后端只读了文件末尾，第一行多半是被截断的半行（`ount":0,…`）；解析不了就丢掉，不当成一条日志显示。
 */
export function parseLogText(text: string, truncated = false): LogEntry[] {
  const entries: LogEntry[] = [];
  text.split(/\r?\n/).forEach((line, index) => {
    if (line.trim()) entries.push(parseLogLine(line, index + 1));
  });
  if (truncated && entries.length > 1 && !entries[0].event) entries.shift();
  return entries.reverse();
}

export function filterLogEntries(entries: LogEntry[], query: string, errorsOnly: boolean): LogEntry[] {
  const needle = query.trim().toLowerCase();
  return entries.filter((entry) => {
    if (errorsOnly && entry.tone !== "error") return false;
    return !needle || entry.raw.toLowerCase().includes(needle);
  });
}

/** 时间显示：当天只显示时分秒，否则带月日。 */
export function formatLogTime(timestampMs: number | null, now = Date.now()): string {
  if (timestampMs === null || !Number.isFinite(timestampMs)) return "";
  const date = new Date(timestampMs);
  const today = new Date(now);
  const pad = (value: number) => String(value).padStart(2, "0");
  const time = `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
  const sameDay = date.getFullYear() === today.getFullYear() && date.getMonth() === today.getMonth() && date.getDate() === today.getDate();
  return sameDay ? time : `${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${time}`;
}

/**
 * 一项诊断信息：`status` 是中文状态（页面用 t() 翻译），`value` 是路径等原样显示的内容。
 * label 同样是中文原文。
 */
export type DiagnosticFact = { label: string; status?: string; value?: string; tone?: "ok" | "warn" | "danger" };

/** 报告里的英文状态词 → 中文。认不出的原样返回。 */
const STATUS_TEXT: Record<string, string> = {
  ok: "正常",
  found: "已找到",
  installed: "已安装",
  running: "运行中",
  enabled: "已启用",
  ready: "就绪",
  missing: "未找到",
  not_found: "未找到",
  notfound: "未找到",
  stopped: "已停止",
  disabled: "已禁用",
  unknown: "未知",
  failed: "失败",
  error: "出错",
  user_stopped: "手动停止",
};

export function diagnosticStatusText(status: string): string {
  return STATUS_TEXT[status.toLowerCase()] ?? status;
}

function pick(source: unknown, path: string[]): unknown {
  let current = source;
  for (const key of path) {
    if (!isRecord(current)) return undefined;
    current = current[key];
  }
  return current;
}

function statusTone(status: unknown): DiagnosticFact["tone"] {
  if (typeof status !== "string") return undefined;
  if (/^(ok|found|running|installed|enabled|ready)$/i.test(status)) return "ok";
  if (/^(missing|stopped|disabled|unknown|not_found|notfound)$/i.test(status)) return "warn";
  if (/fail|error|crash/i.test(status)) return "danger";
  return undefined;
}

/**
 * 诊断报告里挑几项一眼要看的：版本、Codex 应用与版本、两个快捷方式、最近一次启动。
 * 报告不是 JSON 或字段不在时返回空数组，页面只显示完整报告。
 */
export function diagnosticFacts(report: string): DiagnosticFact[] {
  let parsed: unknown;
  try {
    parsed = JSON.parse(report);
  } catch {
    return [];
  }
  if (!isRecord(parsed)) return [];
  const facts: DiagnosticFact[] = [];
  const text = (value: unknown) => (typeof value === "string" || typeof value === "number") && String(value) ? String(value) : undefined;
  const pushValue = (label: string, value: unknown) => {
    const shown = text(value);
    if (shown) facts.push({ label, value: shown });
  };
  // 带状态的一项：状态翻成中文，路径、阶段、消息原样跟在后面。
  const pushStatus = (label: string, record: unknown, details: string[]) => {
    if (!isRecord(record)) return;
    const status = text(record.status);
    const rest = details.map((key) => text(record[key])).filter((item): item is string => Boolean(item)).map(diagnosticStatusText);
    if (!status && !rest.length) return;
    facts.push({ label, status: status ? diagnosticStatusText(status) : undefined, value: rest.join(" · ") || undefined, tone: statusTone(status) });
  };
  pushValue("Codex++ 版本", parsed.version);
  pushStatus("Codex 应用", pick(parsed, ["overview", "codex_app"]), ["path"]);
  pushValue("Codex 版本", pick(parsed, ["overview", "codex_version"]));
  pushStatus("静默启动入口", pick(parsed, ["overview", "silent_shortcut"]), ["path"]);
  pushStatus("管理入口", pick(parsed, ["overview", "management_shortcut"]), ["path"]);
  pushStatus("最近一次启动", pick(parsed, ["overview", "latest_launch"]), ["phase", "message"]);
  return facts;
}
