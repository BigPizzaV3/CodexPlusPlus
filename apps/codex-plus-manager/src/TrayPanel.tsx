/**
 * 托盘快捷面板：三个页签——
 * 概览（Codex 状态与启动 / 重启 / 停止、今日速览、常用入口）、
 * 用量（今天 / 7 天的请求与 token、订阅额度、余额）、网关（各 Agent 换模型、最近请求）。
 * 旧版的直连供应商切换不放进面板：供应商统一在网关里管。
 *
 * 会拉起进程的操作（启动 / 重启 / 停止 Codex）不在面板里重做一遍，而是发给主窗口，
 * 走主窗口现有的流程（校验、冲突弹窗、等待启动结果），做完主窗口回一个结果。
 */
import { useEffect, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import {
  Activity,
  ChartColumn,
  Check,
  ChevronDown,
  ChevronRight,
  ExternalLink,
  House,
  Layers,
  MessagesSquare,
  Play,
  Plug,
  Split,
  RefreshCw,
  RotateCw,
  ScrollText,
  Settings,
  Square,
  Waypoints,
  type LucideIcon,
} from "lucide-react";
import { t, tf } from "./i18n";
import { ModelList, Floating } from "./GatewayAgents";
import {
  agentModels,
  agentName,
  bindingFromDraft,
  costText,
  draftFromBinding,
  formatMoney,
  formatTokens,
  modelDisplay,
  prettyModelName,
  quotaWindowLabel,
  relativeTime,
  type GatewayAgentView,
  type GatewayPayload,
  type GatewayResult,
  type ProviderBalance,
  type UsageSummary,
  maskAccountName,
} from "./gateway-model";
import { AGENT_ICONS, APP_LOGO_URL, AutoTag, Avatar, AgentIcon, ModelViewGlyph } from "./gateway-ui";
import {
  activeRelayItem,
  codexEnhanced,
  codexState,
  gatewayConnected,
  gatewayIntroNeeded,
  readStoredTrayTab,
  storeTrayTab,
  trayQuotaRows,
  usageGlance,
  TRAY_REQUEST_EVENT,
  TRAY_RESULT_EVENT,
  TRAY_TABS,
  type CodexState,
  type TrayOverview,
  type TrayRelayKind,
  type TrayRequest,
  type TrayResult,
  type TrayRoute,
  type TraySettings,
  type TrayTabId,
} from "./tray-panel-model";
import "./GatewayPage.css";
import "./TrayPanel.css";

/** 面板与主窗口共用主题设置（主窗口写在 localStorage 里）。 */
function applyTheme() {
  let theme = "dark";
  try {
    theme = window.localStorage.getItem("codex-plus-theme") === "light" ? "light" : "dark";
  } catch {
    // 读不到就用深色。
  }
  document.documentElement.classList.toggle("dark", theme === "dark");
  document.documentElement.classList.toggle("light", theme === "light");
}

type LogEntry = {
  id: string;
  timestampMs: number;
  model: string;
  upstreamModel?: string;
  relayName?: string;
  statusCode?: number;
  success?: boolean;
  state?: string | null;
  durationMs?: number;
  agent?: string | null;
  usage?: { inputTokens: number; outputTokens: number } | null;
};

type UsageRange = "today" | "7d";

/** 网关页签要看实时请求，刷得勤一些；其它页签慢一点。都只在面板可见时刷。 */
const REFRESH_MS: Record<TrayTabId, number> = { home: 5000, usage: 8000, gateway: 3000 };
/** 发给主窗口的请求最多等这么久（启动 Codex 本身最多等 30 秒）。 */
const REQUEST_TIMEOUT_MS = 90_000;

const TAB_META: Record<TrayTabId, { label: string; icon: LucideIcon }> = {
  home: { label: "概览", icon: House },
  usage: { label: "用量", icon: ChartColumn },
  gateway: { label: "网关", icon: Waypoints },
};

type Pending = { id: string; kind: TrayRequest["kind"]; target?: string };
type Flash = { tone: "ok" | "error"; text: string };

export function TrayPanel() {
  const [tab, setTabState] = useState<TrayTabId>(() => readStoredTrayTab());
  const [overview, setOverview] = useState<TrayOverview | null>(null);
  const [settings, setSettings] = useState<TraySettings | null>(null);
  const [payload, setPayload] = useState<GatewayPayload | null>(null);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [range, setRange] = useState<UsageRange>("today");
  const [usage, setUsage] = useState<Partial<Record<UsageRange, UsageSummary>>>({});
  const [balances, setBalances] = useState<ProviderBalance[] | null>(null);
  const [balancesLoading, setBalancesLoading] = useState(false);
  const [pending, setPending] = useState<Pending | null>(null);
  const [flash, setFlash] = useState<Flash | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const pendingRef = useRef<Pending | null>(null);
  const tabRef = useRef(tab);
  const rangeRef = useRef(range);
  tabRef.current = tab;
  rangeRef.current = range;

  const loadOverview = async () => {
    try {
      setOverview(await invoke<TrayOverview>("load_overview"));
    } catch {
      // 读不到时保持上一次的状态。
    }
  };
  const loadSettings = async () => {
    try {
      const result = await invoke<{ settings?: TraySettings }>("load_settings");
      if (result.settings) setSettings(result.settings);
    } catch {
      // 同上。
    }
  };
  const loadGateway = async () => {
    try {
      const result = await invoke<GatewayResult>("load_gateway");
      if (result.gateway && Array.isArray(result.pool)) setPayload(result as GatewayPayload);
    } catch {
      // 网关读不到时网关页签显示说明。
    }
  };
  const loadLogs = async () => {
    try {
      const result = await invoke<{ entries?: LogEntry[] }>("load_request_logs", { limit: 40 });
      setLogs(result.entries ?? []);
    } catch {
      setLogs([]);
    }
  };
  const loadUsage = async (which: UsageRange) => {
    try {
      const result = await invoke<{ status: string } & Partial<UsageSummary>>("load_gateway_usage", { range: which });
      if (result.status === "ok" && result.totals) setUsage((current) => ({ ...current, [which]: result as UsageSummary }));
    } catch {
      // 读不到就保持空。
    }
  };
  const loadBalances = async () => {
    setBalancesLoading(true);
    try {
      const result = await invoke<{ balances?: ProviderBalance[] }>("load_gateway_balances");
      setBalances(result.balances ?? []);
    } catch {
      setBalances([]);
    } finally {
      setBalancesLoading(false);
    }
  };

  /** 只读当前页签用得到的数据；Codex 状态在页头也要用，每次都读。 */
  const loadTab = async (which: TrayTabId) => {
    const jobs: Array<Promise<void>> = [loadOverview()];
    if (which === "home") jobs.push(loadSettings(), loadUsage("today"));
    if (which === "usage") jobs.push(loadUsage(rangeRef.current), loadGateway());
    if (which === "gateway") jobs.push(loadGateway(), loadLogs());
    await Promise.all(jobs);
  };

  const setTab = (next: TrayTabId) => {
    setTabState(next);
    storeTrayTab(next);
    void loadTab(next);
  };

  useEffect(() => {
    applyTheme();
    void loadTab(tabRef.current);
    const onStorage = () => applyTheme();
    const onFocus = () => {
      applyTheme();
      void loadTab(tabRef.current);
    };
    window.addEventListener("storage", onStorage);
    window.addEventListener("focus", onFocus);
    return () => {
      window.removeEventListener("storage", onStorage);
      window.removeEventListener("focus", onFocus);
    };
    // 只在挂载时注册。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const timer = window.setInterval(() => {
      if (document.visibilityState === "visible") void loadTab(tab);
    }, REFRESH_MS[tab]);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab]);

  // 主窗口做完面板发去的请求后回一个结果。
  useEffect(() => {
    let disposed = false;
    let stop: (() => void) | undefined;
    void listen<TrayResult>(TRAY_RESULT_EVENT, (event) => {
      const result = event.payload;
      const current = pendingRef.current;
      if (disposed || !current || !result || result.id !== current.id) return;
      finishPending(current, result.ok, result.message);
    }).then((unlisten) => {
      if (disposed) unlisten();
      else stop = unlisten;
    });
    return () => {
      disposed = true;
      stop?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!flash) return;
    const timer = window.setTimeout(() => setFlash(null), 4000);
    return () => window.clearTimeout(timer);
  }, [flash]);

  const finishPending = (current: Pending, ok: boolean, message?: string) => {
    pendingRef.current = null;
    setPending(null);
    if (current.kind !== "navigate") {
      if (!ok) setFlash({ tone: "error", text: message || t("没有完成，详情见主窗口。") });
    }
    void loadTab(tabRef.current);
  };

  /** 把请求发给主窗口。同一时间只处理一个。 */
  const request = async (body: { kind: "launch" | "restart" | "stop" }, target?: string) => {
    if (pendingRef.current) return;
    const id = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
    const next: Pending = { id, kind: body.kind, target };
    pendingRef.current = next;
    setPending(next);
    setFlash(null);
    window.setTimeout(() => {
      if (pendingRef.current?.id === id) finishPending(next, false, t("主窗口没有回应，请打开 Codex++ 查看。"));
    }, REQUEST_TIMEOUT_MS);
    try {
      await emit(TRAY_REQUEST_EVENT, { ...body, id } satisfies TrayRequest);
    } catch (error) {
      finishPending(next, false, String(error));
    }
  };

  /** 在主窗口打开某页：先让主窗口切页，再把它叫出来（面板随之收起）。 */
  const openRoute = async (route: TrayRoute) => {
    try {
      await emit(TRAY_REQUEST_EVENT, { id: `nav-${Date.now().toString(36)}`, kind: "navigate", route } satisfies TrayRequest);
    } catch {
      // 切页失败也照样打开主窗口。
    }
    void invoke("tray_panel_action", { action: "open" });
  };

  const action = (name: "open" | "quit") => void invoke("tray_panel_action", { action: name });

  const refreshNow = async () => {
    setRefreshing(true);
    try {
      await loadTab(tab);
    } finally {
      setRefreshing(false);
    }
  };

  const state = codexState(overview);

  return (
    <div className="gateway-page tray-panel">
      <header className="tp-head" data-tauri-drag-region>
        <img alt="" src={APP_LOGO_URL} />
        <div className="tp-head-title" data-tauri-drag-region>
          <strong>Codex++</strong>
          <span data-tauri-drag-region>
            <i className="tp-dot" data-tone={codexTone(state)} />
            {codexStateText(state)}
          </span>
        </div>
        <button aria-label={t("刷新")} className="gw-icon-button" onClick={() => void refreshNow()} title={t("刷新")} type="button">
          <RefreshCw className={`h-4 w-4 ${refreshing ? "animate-spin" : ""}`} />
        </button>
        <button aria-label={t("打开 Codex++")} className="gw-icon-button" onClick={() => action("open")} title={t("打开 Codex++")} type="button">
          <ExternalLink className="h-4 w-4" />
        </button>
      </header>

      <nav aria-label={t("快捷面板页签")} className="tp-tabs" role="tablist">
        {TRAY_TABS.map((id) => {
          const meta = TAB_META[id];
          const Icon = meta.icon;
          const badge = id === "gateway" && gatewayConnected(payload);
          return (
            <button aria-selected={tab === id} className="tp-tab" data-active={tab === id} key={id} onClick={() => setTab(id)} role="tab" type="button">
              <Icon aria-hidden="true" />
              <span>{t(meta.label)}</span>
              {badge ? <i className="tp-tab-badge" title={t("有 Agent 已接入网关")} /> : null}
            </button>
          );
        })}
      </nav>

      <div className="tp-body" role="tabpanel">
        {tab === "home" ? (
          <HomeTab
            onOpenRoute={openRoute}
            onRequest={request}
            onShowUsage={() => setTab("usage")}
            overview={overview}
            pending={pending}
            settings={settings}
            today={usage.today ?? null}
          />
        ) : null}
        {tab === "usage" ? (
          <UsageTab
            balances={balances}
            balancesLoading={balancesLoading}
            onLoadBalances={() => void loadBalances()}
            onOpenRoute={openRoute}
            onRange={(next) => {
              setRange(next);
              rangeRef.current = next;
              void loadUsage(next);
            }}
            payload={payload}
            range={range}
            summary={usage[range] ?? null}
          />
        ) : null}
        {tab === "gateway" ? <GatewayTab logs={logs} onOpenRoute={openRoute} onPayload={setPayload} payload={payload} /> : null}
      </div>

      {flash ? (
        <div className="tp-flash" data-tone={flash.tone} role="status">
          {flash.text}
        </div>
      ) : null}

      <footer className="tp-foot">
        <button className="tp-foot-button" onClick={() => void openRoute("settings")} type="button">
          <Settings aria-hidden="true" />
          {t("设置")}
        </button>
        <span />
        <button className="tp-foot-button" onClick={() => action("quit")} type="button">{t("退出")}</button>
      </footer>
    </div>
  );
}

