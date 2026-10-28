/** 托盘快捷面板的纯数据逻辑：页签、Codex 状态、供应商列表、用量速览。不碰 i18n，文案由组件翻译。 */
import type { GatewayPayload, GatewayPoolEntry, QuotaWindow, UsageSummary } from "./gateway-model.ts";

export type TrayTabId = "home" | "usage" | "gateway";

/** 页签顺序：先放所有用户都用得上的，网关放最后。 */
export const TRAY_TABS: readonly TrayTabId[] = ["home", "usage", "gateway"];

export const TRAY_TAB_STORAGE_KEY = "codex-plus-tray-tab";

/** 存储里的值不认识（旧版本、被改坏，或已去掉的「供应商」页签）时回到「概览」。 */
export function restoreTrayTab(raw: string | null | undefined): TrayTabId {
  return TRAY_TABS.includes(raw as TrayTabId) ? (raw as TrayTabId) : "home";
}

type TabStorage = Pick<Storage, "getItem" | "setItem">;

function defaultStorage(): TabStorage | null {
  try {
    return typeof window === "undefined" ? null : window.localStorage;
  } catch {
    return null;
  }
}

/** 读上次选中的页签；隐私模式等读不到时用默认。 */
export function readStoredTrayTab(storage: TabStorage | null = defaultStorage()): TrayTabId {
  try {
    return restoreTrayTab(storage?.getItem(TRAY_TAB_STORAGE_KEY));
  } catch {
    return "home";
  }
}

export function storeTrayTab(tab: TrayTabId, storage: TabStorage | null = defaultStorage()): void {
  try {
    storage?.setItem(TRAY_TAB_STORAGE_KEY, tab);
  } catch {
    // 写不进去只是下次不记得，不影响使用。
  }
}

/** load_overview 里面板用得到的部分。 */
export type TrayOverview = {
  status?: string;
  codex_app?: { status: string; path: string | null } | null;
  codex_version?: string | null;
  runtime_health?: {
    codex_app?: { status: string } | null;
    debugger?: { status: string } | null;
  } | null;
};

export type CodexState = "unknown" | "missing" | "stopped" | "running";

export function codexState(overview: TrayOverview | null): CodexState {
  if (!overview) return "unknown";
  if (overview.codex_app?.status === "missing") return "missing";
  const status = overview.runtime_health?.codex_app?.status;
  if (status === "running") return "running";
  if (!status || status === "not_checked") return "unknown";
  return "stopped";
}

/** Codex 在运行且 Codex++ 的调试连接已连上（增强功能生效）。 */
export function codexEnhanced(overview: TrayOverview | null): boolean {
  return codexState(overview) === "running" && overview?.runtime_health?.debugger?.status === "connected";
}

/** load_settings 里面板用得到的部分。 */
export type TrayRelayProfile = {
  id: string;
  name: string;
  model?: string;
  baseUrl?: string;
  protocol?: string;
  relayMode?: string;
  aggregate?: unknown;
};

export type TraySettings = {
  relayProfilesEnabled?: boolean;
  activeRelayId?: string;
  relayProfiles?: TrayRelayProfile[];
};

/** Codex 接入网关时写进 Codex 的那个特殊供应商。 */
export const GATEWAY_RELAY_ID = "codex-plus-gateway";

export type TrayRelayKind = "gateway" | "aggregate" | "conversion" | "official" | "direct";

export type TrayRelayItem = {
  id: string;
  name: string;
  /** 次要信息：模型 · 主机名。 */
  detail: string;
  /** 供应商地址，用来认品牌图标。 */
  baseUrl: string;
  kind: TrayRelayKind;
  active: boolean;
};

function hostOf(url: string | undefined): string {
  const text = (url ?? "").trim();
  if (!text) return "";
  try {
    return new URL(text).host;
  } catch {
    return text;
  }
}

function relayKind(profile: TrayRelayProfile): TrayRelayKind {
  if (profile.id === GATEWAY_RELAY_ID) return "gateway";
  if (profile.relayMode === "aggregate" || profile.aggregate) return "aggregate";
  if (profile.relayMode === "official") return "official";
  if (profile.protocol && profile.protocol !== "responses") return "conversion";
  return "direct";
}

/** 供应商页签的列表：当前在用的排最前，其余保持设置里的顺序。 */
export function trayRelayItems(settings: TraySettings | null): TrayRelayItem[] {
  if (!settings) return [];
  const active = settings.activeRelayId ?? "";
  const items = (settings.relayProfiles ?? []).map((profile) => {
    const model = (profile.model ?? "").replace(/\[[^\]]*\]$/, "").trim();
    return {
      id: profile.id,
      name: profile.name.trim(),
      detail: [model, hostOf(profile.baseUrl)].filter(Boolean).join(" · "),
      baseUrl: (profile.baseUrl ?? "").trim(),
      kind: relayKind(profile),
      active: profile.id === active,
    };
  });
  return [...items.filter((item) => item.active), ...items.filter((item) => !item.active)];
}

