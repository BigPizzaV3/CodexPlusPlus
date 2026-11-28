import { lazy, Suspense, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronRight, Download, Layers3, Plus } from "lucide-react";
import { appConfirm } from "./app-dialog";
import { CxBadge, CxCard, CxEmpty, CxLoading, CxRow, CxSection } from "./cx-layout";
import { AgentDisconnectDialog } from "./AgentDisconnectDialog";
import { ConnectionModeSwitch, type CodexConnectionMode } from "./CodexConnectionMode";
import type { PoolProviderDialogState } from "./CodexPoolEditHost";
import { AgentModelPicker } from "./GatewayAgents";
import { AgentGlyph } from "./hub-shared";
import { Menu, MenuItem, MenuSeparator } from "./ManagementPages";
import { TopbarActions } from "./TopbarActions";
import { ProviderAdder } from "./GatewayProviders";
import { publishGatewayPayload, useGatewayPayload } from "./gateway-store";
import { Avatar, Switch, makeRun, type Notify, type Run } from "./gateway-ui";
import {
  agentModels,
  bindingFromDraft,
  draftFromBinding,
  groupModelId,
  maskAccountName,
  pendingImports,
  presetHost,
  protocolLabel,
  providerEnabled,
  validateDraft,
  type GatewayAgentId,
  type GatewayPoolEntry,
  type GroupMode,
} from "./gateway-model";
import { t, tf } from "./i18n";
import { entriesForPage } from "./provider-owner";
import { OwnerBadge, useShowOtherAgents } from "./ProviderOwner";
import { credentialMenuLabel, credentialSummary, hasCredentialSettings } from "./ProviderCredentials";
import { ProviderCredentialsHost } from "./ProviderCredentialsHost";

const PoolProviderEditHost = lazy(() => import("./CodexPoolEditHost").then((module) => ({ default: module.PoolProviderEditHost })));

/** Claude Desktop 在网关里的绑定：3p 模式直接用它，完整功能模式也按它的模型和「挑选」列模型。 */
const AGENT: GatewayAgentId = "claude-desktop";
/** Claude Code（命令行）在网关里的绑定。 */
const CODE: GatewayAgentId = "claude";
/** 从 CC Switch、ClaudeMom、Magpie 导进来的也算 Claude 的来源，导入按钮上提示还有几个没导。 */
const CLAUDE_IMPORT_SOURCES = new Set(["claude", "ccswitch", "claudemom", "magpie"]);

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

/**
 * Claude Code 能不能不经 Codex++ 直连这一家：Anthropic 协议、用 Key 的普通供应商（订阅账号要代登录），
 * 地址末尾不带 `#`（Claude Code 自己补 /v1，表达不了「不补」）。后端 `claude_direct::rejection` 同一套规则。
 */
const directCapable = (entry: GatewayPoolEntry) =>
  entry.protocol === "anthropicMessages" && !entry.account && Boolean(entry.baseUrl.trim()) && !entry.baseUrl.trim().endsWith("#");

/**
 * Claude Code / Claude Desktop 的独立「供应商」页：
 * - `agent="claude"`（Claude Code）：支持顶部「聚合 / 直连」切换，开关只改 `claude` 绑定，直连单选 Anthropic 协议供应商；
 * - `agent="claude-desktop"`（Claude Desktop）：始终经本地服务，纯多选开关，只改 `claude-desktop` 绑定，无直连干扰。
 * 两边共用同一份共享供应商池，开关、模型组默认模型与新增默认启用各自独立。
 */