function codexTone(state: CodexState): string {
  return state === "running" ? "on" : state === "missing" ? "error" : "off";
}

function codexStateText(state: CodexState): string {
  if (state === "running") return t("Codex 运行中");
  if (state === "stopped") return t("Codex 未运行");
  if (state === "missing") return t("没有找到 Codex");
  return t("正在检查 Codex…");
}

function Section({ title, aside, children }: { title: string; aside?: ReactNode; children: ReactNode }) {
  return (
    <section className="tp-section">
      <div className="tp-section-head">
        <span>{title}</span>
        {aside}
      </div>
      {children}
    </section>
  );
}

function LinkButton({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <button className="tp-link" onClick={onClick} type="button">
      {label}
      <ChevronRight aria-hidden="true" />
    </button>
  );
}

/* —————————————— 概览 —————————————— */

const QUICK_LINKS: Array<{ route: TrayRoute; label: string; icon: LucideIcon }> = [
  { route: "sessions", label: "会话", icon: MessagesSquare },
  { route: "requestLog", label: "请求日志", icon: ScrollText },
  { route: "gateway", label: "网关接入", icon: Waypoints },
  { route: "gatewayProviders", label: "供应商池", icon: Layers },
  { route: "gatewayRouting", label: "路由", icon: Split },
  { route: "gatewayUsage", label: "用量统计", icon: Activity },
];

