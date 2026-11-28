// Codex「连接与供应商」页的两种连接方式与统一供应商列表：
// - 聚合（多选）：Codex 经 Codex++ 本地服务，打开开关的供应商合在一起，模型都列进 Codex 的模型选择器，不用切换；
// - 直连（单选）：Codex 直接连一家，关掉 Codex++ 也能用（需协议转换的除外），换供应商点「使用」。
// 两种模式共用同一份列表、同一套行样式与菜单；当前模式不支持的条目默认折叠收在列表下方一行，不丢失也不干扰选择。
import { lazy, Suspense, useMemo, useState } from "react";
import { Cable, ChevronRight, Layers3, Loader2, Waypoints } from "lucide-react";
import { Avatar, Switch, type Notify, type Run } from "./gateway-ui";
import {
  bindingFromDraft,
  draftFromBinding,
  groupModeLabel,
  groupModelId,
  maskAccountName,
  presetHost,
  protocolLabel,
  providerEnabled,
  validateDraft,
  type GatewayPayload,
  type GatewayPoolEntry,
} from "./gateway-model";
import { Menu, MenuItem, MenuSeparator } from "./ManagementPages";
import { OwnerBadge } from "./ProviderOwner";
import { credentialMenuLabel, credentialSummary, hasCredentialSettings } from "./ProviderCredentials";
import { CxBadge, CxCard, CxEmpty, CxLoading, CxRow, CxSection } from "./cx-layout";
import { appConfirm } from "./app-dialog";
import { confirmRemoveRelayProfile, multiLaunchSupported, relayProfileConfigBrief, relayProfileModeLabel } from "./app-shared";
import { duplicateRelayProfile, isAggregateRelayProfile, relayProviderView } from "./relay-profile-editing";
import type { Actions, BackendSettings, RelayProfile } from "./app-types";
import { buildCodexUnifiedProviders, codexItemSupportsAggregate, codexItemSupportsDirect, type CodexUnifiedProvider } from "./codex-unified-providers";
import { t, tf } from "@/i18n";

export type CodexConnectionMode = "aggregate" | "direct";

// 默认模型下拉在 GatewayAgents 里，那是按需加载的大模块：这里也按需引入，不把它打回主包。
const AgentModelPicker = lazy(() => import("./GatewayAgents").then((module) => ({ default: module.AgentModelPicker })));
// 「账号与分配」弹窗连着账号登录，也在大模块里：点开时才加载。
const ProviderCredentialsHost = lazy(() => import("./ProviderCredentialsHost").then((module) => ({ default: module.ProviderCredentialsHost })));
// 共享供应商的就地编辑与测试弹窗：按需加载，避免拖大主包。
import type { PoolProviderDialogState } from "./CodexPoolEditHost";
const PoolProviderEditHost = lazy(() => import("./CodexPoolEditHost").then((module) => ({ default: module.PoolProviderEditHost })));

// 统一列表的合并规则和 Codex 里的供应商页共用一份（codex-unified-providers.ts，经 42 分片打包给注入页）。
export { buildCodexUnifiedProviders, codexItemSupportsAggregate, codexItemSupportsDirect, type CodexUnifiedProvider };

/** 「聚合 / 直连」两段切换：Codex 页和 Claude 页共用。 */
export function ConnectionModeSwitch({
  label,
  mode,
  connecting,
  canAggregate = true,
  aggregateTitle,
  directTitle,
  onAggregate,
  onDirect,
}: {
  label: string;
  mode: CodexConnectionMode;
  connecting: boolean;
  canAggregate?: boolean;
  aggregateTitle: string;
  directTitle: string;
  onAggregate: () => void;
  onDirect: () => void;
}) {
  const aggregate = mode === "aggregate";
  return (
    <div aria-label={label} className="mg-segmented cx-connection-switch" role="radiogroup">
      <button
        aria-checked={aggregate}
        data-active={aggregate}
        disabled={connecting || aggregate || !canAggregate}
        onClick={onAggregate}
        role="radio"
        title={aggregateTitle}
        type="button"
      >
        {connecting ? <Loader2 className="animate-spin" /> : <Waypoints />}
        {t("聚合")}
      </button>
      <button
        aria-checked={!aggregate}
        data-active={!aggregate}
        disabled={connecting || !aggregate}
        onClick={onDirect}
        role="radio"
        title={directTitle}
        type="button"
      >
        <Cable />
        {t("直连")}
      </button>
    </div>
  );
}

