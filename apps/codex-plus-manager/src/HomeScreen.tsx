// 首页。
import {
  ArrowRight,
  Blocks,
  Cable,
  ChevronRight,
  ExternalLink,
  Play,
  Network,
  RefreshCw,
  RotateCcw,
  Settings,
  Sparkles,
  Waypoints,
  Plug,
  BarChart3,
  MessagesSquare,
  type LucideIcon,
} from "lucide-react";
import { AgentGlyph, useGatewayPayload, useTodayUsage } from "./hub-shared";
import { TopbarActions } from "./TopbarActions";
import { ClaudeAutoTakeoverRow } from "./ClaudeAutoTakeover";
import { providersHidden, type AgentFocus } from "./agent-focus-model";
import { t, tf } from "@/i18n";
import { CxCard, CxRow, CxSection, CxStatusIcon } from "./cx-layout";
import type { Actions, AdItem, AdsResult, AutoTakeoverResult, ToolEntry } from "./app-types";
import { autoTakeoverView, enhancementPlatform, StatusBadge, ToggleVisual } from "./app-shared";
import { type Route, routeVisible, type ToolId } from "./routes";

/// 置顶赞助位：只读广告源的 `top_ad/topAds`（`top_ad` 可以是单个对象或数组），过期的不显示。
function featuredAds(ads: AdsResult | null): AdItem[] {
  const sourceAds = ads?.topAds?.length ? ads.topAds : ads?.topAd ? [ads.topAd] : [];
  return sourceAds.filter((ad) => !isExpiredAd(ad));
}