function HomeTab({
  overview,
  settings,
  today,
  pending,
  onRequest,
  onOpenRoute,
  onShowUsage,
}: {
  overview: TrayOverview | null;
  settings: TraySettings | null;
  today: UsageSummary | null;
  pending: Pending | null;
  onRequest: (body: { kind: "launch" | "restart" | "stop" }) => Promise<void>;
  onOpenRoute: (route: TrayRoute) => Promise<void>;
  onShowUsage: () => void;
}) {
  const state = codexState(overview);
  const active = activeRelayItem(settings);
  const glance = usageGlance(today);
  const busy = pending !== null;
  const busyKind = pending && pending.kind !== "navigate" ? pending.kind : null;
  const connection = settings?.relayProfilesEnabled === false
    ? t("供应商配置已关闭")
    : !active
      ? t("还没有配置供应商")
      : active.kind === "gateway"
        ? t("经 Codex++ 网关")
        : tf("直连 · {0}", [active.name || t("未命名供应商")]);
  return (
    <>
      <div className="tp-card tp-codex">
        <div className="tp-codex-top">
          <img alt="" src={AGENT_ICONS.codexApp} />
          <div className="tp-codex-name">
            <strong>Codex</strong>
            <small>{overview?.codex_version ? `v${overview.codex_version}` : state === "missing" ? t("没有找到 Codex 应用") : "—"}</small>
          </div>
          <span className="tp-state" data-tone={codexTone(state)}>
            <i className="tp-dot" data-tone={codexTone(state)} />
            {state === "running" ? t("运行中") : state === "stopped" ? t("未运行") : state === "missing" ? t("未找到") : t("检查中")}
          </span>
        </div>
        <div className="tp-codex-facts">
          <span title={active?.detail}>
            <Plug aria-hidden="true" />
            {connection}
          </span>
          {state === "running" ? (
            <span>
              <Check aria-hidden="true" />
              {codexEnhanced(overview) ? t("增强功能已生效") : t("增强功能未连接")}
            </span>
          ) : null}
        </div>
        <div className="tp-codex-actions">
          {state === "running" ? (
            <>
              <button className="tp-button" data-variant="primary" disabled={busy} onClick={() => void onRequest({ kind: "restart" })} type="button">
                <RotateCw aria-hidden="true" className={busyKind === "restart" ? "animate-spin" : ""} />
                {busyKind === "restart" ? t("正在重启…") : t("重启 Codex")}
              </button>
              <button className="tp-button" disabled={busy} onClick={() => void onRequest({ kind: "stop" })} type="button">
                <Square aria-hidden="true" />
                {busyKind === "stop" ? t("正在停止…") : t("停止")}
              </button>
            </>
          ) : (
            <button className="tp-button" data-variant="primary" disabled={busy || state === "missing"} onClick={() => void onRequest({ kind: "launch" })} type="button">
              <Play aria-hidden="true" />
              {busyKind === "launch" ? t("正在启动…") : t("启动 Codex")}
            </button>
          )}
        </div>
      </div>

      <button className="tp-card tp-today" onClick={onShowUsage} title={t("查看用量")} type="button">
        <Stat label={t("今日请求")} value={glance ? String(glance.calls) : "—"} />
        <Stat label="Token" value={glance ? formatTokens(glance.inputTokens + glance.outputTokens) : "—"} />
        <Stat label={t("成功率")} value={glance?.successRate != null ? `${Math.round(glance.successRate * 100)}%` : "—"} />
        <ChevronRight aria-hidden="true" className="tp-today-arrow" />
      </button>

      <Section title={t("快速打开")}>
        <div className="tp-quick">
          {QUICK_LINKS.map((link) => {
            const Icon = link.icon;
            return (
              <button className="tp-quick-item" key={link.route} onClick={() => void onOpenRoute(link.route)} type="button">
                <Icon aria-hidden="true" />
                <span>{t(link.label)}</span>
              </button>
            );
          })}
        </div>
      </Section>
    </>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <span className="tp-stat">
      <strong>{value}</strong>
      <small>{label}</small>
    </span>
  );
}

