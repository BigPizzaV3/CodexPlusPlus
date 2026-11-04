import { flowProviderSlot } from "./flow-provider-colors";
import "./flow-provider-colors.css";
/** 网关总览：今日数字一排；任意 Agent → Codex++ → 任意供应商的三列拓扑；最近请求列表。 */
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowUpRight, ChevronDown, Plus } from "lucide-react";
import { t, tf } from "./i18n";
import {
  agentName,
  cacheHitRate,
  formatMoney,
  formatTokens,
  pendingImportCount,
  routeNarrative,
  type GatewayPayload,
  type RouteRecordView,
  type UsageSummary,
} from "./gateway-model";
import { hubAgents, hubLinks, hubPoolState, hubProviders, hubTotals, routeRecordsByEntry, successRate, type CodexSummary, type HubLink } from "./hub-overview-model";
import { AGENT_ICONS, APP_LOGO_URL, Avatar, KeyCount, MemberStatusLabel, agentIconIsColor, makeRun, type Notify, type Run } from "./gateway-ui";
import { publishGatewayPayload, useGatewayPayload } from "./gateway-store";
import { AgentConnectDialog } from "./GatewayAgents";
import { busyLinks, liveFlowRoutes, flowRoutePosition, flowAgent, pulseHops, type FlowEntry, type LiveFlowRoute } from "./request-flow-model";
import { FlowRequestRow, useRequestFlow } from "./RequestFlow";
import { agentVisible, singleAgent } from "./agent-focus-model";
import { useAgentFocus } from "./agent-focus-store";
import { ProviderAdder } from "./GatewayProviders";
import "./HubOverview.css";
import { CxCard, CxEmpty, CxSection, CxStats } from "./cx-layout";

const LIVE_REFRESH_MS = 3000;
/** 拓扑图光点和「最近请求」跟着真实请求走，读得勤一些（只读日志末尾，开销很小）。 */
const FLOW_REFRESH_MS = 1500;
/** 「最近请求」列出的条数；更多的去请求日志看。 */
const FEED_LIMIT = 10;
const USAGE_REFRESH_MS = 30_000;

/** 侧栏状态卡、总览、网关页共用的网关数据在 gateway-store 里；这里转出 hook，保持原来的引用路径。 */
export { useGatewayPayload };

