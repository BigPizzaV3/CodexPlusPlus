import { useState } from "react";
import { Download, Layers3, Plus } from "lucide-react";
import { CxBadge, CxCard, CxEmpty, CxFilters, CxLoading, CxRow, CxSection } from "./cx-layout";
import { AgentModelPicker } from "./GatewayAgents";
import { AgentGlyph } from "./hub-shared";
import { Menu, MenuItem } from "./ManagementPages";
import { TopbarActions } from "./TopbarActions";
import { ProviderAdder } from "./GatewayProviders";
import { publishGatewayPayload, useGatewayPayload } from "./gateway-store";
import { Avatar, Switch, makeRun, type Notify, type Run } from "./gateway-ui";
import {
  bindingFromDraft,
  draftFromBinding,
  groupModelId,
  pendingImports,
  presetHost,
  protocolLabel,
  providerEnabled,
  validateDraft,
  type GatewayAgentId,
  type GroupMode,
} from "./gateway-model";
import { t, tf } from "./i18n";
import { entriesForPage } from "./provider-owner";
import { OwnerBadge, ShowOtherAgentsToggle, useShowOtherAgents } from "./ProviderOwner";

/** Claude Desktop 在网关里的绑定：3p 模式直接用它，完整功能模式也按它的模型和「挑选」列模型。 */
const AGENT: GatewayAgentId = "claude-desktop";
/** Claude Code（命令行）在网关里的绑定。 */
const CODE: GatewayAgentId = "claude";
/** 从 CC Switch、ClaudeMom、Magpie 导进来的也算 Claude 的来源，导入按钮上提示还有几个没导。 */
const CLAUDE_IMPORT_SOURCES = new Set(["claude", "ccswitch", "claudemom", "magpie"]);

/**
 * 供应商列表的分页，和 Codex「连接与供应商」那排筛选标签同一种用法：按 Claude 发过去要不要转协议分。
 * Anthropic 协议的直接发，OpenAI 协议（Responses / Chat Completions）的由本地服务转换。
 * 「模型组」对应 Codex 的「模型聚合 / 故障转移」：本地服务「路由」里建的组，Claude 一样能用，选成默认模型就按组调度。
 */
type ClaudeProviderView = "native" | "converted" | "groups";
/** 分页标签上的悬停说明。 */
function viewTip(view: ClaudeProviderView): string {
  if (view === "native") return t("Anthropic 协议的供应商：Claude 的请求直接发过去，不做转换。");
  if (view === "converted") return t("OpenAI 协议（Responses / Chat Completions）的供应商：本地服务把 Claude 的请求转成它们的协议。");
  return t("本地服务「路由」里建的模型组：几家的模型合成一个，依次、轮换、按权重调度，出错换下一家。设成默认模型就按组走。");
}
/** 模型组的调度方式，和「路由」页编辑组时的叫法一样。 */
function groupModeLabel(mode: GroupMode): string {
  switch (mode) {
    case "rotate": return t("轮换");
    case "weighted": return t("按权重");
    case "usage": return t("额度优先");
    case "fast": return t("速度优先");
    case "manual": return t("手动");
    default: return t("依次");
  }
}
const viewOf = (protocol: string): ClaudeProviderView => (protocol === "anthropicMessages" ? "native" : "converted");

/**
 * Claude 的「供应商」页：Claude Desktop、Claude Code 各用哪个默认模型，Claude Desktop 用哪些供应商，加供应商、从 CC Switch / ClaudeMom 导入。
 * 供应商存在网关的共享供应商里（和 Codex 等共用一份，网关负责把 Anthropic 请求转成各家的协议），这里只是 Claude 视角的入口：
 * 开关一次管 Claude Desktop 和 Claude Code 两份「挑选」（排除名单），默认模型就是 Agent 接入页那一行的模型下拉。
 * 「模型组」分页列本地服务「路由」里的组，可以直接设成两个 Claude 的默认模型。
 *
 * 版式和 Codex 的「连接与供应商」同一副骨架，两页对照着看得懂：
 * 「导入」「添加供应商」在标题行右侧（添加是主按钮）→ 顶部一张卡说这个 Agent 怎么接的（Codex 是登录，这里是两个默认模型）
 * → 「供应商 N」分区里只列供应商，N 是列出来的数目。
 *
 * Codex 的直连供应商默认以引用的方式在共享供应商里（标「来自 Codex」，地址、Key、模型在 Codex 那边改）；
 * 不想看到它们，取消勾选「显示其他 Agent 的供应商」（按页记住）。
 */