/* —————————————— 用量 —————————————— */

function UsageTab({
  range,
  summary,
  payload,
  balances,
  balancesLoading,
  onRange,
  onLoadBalances,
  onOpenRoute,
}: {
  range: UsageRange;
  summary: UsageSummary | null;
  payload: GatewayPayload | null;
  balances: ProviderBalance[] | null;
  balancesLoading: boolean;
  onRange: (range: UsageRange) => void;
  onLoadBalances: () => void;
  onOpenRoute: (route: TrayRoute) => Promise<void>;
}) {
  const glance = usageGlance(summary);
  const cost = costText(summary?.costs);
  const quotas = trayQuotaRows(payload?.pool, Date.now());
  const hasPool = Boolean(payload?.pool.length);
  return (
    <>
      <div className="tp-range" role="tablist">
        {(["today", "7d"] as const).map((id) => (
          <button aria-selected={range === id} data-active={range === id} key={id} onClick={() => onRange(id)} role="tab" type="button">
            {id === "today" ? t("今天") : t("7 天")}
          </button>
        ))}
        <span />
        <LinkButton label={t("完整统计")} onClick={() => void onOpenRoute("gatewayUsage")} />
      </div>

      <div className="tp-card tp-stats">
        <Stat label={t("请求")} value={glance ? String(glance.calls) : "—"} />
        <Stat label={t("成功率")} value={glance?.successRate != null ? `${Math.round(glance.successRate * 100)}%` : "—"} />
        <Stat label={t("输入")} value={glance ? formatTokens(glance.inputTokens) : "—"} />
        <Stat label={t("输出")} value={glance ? formatTokens(glance.outputTokens) : "—"} />
      </div>
      {cost ? <p className="tp-cost">{tf("按填写的价格估算：{0}", [cost])}</p> : null}

      {glance && glance.topModels.length ? (
        <Section title={t("用得最多的模型")}>
          <div className="tp-card tp-bars">
            {glance.topModels.map((model) => (
              <div className="tp-bar-row" key={model.key}>
                <span className="tp-bar-label" title={model.label}>{prettyModelName(model.label)}</span>
                <span className="tp-bar-track"><i style={{ width: `${Math.max(3, Math.round(model.share * 100))}%` }} /></span>
                <span className="tp-bar-value">{formatTokens(model.tokens)}</span>
              </div>
            ))}
          </div>
        </Section>
      ) : null}
      {glance && glance.calls === 0 ? <p className="tp-note">{t("只统计经过 Codex++ 的请求（网关或协议转换）。直连时请求不经过 Codex++，这里不会有记录。")}</p> : null}

      {quotas.length ? (
        <Section title={t("订阅额度")}>
          <div className="tp-card tp-list">
            {quotas.map((row) => (
              <div className="tp-quota" key={row.providerId}>
                <div className="tp-quota-name">
                  <Avatar name={row.name} size="sm" />
                  <strong>{maskAccountName(row.name)}</strong>
                  {row.plan && !row.name.includes(row.plan) ? <em className="tp-kind">{row.plan}</em> : null}
                </div>
                {row.windows.map(({ window, remaining }, index) => {
                  const label = quotaWindowLabel(window);
                  return (
                    <div className="tp-bar-row" key={index}>
                      <span className="tp-bar-label">{label.args.length ? tf(label.text, label.args) : t(label.text)}</span>
                      <span className="tp-bar-track" data-tone={remaining < 0.15 ? "low" : remaining < 0.4 ? "mid" : "ok"}><i style={{ width: `${Math.round(remaining * 100)}%` }} /></span>
                      <span className="tp-bar-value">{tf("剩 {0}%", [Math.round(remaining * 100)])}</span>
                    </div>
                  );
                })}
              </div>
            ))}
          </div>
        </Section>
      ) : null}

      {hasPool ? (
        <Section
          aside={(
            <button className="tp-link" disabled={balancesLoading} onClick={onLoadBalances} type="button">
              {balancesLoading ? t("查询中…") : balances ? t("重新查询") : t("查询余额")}
            </button>
          )}
          title={t("余额")}
        >
          {balances === null ? (
            <p className="tp-note">{t("会联网查询网关供应商池里支持余额接口的供应商。")}</p>
          ) : balances.length === 0 ? (
            <p className="tp-note">{t("没有可以查询余额的供应商。")}</p>
          ) : (
            <div className="tp-card tp-list">
              {balances.map((balance) => (
                <div className="tp-balance" key={balance.providerId}>
                  <Avatar baseUrl={balance.baseUrl} name={balance.provider} size="sm" />
                  <strong>{maskAccountName(balance.provider)}</strong>
                  <span data-tone={balance.error ? "error" : undefined}>
                    {balance.error ? t("查询失败") : balance.amount != null ? formatMoney(balance.amount, balance.currency) : "—"}
                  </span>
                </div>
              ))}
            </div>
          )}
        </Section>
      ) : null}
    </>
  );
}