/// 首页：整个程序的落脚页。上面是各 Agent 与网关的状态，中间是精选推荐和其他赞助商，最下面是常用入口。
///
/// 推荐：置顶位全部做大卡片，排成一行均分宽度；推荐池里的赞助商全部列出，两列小卡片，紧跟在精选下面。
/// 置顶位和推荐池里同一家常有两个 id、同一个落地页，按 adIdentity 去重。只展示赞助商，普通推荐不显示。
export function HomeScreen({
  ads,
  actions,
  tools,
  focus,
  codexRunning,
  launchPending,
  onLaunchCodex,
  onNavigate,
  onOpenAgent,
  autoTakeover,
  autoTakeoverOn,
  watcherEnabled,
}: {
  ads: AdsResult | null;
  actions: Actions;
  tools: ToolEntry[];
  focus: AgentFocus;
  codexRunning: boolean;
  launchPending: boolean;
  onLaunchCodex: () => void;
  onNavigate: (route: Route) => void;
  onOpenAgent: (agent: ToolId, destination?: Route) => void;
  autoTakeover: AutoTakeoverResult | null;
  /** 设置还没读到时为 null，开关先禁用。 */
  autoTakeoverOn: boolean | null;
  watcherEnabled: boolean | null;
}) {
  const gateway = useGatewayPayload(10_000);
  const usage = useTodayUsage();
  const enhanceOnly = providersHidden(focus);
  const connected = new Set((gateway?.agents ?? []).filter((agent) => agent.enabled).map((agent) => agent.agent as string));
  // 和侧栏一样：没有 Agent 接入网关时不显示网关那一行，也不催其他 Agent 接入。
  const gatewayInUse = connected.size > 0;

  const topAds = featuredAds(ads);
  const topIdentities = new Set(topAds.map(adIdentity));
  const pool = (ads?.ads ?? []).filter((ad) => !isExpiredAd(ad) && !topIdentities.has(adIdentity(ad)));
  // 大横幅只放置顶位（top_ad / topAds），没有置顶位就不显示；推荐池里的赞助商只进下面的列表。
  const heroes = topAds;
  const rest = pool.filter((ad) => ad.type === "sponsor");
  const open = (ad: AdItem) => void actions.openExternalUrl(ad.url);

  // agent：Codex 自己的页面，先切到 Codex 再打开。
  const shortcuts: Array<{ route: Route; label: string; detail: string; icon: LucideIcon; agent?: ToolId }> = [
    { route: "userScripts", label: t("万物皆拓展"), detail: t("给 Codex、Claude 加功能的脚本"), icon: Blocks },
    { route: "context", label: t("MCP 与 Skills"), detail: t("一处管理所有 Agent 的 MCP 和 Skill"), icon: Network },
    { route: "sessions", label: t("会话"), detail: t("找回、继续各 Agent 的本地会话"), icon: MessagesSquare },
    { route: "relay", label: t("连接与供应商"), detail: t("切换 Codex 的供应商与连接方式"), icon: Cable, agent: "codex" },
    { route: "gatewayUsage", label: t("用量"), detail: t("调用次数、Token 与费用"), icon: BarChart3 },
    { route: "settings", label: t("设置"), detail: t("主题、更新、日志与诊断"), icon: Settings },
  ];

  // 自动接管紧挨着 Codex 那一行：打开后自己直接开 Codex 也有增强，不必再从这里点启动。
  // 关掉 Codex++ 就没人监听，所以没开登录自启时在这一行提醒并给开关。
  const takeover = autoTakeoverView(autoTakeover);
  const takeoverNeedsLogin = autoTakeoverOn === true && watcherEnabled === false;
  const takeoverRow = (
    <CxRow
      badges={autoTakeoverOn ? <StatusBadge status={takeover.status} label={takeover.label} /> : null}
      detail={autoTakeoverOn
        ? [takeover.detail, takeoverNeedsLogin ? t("Codex++ 没有设为登录时打开，退出 Codex++ 后不会接管。") : ""].filter(Boolean).join(" ")
        : enhancementPlatform === "windows"
          ? t("打开后，直接打开 Codex（开始菜单、任务栏）也会自动接管并注入增强，不用再从 Codex++ 启动。")
          : t("打开后，直接打开 Codex（Dock、启动台）也会自动接管并注入增强，不用再从 Codex++ 启动。")}
      icon={<CxStatusIcon tone={autoTakeoverOn ? (takeover.status === "ok" ? "ok" : takeover.status === "failed" ? "danger" : "warn") : "off"}><Plug /></CxStatusIcon>}
      key="codex-auto-takeover"
      title={t("自动接管官方 App")}
      tooltip={t("官方 App 已在运行且没有调试端口时，会先退出再带参数重开（可能中断正在进行的操作）。")}
    >
      {takeoverNeedsLogin ? (
        <button className="cx-secondary" onClick={() => void actions.setWatcherEnabled(true)} type="button">{t("登录时打开 Codex++")}</button>
      ) : null}
      <label className="switch-row compact relay-master-switch relay-master-switch-compact">
        <input
          aria-label={t("开启自动接管")}
          checked={autoTakeoverOn === true}
          disabled={autoTakeoverOn === null}
          onChange={(event) => void actions.setAutoTakeover(event.currentTarget.checked)}
          type="checkbox"
        />
        <span>{t("自动接管")}</span>
        <ToggleVisual />
      </label>
    </CxRow>
  );

  const agentRow = (tool: ToolEntry) => {
    const viaGateway = connected.has(tool.id);
    if (tool.id === "codex") {
      const mode = viaGateway ? t("经共享供应商") : tool.activeRelayName ? tf("直连 · {0}", [tool.activeRelayName]) : t("直连");
      return [
        <CxRow
          detail={`${codexRunning ? t("运行中") : t("未运行")} · ${mode}`}
          icon={<CxStatusIcon tone={codexRunning ? "ok" : "off"}><AgentGlyph agent="codex" className="home-agent-glyph" /></CxStatusIcon>}
          key={tool.id}
          onClick={() => onOpenAgent(tool.id)}
          title={tool.name}
        >
          <button className="cx-secondary" disabled={launchPending} onClick={(event) => { event.stopPropagation(); onLaunchCodex(); }} type="button">
            {codexRunning ? <RotateCcw /> : <Play />}
            {codexRunning ? t("重启") : t("启动")}
          </button>
        </CxRow>,
        takeoverRow,
      ];
    }
    const row = (
      <CxRow
        detail={viaGateway ? t("已经通过 Codex++ 本地服务接入") : gatewayInUse ? t("未接入本地服务") : t("直连")}
        icon={<CxStatusIcon tone={viaGateway ? "ok" : "off"}><AgentGlyph agent={tool.id} className="home-agent-glyph" /></CxStatusIcon>}
        key={tool.id}
        onClick={() => onOpenAgent(tool.id)}
        title={tool.name}
      >
        {viaGateway || enhanceOnly || !gatewayInUse ? <ChevronRight aria-hidden="true" className="cx-set-chevron" /> : (
          <button className="cx-secondary" onClick={(event) => { event.stopPropagation(); onNavigate("gateway"); }} type="button">{t("接入")}</button>
        )}
      </CxRow>
    );
    // Claude 那一行下面跟它的自动接管（只在完整功能模式下动作），和 Codex 行一样。
    return tool.id === "claude"
      ? [row, <ClaudeAutoTakeoverRow key="claude-auto-takeover" loginReminder onEnableLogin={() => actions.setWatcherEnabled(true)} showIcon />]
      : row;
  };

  const gatewayDetail = gateway?.helperRunning
    ? [
        tf("运行中 · 端口 {0}", [gateway.port]),
        tf("{0} 个供应商", [gateway.pool.length]),
        tf("{0} 个 Agent 已接入", [connected.size]),
        usage ? tf("今日 {0} 次调用", [usage.totals.calls]) : "",
      ].filter(Boolean).join(" · ")
    : t("本地服务未运行");

  return (
    <div className="tp-page home-page">
      <TopbarActions>
        <button className="cx-secondary" onClick={() => void actions.refreshAds()} type="button">
          <RefreshCw />
          {t("刷新推荐")}
        </button>
      </TopbarActions>

      <CxSection title={t("状态")}>
        <CxCard>
          {tools.map(agentRow)}
          {enhanceOnly || !gatewayInUse ? null : (
            <CxRow
              detail={gatewayDetail}
              icon={<CxStatusIcon tone={gateway?.helperRunning ? "ok" : "off"}><Waypoints /></CxStatusIcon>}
              onClick={() => onNavigate("gateway")}
              title={t("本地服务")}
            >
              <ChevronRight aria-hidden="true" className="cx-set-chevron" />
            </CxRow>
          )}
        </CxCard>
      </CxSection>

      {heroes.length ? (
        <CxSection aside={<span>{t("赞助商推荐：支持 Codex++ 的服务")}</span>} title={t("精选推荐")}>
          <div className="home-heroes" data-dense={heroes.length >= 3 || undefined} style={{ gridTemplateColumns: `repeat(${heroes.length}, minmax(0, 1fr))` }}>
            {heroes.map((ad) => (
              <div
                className="home-hero"
                key={adIdentity(ad)}
                onClick={() => open(ad)}
                onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); open(ad); } }}
                role="link"
                tabIndex={0}
                title={`${ad.description}\n${ad.url}`}
              >
                {ad.image ? <img alt="" className="home-hero-image" src={ad.image} /> : <span className="home-hero-image home-hero-placeholder"><Sparkles aria-hidden="true" /></span>}
                <div className="home-hero-body">
                  <div className="home-hero-head">
                    <strong>{formatAdTitle(ad.title)}</strong>
                  </div>
                  <p className="home-hero-text">{ad.description}</p>
                  {ad.highlights?.length ? (
                    <div className="home-hero-chips">
                      {ad.highlights.filter(Boolean).slice(0, 4).map((item) => <span key={item}>{item}</span>)}
                    </div>
                  ) : null}
                  <div className="home-hero-foot">
                    <span className="home-hero-host">{adHost(ad.url)}</span>
                    <span className="home-hero-cta">{t("了解更多")}<ArrowRight aria-hidden="true" /></span>
                  </div>
                </div>
              </div>
            ))}
          </div>
        </CxSection>
      ) : null}

      {rest.length ? (
        <CxSection count={rest.length} title={t("更多赞助商")}>
          <div className="home-sponsors">
            {rest.map((ad) => (
              <div
                className="home-sponsor"
                key={adIdentity(ad)}
                onClick={() => open(ad)}
                onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); open(ad); } }}
                role="link"
                tabIndex={0}
                title={`${ad.description}\n${ad.url}`}
              >
                {ad.image ? <img alt="" className="home-sponsor-thumb" loading="lazy" src={ad.image} /> : <span className="home-sponsor-thumb home-hero-placeholder"><Sparkles aria-hidden="true" /></span>}
                <div className="home-sponsor-body">
                  <div className="home-sponsor-head">
                    <strong>{formatAdTitle(ad.title)}</strong>
                    {(ad.highlights ?? []).filter(Boolean).slice(0, 2).map((item) => <span className="home-sponsor-chip" key={item}>{item}</span>)}
                  </div>
                  <p className="home-sponsor-text">{ad.description}</p>
                </div>
                <ExternalLink aria-hidden="true" className="home-sponsor-go" />
              </div>
            ))}
          </div>
        </CxSection>
      ) : null}

      <CxSection title={t("常用")}>
        <div className="home-shortcuts">
          {shortcuts.filter((item) => routeVisible(focus, item.route)).map((item) => {
            const Icon = item.icon;
            return (
              <button className="home-shortcut" key={item.route} onClick={() => (item.agent ? onOpenAgent(item.agent, item.route) : onNavigate(item.route))} type="button">
                <Icon aria-hidden="true" />
                <span>
                  <strong>{item.label}</strong>
                  <small>{item.detail}</small>
                </span>
              </button>
            );
          })}
        </div>
      </CxSection>

    </div>
  );
}