/**
 * 「供应商」分区标题行右侧的连接方式控件：两段切换，聚合时紧跟 Codex 的默认模型下拉。
 */
export function CodexModeControls({
  mode,
  connecting,
  canSwitch,
  onAggregate,
  onDirect,
  payload,
  hiddenProviders,
  busy,
  run,
  notify,
}: {
  mode: CodexConnectionMode;
  connecting: boolean;
  /** 配置切换总开关关着时不能切。 */
  canSwitch: boolean;
  onAggregate: () => void;
  onDirect: () => void;
  payload: GatewayPayload | null;
  /** 列表里隐藏了的供应商（别的 Agent 的）：默认模型下拉也不列它们的模型。 */
  hiddenProviders?: string[];
  busy: string | null;
  run: Run;
  notify: Notify;
}) {
  return (
    <>
      <ConnectionModeSwitch
        aggregateTitle={canSwitch
          ? t("聚合：Codex 经 Codex++ 本地服务，下面打开的供应商合在一起，模型都列进 Codex 的模型选择器，不用切换；需要 Codex++ 在后台运行")
          : t("先在页面底部「更多设置」里打开供应商配置切换")}
        canAggregate={canSwitch}
        connecting={connecting}
        directTitle={mode === "aggregate"
          ? t("查看直连供应商列表，点任一家「使用」即可直接切换过去")
          : t("直连：Codex 直接连一家供应商，关掉 Codex++ 也能用（需协议转换的除外）；换供应商点「使用」")}
        label={t("Codex 的连接方式")}
        mode={mode}
        onAggregate={onAggregate}
        onDirect={onDirect}
      />
      {mode === "aggregate" ? (
        <Suspense fallback={null}>
          <AgentModelPicker agentId="codex" busy={busy} hiddenProviders={hiddenProviders} notify={notify} payload={payload} run={run} />
        </Suspense>
      ) : null}
    </>
  );
}

/**
 * Codex 统一供应商列表：直连和聚合共用同一份列表与行样式，只切换右侧控件（聚合 = 多选开关，直连 = 单选使用）；
 * 当前模式不支持的条目默认折叠收在列表下方一行，点开仍可查看、编辑和删除。
 */