export function ClaudeProviders({
  agent = "claude",
  notify,
  onManageGroups,
  onEditCodexProvider,
}: {
  /** 当前页面服务的 Claude 客户端：命令行（`claude`）或桌面端（`claude-desktop`）。 */
  agent?: "claude" | "claude-desktop";
  notify: Notify;
  onManageGroups?: () => void;
  /** 引用 Codex 直连供应商的条目：切到 Codex「连接与供应商」并直接打开它的编辑页。 */
  onEditCodexProvider?: (relayId: string) => void;
}) {
  const isCode = agent === "claude";
  const agentId: GatewayAgentId = isCode ? CODE : AGENT;
  const payload = useGatewayPayload(10_000);
  const [busy, setBusy] = useState<string | null>(null);
  const run: Run = makeRun(setBusy, publishGatewayPayload, notify);
  const [adding, setAdding] = useState<"catalog" | "import" | null>(null);
  // 「账号与分配」：打开的是哪一家；打开过就一直挂着，关掉后登录提示照样在。
  const [credentialsId, setCredentialsId] = useState<string | null>(null);
  const [credentialsUsed, setCredentialsUsed] = useState(false);
  // 就地编辑 / 模型 / 窗口 / 思考等级 / 价格 / 代理 / 限速 / 测试弹窗。
  const [poolDialog, setPoolDialog] = useState<PoolProviderDialogState | null>(null);
  const [editHostUsed, setEditHostUsed] = useState(false);
  // 直连模式下「仅支持聚合模式」折叠区（仅 Claude Code 使用）。
  const [foldedOpen, setFoldedOpen] = useState(false);
  const openPoolDialog = (next: PoolProviderDialogState) => {
    setEditHostUsed(true);
    setPoolDialog(next);
  };
  const removePoolProvider = async (entry: GatewayPoolEntry) => {
    const ok = await appConfirm(tf("从本地服务删除「{0}」？直连模式里的供应商不受影响。", [maskAccountName(entry.name)]), {
      title: t("删除供应商"),
      confirmText: t("删除"),
      danger: true,
    });
    if (ok) await run(`remove:${entry.id}`, "delete_gateway_provider", { id: entry.id }, t("删除供应商"));
  };

  const binding = payload?.gateway.bindings[agentId];
  const exclude = binding?.poolExclude ?? [];
  const allPool = payload?.pool ?? [];
  // 「显示其他 Agent 的供应商」：不勾时不列 Codex 的直连供应商（开关不变，只是不列）。
  const [showOtherAgents, setShowOtherAgents] = useShowOtherAgents("claude");
  const pool = entriesForPage(allPool, "claude", showOtherAgents);
  // 能用的模型组：有成员的。
  const groups = (payload?.groups ?? []).filter((group) => group.members.length > 0);
  const pendingCount = pendingImports(payload).filter((item) => CLAUDE_IMPORT_SOURCES.has(item.source)).length;

  const claudeCode = isCode ? payload?.agents.find((item) => item.agent === "claude") : undefined;
  const codeMode: CodexConnectionMode = claudeCode?.enabled ? "aggregate" : "direct";
  // Claude Desktop 始终走聚合；Claude Code 在顶部「聚合 / 直连」先切展示视图（未安装 Claude Code 时显示聚合列表）。
  const actualMode: CodexConnectionMode = isCode && claudeCode ? codeMode : "aggregate";
  const [viewMode, setViewMode] = useState<CodexConnectionMode>(actualMode);
  useEffect(() => {
    setViewMode(actualMode);
  }, [actualMode]);
  const mode: CodexConnectionMode = isCode && claudeCode ? viewMode : "aggregate";

  const directPoolId = isCode && codeMode === "direct" ? claudeCode?.directPoolProvider ?? null : null;
  const directEntry = directPoolId ? allPool.find((entry) => entry.id === directPoolId) : undefined;
  // 直连时点「聚合」且当前没有任何可用模型时，先把模型下拉亮出来。
  const [pickingAggregate, setPickingAggregate] = useState(false);
  // 极端情况无法自动断开时兜底弹出的断开预览对话框。
  const [directTarget, setDirectTarget] = useState<"restore" | { id: string; name: string } | null>(null);

  const claudeCodeDetail = !claudeCode
    ? t("正在读取…")
    : claudeCode.drift
      ? claudeCode.drift.by ? tf("配置被 {0} 改掉了，没有经过本地服务", [claudeCode.drift.by]) : t("配置被别的工具改掉了，没有经过本地服务")
      : claudeCode.enabled && mode === "direct"
        ? t("当前仍经本地服务运行；在上方任选一家点「使用」即可切到直连")
        : claudeCode.enabled
          ? t("聚合：经本地服务用上面打开的供应商，OpenAI 协议的自动转换；需要 Codex++ 在后台运行")
          : directEntry
            ? tf("直连 {0}：不经 Codex++，关掉 Codex++ 也能用；换一家在上面点「使用」", [directEntry.name])
            : t("直连：用它自己的 ~/.claude/settings.json，不经 Codex++；上方切到「直连」后可任选一家点「使用」");

  /** Claude Code 改为聚合：沿用上次默认模型；没选过或已失效时自动挑第一个可用模型，一步切过去。 */
  const connectCodeAggregate = async () => {
    let draft = draftFromBinding(binding);
    const invalid = !draft.model.trim() || Boolean(validateDraft(draft, payload, undefined, null));
    if (invalid) {
      const fallback = agentModels(payload, draft.poolExclude)[0] ?? "";
      if (fallback) draft = { ...draft, model: fallback };
    }
    const error = draft.model.trim() ? validateDraft(draft, payload, undefined, null) : null;
    if (!draft.model.trim() || error) {
      setPickingAggregate(true);
      setViewMode("aggregate");
      void notify(t("还不能切到聚合"), error ? t(error) : t("先打开至少一家带模型的供应商，再切到聚合。"), "warning");
      return;
    }
    if (await run("code:aggregate", "save_gateway_binding", { agent: CODE, binding: bindingFromDraft(draft, binding) }, t("保存本地服务绑定"))) {
      setPickingAggregate(false);
    }
  };

  /** Claude Code 直连某一家供应商：聚合状态下自动完成安全断开并直连，极端情况才退回断开预览框。 */
  const goDirect = async (entry: GatewayPoolEntry) => {
    if (claudeCode?.requiresDisconnectPreview) {
      try {
        const previewRes = await invoke<{ status: string; preview?: { previewToken?: string; canDisconnect?: boolean } }>(
          "preview_gateway_disconnect",
          { agent: CODE },
        );
        const token = previewRes.preview?.previewToken?.trim() ?? "";
        if (previewRes.status === "ok" && previewRes.preview?.canDisconnect && token) {
          await run(`direct:${entry.id}`, "use_claude_code_pool_provider", { id: entry.id, previewToken: token }, t("Claude Code 直连"));
          return;
        }
      } catch {
        // 退回下方预览对话框
      }
      setDirectTarget({ id: entry.id, name: entry.name });
      return;
    }
    void run(`direct:${entry.id}`, "use_claude_code_pool_provider", { id: entry.id }, t("Claude Code 直连"));
  };

  /**
   * 开关一家供应商给不给当前 Agent 用：只改当前 Agent 自己的排除名单（Claude Code 和 Claude Desktop 互不影响）。
   */
  const toggle = (providerId: string, used: boolean) => {
    void run(`exclude:${providerId}`, "set_gateway_pool_member", { agents: [agentId], id: providerId, used }, t("保存供应商开关"), true);
  };

  /** 把一个模型组设成当前 Agent 的默认模型：和默认模型下拉里选「我的组」一样，会经本地服务接入。 */
  const useGroup = async (groupId: string) => {
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

  const usesGroup = (groupId: string) => {
    const entry = payload?.agents.find((item) => item.agent === agentId);
    return Boolean(entry?.enabled) && payload?.gateway.bindings[agentId]?.model.trim().toLowerCase() === groupModelId(groupId).toLowerCase();
  };

  const agentDisplayName = isCode ? "Claude Code" : "Claude Desktop";
  const renderGroupRow = (group: (typeof groups)[number]) => {
    const active = usesGroup(group.id);
    return (
      <CxRow
        badges={(
          <>
            {active ? <CxBadge tone="ok">{t("默认模型")}</CxBadge> : null}
            {group.disabled ? <CxBadge>{t("已停用")}</CxBadge> : null}
          </>
        )}
        detail={[groupModeLabel(group.mode), tf("{0} 个成员", [group.members.length])].join(" · ")}
        icon={<Layers3 />}
        key={group.id}
        muted={group.disabled}
        title={group.name}
      >
        <Switch
          label={tf("启用模型组 {0}", [group.name])}
          on={!group.disabled}
          onToggle={() => void run(`group:${group.id}`, "set_gateway_group_enabled", { id: group.id, enabled: Boolean(group.disabled) }, t("保存组"), true)}
          pending={busy === `group:${group.id}`}
        />
        <Menu label={t("模型组操作")}>
          <MenuItem disabled={group.disabled || active || busy !== null} onClick={() => void useGroup(group.id)}>
            {tf("设为 {0} 的默认模型", [agentDisplayName])}
          </MenuItem>
          {onManageGroups ? <MenuItem onClick={onManageGroups}>{t("在「路由」里管理")}</MenuItem> : null}
        </Menu>
      </CxRow>
    );
  };

  const primaryPool = mode === "direct" ? pool.filter(directCapable) : pool;
  const foldedPool = mode === "direct" ? pool.filter((entry) => !directCapable(entry)) : [];

  const renderProviderRow = (entry: GatewayPoolEntry, folded: boolean) => {
    const enabled = providerEnabled(entry);
    const on = enabled && !exclude.includes(entry.id);
    const codeDirect = isCode && codeMode === "direct" && directPoolId === entry.id;
    const linkedCodexId = entry.linkedCodexRelay ?? null;
    const poolEditKind = !entry.account || entry.account === "chatgpt" ? "edit" : "models";
    const onRowClick = linkedCodexId
      ? (onEditCodexProvider ? () => onEditCodexProvider(linkedCodexId) : undefined)
      : () => openPoolDialog({ kind: poolEditKind, id: entry.id, scope: "full" });
    const muted = folded || (mode === "aggregate" ? !on : !enabled);
    const toggleAggregate = async () => {
      if (!enabled) {
        const ok = await run(`toggle:${entry.id}`, "set_gateway_provider_enabled", { id: entry.id, enabled: true }, t("启用供应商"), true);
        if (ok && exclude.includes(entry.id)) {
          toggle(entry.id, true);
        }
        return;
      }
      toggle(entry.id, !on);
    };

    return (
      <CxRow
        badges={(
          <>
            {!enabled ? <CxBadge>{t("已停用")}</CxBadge> : null}
            {folded ? <CxBadge>{t("仅聚合")}</CxBadge> : null}
            <OwnerBadge entry={entry} page="claude" />
          </>
        )}
        detail={[protocolLabel(entry.protocol), presetHost(entry.baseUrl), tf("{0} 个模型", [entry.models.length]), credentialSummary(entry, payload)].filter(Boolean).join(" · ")}
        icon={<Avatar baseUrl={entry.baseUrl} name={entry.name} size="sm" />}
        key={entry.id}
        muted={muted}
        onClick={onRowClick}
        title={maskAccountName(entry.name)}
      >
        {folded ? null : mode === "direct" ? (
          codeDirect ? (
            <CxBadge tone="ok">{t("使用中")}</CxBadge>
          ) : (
            <button
              className="cx-secondary"
              disabled={busy !== null || !enabled}
              onClick={() => void goDirect(entry)}
              title={!enabled ? t("该供应商已停用，请先在菜单里启用") : t("Claude Code 不经 Codex++ 直接连这一家（地址和 Key 写进它自己的 settings.json），关掉 Codex++ 也能用")}
              type="button"
            >
              {t("使用")}
            </button>
          )
        ) : (
          <Switch
            label={tf("给 {0} 用 {1}", [agentDisplayName, maskAccountName(entry.name)])}
            on={on}
            onToggle={() => void toggleAggregate()}
            pending={busy === `exclude:${entry.id}` || busy === `toggle:${entry.id}`}
            title={!enabled ? t("该供应商已停用，点击将重新启用") : undefined}
          />
        )}
        <Menu label={t("供应商操作")}>
          {linkedCodexId ? (
            <>
              {onEditCodexProvider ? <MenuItem onClick={() => onEditCodexProvider(linkedCodexId)}>{t("在 Codex 里编辑")}</MenuItem> : null}
              <MenuItem onClick={() => openPoolDialog({ kind: "price", id: entry.id, scope: "gateway" })}>{t("网关设置")}</MenuItem>
              {hasCredentialSettings(entry) ? (
                <MenuItem onClick={() => { setCredentialsUsed(true); setCredentialsId(entry.id); }}>{credentialMenuLabel(entry)}</MenuItem>
              ) : null}
              <MenuItem
                disabled={!enabled}
                onClick={() => openPoolDialog({ kind: "test", id: entry.id })}
                title={!enabled ? t("请先启用供应商再测试") : undefined}
              >
                {t("测试")}
              </MenuItem>
              <MenuSeparator />
              <MenuItem
                onClick={() => void run(`toggle:${entry.id}`, "set_gateway_provider_enabled", { id: entry.id, enabled: !enabled }, t("切换供应商"), true)}
              >
                {enabled ? t("停用") : t("启用")}
              </MenuItem>
            </>
          ) : (
            <>
              <MenuItem onClick={() => openPoolDialog({ kind: poolEditKind, id: entry.id, scope: "full" })}>
                {t("编辑")}
              </MenuItem>
              {hasCredentialSettings(entry) ? (
                <MenuItem onClick={() => { setCredentialsUsed(true); setCredentialsId(entry.id); }}>{credentialMenuLabel(entry)}</MenuItem>
              ) : null}
              <MenuItem
                disabled={!enabled}
                onClick={() => openPoolDialog({ kind: "test", id: entry.id })}
                title={!enabled ? t("请先启用供应商再测试") : undefined}
              >
                {t("测试")}
              </MenuItem>
              <MenuSeparator />
              <MenuItem
                onClick={() => void run(`toggle:${entry.id}`, "set_gateway_provider_enabled", { id: entry.id, enabled: !enabled }, t("切换供应商"), true)}
              >
                {enabled ? t("停用") : t("启用")}
              </MenuItem>
              <MenuItem danger onClick={() => void removePoolProvider(entry)}>{t("删除供应商")}</MenuItem>
            </>
          )}
        </Menu>
      </CxRow>
    );
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
      <CxSection
        aside={(
          <>
            {claudeCode ? (
              <ConnectionModeSwitch
                aggregateTitle={t("Claude Code 改走 Codex++ 本地服务，用下面打开的全部供应商；新开的会话生效")}
                connecting={busy === "code:aggregate" || busy === "disable:claude"}
                directTitle={mode === "aggregate"
                  ? t("查看直连供应商列表，点任一家「使用」即可直接切换过去")
                  : t("直连：Claude Code 不经 Codex++ 直接连一家 Anthropic 协议供应商，关掉 Codex++ 也能用")}
                label={t("Claude Code 的连接方式")}
                mode={mode}
                onAggregate={() => {
                  if (codeMode === "aggregate") setViewMode("aggregate");
                  else void connectCodeAggregate();
                }}
                onDirect={() => setViewMode("direct")}
              />
            ) : null}
            {allPool.length || onManageGroups ? (
              <Menu label={t("更多供应商操作")}>
                {onManageGroups ? <MenuItem onClick={onManageGroups}>{t("管理模型组")}</MenuItem> : null}
                {allPool.length ? (
                  <MenuItem checked={showOtherAgents} onClick={() => setShowOtherAgents(!showOtherAgents)} title={t("Codex 的直连供应商默认共享给 Claude；不勾时这里不列它们")}>
                    {t("显示其他 Agent 的供应商")}
                  </MenuItem>
                ) : null}
              </Menu>
            ) : null}
          </>
        )}
        className="claude-providers"
        // 和 Codex 页一样：同一份供应商列表，计数在聚合与直连两种模式下保持一致。
        count={pool.length || undefined}
        title={t("供应商")}
      >
        <div className="gw-pool-lists">
          <CxCard>
            {!payload ? (
              <CxLoading label={t("正在读取供应商…")} />
            ) : !allPool.length ? (
              <CxEmpty detail={t("DeepSeek、Kimi、智谱这些 OpenAI 协议的也能用，本地服务会自动转换。")} title={t("还没有供应商")}>
                <button className="cx-secondary" onClick={() => setAdding("catalog")} type="button"><Plus />{t("添加供应商")}</button>
                <button className="cx-secondary" onClick={() => setAdding("import")} type="button"><Download />{t("从 CC Switch / ClaudeMom / Magpie 导入")}</button>
              </CxEmpty>
            ) : !pool.length ? (
              <CxEmpty detail={t("其他 Agent 的供应商已隐藏；勾上「显示其他 Agent 的供应商」就能看到。")} title={t("这里没有 Claude 自己的供应商")} />
            ) : !primaryPool.length ? (
              <CxEmpty
                detail={t("仅支持聚合模式的供应商需经本地服务转发或转换协议，切到「聚合」后即可启用。")}
                title={t("当前没有可直接直连的供应商")}
              >
                <button className="cx-secondary" onClick={() => setAdding("catalog")} type="button"><Plus />{t("添加供应商")}</button>
              </CxEmpty>
            ) : (
              primaryPool.map((entry) => renderProviderRow(entry, false))
            )}
          </CxCard>
          {foldedPool.length ? (
            <>
              <button
                aria-expanded={foldedOpen}
                className="gw-pool-off-toggle"
                onClick={() => setFoldedOpen(!foldedOpen)}
                type="button"
              >
                <ChevronRight aria-hidden="true" className="h-3.5 w-3.5" />
                <span>{tf("{0} 家仅支持聚合模式", [foldedPool.length])}</span>
              </button>
              {foldedOpen ? (
                <CxCard>
                  {foldedPool.map((entry) => renderProviderRow(entry, true))}
                </CxCard>
              ) : null}
            </>
          ) : null}
        </div>
      </CxSection>
      {/* 已建模型组直接展示在供应商列表下方（聚合模式下可见），和 Codex 页一致。 */}
      {mode === "aggregate" && groups.length > 0 ? (
        <CxSection
          aside={onManageGroups ? (
            <button className="cx-secondary" onClick={onManageGroups} type="button">{t("在「路由」里管理")}</button>
          ) : undefined}
          count={groups.length}
          title={t("模型组")}
        >
          <CxCard>{groups.map(renderGroupRow)}</CxCard>
        </CxSection>
      ) : null}
      {/* 列表下面一张卡：只显示当前 Agent 自己的默认模型与连接状态。 */}
      <CxSection className="claude-providers-agents" title={t("默认模型")}>
        <CxCard>
          {isCode ? (
            payload && !claudeCode ? (
              <CxRow
                detail={t("未检测到本机 Claude Code；安装后即可在这里选择默认模型或切换直连 / 聚合。")}
                icon={<AgentGlyph agent="claude" />}
                title={t("Claude Code（命令行）")}
              />
            ) : (
              <CxRow
                badges={claudeCode?.version ? <CxBadge>{claudeCode.version}</CxBadge> : null}
                detail={claudeCodeDetail}
                icon={<AgentGlyph agent="claude" />}
                title={t("Claude Code（命令行）")}
              >
                {codeMode === "aggregate" || pickingAggregate || !claudeCode ? (
                  <AgentModelPicker agentId="claude" busy={busy} notify={notify} payload={payload} run={run} />
                ) : null}
              </CxRow>
            )
          ) : (
            <CxRow
              detail={t("始终经 Codex++ 本地服务：上面打开的供应商，聊天模型都会列进 Claude Desktop 的模型选择器；启动后默认用这个模型。")}
              icon={<AgentGlyph agent={AGENT} />}
              title="Claude Desktop"
            >
              <AgentModelPicker agentId={AGENT} busy={busy} notify={notify} payload={payload} run={run} />
            </CxRow>
          )}
        </CxCard>
      </CxSection>
      {credentialsUsed ? (
        <ProviderCredentialsHost notify={notify} onClose={() => setCredentialsId(null)} openId={credentialsId} payload={payload} run={run} />
      ) : null}
      {editHostUsed ? (
        <Suspense fallback={null}>
          <PoolProviderEditHost
            busy={busy}
            dialog={poolDialog}
            onClose={() => setPoolDialog(null)}
            onOpen={setPoolDialog}
            payload={payload}
            run={run}
          />
        </Suspense>
      ) : null}
      {adding ? (
        <ProviderAdder
          busy={busy}
          directOnly={isCode && mode === "direct"}
          // 新加的供应商默认只给当前 Agent 打开（另一边需要时到它自己的页面拨开即可）。
          includeAgents={[agentId]}
          notify={notify}
          // 从这里加的、导入的记成 Claude 的：Codex 页「显示其他 Agent 的供应商」按它筛。
          owner="claude"
          scope={agent}
          onClose={() => setAdding(null)}
          payload={payload}
          run={run}
          startWith={adding}
        />
      ) : null}
      {isCode && directTarget && claudeCode ? (
        <AgentDisconnectDialog
          agent={claudeCode}
          busy={busy !== null}
          onApply={(previewToken) => directTarget === "restore"
            ? run("disable:claude", "disable_gateway_binding", { agent: CODE, previewToken }, t("断开并还原"))
            : run(`direct:${directTarget.id}`, "use_claude_code_pool_provider", { id: directTarget.id, previewToken }, t("断开并直连"))}
          onFullSwitch={directTarget === "restore"
            ? (previewToken) => run("disable:claude", "disable_gateway_binding_full_switch", { agent: CODE, previewToken }, t("断开并还原"))
            : undefined}
          onClose={() => setDirectTarget(null)}
        />
      ) : null}
    </>
  );
}