/** 今日用量（总览统计、首页网关一行共用），定时刷新。 */
export function useTodayUsage(): UsageSummary | null {
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
  if (url && agentIconIsColor(agent)) return <img alt="" className={`hub-agent-glyph hub-agent-glyph-plain ${className}`} src={url} />;
  // 没有打包图标的通用 Agent：名字首字母，不画空的遮罩方块。
  if (!url) return <i aria-hidden="true" className={`hub-agent-glyph hub-agent-glyph-plain gw-agent-letter ${className}`}>{(agentName(agent).trim()[0] ?? "?").toUpperCase()}</i>;
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
  // 没勾选的 Agent 不在总览里出现；单 Agent 模式下网关就是这一个 Agent 专用的供应商池。
  const focus = useAgentFocus();
  const solo = singleAgent(focus);
  const agents = hubAgents(payload, codex).filter((agent) => agentVisible(focus, agent.id));
  const providers = hubProviders(payload, callsByProvider);
  // 连线的「活跃」改由真实请求决定：有请求在进行的连线流动，每条请求一颗来回的光点。
  const links = hubLinks(agents, providers).map((link) => (link.tone === "live" ? { ...link, tone: "idle" as const } : link));
  const totals = hubTotals(payload);
  const requestFlow = useRequestFlow(FLOW_REFRESH_MS);
  const flowEntries = requestFlow.entries.filter((entry) => agentVisible(focus, flowAgent(entry)));
  const flow = useMemo(
    () => ({ busy: busyLinks(flowEntries), routes: liveFlowRoutes(flowEntries) }),
    // 连线随进行中的请求、路由和响应阶段变化。
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [flowEntries.filter((entry) => entry.state === "running").map((entry) => `${entry.id}:${entry.relayId}:${entry.phase}:${entry.firstTokenMs}:${entry.estimatedOutputTokens}`).join("|")],
  );
  const feed = flowEntries.slice(0, FEED_LIMIT);
  const routeRecords = routeRecordsByEntry(payload, feed);
  const connected = agents.filter((agent) => agent.enabled).length;
  // 「接入 Agent」「添加供应商」就地弹窗，不跳到对应页面。
  const [adding, setAdding] = useState<"agent" | "provider" | "import" | null>(null);
  const pendingImports = pendingImportCount(payload);
  const [busy, setBusy] = useState<string | null>(null);
  const run: Run = makeRun(setBusy, publishGatewayPayload, notify);
  const poolState = hubPoolState(payload);

  // 参照 Cursor 的 Customize：顶部一排今日数字，中间一张拓扑卡（Codex++ 的卖点：任意 Agent → 网关 → 多家供应商），
  // 下面「最近请求」一个列表。现状说明不再单独写一段，拓扑图自己就说明了。
  return (
    <div className="hub-page cx-page">
      <CxSection
        aside={<button className="hub-link" onClick={() => onNavigate("gatewayUsage")} type="button">{t("用量明细")}<ArrowUpRight className="h-3.5 w-3.5" /></button>}
        title={t("今日")}
      >
        <UsageStats showAgents={!solo} usage={usage} />
      </CxSection>

      <CxCard className="hub-topology-card">
        <HubTopology className="hub-topology-compact" flow={flow} links={links}>
          <div className="hub-col">
            <div className="hub-col-head"><span>{t("Agent")}</span><span>{t("请求")}</span></div>
            {agents.map((agent) => (
              <AgentNode agent={agent} key={agent.id} onOpen={() => onOpenAgent(agent.id)} />
            ))}
            {solo ? null : (
              <button className="hub-node hub-node-ghost" onClick={() => setAdding("agent")} type="button">
                <Plus aria-hidden="true" className="h-4 w-4" />
                {t("接入 Agent")}
              </button>
            )}
          </div>

          <div className="hub-core" data-node="hub" data-running={payload?.helperRunning ?? false}>
            <div className="hub-core-head">
              <img alt="" src={APP_LOGO_URL} />
              <span>
                <strong>Codex++</strong>
                <span className="hub-addr">
                  <i className="hub-dot" data-tone={payload?.helperRunning ? "ok" : "off"} />
                  {payload ? `127.0.0.1:${payload.port}` : t("读取中…")}
                </span>
              </span>
            </div>
            {/* 能力说明压成一行小字；完整说明放在悬停提示里，不在页面上再写一段。 */}
            <p
              className="hub-caps"
              title={!payload
                ? t("正在读取网关状态…")
                : solo
                  ? tf("{0} 通过 Codex++ 使用 {1} 个供应商。请求按模型名路由，Key 冷却或供应商熔断时自动换上游。", [agentName(solo), providers.length])
                  : tf("{0} 个 Agent 通过 Codex++ 共用 {1} 个供应商。请求按模型名路由，Key 冷却或供应商熔断时自动换上游。", [connected, providers.length])}
            >
              {[
                t("按模型名路由"),
                t("协议互转"),
                payload?.gateway.keyPolicy.mode === "roundRobin" ? t("逐请求轮换 Key") : t("会话粘性 Key"),
                ...(payload?.groups.length ? [tf("{0} 个模型组", [payload.groups.length])] : []),
              ].join(" · ")}
            </p>
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
              <button className="hub-node" data-node={`provider:${provider.id}`} data-flow-provider={flowProviderSlot(provider.id)} key={provider.id} onClick={() => onNavigate("gatewayProviders")} title={provider.name} type="button">
                <i className="flow-provider-mark" aria-hidden="true" />
                <Avatar baseUrl={provider.baseUrl} name={provider.name} size="sm" />
                <span className="hub-node-text">
                  <strong>
                    <span className="hub-node-name">{provider.name}</span>
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
      </CxCard>

      <CxSection
        aside={<button className="hub-link" onClick={() => onNavigate("requestLog")} type="button">{t("请求日志")}<ArrowUpRight className="h-3.5 w-3.5" /></button>}
        title={t("最近请求")}
      >
        <CxCard>
          {!requestFlow.loaded ? (
            <CxEmpty title={t("读取中…")} />
          ) : feed.length === 0 ? (
            <CxEmpty title={t("还没有请求经过网关")} detail={t("在已接入的 Agent 里发一条消息试试。")} />
          ) : (
            <ul className="hub-flow-list">{feed.map((entry) => <FeedRow entry={entry} key={entry.id} payload={payload} record={routeRecords.get(entry.id) ?? null} />)}</ul>
          )}
        </CxCard>
      </CxSection>

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

/** 左列一个 Agent：名字和当前模型同排，一行一个，省出纵向空间多放几个。 */
function AgentNode({ agent, onOpen }: { agent: ReturnType<typeof hubAgents>[number]; onOpen: () => void }) {
  const detail = agent.direct
    ? (agent.detail ? tf("直连 · {0}", [agent.detail]) : t("直连模式"))
    : agent.enabled ? agent.detail || t("已接入") : t("未接入网关");
  return (
    <button className="hub-node" data-node={`agent:${agent.id}`} data-off={!agent.enabled && !agent.direct} onClick={onOpen} title={`${agent.name} · ${detail}`} type="button">
      <span className="hub-node-icon"><AgentGlyph agent={agent.id} /></span>
      <span className="hub-node-text">
        <strong>
          <span className="hub-node-name">{agent.name}</span>
          <i className="hub-dot" data-tone={agent.direct ? "direct" : agent.enabled ? "ok" : "off"} />
        </strong>
        <small>{detail}</small>
      </span>
      {agent.enabled ? <span className="hub-node-num">{agent.requests}</span> : null}
    </button>
  );
}

/** 一条最近请求；对得上网关路由记录的那条能点开，列出这次怎么走的：计划、每次尝试、最后谁回答。 */
function FeedRow({ entry, record, payload }: { entry: FlowEntry; record: RouteRecordView | null; payload: GatewayPayload | null }) {
  const [open, setOpen] = useState(false);
  const agent = flowAgent(entry);
  return (
    <li data-open={open}>
      <button aria-expanded={open} className="hub-flow-toggle" disabled={!record} onClick={() => setOpen(!open)} type="button">
        <FlowRequestRow entry={entry} />
        <ChevronDown aria-hidden className="hub-feed-chevron h-3.5 w-3.5" />
      </button>
      {open && record ? (
        <ol className="hub-feed-trace">
          {routeNarrative(agent, record, payload).map((line, index) => (
            <li data-tone={line.tone} key={index}>{tf(line.text, line.args.map((arg) => (typeof arg === "object" ? t(arg.zh) : arg)))}</li>
          ))}
        </ol>
      ) : null}
    </li>
  );
}

/** 今日数字一排（CxStats）：调用、成功率、Tokens、估算费用。读取中或还没请求时照样占位，版面不跳。 */
function UsageStats({ usage, showAgents }: { usage: UsageSummary | null; showAgents: boolean }) {
  const totals = usage?.totals;
  const rate = totals ? successRate(totals) : null;
  const cache = totals ? cacheHitRate(totals) : null;
  const cost = (usage?.costs ?? []).filter((item) => item.amount > 0).map((item) => formatMoney(item.amount, item.currency)).join(" + ");
  const empty = !usage ? t("读取中…") : totals?.calls === 0 ? t("今天还没有经过网关的请求。") : null;
  return (
    <CxStats
      items={[
        { label: t("调用"), value: totals ? totals.calls : "—", hint: empty ?? (showAgents && usage ? <AgentSplit usage={usage} /> : null) },
        { label: t("成功率"), value: rate === null ? "—" : `${(rate * 100).toFixed(1)}%`, hint: totals?.errors ? tf("{0} 次出错", [totals.errors]) : null },
        { label: "Tokens", value: totals?.calls ? formatTokens(totals.inputTokens + totals.outputTokens) : "—", hint: cache === null ? null : tf("缓存命中 {0}%", [Math.round(cache * 100)]) },
        { label: t("估算费用"), value: cost || "—", hint: cost ? null : t("在供应商里填上价格后估算") },
      ]}
    />
  );
}

/** 今日调用按 Agent 拆分，一行文字（「Codex 56% · Claude Code 44%」）；单 Agent 模式下没有意义，不画。 */
function AgentSplit({ usage }: { usage: UsageSummary }) {
  const focus = useAgentFocus();
  const groups = usage.byAgent.filter((group) => agentVisible(focus, group.key));
  const agentTotal = groups.reduce((sum, group) => sum + group.calls, 0) || 1;
  if (!groups.length) return null;
  return (
    <span aria-label={t("按 Agent 拆分")} className="hub-split-text">
      {groups.map((group) => `${agentName(group.key)} ${Math.round((group.calls / agentTotal) * 100)}%`).join(" · ")}
    </span>
  );
}

type Path = { d: string; tone: HubLink["tone"]; weight: number; key: string; x1: number; y1: number; x2: number; y2: number };

/** 真实请求驱动的连线动画：当前进行中的路线驱动连续光流，完成结果只在请求行展示。 */
export type TopologyFlow = { busy: Set<string>; routes: LiveFlowRoute[] };

/**
 * 三列布局 + 一层 SVG 连线。连线端点按真实节点位置量出来，窗口或内容变化时重画。Computer Use 页也用它。
 * 传了 `flow` 时不再画循环的装饰光点，改由真实请求驱动；`ports` 在连线两端画小圆点。
 * `vertical` 时节点上下排（托盘用）：连线从上面节点的底边连到下面节点的顶边。
 */
export function HubTopology({
  links,
  children,
  flow,
  ports = false,
  vertical = false,
  className = "",
}: {
  links: HubLink[];
  children: React.ReactNode;
  flow?: TopologyFlow;
  ports?: boolean;
  vertical?: boolean;
  className?: string;
}) {
  const root = useRef<HTMLDivElement>(null);
  const pathRefs = useRef(new Map<string, SVGPathElement>());
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
        const x1 = vertical ? a.left + a.width / 2 - box.left : a.right - box.left;
        const y1 = vertical ? a.bottom - box.top : a.top + a.height / 2 - box.top;
        const x2 = vertical ? b.left + b.width / 2 - box.left : b.left - box.left;
        const y2 = vertical ? b.top - box.top : b.top + b.height / 2 - box.top;
        const d = vertical
          ? `M${x1},${y1} C${x1},${(y1 + y2) / 2} ${x2},${(y1 + y2) / 2} ${x2},${y2}`
          : `M${x1},${y1} C${(x1 + x2) / 2},${y1} ${(x1 + x2) / 2},${y2} ${x2},${y2}`;
        next.push({ d, tone: link.tone, weight: link.weight, key: `${link.from}>${link.to}`, x1, y1, x2, y2 });
      }
      setPaths(next);
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
    // 连线只随签名变化重算。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [signature, vertical]);

  return (
    <div className={`hub-topology ${className}`.trim()} data-vertical={vertical || undefined} ref={root}>
      <svg aria-hidden="true" className="hub-links">
        {paths.map((path, index) => {
          const busy = flow?.busy.has(path.key) ?? false;
          return (
            <g key={path.key}>
              <path
                d={path.d}
                data-busy={busy || undefined}
                data-tone={path.tone}
                id={`hub-link-${index}`}
                ref={(element) => {
                  if (element) pathRefs.current.set(path.key, element);
                  else pathRefs.current.delete(path.key);
                }}
                style={{ strokeWidth: 1.25 + path.weight * 3 }}
              />
              {ports
                ? [[path.x1, path.y1], [path.x2, path.y2]].map(([cx, cy], end) => (
                  <circle className="hub-port" cx={cx} cy={cy} data-busy={busy || undefined} data-tone={path.tone} key={end} r={3.5} />
                ))
                : null}
              {!flow && path.tone === "live"
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
          );
        })}
        {flow ? <FlowStreamLayer pathRefs={pathRefs} routes={flow.routes} paths={paths} /> : null}
      </svg>
      {children}
    </div>
  );
}

/** 整条路线共用时钟和像素速度；经过 hub 的间隙也计入路程，光流在节点下穿过。 */
function FlowStreamLayer({ routes, paths, pathRefs }: {
  routes: LiveFlowRoute[];
  paths: Path[];
  pathRefs: React.MutableRefObject<Map<string, SVGPathElement>>;
}) {
  const root = useRef<SVGGElement>(null);
  const epoch = useRef(performance.now());
  const signature = routes.map((route) => route.key).join("|");
  const visible = routes.filter((route) => pulseHops({ ...route, kind: "out" }).every((hop) => paths.some((path) => path.key === hop.key)));

  useEffect(() => {
    const element = root.current;
    if (!element) return;
    const motion = window.matchMedia("(prefers-reduced-motion: reduce)");
    let frame = 0;
    const tracks = visible.map((route) => {
      const hops = pulseHops({ ...route, kind: route.reverse ? "back" : "out" });
      const segments: Array<{ length: number; point: (distance: number) => { x: number; y: number } }> = [];
      for (const hop of hops) {
        const path = pathRefs.current.get(hop.key)!;
        const length = path.getTotalLength();
        const point = (distance: number) => path.getPointAtLength(hop.reverse ? length - distance : distance);
        if (segments.length) {
          const previous = segments[segments.length - 1];
          const a = previous.point(previous.length);
          const b = point(0);
          const gap = Math.hypot(b.x - a.x, b.y - a.y);
          segments.push({ length: gap, point: (distance) => {
            const t = gap ? distance / gap : 0;
            return { x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t };
          } });
        }
        segments.push({ length, point });
      }
      const lengths = segments.map((segment) => segment.length);
      const total = lengths.reduce((a, b) => a + b, 0);
      const group = [...element.children].find((child) => child.getAttribute("data-route") === route.key);
      return { segments, lengths, total, packets: group ? [...group.children] : [] };
    });
    const draw = (now: number) => {
      for (const track of tracks) {
        const sample = (distance: number) => {
          const position = flowRoutePosition(track.lengths, distance);
          return position ? track.segments[position.segment].point(position.progress * track.lengths[position.segment]) : null;
        };
        track.packets.forEach((packet, index) => {
          const cycle = track.total + 32;
          const head = (((now - epoch.current) * 0.075 + index * cycle / 2) % cycle);
          [...packet.children].forEach((mark, tail) => {
            const point = sample(head - tail * 2.5);
            mark.setAttribute("visibility", point ? "visible" : "hidden");
            if (point) mark.setAttribute("transform", `translate(${point.x} ${point.y})`);
          });
        });
      }
      if (!motion.matches) frame = requestAnimationFrame(draw);
    };
    const restart = () => {
      cancelAnimationFrame(frame);
      if (!motion.matches && tracks.length) frame = requestAnimationFrame(draw);
    };
    restart();
    motion.addEventListener("change", restart);
    return () => { cancelAnimationFrame(frame); motion.removeEventListener("change", restart); };
    // 路线按稳定签名更新，轮询相同数据时不重启动画。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [signature, paths, pathRefs]);

  return <g className="hub-streams" ref={root}>
    {visible.map((route) => <g data-flow-provider={flowProviderSlot(route.relayId)} data-route={route.key} data-direction={route.reverse ? "back" : "out"} key={route.key}>
      {[0, 1].map((packet) => <g key={packet}>
        {Array.from({ length: 10 }, (_, tail) => <circle className="hub-stream-dot" key={tail}
          r={tail === 0 ? 3.5 : 3 - tail * 0.22} opacity={1 - tail / 11} visibility="hidden" />)}
      </g>)}
    </g>)}
  </g>;
}