export function CodexProviderList({
  mode,
  takenOverByGateway,
  switching,
  payload,
  entries,
  profiles,
  activeOfficialDirect,
  activeNativeAccountRef,
  activeOfficialExternalId,
  busy,
  run,
  notify,
  form,
  actions,
  onFormChange,
  onEditLinked,
  onSwitchProfile,
  onSwitchChatgptDirect,
  onAdd,
}: {
  mode: CodexConnectionMode;
  /** Codex 当前实际是否处于聚合接入状态（直连视图预览时为 true，不把直连项标成「使用中」）。 */
  takenOverByGateway: boolean;
  switching: boolean;
  payload: GatewayPayload | null;
  /** 共享供应商（按「显示其他 Agent 的供应商」筛过）。 */
  entries: GatewayPoolEntry[];
  /** Codex 直连供应商（不含托管的本地服务占位条目）。 */
  profiles: RelayProfile[];
  /** 当前正以纯官方登录直连（非聚合、非混合 API）。 */
  activeOfficialDirect: boolean;
  /** 当前选作 Codex 登录身份的 ChatGPT 账号 id（没有或为本地账号时为 null）。 */
  activeNativeAccountRef: string | null;
  /** 当纯官方直连正在使用非池内外部登录时，保留该 OpenAI Official 条目可见。 */
  activeOfficialExternalId: string | null;
  busy: string | null;
  run: Run;
  notify: Notify;
  form: BackendSettings;
  actions: Actions;
  onFormChange: (next: BackendSettings) => Promise<unknown> | void;
  /** 打开 Codex 直连供应商编辑页。 */
  onEditLinked: (relayId: string) => boolean;
  /** 直连模式下选择使用某家直连供应商。 */
  onSwitchProfile: (profileId: string) => void;
  /** 直连模式下选择使用某个 ChatGPT 账号（写入登录态 + 切到官方直连）。 */
  onSwitchChatgptDirect?: (accountRef: string, accountName: string) => void;
  onAdd: () => void;
}) {
  // 「账号与分配」：打开的是哪一家；打开过就一直挂着，关掉后登录提示照样在。
  const [credentialsId, setCredentialsId] = useState<string | null>(null);
  const [credentialsUsed, setCredentialsUsed] = useState(false);
  // 共享供应商的就地编辑 / 同步 / 价格 / 代理 / 限速 / 测试弹窗。
  const [poolDialog, setPoolDialog] = useState<PoolProviderDialogState | null>(null);
  const [editHostUsed, setEditHostUsed] = useState(false);
  // 底部「仅支持另一模式」的折叠状态。
  const [foldedOpen, setFoldedOpen] = useState(false);

  const unified = useMemo(
    () => buildCodexUnifiedProviders(profiles, entries, { activeOfficialExternalId }),
    [profiles, entries, activeOfficialExternalId],
  );
  const primaryItems = useMemo(
    () => unified.filter((item) => (mode === "aggregate" ? codexItemSupportsAggregate(item) : codexItemSupportsDirect(item))),
    [unified, mode],
  );
  const foldedItems = useMemo(
    () => unified.filter((item) => (mode === "aggregate" ? !codexItemSupportsAggregate(item) : !codexItemSupportsDirect(item))),
    [unified, mode],
  );

  if (mode === "aggregate" && !payload && !profiles.length) {
    return <CxCard><CxLoading label={t("正在读取供应商…")} /></CxCard>;
  }

  const exclude = payload?.gateway.bindings?.codex?.poolExclude ?? [];

  if (!unified.length) {
    if (mode === "aggregate" && (payload?.pool.length ?? 0) > 0) {
      return (
        <CxCard>
          <CxEmpty detail={t("其他 Agent 的供应商已隐藏；勾上「显示其他 Agent 的供应商」就能看到。")} title={t("这里没有 Codex 自己的供应商")} />
        </CxCard>
      );
    }
    return (
      <CxCard>
        <CxEmpty detail={t("加进来的供应商直连和聚合两种模式都能用；Claude 等其它 Agent 也能共用。")} title={t("还没有供应商")}>
          <button className="cx-secondary" onClick={onAdd} type="button">{t("添加供应商")}</button>
        </CxEmpty>
      </CxCard>
    );
  }

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

  const renderRow = (item: CodexUnifiedProvider, folded: boolean) => {
    const { profile, poolEntry: entry } = item;
    const isChatgptAccount = entry?.account === "chatgpt";
    const enabled = entry ? providerEnabled(entry) : true;
    const aggOn = Boolean(entry) && enabled && !exclude.includes(entry!.id);
    const isDirectActive = profile
      ? !takenOverByGateway && form.activeRelayId === profile.id
      : isChatgptAccount && Boolean(entry) && activeOfficialDirect && activeNativeAccountRef === entry!.id;
    const poolEditKind = entry && (!entry.account || entry.account === "chatgpt") ? "edit" : "models";
    const onRowClick = profile
      ? () => { onEditLinked(profile.id); }
      : entry
        ? () => openPoolDialog({ kind: poolEditKind, id: entry.id, scope: "full" })
        : undefined;

    const detail = entry && (!entry.linkedCodexRelay || entry.baseUrl.trim())
      ? [isChatgptAccount ? t("官方订阅") : protocolLabel(entry.protocol), presetHost(entry.baseUrl), tf("{0} 个模型", [entry.models.length]), credentialSummary(entry, payload)].filter(Boolean).join(" · ")
      : profile
        ? [relayProfileModeLabel(profile), relayProfileConfigBrief(profile)].filter(Boolean).join(" · ")
        : "";

    const muted = folded || (mode === "aggregate" ? !aggOn : (!isChatgptAccount && !enabled));
    const toggleAggregate = async () => {
      if (!entry) return;
      if (!enabled) {
        const ok = await run(`toggle:${entry.id}`, "set_gateway_provider_enabled", { id: entry.id, enabled: true }, t("启用供应商"), true);
        if (ok && exclude.includes(entry.id)) {
          await run(`exclude:${entry.id}`, "set_gateway_pool_member", { agents: ["codex"], id: entry.id, used: true }, t("保存供应商开关"), true);
        }
        return;
      }
      await run(`exclude:${entry.id}`, "set_gateway_pool_member", { agents: ["codex"], id: entry.id, used: !aggOn }, t("保存供应商开关"), true);
    };

    return (
      <CxRow
        badges={(
          <>
            {!enabled && (mode === "aggregate" || !isChatgptAccount) ? <CxBadge>{t("已停用")}</CxBadge> : null}
            {folded ? <CxBadge>{mode === "direct" ? t("仅聚合") : t("仅直连")}</CxBadge> : null}
            {!folded && mode === "direct" && profile && relayProviderView(profile) !== "direct" ? (
              <CxBadge title={t("这一家需要 Codex++ 转发或转换协议，关掉 Codex++ 后无法直连")}>{t("需 Codex++")}</CxBadge>
            ) : null}
            {entry ? <OwnerBadge entry={entry} page="codex" /> : null}
          </>
        )}
        detail={detail}
        icon={<Avatar baseUrl={item.baseUrl} name={item.name} size="sm" />}
        key={item.key}
        muted={muted}
        onClick={onRowClick}
        title={maskAccountName(item.name)}
      >
        {folded ? null : mode === "aggregate" && entry ? (
          <Switch
            label={tf("给 Codex 用 {0}", [maskAccountName(entry.name)])}
            on={aggOn}
            onToggle={() => void toggleAggregate()}
            pending={busy === `exclude:${entry.id}` || busy === `toggle:${entry.id}`}
            title={!enabled ? t("该供应商已停用，点击将重新启用") : undefined}
          />
        ) : profile ? (
          isDirectActive ? (
            <CxBadge tone="ok">{t("使用中")}</CxBadge>
          ) : (
            <button
              className="cx-secondary"
              disabled={switching}
              onClick={() => onSwitchProfile(profile.id)}
              type="button"
            >
              {t("使用")}
            </button>
          )
        ) : isChatgptAccount && entry ? (
          isDirectActive ? (
            <CxBadge tone="ok">{t("使用中")}</CxBadge>
          ) : (
            <button
              className="cx-secondary"
              disabled={switching}
              onClick={() => onSwitchChatgptDirect?.(entry.id, entry.name)}
              title={t("把这个 ChatGPT 账号写入 Codex 登录态并走官方直连，不经本地服务")}
              type="button"
            >
              {t("使用")}
            </button>
          )
        ) : null}
        <Menu label={t("供应商操作")}>
          {profile ? (
            <>
              <MenuItem onClick={() => { onEditLinked(profile.id); }}>{t("编辑")}</MenuItem>
              {entry && hasCredentialSettings(entry) ? (
                <MenuItem onClick={() => { setCredentialsUsed(true); setCredentialsId(entry.id); }}>{credentialMenuLabel(entry)}</MenuItem>
              ) : null}
              <MenuItem
                disabled={isAggregateRelayProfile(profile)}
                onClick={() => {
                  if (isAggregateRelayProfile(profile)) return;
                  if (entry && entry.baseUrl.trim()) {
                    openPoolDialog({ kind: "test", id: entry.id });
                  } else {
                    void actions.testRelayProfile(profile);
                  }
                }}
              >
                {t("测试")}
              </MenuItem>
              <MenuSeparator />
              <MenuItem
                disabled={!multiLaunchSupported(profile) || actions.multiLaunchPending === profile.id}
                onClick={() => { if (multiLaunchSupported(profile)) void actions.launchMultiInstance(profile); }}
              >
                {t("用这个供应商多开")}
              </MenuItem>
              <MenuItem onClick={() => void onFormChange(duplicateRelayProfile(form, profile.id))}>{t("复制")}</MenuItem>
              <MenuSeparator />
              <MenuItem
                danger
                disabled={form.relayProfiles.length <= 1}
                onClick={() => {
                  void confirmRemoveRelayProfile(form, profile, actions, "relay").then((next) => {
                    if (next) void onFormChange(next);
                  });
                }}
              >
                {t("删除供应商")}
              </MenuItem>
            </>
          ) : entry ? (
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
          ) : null}
        </Menu>
      </CxRow>
    );
  };

  return (
    <div className="gw-pool-lists">
      {credentialsUsed ? (
        <Suspense fallback={null}>
          <ProviderCredentialsHost notify={notify} onClose={() => setCredentialsId(null)} openId={credentialsId} payload={payload} run={run} />
        </Suspense>
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
      <CxCard>
        {primaryItems.length ? (
          /* 供应商一多整张卡片就高出窗口：列表在卡片内部滚动（#3324），列表下面还有折叠区时也够得着。 */
          <div className="gw-pool-scroll">
            {primaryItems.map((item) => renderRow(item, false))}
          </div>
        ) : (
          <CxEmpty
            detail={mode === "direct"
              ? t("仅支持聚合模式的供应商需经本地服务转发或转换协议，切到「聚合」后即可启用。")
              : t("仅支持直连模式的供应商不会经本地服务转发，切到「直连」后可点「使用」。")}
            title={mode === "direct" ? t("当前没有可直接直连的供应商") : t("当前没有可用于聚合的供应商")}
          >
            <button className="cx-secondary" onClick={onAdd} type="button">{t("添加供应商")}</button>
          </CxEmpty>
        )}
      </CxCard>
      {foldedItems.length ? (
        <>
          <button
            aria-expanded={foldedOpen}
            className="gw-pool-off-toggle"
            onClick={() => setFoldedOpen(!foldedOpen)}
            type="button"
          >
            <ChevronRight aria-hidden="true" className="h-3.5 w-3.5" />
            <span>{mode === "direct" ? tf("{0} 家仅支持聚合模式", [foldedItems.length]) : tf("{0} 家仅支持直连模式", [foldedItems.length])}</span>
          </button>
          {foldedOpen ? (
            <CxCard>
              <div className="gw-pool-scroll">
                {foldedItems.map((item) => renderRow(item, true))}
              </div>
            </CxCard>
          ) : null}
        </>
      ) : null}
    </div>
  );
}

/**
 * 聚合模式下的「模型组」列表：在 Codex「连接与供应商」页直接列出在「本地服务 → 路由」里建好的模型组，
 * 可直接启停或在三点菜单里设为 Codex 默认模型，不用切去本地服务里选。
 */
export function CodexModelGroupsSection({
  payload,
  busy,
  run,
  notify,
  onManageGroups,
}: {
  payload: GatewayPayload | null;
  busy: string | null;
  run: Run;
  notify: Notify;
  onManageGroups?: () => void;
}) {
  const groups = (payload?.groups ?? []).filter((group) => group.members.length > 0);
  if (!groups.length) return null;

  const codexAgent = payload?.agents.find((item) => item.agent === "codex");
  const codexBinding = payload?.gateway.bindings.codex;
  const currentModel = (codexAgent?.enabled ? codexAgent.defaultModel : codexBinding?.model ?? "").trim().toLowerCase();

  const useGroup = async (groupId: string) => {
    const connected = codexAgent?.enabled ?? false;
    const next = { ...draftFromBinding(codexBinding), model: groupModelId(groupId) };
    const error = validateDraft(next, payload, connected ? codexBinding?.model : undefined, null);
    if (error) {
      void notify(t("还不能接入"), t(error), "warning");
      return;
    }
    await run(
      `group:codex:${groupId}`,
      "save_gateway_binding",
      { agent: "codex", binding: { ...bindingFromDraft(next, codexBinding), enabled: true }, reconnect: true },
      t("保存本地服务绑定"),
      connected,
    );
  };

  return (
    <CxSection
      aside={onManageGroups ? (
        <button className="cx-secondary" onClick={onManageGroups} type="button">{t("在「路由」里管理")}</button>
      ) : undefined}
      count={groups.length}
      title={t("模型组")}
    >
      <CxCard>
        {groups.map((group) => {
          const isCurrent = Boolean(codexAgent?.enabled) && currentModel === groupModelId(group.id).toLowerCase();
          return (
            <CxRow
              badges={(
                <>
                  {isCurrent ? <CxBadge tone="ok">{t("当前默认")}</CxBadge> : null}
                  {group.disabled ? <CxBadge>{t("已停用")}</CxBadge> : null}
                </>
              )}
              detail={[t(groupModeLabel(group.mode)), tf("{0} 个成员", [group.members.length])].join(" · ")}
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
                <MenuItem disabled={group.disabled || isCurrent || busy !== null} onClick={() => void useGroup(group.id)}>
                  {t("设为 Codex 默认模型")}
                </MenuItem>
                {onManageGroups ? <MenuItem onClick={onManageGroups}>{t("在「路由」里管理")}</MenuItem> : null}
              </Menu>
            </CxRow>
          );
        })}
      </CxCard>
    </CxSection>
  );
}