/// 卡片右上角显示落地页域名，点之前就知道会打开哪里。
function adHost(url: string): string {
  try {
    return new URL(url).host.replace(/^www\./, "");
  } catch {
    return "";
  }
}

/// 广告标题原样展示。
///
/// 以前这里会在 `｜` / `|` 处截断，把「火山引擎｜方舟 Agent Plan」显示成
/// 「火山引擎」—— 后半段是作者写的产品名，不该被我们悄悄丢掉。既然卡片
/// 已经改成按内容自适应高度，就不再需要在标题上省这一点空间。
function formatAdTitle(title: string) {
  return title.trim() || title;
}

/// 广告的「同一家」判据。
///
/// 只比 id 是不够的：置顶位和推荐池里的同一条赞助商往往有两个 id
/// （`jojocode-top` vs `jojocode-codex-relay`），但指向同一个去处。
/// 所以按落地 URL（去 query / 尾斜杠）比，退回标题。
function adIdentity(ad: AdItem): string {
  const url = (ad.url || "").trim();
  if (url) {
    try {
      const parsed = new URL(url);
      return `${parsed.host}${parsed.pathname}`.replace(/\/+$/, "").toLowerCase();
    } catch {
      return url.replace(/[?#].*$/, "").replace(/\/+$/, "").toLowerCase();
    }
  }
  return (ad.title || "").trim().toLowerCase();
}

function isExpiredAd(ad: AdItem) {
  if (!ad.expires_at) return false;
  const expiresAt = Date.parse(ad.expires_at);
  return Number.isFinite(expiresAt) && expiresAt < Date.now();
}
