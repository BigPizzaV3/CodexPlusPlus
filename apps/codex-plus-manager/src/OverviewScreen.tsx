// Agent 概览页。
import { CheckCircle2, ChevronDown, Plus, RefreshCw, RotateCcw, Tag } from "lucide-react";
import { useState } from "react";
import { summarizeRecentRequests, type RecentRequestSummary } from "./overview-model";
import { AppSelect } from "./AppSelect";
import { useGatewayPayload } from "./HubOverview";
import { groupDisplayName } from "./gateway-model";
import { AgentDashboard, type DashboardAgent, type DashboardRequestLogs, type DashboardStat, type DashboardStatus } from "./AgentDashboard";
import { t, tf } from "@/i18n";
import { syncLegacyRelayFields } from "./relay-profile-editing";
import * as AgentCx from "./cx-layout"; // cx-pages:agents：Agent 页面在函数内解构，避免和别的分组的具名导入重名
import type { Actions, BackendSettings, OverviewResult, RequestLogEntry, ToolEntry } from "./app-types";
import {
  CODEX_GATEWAY_RELAY_ID,
  codexGatewayModelCount,
  healthItems,
  normalizeSettings,
  relayProtocolLabel,
  requestLogAgent,
  statusClass,
  statusLabel,
} from "./app-shared";
import type { Route, ToolId } from "./routes";