/* —————————————— 网关 —————————————— */

function GatewayTab({
  payload,
  logs,
  onPayload,
  onOpenRoute,
}: {
  payload: GatewayPayload | null;
  logs: LogEntry[];
  onPayload: (payload: GatewayPayload) => void;
  onOpenRoute: (route: TrayRoute) => Promise<void>;
}) {
  const [message, setMessage] = useState("");
  const run = async (command: string, args: Record<string, unknown>) => {
    try {
      const result = await invoke<GatewayResult>(command, args);
      if (result.gateway && Array.isArray(result.pool)) onPayload(result as GatewayPayload);
      setMessage(result.status === "ok" ? "" : result.message);
    } catch (error) {
      setMessage(String(error));
    }
  };

  if (gatewayIntroNeeded(payload)) {
    return (
      <div className="tp-card tp-intro">
        <Waypoints aria-hidden="true" className="tp-intro-icon" />
        <strong>{t("还没用上网关")}</strong>
        <p>{t("网关把 Codex、Claude Code、Grok 接到同一个供应商池：一处管理 Key 和模型，在这里就能随时给每个 Agent 换模型。")}</p>
        <div className="tp-intro-actions">
          <button className="tp-button" data-variant="primary" onClick={() => void onOpenRoute("gatewayProviders")} type="button">{t("添加供应商")}</button>
          <button className="tp-button" onClick={() => void onOpenRoute("gateway")} type="button">{t("了解网关")}</button>
        </div>
      </div>
    );
  }

  const now = Date.now();
  const recent = logs.slice(0, 8);
  return (
    <>
      <div className="tp-gateway-status">
        <i className="tp-dot" data-tone={payload?.helperRunning ? "on" : "off"} />
        <span>{payload?.helperRunning ? tf("网关运行中 · 127.0.0.1:{0}", [payload.port]) : t("网关没有运行")}</span>
        <LinkButton label={t("管理")} onClick={() => void onOpenRoute("gateway")} />
      </div>

      <Section title={t("各 Agent 用的模型")}>
        <div className="tp-card tp-list">
          {(payload?.agents ?? []).map((agent) => (
            <PanelAgentRow agent={agent} key={agent.agent} onRun={run} payload={payload} />
          ))}
        </div>
        {message ? <p className="tp-error">{message}</p> : null}
      </Section>

      <Section aside={<LinkButton label={t("全部")} onClick={() => void onOpenRoute("requestLog")} />} title={t("最近请求")}>
        <div className="tp-card tp-list">
          {recent.map((entry) => {
            const running = entry.state === "running";
            const ok = !running && entry.success !== false && (entry.statusCode ?? 200) < 400;
            const when = relativeTime(entry.timestampMs, now);
            const agent = entry.agent || "codex";
            const tokens = entry.usage ? `${formatTokens(entry.usage.inputTokens)} → ${formatTokens(entry.usage.outputTokens)}` : "";
            return (
              <div className="tp-log" key={entry.id}>
                <img alt="" src={AGENT_ICONS[agent === "codex" ? "codexApp" : agent] ?? AGENT_ICONS.codexApp} title={agentName(agent)} />
                <div className="tp-log-main">
                  <strong>{entry.upstreamModel || entry.model || "—"}</strong>
                  <small>{[maskAccountName(entry.relayName ?? ""), tokens, entry.durationMs ? `${(entry.durationMs / 1000).toFixed(1)}s` : ""].filter(Boolean).join(" · ")}</small>
                </div>
                <span className="tp-log-side">
                  <i className="tp-dot" data-tone={running ? "busy" : ok ? "on" : "error"} />
                  {running ? t("进行中") : when.args.length ? tf(when.text, when.args) : t(when.text)}
                </span>
              </div>
            );
          })}
          {recent.length === 0 ? <div className="tp-empty">{t("还没有请求")}</div> : null}
        </div>
      </Section>
    </>
  );
}