export function activeRelayItem(settings: TraySettings | null): TrayRelayItem | null {
  return trayRelayItems(settings).find((item) => item.active) ?? null;
}

/** 网关里有 Agent 已接入。 */
export function gatewayConnected(payload: GatewayPayload | null): boolean {
  return Boolean(payload?.agents.some((agent) => agent.enabled));
}

/** 网关页签要不要显示「还没用上网关」的说明：池里没有供应商、也没有 Agent 接入。 */
export function gatewayIntroNeeded(payload: GatewayPayload | null): boolean {
  if (!payload) return true;
  return payload.pool.length === 0 && !gatewayConnected(payload);
}

/** 用量速览的几个数。 */
export type TrayUsageGlance = {
  calls: number;
  errors: number;
  /** 成功率 0–1；没有请求时为 null。 */
  successRate: number | null;
  inputTokens: number;
  outputTokens: number;
  cachedTokens: number;
  /** 按 token 排的前几个模型，share 是占总 token 的比例。 */
  topModels: Array<{ key: string; label: string; tokens: number; share: number }>;
};

export function usageGlance(summary: UsageSummary | null, topCount = 3): TrayUsageGlance | null {
  if (!summary?.totals) return null;
  const totals = summary.totals;
  const tokenOf = (group: { inputTokens: number; outputTokens: number }) => group.inputTokens + group.outputTokens;
  const totalTokens = Math.max(1, tokenOf(totals));
  const topModels = [...(summary.byModel ?? [])]
    .sort((a, b) => tokenOf(b) - tokenOf(a))
    .slice(0, topCount)
    .filter((group) => tokenOf(group) > 0)
    .map((group) => ({ key: group.key, label: group.label || group.key, tokens: tokenOf(group), share: tokenOf(group) / totalTokens }));
  return {
    calls: totals.calls,
    errors: totals.errors,
    successRate: totals.calls > 0 ? (totals.calls - totals.errors) / totals.calls : null,
    inputTokens: totals.inputTokens,
    outputTokens: totals.outputTokens,
    cachedTokens: totals.cachedTokens,
    topModels,
  };
}

/** 订阅账号的额度：每个窗口剩多少（0–1），过了重置时间的按满额。 */
export type TrayQuotaRow = {
  providerId: string;
  name: string;
  plan: string;
  windows: Array<{ window: QuotaWindow; remaining: number }>;
};

export function trayQuotaRows(pool: GatewayPoolEntry[] | undefined, nowMs: number): TrayQuotaRow[] {
  return (pool ?? [])
    .filter((entry) => entry.enabled !== false && entry.quota && entry.quota.windows.length > 0)
    .map((entry) => ({
      providerId: entry.id,
      name: entry.name,
      plan: entry.plan ?? "",
      windows: entry.quota!.windows.map((window) => ({
        window,
        remaining: window.resetsAtMs !== null && window.resetsAtMs <= nowMs ? 1 : Math.min(1, Math.max(0, 1 - window.usedPercent / 100)),
      })),
    }));
}

/** 面板发给主窗口的请求（主窗口执行后回一个结果）。 */
export type TrayRequest =
  | { id: string; kind: "navigate"; route: TrayRoute }
  | { id: string; kind: "launch" | "restart" | "stop" };

export type TrayResult = { id: string; ok: boolean; message?: string };

/** 面板能直接打开的主窗口页面。 */
export const TRAY_ROUTES = ["overview", "sessions", "requestLog", "gateway", "gatewayProviders", "gatewayRouting", "gatewayUsage", "settings"] as const;
export type TrayRoute = (typeof TRAY_ROUTES)[number];

export function isTrayRoute(value: unknown): value is TrayRoute {
  return typeof value === "string" && (TRAY_ROUTES as readonly string[]).includes(value);
}

export const TRAY_REQUEST_EVENT = "codex-plus-tray-request";
export const TRAY_RESULT_EVENT = "codex-plus-tray-result";
/** 主窗口注册好 TRAY_REQUEST_EVENT 监听后发出（后端据此判断重建的主窗口能否接收请求，#2677）。 */
export const MAIN_WINDOW_READY_EVENT = "codex-plus-main-window-ready";

/** 校验主窗口收到的请求，字段不对的一律丢掉。 */
export function parseTrayRequest(value: unknown): TrayRequest | null {
  if (!value || typeof value !== "object") return null;
  const raw = value as Record<string, unknown>;
  if (typeof raw.id !== "string" || !raw.id) return null;
  if (raw.kind === "navigate") return isTrayRoute(raw.route) ? { id: raw.id, kind: "navigate", route: raw.route } : null;
  if (raw.kind === "launch" || raw.kind === "restart" || raw.kind === "stop") return { id: raw.id, kind: raw.kind };
  return null;
}
