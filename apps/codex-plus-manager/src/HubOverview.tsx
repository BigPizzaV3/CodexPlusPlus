/** 网关总览：任意 Agent → Codex++ → 任意供应商。左列 Agent、中间网关、右列供应商池，下方是最近请求与今日用量。 */
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowUpRight, ChevronDown, Plus } from "lucide-react";
import { t, tf } from "./i18n";
import {
  agentName,
  cacheHitRate,
  formatElapsed,
  formatMoney,
  formatTokens,
  routeNarrative,
  pendingImportCount,
  type GatewayPayload,
  type UsageSummary,
} from "./gateway-model";
import { hubAgents, hubFeed, hubLinks, hubPoolState, hubProviders, hubTotals, successRate, type CodexSummary, type HubFeedItem, type HubLink } from "./hub-overview-model";
import { AGENT_ICONS, APP_LOGO_URL, Avatar, KeyCount, MemberStatusLabel, makeRun, type Notify, type Run } from "./gateway-ui";
import { publishGatewayPayload, useGatewayPayload } from "./gateway-store";
import { AgentConnectDialog } from "./GatewayAgents";
import { ProviderAdder } from "./GatewayProviders";
import "./HubOverview.css";

const LIVE_REFRESH_MS = 3000;
const USAGE_REFRESH_MS = 30_000;

/** 侧栏状态卡、总览、网关页共用的网关数据在 gateway-store 里；这里转出 hook，保持原来的引用路径。 */
export { useGatewayPayload };

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
  notify,
  onOpenAgent,
  onNavigate,
}: {
  codex: CodexSummary;
  notify: Notify;
  onOpenAgent: (agent: string) => void;
  /** 跳到网关的某个页签或其它页面。 */
  onNavigate: (target: "gateway" | "gatewayProviders" | "gatewayUsage" | "requestLog") => void;
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
  // 「接入 Agent」「添加供应商」就地弹窗，不跳到对应页面。
  const [adding, setAdding] = useState<"agent" | "provider" | "import" | null>(null);
  const pendingImports = pendingImportCount(payload);
  const [busy, setBusy] = useState<string | null>(null);
  const run: Run = makeRun(setBusy, publishGatewayPayload, notify);
  const poolState = hubPoolState(payload);

  return (
    <div className="hub-page">
      {/* 页面标题已在外壳顶栏，这里只留一句现状说明。 */}
      <header className="hub-intro">
        <p>
          {payload
            ? tf("{0} 个 Agent 通过 Codex++ 共用 {1} 个供应商。请求按模型名路由，Key 冷却或供应商熔断时自动换上游。", [connected, providers.length])
            : t("正在读取网关状态…")}
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
            <button className="hub-node hub-node-ghost" onClick={() => setAdding("agent")} type="button">
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
              {payload?.groups.length ? <span>{tf("{0} 个模型组", [payload.groups.length])}</span> : null}
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
            {poolState === "loading" ? <p className="hub-empty">{t("读取中…")}</p> : null}
            {poolState === "empty" ? <p className="hub-empty">{t("供应商池还是空的。")}</p> : null}
            {poolState === "allOff" ? <p className="hub-empty">{tf("池里的 {0} 个供应商都已关闭。", [payload?.pool.length ?? 0])}</p> : null}
            {poolState === "empty" && pendingImports ? (
              <button className="hub-node hub-node-ghost hub-node-import" onClick={() => setAdding("import")} type="button">
                {tf("导入直连模式的 {0} 个供应商", [pendingImports])}
              </button>
            ) : null}
            {providers.map((provider) => (
              <button className="hub-node" data-node={`provider:${provider.id}`} key={provider.id} onClick={() => onNavigate("gatewayProviders")} type="button">
                <Avatar baseUrl={provider.baseUrl} name={provider.name} size="sm" />
                <span className="hub-node-text">
                  <strong>
                    {provider.name}
                    {provider.status.state === "cooling" ? <i className="hub-dot" data-tone="warn" /> : null}
                  </strong>
                  <small data-state={provider.status.state}>
                    {provider.status.state === "idle" ? <KeyCount count={provider.keyCount} /> : <MemberStatusLabel status={provider.status} />}
                  </small>
                </span>
                {provider.calls !== null ? <span className="hub-node-num">{provider.calls}</span> : null}
              </button>
            ))}
            <button className="hub-node hub-node-ghost" onClick={() => setAdding("provider")} type="button">
              <Plus aria-hidden="true" className="h-4 w-4" />
              {t("添加供应商")}
            </button>
          </div>
        </HubTopology>
      </section>

      <div className="hub-grid">
        <section className="hub-card">
          <div className="hub-card-head">
            <h3>{t("最近请求")}</h3>
            <button className="hub-link" onClick={() => onNavigate("requestLog")} type="button">{t("请求日志")}<ArrowUpRight className="h-3.5 w-3.5" /></button>
          </div>
          {feed.length === 0 ? (
            <p className="hub-empty">{t("还没有请求经过网关。在已接入的 Agent 里发一条消息试试。")}</p>
          ) : (
            <ul className="hub-feed">{feed.map((item) => <FeedRow item={item} key={`${item.agent}-${item.finishedAtMs}`} payload={payload} />)}</ul>
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

      {adding === "agent" ? (
        <AgentConnectDialog
          busy={busy}
          notify={notify}
          onClose={() => setAdding(null)}
          onManage={() => { setAdding(null); onNavigate("gateway"); }}
          payload={payload}
          run={run}
        />
      ) : null}
      {adding === "provider" || adding === "import" ? (
        <ProviderAdder busy={busy} notify={notify} onClose={() => setAdding(null)} payload={payload} run={run} startWith={adding === "import" ? "import" : "catalog"} />
      ) : null}
    </div>
  );
}

/** 一条最近请求；点开列出这次怎么走的：计划、每次尝试、最后谁回答。 */
function FeedRow({ item, payload }: { item: HubFeedItem; payload: GatewayPayload | null }) {
  const [open, setOpen] = useState(false);
  const label: Record<HubFeedItem["result"], string> = {
    answered: t("成功"),
    rerouted: t("换上游"),
    failed: t("失败"),
    cooling: t("冷却"),
    busy: t("进行中"),
  };
  const record = payload?.activity?.agents.find((agent) => agent.agent === item.agent)?.last;
  return (
    <li data-open={open}>
      <button aria-expanded={open} className="hub-feed-row" disabled={!record} onClick={() => setOpen(!open)} type="button">
        <span className="hub-feed-agent"><AgentGlyph agent={item.agent} />{agentName(item.agent)}</span>
        <span className="hub-feed-route">
          {item.requested}
          <i>→</i>
          <b>{item.provider ? `${item.provider} / ${item.model}` : item.model}</b>
        </span>
        <span className="hub-feed-ms">{formatElapsed(item.elapsedMs)}</span>
        <span className="hub-badge" data-result={item.result}>{label[item.result]}</span>
        <ChevronDown aria-hidden className="hub-feed-chevron h-3.5 w-3.5" />
      </button>
      {open && record ? (
        <ol className="hub-feed-trace">
          {routeNarrative(item.agent, record, payload).map((line, index) => (
            <li data-tone={line.tone} key={index}>{tf(line.text, line.args.map((arg) => (typeof arg === "object" ? t(arg.zh) : arg)))}</li>
          ))}
        </ol>
      ) : null}
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