export function ClaudeProviders({ notify, onManage, onManageGroups }: { notify: Notify; onManage?: () => void; onManageGroups?: () => void }) {
  const payload = useGatewayPayload(10_000);
  const [busy, setBusy] = useState<string | null>(null);
  const run: Run = makeRun(setBusy, publishGatewayPayload, notify);
  const [adding, setAdding] = useState<"catalog" | "import" | null>(null);
  // 没选过时落在有供应商的那一页（先「无需转换」）。
  const [chosenView, setChosenView] = useState<ClaudeProviderView | null>(null);
  const binding = payload?.gateway.bindings[AGENT];
  const codeBinding = payload?.gateway.bindings[CODE];
  const exclude = binding?.poolExclude ?? [];
  const codeExclude = codeBinding?.poolExclude ?? [];
  const allPool = payload?.pool ?? [];
  // 「显示其他 Agent 的供应商」：不勾时不列 Codex 的直连供应商（开关不变，只是不列）。
  const [showOtherAgents, setShowOtherAgents] = useShowOtherAgents("claude");
  const pool = entriesForPage(allPool, "claude", showOtherAgents);
  // 能给 Claude 用的组：打开着、有成员的（和后端列进模型列表的条件一致，成员全被排除的照样列出，选了也会报错提示）。
  const groups = (payload?.groups ?? []).filter((group) => group.members.length > 0);
  const viewCount = (view: ClaudeProviderView) => view === "groups" ? groups.length : pool.filter((entry) => viewOf(entry.protocol) === view).length;
  const view: ClaudeProviderView = chosenView ?? (viewCount("native") || !viewCount("converted") ? "native" : "converted");
  const visiblePool = view === "groups" ? [] : pool.filter((entry) => viewOf(entry.protocol) === view);
  const pendingCount = pendingImports(payload).filter((item) => CLAUDE_IMPORT_SOURCES.has(item.source)).length;
  // Claude Code（命令行）：和 Agent 接入页那一行同一个模型下拉，选一个共享供应商的模型就是让它经本地服务，
  // 下拉顶部「断开」就还原成它自己的 settings.json。本地服务里没有这个 Agent 时不显示这一行。
  const claudeCode = payload?.agents.find((item) => item.agent === "claude");
  const claudeCodeDetail = !claudeCode
    ? t("正在读取…")
    : claudeCode.drift
      ? claudeCode.drift.by ? tf("配置被 {0} 改掉了，没有经过本地服务", [claudeCode.drift.by]) : t("配置被别的工具改掉了，没有经过本地服务")
      : claudeCode.enabled
        ? t("经本地服务：用下面这些共享供应商，OpenAI 协议的自动转换")
        : t("直连：用它自己的 ~/.claude/settings.json；选一个模型就改为经本地服务");

  /**
   * 开关一家供应商给不给 Claude 用：Claude Desktop 和 Claude Code 一起改（两份排除名单）。
   * 只改名单，不接入也不断开；还没接入的 Claude Code 也记下，接入后照这份名单用。
   */
  const toggle = (providerId: string, used: boolean) => {
    void run(`exclude:${providerId}`, "set_gateway_pool_member", { agents: [AGENT, CODE], id: providerId, used }, t("保存供应商开关"), true);
  };
  /** 把一个模型组设成 Claude Desktop / Claude Code 的默认模型：和默认模型下拉里选「我的组」一样，会经本地服务接入。 */
  const useGroup = async (agentId: GatewayAgentId, groupId: string) => {
    const previous = payload?.gateway.bindings[agentId];
    const connected = payload?.agents.find((item) => item.agent === agentId)?.enabled ?? false;
    const next = { ...draftFromBinding(previous), model: groupModelId(groupId) };
    const error = validateDraft(next, payload, connected ? previous?.model : undefined, null);
    if (error) {
      void notify(t("还不能接入"), t(error), "warning");
      return;
    }
    await run(`group:${agentId}:${groupId}`, "save_gateway_binding", { agent: agentId, binding: bindingFromDraft(next, previous) }, t("保存本地服务绑定"), connected);
  };
  const usesGroup = (agentId: GatewayAgentId, groupId: string) => {
    const agent = payload?.agents.find((item) => item.agent === agentId);
    return Boolean(agent?.enabled) && payload?.gateway.bindings[agentId]?.model.trim().toLowerCase() === groupModelId(groupId).toLowerCase();
  };

  return (
    <>
      {/* 和 Codex 的「连接与供应商」一样：导入、添加在标题行右侧，添加是主按钮。 */}
      <TopbarActions>
        <button className="cx-secondary" disabled={!payload || busy !== null} onClick={() => setAdding("import")} title={t("从 CC Switch、ClaudeMom、Magpie 或直连模式导入")} type="button">
          <Download />{pendingCount ? tf("导入 · {0}", [pendingCount]) : t("导入")}
        </button>
        <button className="cx-primary" disabled={!payload || busy !== null} onClick={() => setAdding("catalog")} type="button">
          <Plus />{t("添加供应商")}
        </button>
      </TopbarActions>
      {/* 顶部一张卡：两个 Claude 各用哪个默认模型（对应 Codex 页顶部的「Codex 登录」）。 */}
      <CxSection className="claude-providers-agents">
        <CxCard>
          <CxRow
            detail={t("Claude 启动后默认用这个模型；下面打开的供应商，它们的聊天模型都会列进 Claude 的模型选择器。")}
            icon={<AgentGlyph agent={AGENT} />}
            title="Claude Desktop"
          >
            <AgentModelPicker agentId={AGENT} busy={busy} notify={notify} payload={payload} run={run} />
          </CxRow>
          {payload && !claudeCode ? null : (
            <CxRow
              badges={claudeCode?.version ? <CxBadge>{claudeCode.version}</CxBadge> : null}
              detail={claudeCodeDetail}
              icon={<AgentGlyph agent="claude" />}
              title={t("Claude Code（命令行）")}
            >
              <AgentModelPicker agentId="claude" busy={busy} notify={notify} payload={payload} run={run} />
            </CxRow>
          )}
        </CxCard>
      </CxSection>
      <CxSection
        // Key、额度、路由组在设置的「本地服务 → 共享供应商」里管，「返回」回到这一页。
        aside={(
          <>
            {allPool.length ? (
              <ShowOtherAgentsToggle checked={showOtherAgents} onChange={setShowOtherAgents} title={t("Codex 的直连供应商默认共享给 Claude；不勾时这里不列它们")} />
            ) : null}
            {onManage ? <button className="cx-secondary" onClick={onManage} type="button">{t("管理共享供应商")}</button> : null}
          </>
        )}
        className="claude-providers"
        // 和 Codex 页一样：下面列了几家就是几（不列的其他 Agent 的供应商不算）。
        count={pool.length || undefined}
        title={t("供应商")}
      >
        {payload && allPool.length ? (
          <CxFilters
            items={(["native", "converted", "groups"] as ClaudeProviderView[]).map((id) => ({
              id,
              label: (
                <span title={viewTip(id)}>
                  {id === "native" ? t("无需转换") : id === "converted" ? t("需协议转换") : t("模型组")}
                </span>
              ),
              count: viewCount(id),
            }))}
            label={t("供应商协议")}
            onChange={setChosenView}
            value={view}
          />
        ) : null}
        <CxCard>
          {!payload ? (
            <CxLoading label={t("正在读取供应商…")} />
          ) : !allPool.length ? (
            <CxEmpty detail={t("DeepSeek、Kimi、智谱这些 OpenAI 协议的也能用，本地服务会自动转换。")} title={t("还没有供应商")}>
              <button className="cx-secondary" onClick={() => setAdding("catalog")} type="button"><Plus />{t("添加供应商")}</button>
              <button className="cx-secondary" onClick={() => setAdding("import")} type="button"><Download />{t("从 CC Switch / ClaudeMom / Magpie 导入")}</button>
            </CxEmpty>
          ) : view === "groups" ? (
            !groups.length ? (
              <CxEmpty detail={t("在本地服务的「路由」里把几家的模型建成一个组：依次、轮换、按权重……出错换下一家。建好后在这里设成 Claude 的默认模型。")} title={t("还没有模型组")}>
                {onManageGroups ? <button className="cx-secondary" onClick={onManageGroups} type="button"><Plus />{t("新建模型组")}</button> : null}
              </CxEmpty>
            ) : groups.map((group) => {
              const desktopUses = usesGroup(AGENT, group.id);
              const codeUses = usesGroup(CODE, group.id);
              return (
                <CxRow
                  badges={(
                    <>
                      {desktopUses ? <CxBadge tone="ok">{t("Desktop 默认")}</CxBadge> : null}
                      {codeUses ? <CxBadge tone="ok">{t("Code 默认")}</CxBadge> : null}
                      {group.disabled ? <CxBadge>{t("已停用")}</CxBadge> : null}
                    </>
                  )}
                  detail={[groupModeLabel(group.mode), tf("{0} 个成员", [group.members.length])].join(" · ")}
                  icon={<Layers3 />}
                  key={group.id}
                  muted={group.disabled}
                  title={group.name}
                >
                  <Menu label={t("模型组操作")}>
                    <MenuItem disabled={group.disabled || desktopUses || busy !== null} onClick={() => void useGroup(AGENT, group.id)}>{t("设为 Claude Desktop 的默认模型")}</MenuItem>
                    {claudeCode ? (
                      <MenuItem disabled={group.disabled || codeUses || busy !== null} onClick={() => void useGroup(CODE, group.id)}>{t("设为 Claude Code 的默认模型")}</MenuItem>
                    ) : null}
                    {onManageGroups ? <MenuItem onClick={onManageGroups}>{t("在「路由」里管理")}</MenuItem> : null}
                  </Menu>
                </CxRow>
              );
            })
          ) : !visiblePool.length ? (
            <CxEmpty
              detail={view === "native" ? t("Anthropic 协议的供应商会显示在这里；OpenAI 协议的在「需协议转换」。") : t("OpenAI 协议的供应商会显示在这里，本地服务自动转换；Anthropic 协议的在「无需转换」。")}
              title={view === "native" ? t("这个分页还没有无需转换的供应商") : t("这个分页还没有需协议转换的供应商")}
            >
              <button className="cx-secondary" onClick={() => setAdding("catalog")} type="button"><Plus />{t("添加供应商")}</button>
            </CxEmpty>
          ) : visiblePool.map((entry) => {
            const enabled = providerEnabled(entry);
            // Claude Code 不在本地服务里（没装）时只看 Desktop。两边不一致（以前在「Agent 接入」里分开挑过）时开关显示关，
            // 行上标出现在只给哪一个用；拨一下两边就一起打开。
            const desktopOn = !exclude.includes(entry.id);
            const codeOn = !claudeCode || !codeExclude.includes(entry.id);
            const on = enabled && desktopOn && codeOn;
            const partial = enabled && desktopOn !== codeOn;
            return (
              <CxRow
                badges={(
                  <>
                    {!enabled ? <CxBadge>{t("已停用")}</CxBadge> : partial ? <CxBadge>{desktopOn ? t("仅 Claude Desktop") : t("仅 Claude Code")}</CxBadge> : null}
                    <OwnerBadge entry={entry} page="claude" />
                  </>
                )}
                detail={[protocolLabel(entry.protocol), presetHost(entry.baseUrl), tf("{0} 个模型", [entry.models.length])].filter(Boolean).join(" · ")}
                icon={<Avatar baseUrl={entry.baseUrl} name={entry.name} size="sm" />}
                key={entry.id}
                muted={!on && !partial}
                title={entry.name}
              >
                <Switch
                  disabled={!enabled}
                  label={tf("给 Claude Desktop 和 Claude Code 用 {0}", [entry.name])}
                  on={on}
                  onToggle={() => toggle(entry.id, !on)}
                  pending={busy === `exclude:${entry.id}`}
                />
              </CxRow>
            );
          })}
        </CxCard>
      </CxSection>
      {adding ? (
        <ProviderAdder
          busy={busy}
          // 新加的供应商默认给两个 Claude 都用，和页上的开关一致。
          includeAgents={claudeCode ? [AGENT, CODE] : [AGENT]}
          notify={notify}
          // 从这里加的、导入的记成 Claude 的：Codex 页「显示其他 Agent 的供应商」按它筛。
          owner="claude"
          onClose={() => setAdding(null)}
          payload={payload}
          run={run}
          startWith={adding}
        />
      ) : null}
    </>
  );
}