export function OverviewScreen({
  overview,
  activeTool,
  toolEntries,
  form,
  onNavigate,
  onOpenList,
  actions,
}: {
  overview: OverviewResult | null;
  activeTool: ToolId;
  toolEntries: ToolEntry[];
  form: BackendSettings;
  onNavigate: (route: Route) => void;
  /** 打开会话页 / 请求日志并选中这个 Agent。 */
  onOpenList: (kind: "sessions" | "requestLog", agent: DashboardAgent) => void;
  actions: Actions;
}) {
  const gatewayPayload = useGatewayPayload(10_000);
  const health = healthItems(overview);
  const tool = toolEntries.find((entry) => entry.id === activeTool);
  const normalized = normalizeSettings(form);
  const activeProfile = normalized.relayProfiles.find((profile) => profile.id === normalized.activeRelayId);
  // 最近请求只在 Codex 概览里用，数据由仪表板读请求日志时一并交过来；失败时静默显示空状态，不打断概览。
  const [recent, setRecent] = useState<RecentRequestSummary | null>(null);
  const summarizeCodexRequests = (result: DashboardRequestLogs | null) => {
    // 网关记录里也有 Claude Code / Grok 的请求，Codex 概览只统计 Codex 自己的。
    const entries = result?.status === "ok" ? ((result.entries ?? []) as RequestLogEntry[]) : [];
    setRecent(summarizeRecentRequests(entries.filter((entry) => requestLogAgent(entry) === "codex"), Date.now()));
  };
  // 「刷新状态」连仪表板的数字、最近会话和请求一起重读。
  const [dashboardRefresh, setDashboardRefresh] = useState(0);
  const switchFromOverview = (profileId: string) => {
    if (!profileId || profileId === normalized.activeRelayId) return;
    void actions.switchRelayProfile(syncLegacyRelayFields({ ...normalized, activeRelayId: profileId }), normalized.activeRelayId);
  };
  const gatewayManaged = activeProfile?.id === CODEX_GATEWAY_RELAY_ID;
  // Codex 的状态栏：经网关 / 直连哪家供应商，操作是网关管理或切换供应商。
  const codexStatus: DashboardStatus = gatewayManaged
    ? { tone: "gateway", text: codexGatewayModelCount(activeProfile) ? tf("经 Codex++ 网关 · {0} 个模型", [codexGatewayModelCount(activeProfile)]) : t("经 Codex++ 网关") }
    : activeProfile
      ? { tone: "relay", text: tf("直连 · {0}", [[activeProfile.name || t("未命名供应商"), relayProtocolLabel(activeProfile.protocol), activeProfile.model].filter(Boolean).join(" · ")]) }
      : { tone: "missing", text: t("还没有配置供应商。") };
  const codexActions = gatewayManaged ? (
    <>
      <button className="cx-secondary" onClick={() => onNavigate("gateway")} type="button">{t("管理接入")}</button>
    </>
  ) : activeProfile ? (
    <>
      <AppSelect
        ariaLabel={t("切换供应商")}
        className="overview-current-select agent-dash-select"
        disabled={actions.relaySwitching || !normalized.relayProfilesEnabled || normalized.relayProfiles.length < 2}
        onChange={(next) => switchFromOverview(next)}
        options={normalized.relayProfiles.map((profile) => ({ value: profile.id, label: profile.name || t("未命名供应商") }))}
        size="sm"
        title={!normalized.relayProfilesEnabled ? t("供应商配置切换已关闭，在供应商配置页打开后才能切换") : t("切换供应商")}
        value={normalized.activeRelayId}
      />
      <button className="cx-secondary" onClick={() => onNavigate("relay")} type="button">{t("管理供应商")}</button>
    </>
  ) : (
    <button className="cx-primary" onClick={() => onNavigate("relay")} type="button"><Plus aria-hidden="true" />{t("添加供应商")}</button>
  );
  const codexStats: DashboardStat[] = [
    { label: t("24 小时请求"), value: recent ? recent.total : "—" },
    { label: t("成功率"), value: recent?.successRate == null ? "—" : `${Math.round(recent.successRate * 100)}%`, tone: recent?.failed ? "warn" : "ok" },
    { label: t("平均耗时"), value: recent?.avgDurationMs == null ? "—" : `${(recent.avgDurationMs / 1000).toFixed(1)}s` },
    { label: t("常用模型"), value: groupDisplayName(gatewayPayload, recent?.topModel ?? "") ?? recent?.topModel ?? "—", mono: false },
  ];
  const { CxBadge, CxCard, CxRow, CxSection, CxStatusIcon } = AgentCx;
  // 健康检查：一张卡一行一项。有问题的项排前面、带橙色 / 红色标签；正常的收成一行「其余 N 项正常」，点开再看。
  const healthRows = [
    {
      title: t("Codex 版本"),
      status: overview?.codex_version ? "ok" : "not_checked",
      ok: Boolean(overview?.codex_version),
      detail: overview?.codex_version ?? t("未检测到 Codex 应用版本。"),
      icon: Tag,
    },
    ...health,
  ];
  const healthProblems = healthRows.filter((item) => !item.ok);
  const healthFine = healthRows.filter((item) => item.ok);
  const [healthExpanded, setHealthExpanded] = useState(false);
  const healthRow = (item: (typeof healthRows)[number]) => (
    <CxRow
      detail={<span title={item.detail}>{item.detail}</span>}
      icon={<CxStatusIcon tone={item.ok ? (statusClass(item.status) === "idle" ? "off" : "ok") : statusClass(item.status) === "bad" ? "danger" : "warn"}><item.icon /></CxStatusIcon>}
      key={item.title}
      muted={item.ok && healthProblems.length > 0}
      title={item.title}
    >
      <CxBadge tone={item.ok ? (statusClass(item.status) === "good" ? "ok" : "neutral") : statusClass(item.status) === "bad" ? "danger" : "warn"}>{statusLabel(item.status)}</CxBadge>
      {"action" in item && item.action === "restart" ? (
        <button className="cx-secondary" onClick={() => void actions.restart()} type="button">
          <RotateCcw aria-hidden="true" />{t("重启并挂载")}
        </button>
      ) : null}
    </CxRow>
  );
  return (
    <>
      {activeTool === "codex" ? (
        // 三个 Agent 的概览同一套仪表板；Codex 另在下面放健康检查。
        <AgentDashboard
          actions={codexActions}
          agent="codex"
          name="Codex"
          notify={actions.showMessage}
          onNavigate={onNavigate}
          onOpenList={onOpenList}
          onRequestLogs={summarizeCodexRequests}
          refreshKey={dashboardRefresh}
          stats={codexStats}
          status={codexStatus}
          version={overview?.codex_version}
        >
          <CxSection
            aside={
              <span className="overview-health-header-actions">
                {healthProblems.length ? <span title={t("概览只展示关键问题，具体配置在对应页面处理")}>{tf("{0} 项需处理", [healthProblems.length])}</span> : null}
                <button aria-label={t("刷新状态")} className="cx-icon-button" onClick={() => { setDashboardRefresh((value) => value + 1); void actions.refreshCurrent(); }} title={t("刷新状态")} type="button">
                  <RefreshCw aria-hidden="true" />
                </button>
              </span>
            }
            className="overview-health"
            title={t("健康检查")}
          >
            <CxCard className="overview-health-list">
              {healthProblems.map(healthRow)}
              {healthExpanded ? healthFine.map(healthRow) : (
                <CxRow
                  className="overview-health-more"
                  detail={healthFine.map((item) => item.title).join(" · ")}
                  icon={<CxStatusIcon tone="ok"><CheckCircle2 /></CxStatusIcon>}
                  onClick={() => setHealthExpanded(true)}
                  title={healthProblems.length ? tf("其余 {0} 项正常", [healthFine.length]) : tf("{0} 项检查全部正常", [healthFine.length])}
                >
                  <ChevronDown aria-hidden="true" className="cx-row-chevron" />
                </CxRow>
              )}
            </CxCard>
          </CxSection>
        </AgentDashboard>
      ) : activeTool === "claude" || activeTool === "grok" ? (
        <AgentDashboard agent={activeTool} name={tool?.name ?? activeTool} notify={actions.showMessage} onNavigate={onNavigate} onOpenList={onOpenList} />
      ) : null}
    </>
  );
}