/** 一个 Agent：名字 + 当前模型下拉；选模型即接入，选「默认」即断开。 */
function PanelAgentRow({
  agent,
  payload,
  onRun,
}: {
  agent: GatewayAgentView;
  payload: GatewayPayload | null;
  onRun: (command: string, args: Record<string, unknown>) => Promise<void>;
}) {
  const [open, setOpen] = useState(false);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const binding = payload?.gateway.bindings[agent.agent];
  const draft = draftFromBinding(binding);
  const display = modelDisplay(payload, draft.model, draft.poolExclude);
  const head = (
    <button
      className="gw-mp-action"
      data-active={!agent.enabled}
      onClick={() => {
        setOpen(false);
        if (agent.enabled) void onRun("disable_gateway_binding", { agent: agent.agent });
      }}
      title={tf("{0} 自带的默认值", [agent.name])}
      type="button"
    >
      <AgentIcon agent={agent.agent} className="gw-popover-agent" />
      {t("用默认（不经 Codex++）")}
      {!agent.enabled ? <Check className="h-3.5 w-3.5" /> : null}
    </button>
  );
  return (
    <div className="tp-agent">
      <AgentIcon agent={agent.agent} className="gw-agent-icon" />
      <strong>{agent.name}</strong>
      <button aria-expanded={open} className="gw-picker-button tp-agent-picker" onClick={() => setOpen(!open)} ref={buttonRef} type="button">
        {!agent.enabled ? <AgentIcon agent={agent.agent} className="gw-picker-agent" /> : <ModelViewGlyph view={display} />}
        <span>{agent.enabled ? display.title : t("默认")}</span>
        {agent.enabled ? <AutoTag view={display} /> : null}
        <ChevronDown className="h-3.5 w-3.5" />
      </button>
      <Floating anchorRef={buttonRef} className="gw-model-popover" onClose={() => setOpen(false)} open={open} width={340}>
        <ModelList
          exclude={draft.poolExclude}
          head={head}
          onPick={(item) => {
            setOpen(false);
            void onRun("save_gateway_binding", { agent: agent.agent, binding: bindingFromDraft({ ...draft, model: item.model }, binding) });
          }}
          payload={payload}
          selectedModel={agent.enabled ? draft.model || agentModels(payload, draft.poolExclude)[0] || "" : ""}
        />
      </Floating>
    </div>
  );
}
