/** 网关总览：任意 Agent → Codex++ → 任意供应商。左列 Agent、中间网关、右列供应商池，下方是路由动态与今日用量。 */
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowUpRight, Plus } from "lucide-react";
import { t, tf } from "./i18n";
import {
  agentName,
  cacheHitRate,
  formatElapsed,
  formatMoney,
  formatTokens,
  type GatewayPayload,
  type GatewayResult,
  type UsageSummary,
} from "./gateway-model";
import { hubAgents, hubFeed, hubLinks, hubProviders, hubTotals, successRate, type CodexSummary, type HubFeedItem, type HubLink } from "./hub-overview-model";
import { AGENT_ICONS, APP_LOGO_URL, Avatar } from "./gateway-ui";
import "./HubOverview.css";

const LIVE_REFRESH_MS = 3000;
const USAGE_REFRESH_MS = 30_000;

/** 侧栏状态卡和总览页共用的网关数据：各自按自己的间隔静默刷新，失败时保留上一次的值。 */
const shared: { payload: GatewayPayload | null } = { payload: null };

export function useGatewayPayload(intervalMs: number): GatewayPayload | null {
  const [payload, setPayload] = useState<GatewayPayload | null>(shared.payload);
  useEffect(() => {
    let cancelled = false;
    const load = () =>
      invoke<GatewayResult>("load_gateway")
        .then((result) => {
          if (cancelled || !result.gateway || !Array.isArray(result.pool)) return;
          shared.payload = result as GatewayPayload;
          setPayload(shared.payload);
        })
        .catch(() => undefined);
    void load();
    const timer = window.setInterval(load, intervalMs);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [intervalMs]);
  return payload;
}

function useTodayUsage(): UsageSummary | null {
  const [summary, setSummary] = useState<UsageSummary | null>(null);
  useEffect(() => {
    let cancelled = false;
    const load = () =>
      invoke<{ status: string } & Partial<UsageSummary>>("load_gateway_usage", { range: "today" })
        .then((result) => {
          if (!cancelled && result.status === "ok" && result.totals) setSummary(result as UsageSummary);
        })
        .catch(() => undefined);
    void load();
    const timer = window.setInterval(load, USAGE_REFRESH_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);
  return summary;
}

/** 单色品牌图标用 mask 着色，深浅主题下都跟随文字颜色。 */
export function AgentGlyph({ agent, className = "" }: { agent: string; className?: string }) {
  const url = AGENT_ICONS[agent];
  return <i aria-hidden="true" className={`hub-agent-glyph ${className}`} style={url ? ({ "--hub-icon": `url("${url}")` } as CSSProperties) : undefined} />;
}

export function HubOverview({
  codex,
  onOpenAgent,
  onNavigate,
}: {
  codex: CodexSummary;
  onOpenAgent: (agent: string) => void;
  /** 跳到网关的某个页签或其它页面。 */
  onNavigate: (target: "gateway" | "gatewayProviders" | "gatewayRouting" | "gatewayUsage") => void;
}) {
  const payload = useGatewayPayload(LIVE_REFRESH_MS);
  const usage = useTodayUsage();
  const callsByProvider = useMemo(
    () => (usage ? new Map(usage.byProvider.map((group) => [group.key, group.calls])) : null),
    [usage],
  );
  const agents = hubAgents(payload, codex);
  const providers = hubProviders(payload, callsByProvider);
  const links = hubLinks(agents, providers);
  const totals = hubTotals(payload);
  const feed = hubFeed(payload);
  const connected = agents.filter((agent) => agent.enabled).length;

  return (
    <div className="hub-page">
      {/* 页面标题已在外壳顶栏，这里只留一句现状说明。 */}
      <header className="hub-intro">
        <p>
          {tf("{0} 个 Agent 通过 Codex++ 共用 {1} 个供应商。请求按模型名路由，Key 冷却或供应商熔断时自动换上游。", [connected, providers.length])}
        </p>
      </header>

      <section className="hub-card">
        <HubTopology links={links}>
          <div className="hub-col">
            <div className="hub-col-head"><span>{t("Agent")}</span><span>{t("请求")}</span></div>
            {agents.map((agent) => (
              <button className="hub-node" data-node={`agent:${agent.id}`} data-off={!agent.enabled && !agent.direct} key={agent.id} onClick={() => onOpenAgent(agent.id)} type="button">
                <span className="hub-node-icon"><AgentGlyph agent={agent.id} /></span>
                <span className="hub-node-text">
                  <strong>
                    {agent.name}
                    <i className="hub-dot" data-tone={agent.direct ? "direct" : agent.enabled ? "ok" : "off"} />
                  </strong>
                  <small>
                    {agent.direct
                      ? (agent.detail ? tf("直连 · {0}", [agent.detail]) : t("直连模式"))
                      : agent.enabled ? agent.detail || t("已接入") : t("未接入网关")}
                  </small>
                </span>
                {agent.enabled ? <span className="hub-node-num">{agent.requests}</span> : null}
              </button>
            ))}
            <button className="hub-node hub-node-ghost" onClick={() => onNavigate("gateway")} type="button">
              <Plus aria-hidden="true" className="h-4 w-4" />
              {t("接入 Agent")}
            </button>
          </div>

          <div className="hub-core" data-node="hub" data-running={payload?.helperRunning ?? false}>
            <img alt="" src={APP_LOGO_URL} />
            <strong>Codex++</strong>
            <span className="hub-addr">
              <i className="hub-dot" data-tone={payload?.helperRunning ? "ok" : "off"} />
              {payload ? `127.0.0.1:${payload.port}` : t("读取中…")}
            </span>
            <div className="hub-caps">
              <span>{t("按模型名路由")}</span>
              <span>{t("协议互转")}</span>
              <span>{payload?.gateway.keyPolicy.mode === "roundRobin" ? t("逐请求轮换 Key") : t("会话粘性 Key")}</span>
              {payload?.groups.length ? <span>{tf("{0} 个路由组", [payload.groups.length])}</span> : null}
            </div>
            <dl>
              <div><dt>{t("请求")}</dt><dd>{totals.requests}</dd></div>
              <div><dt>{t("换上游")}</dt><dd data-tone={totals.rerouted ? "warn" : undefined}>{totals.rerouted}</dd></div>
              <div><dt>{t("冷却 Key")}</dt><dd data-tone={totals.coolingKeys ? "warn" : undefined}>{totals.coolingKeys}</dd></div>
              <div><dt>{t("熔断")}</dt><dd data-tone={totals.openBreakers ? "err" : undefined}>{totals.openBreakers}</dd></div>
            </dl>
          </div>

          <div className="hub-col">
            <div className="hub-col-head"><span>{t("供应商池")}</span><span>{t("今日调用")}</span></div>
            {providers.length === 0 ? <p className="hub-empty">{t("供应商池还是空的。")}</p> : null}
            {providers.map((provider) => (
              <button className="hub-node" data-node={`provider:${provider.id}`} key={provider.id} onClick={() => onNavigate("gatewayProviders")} type="button">
                <Avatar baseUrl={provider.baseUrl} name={provider.name} size="sm" />
                <span className="hub-node-text">
                  <strong>
                    {provider.name}
                    {provider.status.state === "cooling" ? <i className="hub-dot" data-tone="warn" /> : null}
                  </strong>
                  <small data-state={provider.status.state}>
                    {provider.status.state === "idle"
                      ? tf("{0} 个 Key", [provider.keyCount])
                      : provider.status.args.length ? tf(provider.status.text, provider.status.args) : t(provider.status.text)}
                  </small>
                </span>
                {provider.calls !== null ? <span className="hub-node-num">{provider.calls}</span> : null}
              </button>
            ))}
            <button className="hub-node hub-node-ghost" onClick={() => onNavigate("gatewayProviders")} type="button">
              <Plus aria-hidden="true" className="h-4 w-4" />
              {t("添加供应商")}
            </button>
          </div>
        </HubTopology>
      </section>

      <div className="hub-grid">
        <section className="hub-card">
          <div className="hub-card-head">
            <h3>{t("路由动态")}</h3>
            <button className="hub-link" onClick={() => onNavigate("gatewayRouting")} type="button">{t("路由")}<ArrowUpRight className="h-3.5 w-3.5" /></button>
          </div>
          {feed.length === 0 ? (
            <p className="hub-empty">{t("还没有请求经过网关。在已接入的 Agent 里发一条消息试试。")}</p>
          ) : (
            <ul className="hub-feed">{feed.map((item) => <FeedRow item={item} key={`${item.agent}-${item.finishedAtMs}`} />)}</ul>
          )}
        </section>
        <section className="hub-card">
          <div className="hub-card-head">
            <h3>{t("今日用量")}</h3>
            <button className="hub-link" onClick={() => onNavigate("gatewayUsage")} type="button">{t("明细")}<ArrowUpRight className="h-3.5 w-3.5" /></button>
          </div>
          <UsageStats usage={usage} />
        </section>
      </div>
    </div>
  );
}

function FeedRow({ item }: { item: HubFeedItem }) {
  const label: Record<HubFeedItem["result"], string> = {
    answered: t("成功"),
    rerouted: t("换上游"),
    failed: t("失败"),
    cooling: t("冷却"),
    busy: t("进行中"),
  };
  return (
    <li>
      <span className="hub-feed-agent"><AgentGlyph agent={item.agent} />{agentName(item.agent)}</span>
      <span className="hub-feed-route">
        {item.requested}
        <i>→</i>
        <b>{item.provider ? `${item.provider} / ${item.model}` : item.model}</b>
      </span>
      <span className="hub-feed-ms">{formatElapsed(item.elapsedMs)}</span>
      <span className="hub-badge" data-result={item.result}>{label[item.result]}</span>
    </li>
  );
}

function UsageStats({ usage }: { usage: UsageSummary | null }) {
  if (!usage) return <p className="hub-empty">{t("读取中…")}</p>;
  const totals = usage.totals;
  if (totals.calls === 0) return <p className="hub-empty">{t("今天还没有经过网关的请求。")}</p>;
  const rate = successRate(totals);
  const cache = cacheHitRate(totals);
  const cost = usage.costs.filter((item) => item.amount > 0).map((item) => formatMoney(item.amount, item.currency)).join(" + ");
  const agentTotal = usage.byAgent.reduce((sum, group) => sum + group.calls, 0) || 1;
  return (
    <>
      <dl className="hub-stats">
        <div><dt>{t("调用")}</dt><dd>{totals.calls}</dd></div>
        <div><dt>{t("成功率")}</dt><dd data-tone="ok">{rate === null ? "—" : `${(rate * 100).toFixed(1)}%`}</dd></div>
        <div><dt>Tokens</dt><dd>{formatTokens(totals.inputTokens + totals.outputTokens)}</dd></div>
        <div><dt>{t("缓存命中")}</dt><dd>{cache === null ? "—" : `${Math.round(cache * 100)}%`}</dd></div>
        {cost ? <div className="hub-stats-wide"><dt>{t("估算费用")}</dt><dd>{cost}</dd></div> : null}
      </dl>
      <div className="hub-split" aria-label={t("按 Agent 拆分")}>
        {usage.byAgent.map((group) => <i data-agent={group.key} key={group.key} style={{ flex: group.calls }} title={`${agentName(group.key)} · ${group.calls}`} />)}
      </div>
      <div className="hub-legend">
        {usage.byAgent.map((group) => (
          <span key={group.key}><i data-agent={group.key} />{agentName(group.key)} {Math.round((group.calls / agentTotal) * 100)}%</span>
        ))}
      </div>
    </>
  );
}

type Path = { d: string; tone: HubLink["tone"]; weight: number; key: string };

/** 三列布局 + 一层 SVG 连线。连线端点按真实节点位置量出来，窗口或内容变化时重画。Computer Use 页也用它。 */
export function HubTopology({ links, children }: { links: HubLink[]; children: React.ReactNode }) {
  const root = useRef<HTMLDivElement>(null);
  const [paths, setPaths] = useState<Path[]>([]);
  const signature = links.map((link) => `${link.from}>${link.to}:${link.tone}:${link.weight.toFixed(2)}`).join("|");

  useLayoutEffect(() => {
    const element = root.current;
    if (!element) return;
    const measure = () => {
      const box = element.getBoundingClientRect();
      const next: Path[] = [];
      for (const link of links) {
        const from = element.querySelector(`[data-node="${link.from}"]`);
        const to = element.querySelector(`[data-node="${link.to}"]`);
        if (!from || !to) continue;
        const a = from.getBoundingClientRect();
        const b = to.getBoundingClientRect();
        const x1 = a.right - box.left;
        const y1 = a.top + a.height / 2 - box.top;
        const x2 = b.left - box.left;
        const y2 = b.top + b.height / 2 - box.top;
        const dx = (x2 - x1) / 2;
        next.push({ d: `M${x1},${y1} C${x1 + dx},${y1} ${x2 - dx},${y2} ${x2},${y2}`, tone: link.tone, weight: link.weight, key: `${link.from}>${link.to}` });
      }
      setPaths(next);
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
    // 连线只随签名变化重算。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [signature]);

  return (
    <div className="hub-topology" ref={root}>
      <svg aria-hidden="true" className="hub-links">
        {paths.map((path, index) => (
          <g key={path.key}>
            <path d={path.d} data-tone={path.tone} id={`hub-link-${index}`} style={{ strokeWidth: 1.25 + path.weight * 3 }} />
            {path.tone === "live"
              ? [0, 1].map((k) => (
                // 第二个点延迟出发；延迟期间它会停在 SVG 原点（拓扑图左上角），先隐藏，动画开始时再显示。
                <circle data-tone="live" key={k} r={2.2 + path.weight * 1.4} visibility="hidden">
                  <set attributeName="visibility" begin={`${k * 1.1}s`} to="visible" />
                  <animateMotion begin={`${k * 1.1}s`} dur="2.2s" repeatCount="indefinite">
                    <mpath href={`#hub-link-${index}`} />
                  </animateMotion>
                </circle>
              ))
              : null}
          </g>
        ))}
      </svg>
      {children}
    </div>
  );
}
