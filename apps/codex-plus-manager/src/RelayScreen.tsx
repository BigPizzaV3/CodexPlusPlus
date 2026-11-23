// Codex 连接页：连接方式、直连供应商列表、旧版代理设置。
import {
  closestCenter,
  DndContext,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import {
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
  ArrowLeft,
  Cable,
  CheckCircle2,
  ChevronRight,
  Download,
  GripVertical,
  KeyRound,
  Loader2,
  FolderOpen,
  PowerOff,
  Plus,
  RefreshCw,
  Search,
  Sparkles,
  ShieldCheck,
  ShieldAlert,
  Trash2,
  Waypoints,
  X,
} from "lucide-react";
import { createPresetPatch } from "@/components/ProviderPresetSelector";
import { PRESETS, type ProviderPreset } from "./presets";
import { useSponsorPresets } from "./sponsor-store";
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "./gateway-ui";
import { AppSelect } from "./AppSelect";
import { useGatewayPayload } from "./hub-shared";
import { publishGatewayPayload } from "./gateway-store";
import { importCandidates, type ImportCandidate, type GatewayResult } from "./gateway-model";
import { Menu, MenuItem, PageHeader } from "./ManagementPages";
import { aggregateMemberCandidates } from "./aggregate-members";
import { NativeAuthControl } from "./NativeAuthControls";
import { relayBillingCacheKey, relayBillingProbeEligible, type RelayBillingProbe } from "./relay-billing";
import { RelayBillingLine, RelayBillingRetry, useRelayBillingProbe } from "./RelayBillingLine";
import { resolveAnchoredMenuLayout, type AnchoredMenuLayout } from "./popover-layer";
import { t, tf } from "@/i18n";
import {
  applyRelayProfilePatchToFiles,
  syncLegacyRelayFields,
  createRelayProfile,
  createAggregateRelayProfile,
  duplicateRelayProfile,
  reorderRelayProfiles,
  isAggregateRelayProfile,
  relayProviderView,
  aggregateProfileForMode,
  normalizeAggregateConfig,
  type ProviderViewKind,
} from "./relay-profile-editing";
import { CxBadge, CxBanner, CxCard, CxEmpty, CxFilters, CxRow, CxSection, CxStatusIcon } from "./cx-layout";
import type {
  Actions,
  AggregateMode,
  BackendSettings,
  CcsProvidersResult,
  CommandResult,
  EnvConflictsResult,
  RelayEnvironmentResult,
  RelayFilesResult,
  RelayProfile,
  SettingsResult,
  SshRemoteLoopbackNoticeData,
} from "./app-types";
import {
  CODEX_GATEWAY_RELAY_ID,
  codexGatewayModelCount,
  confirmRemoveRelayProfile,
  isSuccessStatus,
  multiLaunchSupported,
  normalizeSettings,
  providerInitial,
  relayProfileConfigBrief,
  relayProfileModeLabel,
  relayProfileTagTone,
  relayProtocolShortLabel,
  SettingsAutoSave,
  stringifyError,
  ToggleVisual,
} from "./app-shared";
import { RelayProfileDetail } from "./RelayEditor";

/** 「Codex 连接」页把「打开某个供应商的编辑页并跳到某项设置」交给嵌在页底「高级」里的列表用。 */
const RelayProfileLinkContext = createContext<((profileId: string, section: "proxy") => void) | null>(null);

export function ProxySettingsScreen({
  dirty,
  form,
  relayEnvironment,
  onFormChange,
  actions,
}: {
  dirty: boolean;
  form: BackendSettings;
  relayEnvironment: RelayEnvironmentResult | null;
  onFormChange: (value: BackendSettings) => void;
  actions: Actions;
}) {
  // Codex 被新版网关接管时，旧版专用的设置不起作用，隐藏。
  const takenOverByGateway = form.activeRelayId === CODEX_GATEWAY_RELAY_ID;
  const proxyProfiles = useMemo(
    () => (form.relayProfiles ?? []).filter((profile) => profile.relayMode !== "aggregate"),
    [form.relayProfiles],
  );
  const proxyProfileKey = proxyProfiles.map((profile) => profile.id).join("|");
  // 每个供应商的独立代理只在这里列出来，一次读完；改代理去供应商编辑页（随「保存供应商」一起写），这里不再单独编辑。
  const [proxies, setProxies] = useState<Record<string, { proxyUrl?: string; configured?: boolean }>>({});
  const openRelayProfile = useContext(RelayProfileLinkContext);
  useEffect(() => {
    let cancelled = false;
    const ids = proxyProfileKey ? proxyProfileKey.split("|") : [];
    if (!ids.length) {
      setProxies({});
      return;
    }
    void invoke<{ status: string; proxies?: Record<string, { proxyUrl?: string; configured?: boolean }> }>("load_relay_proxies", { profileIds: ids })
      .then((result) => {
        if (!cancelled && result.status === "ok") setProxies(result.proxies ?? {});
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [proxyProfileKey]);

  // 嵌在「Codex 连接」页底部的「高级」折叠区里：每项一行，说明一行，长说明放悬停。
  return (
    <div className="settings-page proxy-settings-page cx-codex-advanced-body">
      <SettingsAutoSave dirty={dirty} form={form} actions={actions} />
      {takenOverByGateway ? (
        <CxRow
          detail={t("Codex 正由 Codex++ 网关接管，旧版代理设置不起作用，已隐藏。断开并还原后恢复。")}
          muted
          title={t("目录外模型映射（旧版）")}
        />
      ) : (
      <>
      <CxRow
        detail={form.protocolProxyMapUnlistedModels
          ? t("注意：手填但没写进模型列表的模型也会被改写，请先把要用的模型都加进列表。")
          : t("后台任务的固定模型名改写为当前供应商的默认模型；接入网关的 Agent 不用这一项。")}
        title={t("目录外模型映射（旧版）")}
        tooltip={t("Codex 的自动审查（codex-auto-review）与标题/摘要等后台任务固定使用 gpt-* 等内部模型名，不经选模；第三方上游没有这些名字，原样转发只会报「模型不存在」并可能反复消耗额度。开启后，不在当前供应商模型列表里的请求模型会改写为该供应商的默认模型再转发（#1183 / #1446 / #1457）；官方登录与聚合供应商不受影响，已配置「自动审批模型」时 codex-auto-review 仍按该配置改写。默认关闭：开启后手填但未写进模型列表的模型也会被改写，请先把要用的模型都加进列表。")}
      >
        <Switch
          label={t("改写目录外模型名")}
          on={form.protocolProxyMapUnlistedModels}
          onToggle={() => onFormChange({ ...form, protocolProxyMapUnlistedModels: !form.protocolProxyMapUnlistedModels })}
        />
      </CxRow>

      <div className="cx-codex-proxy-head" title={t("为旧版的单个供应商单独指定 HTTP 代理；留空表示沿用系统代理。网关供应商的代理在供应商池里编辑供应商时设置。")}>
        <strong>{t("供应商独立代理（旧版供应商）")}</strong>
        <small>{t("点「编辑」到供应商编辑页里改，随「保存供应商」一起保存；当前供应商会重新应用，重启 Codex 后作用于会话请求。")}</small>
      </div>
      {proxyProfiles.length ? proxyProfiles.map((profile) => {
        const proxy = proxies[profile.id];
        const url = (proxy?.proxyUrl ?? "").trim();
        return (
          <CxRow
            detail={url || (proxy?.configured ? t("已配置独立代理（来自环境变量）") : t("未配置独立代理，沿用系统代理"))}
            key={profile.id}
            title={profile.name || t("未命名供应商")}
          >
            {openRelayProfile ? (
              <button
                aria-label={tf("编辑 {0} 的独立代理", [profile.name || t("未命名供应商")])}
                className="cx-secondary"
                onClick={() => openRelayProfile(profile.id, "proxy")}
                type="button"
              >
                {t("编辑")}
              </button>
            ) : null}
          </CxRow>
        );
            }) : (
        <CxRow detail={t("还没有可配置独立代理的供应商。")} muted title={t("供应商独立代理（旧版供应商）")} />
      )}
      </>
      )}

      <RelayEnvironmentScreen result={relayEnvironment} actions={actions} />
    </div>
  );
}

function RelayEnvironmentScreen({ result, actions }: { result: RelayEnvironmentResult | null; actions: Actions }) {
  const proxyVariables = result?.proxyEnvironment.variables ?? [];
  const proxyVariableLabels = proxyVariables.map((item) => {
    const source = item.source === "user" ? t("用户环境") : item.source === "system" ? t("系统环境") : t("进程环境");
    return tf("{0}（{1}）", [item.name, source]);
  });
  const checks = [
    {
      id: "clash-verge-tun",
      title: t("Clash Verge Rev TUN 模式"),
      passed: result ? !result.clashVergeTun.enabled : false,
      detail: result
        ? result.clashVergeTun.enabled
          ? tf("检测到 TUN 模式已开启，请在 Clash Verge Rev 中关闭。配置：{0}", [result.clashVergeTun.configPath || t("未记录路径")])
          : result.clashVergeTun.configPath
            ? tf("TUN 模式已关闭。配置：{0}", [result.clashVergeTun.configPath])
            : t("未发现 Clash Verge Rev 配置，按未开启处理。")
        : t("等待检测。"),
    },
    {
      id: "proxy-environment",
      title: t("系统代理环境变量"),
      passed: result ? proxyVariables.length === 0 : false,
      detail: result
        ? proxyVariables.length
          ? tf("检测到代理环境变量：{0}。请清理后重新启动 Codex。", [proxyVariableLabels.join(t("、"))])
          : t("未检测到 HTTP_PROXY、HTTPS_PROXY、ALL_PROXY 或 FTP_PROXY。")
        : t("等待检测。"),
    },
    {
      id: "codex-dotenv",
      title: t("Codex .env 文件"),
      passed: result ? !result.codexEnvFile.exists : false,
      detail: result
        ? result.codexEnvFile.exists
          ? tf("检测到可能干扰供应商配置的 .env 文件：{0}", [result.codexEnvFile.path])
          : tf("未发现 .env 文件：{0}", [result.codexEnvFile.path])
        : t("等待检测。"),
    },
  ];
  const failed = result ? checks.filter((check) => !check.passed) : [];

  // 一行：全部正常时只显示结论；有问题时下面逐条列出要处理的项。
  return (
    <>
      <CxRow
        detail={!result ? t("正在读取本机环境") : failed.length ? tf("{0} 项需要处理，处理后重启 Codex", [failed.length]) : t("Clash TUN、代理环境变量、.env 文件都正常")}
        icon={<CxStatusIcon tone={!result ? undefined : failed.length ? "warn" : "ok"}><ShieldCheck aria-hidden="true" /></CxStatusIcon>}
        title={t("中转站环境检测")}
        tooltip={t("直连第三方中转站时，Clash Verge Rev 的 TUN 模式、系统代理环境变量和 ~/.codex/.env 都可能让请求走错地方。")}
      >
        <button className="cx-secondary" onClick={() => void actions.refreshRelayEnvironment()} type="button">
          <RefreshCw aria-hidden="true" />
          {t("重新检测")}
        </button>
      </CxRow>
      {failed.map((check) => (
        <CxRow
          className="cx-env-issue"
          detail={check.detail}
          icon={<CxStatusIcon tone="warn"><ShieldAlert aria-hidden="true" /></CxStatusIcon>}
          key={check.id}
          title={check.title}
        />
      ))}
    </>
  );
}

/** 直连用户点过「不再提示」：不再提醒把直连供应商加入网关供应商池。 */
const DIRECT_POOL_NUDGE_KEY = "codex-plus-plus.direct-pool-nudge-dismissed";

/** 托管的网关供应商带 openai 会话身份：Codex 保留 ChatGPT 官方登录，请求经本地代理进网关。 */
function codexGatewayKeepsLogin(profile: RelayProfile | undefined): boolean {
  return /^\s*model_provider\s*=\s*["']openai["']/m.test(profile?.configContents ?? "");
}

/**
 * 「Codex 连接」页顶部的「连接方式」：Codex 要么经 Codex++ 网关（供应商池），要么直连下面某个供应商，二选一。
 * - Codex 还指向托管供应商、网关里却没有它的接入记录（降级后又升级、gateway.json 换过或坏了）：
 *   给「切回直连」，否则页面一直显示「被网关接管」却断不开；
 * - Codex 正经网关接入：网关一行「使用中」，可去管理或断开并还原；
 * - 断开过但托管供应商还在（Codex 已从网关切回直连）：网关一行给「重新接入」；
 * - 从没接入过：建议改用新版网关（这里的直连配置照旧生效）。
 */
export function LegacyRelayGatewayHint({
  form,
  onOpenGateway,
  onManageAgents,
  onDisconnect,
  onConnect,
  connecting = false,
}: {
  form: BackendSettings;
  onOpenGateway: () => void;
  onManageAgents: () => void;
  onDisconnect: () => void;
  /** 一键切到网关（沿用上次的接入设置）；不传时退回打开「Agent 接入」。 */
  onConnect?: () => void;
  connecting?: boolean;
}) {
  const managed = form.relayProfiles.find((profile) => profile.id === CODEX_GATEWAY_RELAY_ID);
  const onGateway = form.activeRelayId === CODEX_GATEWAY_RELAY_ID;
  // 网关里有没有开着的 Codex 接入记录：读共享的网关数据（和本页「供应商池」标记同一份轮询），不再单独请求；
  // 还没读到时当作未知，不误报「还指向网关托管的供应商」。
  const gatewayPayload = useGatewayPayload(15_000);
  const bound = gatewayPayload ? gatewayPayload.gateway?.bindings?.codex?.enabled === true : null;
  const orphan = onGateway && (!managed || bound === false);
  const viaGateway = onGateway && !orphan;
  const direct = form.relayProfiles.find((profile) => profile.id === form.activeRelayId && profile.id !== CODEX_GATEWAY_RELAY_ID);
  const count = codexGatewayModelCount(managed);
  // 状态标题：屏幕阅读器和悬停提示都读这一句。
  const status = orphan
    ? t("Codex 还指向网关托管的供应商")
    : viaGateway
      ? t("Codex 当前经 Codex++ 网关（新版）")
      : managed
        ? t("Codex 已从网关切回直连")
        : t("建议改用新版网关");
  const connect = onConnect ?? onManageAgents;
  // 两段式切换：经网关 ⇄ 直连。切到网关是一步；切回直连先预览要还原哪些文件，再确认。
  const modeSwitch = orphan ? null : (
    <div aria-label={t("Codex 的连接方式")} className="mg-segmented cx-connection-switch" role="radiogroup">
      <button
        aria-checked={viaGateway}
        data-active={viaGateway}
        disabled={connecting || viaGateway || !form.relayProfilesEnabled}
        onClick={connect}
        role="radio"
        title={form.relayProfilesEnabled ? t("Codex 改走 Codex++ 网关，用供应商池里的模型；新开的会话生效") : t("先在页面底部「高级」里打开供应商配置切换")}
        type="button"
      >
        {connecting ? <Loader2 className="animate-spin" /> : <Waypoints />}
        {t("经网关")}
      </button>
      <button
        aria-checked={!onGateway}
        data-active={!onGateway}
        disabled={!onGateway}
        onClick={onDisconnect}
        role="radio"
        title={t("断开网关，还原接入前的直连配置；会先列出要还原的文件，确认后才改")}
        type="button"
      >
        <Cable />
        {t("直连")}
      </button>
    </div>
  );
  // 一行说清：现在怎么连（图标 + 标题 + 一句说明），右侧是「经网关 | 直连」切换和三点菜单。
  // 另一种方式是什么、有什么好处，放在切换按钮的悬停说明里，不再单独占一行。
  const title = orphan
    ? t("Codex 还指向网关托管的供应商")
    : viaGateway
      ? t("经 Codex++ 网关")
      : direct
        ? tf("直连 · {0}", [direct.name || t("未命名供应商")])
        : t("直连");
  const detail = orphan
    ? t("网关里没有 Codex 的接入记录（可能降级后又升级过，或网关状态文件损坏），请求可能连不上。点「切回直连」切回接入前或列表里第一个直连供应商。")
    : viaGateway
      ? count
        ? tf("用供应商池里的 {0} 个模型：多 Key 轮换、按模型路由、出错自动换下一家", [count])
        : t("用供应商池里的模型：多 Key 轮换、按模型路由、出错自动换下一家")
      : t("Codex 直接请求这个供应商，不经网关；切到「经网关」后所有 Agent 共用供应商池");
  return (
    <CxCard className="cx-codex-connection">
      <CxRow
        badges={viaGateway && codexGatewayKeepsLogin(managed) ? <CxBadge>{t("保留 ChatGPT 登录")}</CxBadge> : null}
        detail={detail}
        icon={<CxStatusIcon tone={orphan ? "warn" : "ok"}>{viaGateway || orphan ? <Waypoints /> : <Cable />}</CxStatusIcon>}
        title={title}
        tooltip={status}
      >
        {orphan ? <button className="cx-primary" onClick={onDisconnect} type="button">{t("切回直连")}</button> : modeSwitch}
        <Menu>
          {viaGateway ? <MenuItem onClick={onManageAgents}>{t("管理接入")}</MenuItem> : <MenuItem onClick={onManageAgents}>{t("在「Agent 接入」里选默认模型")}</MenuItem>}
          <MenuItem onClick={onOpenGateway}>{t("打开供应商池")}</MenuItem>
          {viaGateway ? <MenuItem danger onClick={onDisconnect}>{t("断开并还原")}</MenuItem> : null}
        </Menu>
      </CxRow>
    </CxCard>
  );
}

/**
 * 直连供应商与网关供应商池的对应：哪些已经复制进池（池里的供应商带 `codex:<id>` 来源）、哪些还能复制。
 * 复制走网关「导入供应商」同一条命令，只往池里加，不改直连配置，也不切换 Codex 的连接方式。
 */
function useCodexPoolLink(actions: Actions, profileIds: string[]) {
  const payload = useGatewayPayload(15_000);
  const [importing, setImporting] = useState<string | null>(null);
  // 只看当前直连列表里还在的供应商：刚删掉、网关数据还没刷新的不算。
  const candidates = importCandidates(payload).filter((item) => (item.source === "codex" || item.source === "aggregate") && profileIds.includes(item.id));
  const stateOf = (profileId: string): "pooled" | "available" | null => {
    const item = candidates.find((candidate) => candidate.id === profileId);
    return item ? (item.imported ? "pooled" : "available") : null;
  };
  const pending = candidates.filter((item) => !item.imported);
  const importItems = async (items: ImportCandidate[], key: string) => {
    if (!items.length || importing) return;
    setImporting(key);
    try {
      const result = await invoke<GatewayResult>("import_gateway_providers", {
        items: items.map((item) => ({ source: item.source, id: item.id })),
        skipAgents: [],
      });
      publishGatewayPayload(result);
      await actions.showMessage(t("加入供应商池"), result.message, isSuccessStatus(result.status) ? "ok" : "failed");
    } catch (error) {
      await actions.showMessage(t("加入供应商池"), stringifyError(error), "failed");
    } finally {
      setImporting(null);
    }
  };
  return {
    ready: payload !== null,
    stateOf,
    pending,
    importing,
    addOne: (profileId: string) => importItems(candidates.filter((item) => item.id === profileId), profileId),
    addAll: () => importItems(pending, "all"),
  };
}
type CodexPoolLink = ReturnType<typeof useCodexPoolLink>;

/** 行右侧的「已在供应商池」标记，或一个「加入供应商池」图标按钮（批量加入在分区标题右侧）。 */
function PoolLinkControl({ link, profileId }: { link: CodexPoolLink; profileId: string }) {
  const state = link.stateOf(profileId);
  if (state === "pooled") return <CxBadge title={t("网关的供应商池里已经有它，所有接入网关的 Agent 都能用")} tone="ok">{t("已在供应商池")}</CxBadge>;
  if (state !== "available") return null;
  return (
    <button
      aria-label={t("加入供应商池")}
      className="cx-icon-button cx-pool-add"
      disabled={link.importing !== null}
      onClick={(event) => {
        event.stopPropagation();
        void link.addOne(profileId);
      }}
      title={t("加入供应商池：复制进网关，所有接入网关的 Agent 都能用；直连配置照旧保留")}
      type="button"
    >
      {link.importing === profileId ? <Loader2 className="animate-spin" /> : <Waypoints />}
    </button>
  );
}

export function RelayScreen({
  addDirectProviderRequested,
  onRequestedProviderOpened,
  settings,
  relayFiles,
  envConflicts,
  ccsProviders,
  form,
  actions,
  connection,
  advanced,
  advancedOpen = false,
  onOpenGateway,
}: {
  addDirectProviderRequested: boolean;
  onRequestedProviderOpened: () => void;
  settings: SettingsResult | null;
  relayFiles: RelayFilesResult | null;
  envConflicts: EnvConflictsResult | null;
  ccsProviders: CcsProvidersResult | null;
  form: BackendSettings;
  actions: Actions;
  /** 页面顶部的「连接方式」（经网关 / 直连）。只在列表视图显示，进编辑页不显示。 */
  connection?: ReactNode;
  /** 页面底部「高级」折叠区里的内容：目录外模型映射、供应商独立代理、环境检测。 */
  advanced?: ReactNode;
  /** 从「旧版代理设置」的旧入口进来时直接展开「高级」。 */
  advancedOpen?: boolean;
  /** 打开网关的供应商池。 */
  onOpenGateway?: () => void;
}) {
  // normalizeSettings 要解析共享 TOML、规整每个供应商，不随每次渲染重算。
  const normalized = useMemo(() => normalizeSettings(form), [form]);
  const takenOverByGateway = normalized.activeRelayId === CODEX_GATEWAY_RELAY_ID;
  // 直连供应商和网关供应商池打通：每行标出是否已在池里，没进池的可以一键复制进去。
  const poolLink = useCodexPoolLink(actions, normalized.relayProfiles.map((profile) => profile.id));
  // 直连用户点过「不再提示」就不再提醒加入供应商池。hook 必须在下面编辑页 / 目录页的提前返回之前。
  const [nudgeDismissed, setNudgeDismissed] = useState(() => {
    try { return window.localStorage.getItem(DIRECT_POOL_NUDGE_KEY) === "1"; } catch { return false; }
  });
  const dismissNudge = () => {
    setNudgeDismissed(true);
    try { window.localStorage.setItem(DIRECT_POOL_NUDGE_KEY, "1"); } catch { /* 存不了就只在本次关掉 */ }
  };
  const [detailProfileId, setDetailProfileId] = useState<string | null>(null);
  const [newProfileDraft, setNewProfileDraft] = useState<RelayProfile | null>(null);
  const [providerCatalogOpen, setProviderCatalogOpen] = useState(false);
  useEffect(() => {
    if (!addDirectProviderRequested) return;
    setNewProfileDraft(null);
    setDetailProfileId(null);
    setProviderCatalogOpen(true);
    onRequestedProviderOpened();
  }, [addDirectProviderRequested, onRequestedProviderOpened]);
  // 目录页的搜索词放在这里：从模板进入编辑页再返回时，目录页会重新挂载，搜索不能丢。
  const [providerCatalogQuery, setProviderCatalogQuery] = useState("");
  // 进入时停在当前供应商所在的分区；找不到当前供应商时，没有直连项就展示协议转换分区。
  const initialProviderView = (): ProviderViewKind => {
    const active = normalized.relayProfiles.find((profile) => profile.id === normalized.activeRelayId);
    if (active) return relayProviderView(active);
    const views = normalized.relayProfiles.map(relayProviderView);
    return !views.includes("direct") && views.includes("routing") ? "routing" : "direct";
  };
  const [providerView, setProviderView] = useState<ProviderViewKind>(initialProviderView);
  const [newAggregateMode, setNewAggregateMode] = useState<AggregateMode | null>(null);
  const [thirdPartyImportOpen, setThirdPartyImportOpen] = useState(false);
  const thirdPartyImportTriggerRef = useRef<HTMLDivElement>(null);
  const [thirdPartyImportPosition, setThirdPartyImportPosition] = useState<AnchoredMenuLayout | null>(null);
  const [ccsDbPathDraft, setCcsDbPathDraft] = useState(normalized.ccsDbPath);
  const [ccsPathSaving, setCcsPathSaving] = useState(false);
  const [ccsBindings, setCcsBindings] = useState<Record<string, string>>({});
  // 路径草稿与已保存的不一致时，列表还是旧库的：先「应用」才能导入。
  const ccsPathDirty = ccsDbPathDraft.trim() !== normalized.ccsDbPath;
  useEffect(() => setCcsDbPathDraft(normalized.ccsDbPath), [normalized.ccsDbPath]);
  const saveCcsDbPath = async (path = ccsDbPathDraft) => {
    setCcsPathSaving(true);
    try {
      const saved = await actions.saveSettingsValue({ ...normalized, ccsDbPath: path.trim() }, true);
      if (saved) await actions.refreshCcsProviders();
    } finally {
      setCcsPathSaving(false);
    }
  };
  const selectCcsDatabase = async () => {
    try {
      const selected = await open({
        title: t("选择 cc-switch 数据库"),
        multiple: false,
        directory: false,
        filters: [{ name: "SQLite", extensions: ["db", "sqlite", "sqlite3"] }],
      });
      const path = Array.isArray(selected) ? selected[0] : selected;
      if (path) {
        setCcsDbPathDraft(path);
        await saveCcsDbPath(path);
      }
    } catch (error) {
      await actions.showMessage(t("cc-switch 导入"), stringifyError(error), "failed");
    }
  };
  const detailProfile = newProfileDraft || (detailProfileId
    ? normalized.relayProfiles.find((profile) => profile.id === detailProfileId) || null
    : null);
  const isNewProfile = !!newProfileDraft;
  const saveRelaySettings = async (next: BackendSettings) => {
    return actions.saveSettingsValue(next, true);
  };
  // 拖拽排序专用保存：松手时乐观写入表单，卡片立即落位；等 save_settings 回读
  // 会让卡片先弹回原位、保存完成后再整体跳一次。详情页保存与「配置切换」开关
  // 仍走 saveRelaySettings，不参与乐观更新。
  // 连续快速拖拽共用一条回滚基线：只有链尾那次失败才回滚，且基线随链中的成功
  // 保存推进，早先的失败不会盖掉后续已落盘的结果。
  const relayReorderSeqRef = useRef(0);
  const relayReorderRollbackRef = useRef<BackendSettings | null>(null);
  const reorderRelaySettings = async (next: BackendSettings) => {
    const seq = ++relayReorderSeqRef.current;
    if (seq === 1) relayReorderRollbackRef.current = form;
    actions.previewSettingsForm(next);
    const saved = await actions.saveSettingsValue(next, true);
    if (seq === relayReorderSeqRef.current) {
      if (!saved) {
        const rollback = relayReorderRollbackRef.current;
        if (rollback) actions.previewSettingsForm(rollback);
      }
      relayReorderSeqRef.current = 0;
      relayReorderRollbackRef.current = null;
    } else if (saved) {
      relayReorderRollbackRef.current = saved;
    }
    return saved;
  };
  const createNewAggregateProfile = (mode: AggregateMode) => {
    const draft = aggregateProfileForMode(createAggregateRelayProfile(normalized), normalized, mode);
    setDetailProfileId(null);
    setNewProfileDraft(draft);
    setNewAggregateMode(mode);
    if (!normalizeAggregateConfig(draft.aggregate, aggregateMemberCandidates(normalized.relayProfiles, draft.id)).members.length) {
      void actions.showMessage(
        t("创建聚合供应商"),
        t("已打开聚合供应商详情；请先添加或完善至少 1 个 API 供应商，或登录一个官方账号，再勾选为成员。"),
        "failed",
      );
    }
  };
  const visibleProfiles = normalized.relayProfiles.filter((profile) => (
    relayProviderView(profile) === providerView
  ));
  // 分区只在首次进入时自动选择；之后尊重用户点击，避免只有协议转换供应商时
  // 「直连」分页被强制弹回、看起来像切换按钮点不动（#2470/#2481/#2495）。
  const providerViewAutoSelectedRef = useRef(false);
  const selectProviderView = (view: ProviderViewKind) => {
    providerViewAutoSelectedRef.current = true;
    setProviderView(view);
  };
  useEffect(() => {
    // 设置读回之前表单是默认值（自带一个 default 供应商），不能据此定分区。
    if (providerViewAutoSelectedRef.current) return;
    if (!settings || !normalized.relayProfiles.length) return;
    providerViewAutoSelectedRef.current = true;
    setProviderView(initialProviderView());
  }, [settings, normalized.relayProfiles, normalized.activeRelayId]);
  // 分页仅控制渲染。卡片的删除、复制和排序必须基于完整配置，避免隐藏的
  // 当前供应商被 syncLegacyRelayFields 当成不存在，改写成直连列表第一项。
  const switchRelayProfileFromList = (profileId: string) => {
    const next = syncLegacyRelayFields({ ...normalized, activeRelayId: profileId });
    void actions.switchRelayProfile(next, normalized.activeRelayId);
  };
  // 「高级」里的供应商代理列表点进来时直接跳到代理输入框。
  const [detailSection, setDetailSection] = useState<"proxy" | null>(null);
  const openRelayProfileSection = useCallback((profileId: string, section: "proxy") => {
    setNewProfileDraft(null);
    setDetailSection(section);
    setDetailProfileId(profileId);
  }, []);
  const editRelayProfile = async (profileId: string) => {
    setNewProfileDraft(null);
    setDetailSection(null);
    setDetailProfileId(
      normalized.relayProfiles.some((item) => item.id === profileId) ? profileId : null,
    );
  };
  useEffect(() => {
    if (!newProfileDraft && detailProfileId && !normalized.relayProfiles.some((profile) => profile.id === detailProfileId)) {
      setDetailProfileId(null);
    }
  }, [detailProfileId, newProfileDraft, normalized.relayProfiles]);
  useEffect(() => {
    if (!newProfileDraft && detailProfileId === normalized.activeRelayId) {
      void actions.refreshRelayFiles();
    }
  }, [detailProfileId, newProfileDraft, normalized.activeRelayId]);
  const openThirdPartyImport = () => {
    const next = !thirdPartyImportOpen;
    if (next) {
      // 菜单改为 fixed 定位：它原本挂在 .mg-content（overflow:auto）里，
      // 面板较矮时后半截会被裁掉（ccswitch 导入按钮、刷新列表都看不到）。
      // 高度必须按触发按钮的位置收窄，否则 cc-switch 供应商一多（几十行），
      // 菜单底部会超出窗口下沿，最后那排按钮永远点不到（issue #1906）。
      const rect = thirdPartyImportTriggerRef.current?.getBoundingClientRect();
      setThirdPartyImportPosition(
        rect
          ? resolveAnchoredMenuLayout(rect, { width: window.innerWidth, height: window.innerHeight })
          : null,
      );
    }
    setThirdPartyImportOpen(next);
    if (!ccsProviders) void actions.refreshCcsProviders(true);
  };
  useEffect(() => {
    if (!thirdPartyImportOpen) return;
    const close = () => setThirdPartyImportOpen(false);
    // 菜单挂在 .mg-page（overflow:hidden）里，只能自己判定外点，不然点空白处收不起来。
    const handlePointerDown = (event: PointerEvent) => {
      if (thirdPartyImportTriggerRef.current?.contains(event.target as Node)) return;
      close();
    };
    document.addEventListener("pointerdown", handlePointerDown, true);
    window.addEventListener("resize", close);
    // capture 监听内层 .mg-content 的滚动：页面滚了菜单会跟触发按钮错位，得收起。
    // 但供应商列表自己也会滚（滚轮、拖滚动条），scroll 事件同样会冒到 window 的
    // capture 阶段——不排除掉它，列表一动菜单就关，等于列表不能滚（issue #1906）。
    const handleScroll = (event: Event) => {
      if (thirdPartyImportTriggerRef.current?.contains(event.target as Node)) return;
      close();
    };
    window.addEventListener("scroll", handleScroll, { capture: true, passive: true });
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown, true);
      window.removeEventListener("resize", close);
      window.removeEventListener("scroll", handleScroll, { capture: true });
    };
  }, [thirdPartyImportOpen]);
  if (detailProfile) {
    return (
      <RelayProfileDetail
        profile={detailProfile}
        relayFiles={!isNewProfile && detailProfile.id === normalized.activeRelayId ? relayFiles : null}
        form={normalized}
        isNew={isNewProfile}
        initialAggregateMode={isNewProfile ? newAggregateMode : null}
        initialSection={isNewProfile ? null : detailSection}
        // 从目录页选模板进来时目录页状态还开着：返回只清掉草稿，回到目录页继续挑。
        onBack={() => {
          setNewProfileDraft(null);
          setNewAggregateMode(null);
          setDetailProfileId(null);
        }}
        onFormChange={saveRelaySettings}
        // 保存后回到供应商列表，目录页一并关掉。
        onSaved={() => {
          setNewProfileDraft(null);
          setNewAggregateMode(null);
          setDetailProfileId(null);
          setProviderCatalogOpen(false);
          setProviderCatalogQuery("");
        }}
        actions={actions}
      />
    );
  }

  if (providerCatalogOpen) {
    return (
      <ProviderCatalogScreen
        form={normalized}
        query={providerCatalogQuery}
        onQueryChange={setProviderCatalogQuery}
        onBack={() => {
          setProviderCatalogOpen(false);
          setProviderCatalogQuery("");
        }}
        // 不关目录页：编辑页优先渲染，从编辑页返回时回到这里而不是供应商列表。
        onSelect={(profile) => {
          setNewProfileDraft(profile);
          setNewAggregateMode(null);
        }}
      />
    );
  }

  const directProfiles = normalized.relayProfiles.filter((profile) => profile.id !== CODEX_GATEWAY_RELAY_ID);
  const openCatalog = () => { setNewProfileDraft(null); setDetailProfileId(null); setProviderCatalogOpen(true); };
  const viewCount = (view: ProviderViewKind) => normalized.relayProfiles.filter((profile) => relayProviderView(profile) === view).length;
  const viewTip: Record<ProviderViewKind, string> = {
    direct: t("直接访问 Responses API 供应商：只写入 Base URL 与 Key，不做协议转换。"),
    modelAggregation: t("Codex 专用的小网关：把几家的模型合并到 Codex 的模型列表，选哪个模型就发到哪家。网关的供应商池默认就是这样分发，而且所有 Agent 都能用。"),
    routing: t("供应商只支持 Chat Completions（或配置了模型路由）：由本地协议代理转换后转发。"),
    failover: t("Codex 专用的小网关：几家组成一个池，出错换下一家、按会话或请求轮换、按权重分。网关的「路由」里用模型组做同样的事，而且所有 Agent 都能用。"),
  };
  const emptyView: Record<ProviderViewKind, { title: string; detail: string; action: string; run: () => void }> = {
    direct: { title: t("这个分页还没有直连供应商"), detail: t("直连分页只显示 Responses 直连且未配置模型路由的供应商；其他供应商在其余分页，配置不会丢失。"), action: t("添加供应商"), run: openCatalog },
    routing: { title: t("这个分页还没有需协议转换的供应商"), detail: t("Chat Completions 协议或配置了模型路由的供应商会显示在这里；其他供应商在其余分页。"), action: t("添加供应商"), run: openCatalog },
    // 新用户优先推荐网关（所有 Agent 都能用）；只想让 Codex 用的，仍可以在这里建。
    failover: { title: t("还没有故障转移配置"), detail: t("推荐用网关「路由」里的模型组：所有 Agent 都能用。只想给 Codex 直连用，在这里建。"), action: t("创建故障转移配置"), run: () => createNewAggregateProfile("provider") },
    modelAggregation: { title: t("还没有模型聚合配置"), detail: t("推荐用网关的供应商池：本身就按模型分发，所有 Agent 都能用。只想给 Codex 直连用，在这里建。"), action: t("创建模型聚合配置"), run: () => createNewAggregateProfile("model") },
  };
  // 「加入供应商池」的提示：还有直连供应商没进池就提一次；网关接管期间同样可以迁移。
  // 不想换到网关的直连用户可以「不再提示」（只关提示，每行的「加入供应商池」照旧在）；已经在用网关的总是提示。
  // 收在「直连供应商」标题右侧的一行：几个还没进池 · 全部加入 · ×（不再提示）。
  const poolHint = normalized.relayProfilesEnabled && poolLink.pending.length && (takenOverByGateway || !nudgeDismissed) ? (
    <span className="cx-pool-hint" title={t("加入后 Codex、Claude Code 等所有接入网关的 Agent 都能用，支持多 Key 轮换和故障转移；直连配置照旧保留。")}>
      <Waypoints aria-hidden="true" />
      {tf("{0} 个还没加入供应商池", [poolLink.pending.length])}
      <button className="cx-secondary" disabled={poolLink.importing !== null} onClick={() => void poolLink.addAll()} type="button">
        {poolLink.importing === "all" ? <Loader2 className="animate-spin" /> : null}
        {t("全部加入")}
      </button>
      {takenOverByGateway ? null : (
        <button aria-label={t("不再提示")} className="cx-icon-button" onClick={dismissNudge} title={t("继续只用直连；以后想换，每个供应商右侧的「加入供应商池」还在")} type="button">
          <X />
        </button>
      )}
    </span>
  ) : null;

  return (
    <div className="mg-page relay-management-page cx-page cx-codex-connection-page">
      <PageHeader
        icon={<KeyRound className="h-5 w-5" />}
        title={t("连接与供应商")}
        actions={(
          <>
            {/* 「从第三方导入」接管期间也保留（#2801）：导入只写直连供应商列表、不动 Codex 配置，
                导进来的供应商可以直接多开，也能「加入供应商池」给网关用。 */}
            <div className="third-party-import" ref={thirdPartyImportTriggerRef}>
              <button className="cx-secondary" onClick={openThirdPartyImport} type="button">
                <Download />
                {t("从第三方导入")}
              </button>
              {thirdPartyImportOpen ? (
                <div
                  className="third-party-import-menu"
                  style={thirdPartyImportPosition
                    ? {
                      position: "fixed",
                      top: thirdPartyImportPosition.top,
                      right: thirdPartyImportPosition.right,
                      maxHeight: thirdPartyImportPosition.maxHeight,
                    }
                    : undefined}
                >
                  <div className="third-party-import-head">
                    <strong>{t("从 cc-switch 导入")}</strong>
                    <small>{t("只读取 cc-switch 的数据库，不会改动它。")}</small>
                  </div>
                  <div className="third-party-import-source">
                    <div className="third-party-import-path">
                      <Input
                        aria-label={t("cc-switch 数据库路径")}
                        onChange={(event) => setCcsDbPathDraft(event.currentTarget.value)}
                        placeholder={t("默认 ~/.cc-switch/cc-switch.db")}
                        value={ccsDbPathDraft}
                      />
                      <Button disabled={ccsPathSaving} onClick={() => void selectCcsDatabase()} size="sm" title={t("选择数据库")} variant="secondary">
                        <FolderOpen className="h-4 w-4" />
                        {t("选择")}
                      </Button>
                      {ccsPathDirty ? (
                        <Button disabled={ccsPathSaving} onClick={() => void saveCcsDbPath()} size="sm">
                          {ccsPathSaving ? t("应用中") : t("应用")}
                        </Button>
                      ) : null}
                    </div>
                    {ccsProviders && isSuccessStatus(ccsProviders.status) ? (
                      <small className="third-party-import-meta" title={ccsProviders.dbPath}>{t("实际来源")}：{ccsProviders.dbPath}</small>
                    ) : null}
                    {ccsProviders?.fallbackReason ? <small className="third-party-import-warning">{ccsProviders.fallbackReason}</small> : null}
                  </div>
                  {/* 供应商列表单独滚动：cc-switch 里几十个供应商时，底部按钮仍然可见可点（issue #1906）。 */}
                  <div className="third-party-import-list">
                    {ccsProviders?.providers.length ? (
                      <div className="third-party-import-list-head">
                        <span>{tf("Codex 供应商 {0} 个", [ccsProviders.providers.length])}</span>
                        <span>{t("导入为")}</span>
                      </div>
                    ) : (
                      <p className="third-party-import-empty">{ccsProviderSummary(ccsProviders)}</p>
                    )}
                    {ccsProviders?.providers.map((provider) => (
                      <label className="third-party-import-row" key={provider.sourceId}>
                        <span className="third-party-import-name">
                          <strong title={provider.name}>{provider.name}</strong>
                          <small title={provider.baseUrl}>{urlHost(provider.baseUrl) || t("未填写地址")}</small>
                        </span>
                        <AppSelect
                          ariaLabel={tf("{0} 导入为", [provider.name])}
                          menuWidth={240}
                          onChange={(next) => setCcsBindings((current) => ({ ...current, [provider.sourceId]: next }))}
                          options={[
                            { value: "", label: t("新增（同名自动关联）") },
                            ...normalized.relayProfiles.map((profile) => ({ value: profile.id, label: `${t("关联已有")}：${profile.name}` })),
                          ]}
                          size="sm"
                          value={ccsBindings[provider.sourceId] || ""}
                        />
                      </label>
                    ))}
                  </div>
                  <div className="third-party-import-actions">
                    {ccsPathDirty ? <small className="third-party-import-warning">{t("路径已修改，点「应用」读取新数据库后再导入。")}</small> : null}
                    <div className="third-party-import-footer">
                      <Button aria-label={t("刷新列表")} onClick={() => void actions.refreshCcsProviders()} size="icon" title={t("刷新列表")} variant="ghost">
                        <RefreshCw className="h-4 w-4" />
                      </Button>
                      <Button
                        disabled={ccsPathSaving || ccsPathDirty || !ccsProviders?.providers.length}
                        onClick={() => {
                          setThirdPartyImportOpen(false);
                          void actions.importCcsProviders(ccsBindings);
                        }}
                      >
                        <Download className="h-4 w-4" />
                        {ccsProviders?.providers.length ? tf("导入 {0} 个供应商", [ccsProviders.providers.length]) : t("导入")}
                      </Button>
                    </div>
                  </div>
                </div>
              ) : null}
            </div>
              <button className="cx-primary" onClick={openCatalog} type="button">
                <Plus />
                {takenOverByGateway ? t("添加直连供应商") : t("添加供应商")}
              </button>
          </>
        )}
      />
      {connection}
      <NativeAuthControl onRestart={() => actions.restart()} />
      <EnvConflictNotice envConflicts={envConflicts} actions={actions} />
      <SshRemoteLoopbackNotice notice={envConflicts?.sshRemoteLoopback ?? null} />
      {!normalized.relayProfilesEnabled ? (
        <CxBanner
          detail={takenOverByGateway
            ? t("Codex++ 无法重新接入 Codex。重新开启只恢复配置写入能力，不会切换直连供应商或断开当前网关。")
            : t("Codex++ 不会写入 Codex 的 config.toml / auth.json，直连供应商列表已隐藏。")}
          icon={<PowerOff />}
          title={t("供应商配置切换已关闭")}
          tone="warn"
        >
          <button className="cx-primary" onClick={() => void saveRelaySettings({ ...normalized, relayProfilesEnabled: true })} type="button">{t("打开配置切换")}</button>
        </CxBanner>
      ) : null}
      {normalized.relayProfilesEnabled && takenOverByGateway ? (
        <CxCard className="cx-takeover">
          <CxEmpty
            detail={t("可以先添加或编辑直连档案。保存后继续使用网关，点上方「直连」预览并确认还原。")}
            title={t("Codex 正由 Codex++ 网关接管")}
          >
            {onOpenGateway ? <button className="cx-secondary" onClick={onOpenGateway} type="button">{t("打开供应商池")}</button> : null}
          </CxEmpty>
          {directProfiles.map((profile) => (
            <CxRow key={profile.id} title={profile.name || t("未命名供应商")} detail={relayProfileModeLabel(profile)}>
              <button className="cx-secondary" onClick={() => void editRelayProfile(profile.id)} type="button">{t("编辑档案")}</button>
            </CxRow>
          ))}
        </CxCard>
      ) : null}
      {/* 总开关关闭时供应商切换不会写入 Codex：分页和列表都没有意义，整块隐藏，只留上方提示条。 */}
      {normalized.relayProfilesEnabled && !takenOverByGateway ? (
        <CxSection
          aside={(
            <>
              {poolHint}
              {(providerView === "failover" || providerView === "modelAggregation") && visibleProfiles.length ? (
                <button className="cx-secondary" onClick={() => createNewAggregateProfile(providerView === "failover" ? "provider" : "model")} type="button">
                  <Plus />
                  {providerView === "failover" ? t("新建故障转移") : t("新建模型聚合")}
                </button>
              ) : null}
            </>
          )}
          count={directProfiles.length}
          title={t("直连供应商")}
        >
          <CxFilters
            items={(["direct", "modelAggregation", "routing", "failover"] as ProviderViewKind[]).map((view) => ({
              id: view,
              label: <span title={viewTip[view]}>{/* 「直连」和分区标题「直连供应商」撞名：按是否要协议转换来叫，和「需协议转换」对照。 */}{view === "direct" ? t("无需转换") : view === "modelAggregation" ? t("模型聚合") : view === "routing" ? t("需协议转换") : t("故障转移")}</span>,
              count: viewCount(view),
            }))}
            label={t("供应商配置模式")}
            onChange={selectProviderView}
            value={providerView}
          />
          {(providerView === "modelAggregation" || providerView === "failover") && visibleProfiles.length ? (
            <p className="cx-aggregate-note">
              <Waypoints aria-hidden="true" />
              <span>{providerView === "modelAggregation"
                ? t("模型聚合是 Codex 专用的小网关。网关的供应商池本身就按模型分发，所有 Agent 都能用；在下面点「加入供应商池」，成员就进了池子。")
                : t("故障转移是 Codex 专用的小网关。网关「路由」里的模型组做的是同一件事，所有 Agent 都能用；在下面点「加入供应商池」，会建好同名模型组。")}</span>
              {onOpenGateway ? <button className="cx-link" onClick={onOpenGateway} type="button">{t("打开供应商池")}</button> : null}
            </p>
          ) : null}
          {!visibleProfiles.length ? (
            <CxCard>
              <CxEmpty detail={emptyView[providerView].detail} title={emptyView[providerView].title}>
                <button className="cx-secondary" onClick={emptyView[providerView].run} type="button">{emptyView[providerView].action}</button>
              </CxEmpty>
            </CxCard>
          ) : null}
          <RelayProfileList
            form={normalized}
            visibleProfiles={visibleProfiles}
            onEdit={(profileId) => void editRelayProfile(profileId)}
            onFormChange={saveRelaySettings}
            onReorder={reorderRelaySettings}
            onSwitchProfile={switchRelayProfileFromList}
            disabled={actions.relaySwitching}
            poolLink={poolLink}
            actions={actions}
          />
        </CxSection>
      ) : null}
      {/* 「切换时保留」对直连和网关都生效（网关重写 Codex 配置也走同一条切换流程），
          所以放在两种模式都看得到的地方；以前藏在直连供应商编辑的「通用配置文件」里，网关用户找不到（#3115）。 */}
      {normalized.relayProfilesEnabled ? (
        <CxSection>
          <CxCard>
            <CxRow
              className="cx-preserve-row"
              detail={t("切换供应商或重启 Codex 时，保留你在 config.toml 里改过的 MCP、项目信任和通知设置。")}
              // 完整规则放悬停：哪些按实时文件保留、哪些始终按目标重写。
              tooltip={`${t("打开后，切换供应商、接入或重写网关、重启 Codex 时：实时 config.toml 里改过的 MCP 服务器（命令、参数、env、超时）以实时文件为准，缺失的项目信任设置和通知命令从实时文件补回。直连和网关都适用。")}\n\n${t("始终按目标重写的供应商设置：model_provider、model、供应商地址与凭据、模型目录。不论开关都保留：Codex++ 没有托管的 MCP / 插件条目、[[skills.config]] 技能启停；网关模式下 developer_instructions 等根级个人设置也会跟着走。")}`}
              title={t("切换时保留实时配置里的改动")}
            >
              <label className="switch-row compact relay-master-switch relay-master-switch-compact">
                <input
                  aria-label={t("切换时保留实时配置里的改动")}
                  checked={normalized.relayPreserveNonproviderSettings === true}
                  onChange={(event) => {
                    const next = { ...normalized, relayPreserveNonproviderSettings: event.currentTarget.checked };
                    void saveRelaySettings(next);
                  }}
                  type="checkbox"
                />
                <span>{t("保留")}</span>
                <ToggleVisual />
              </label>
            </CxRow>
          </CxCard>
        </CxSection>
      ) : null}
      {/* 高级：配置切换总开关、目录外模型映射、供应商独立代理、环境检测（原「旧版代理设置」页）。
          都是直连专用：被网关接管时整块不显示；配置切换被关掉时仍显示，好把它重新打开。 */}
      {takenOverByGateway && normalized.relayProfilesEnabled ? null : (
      <CxSection>
        <CxCard>
          <details className="cx-set-disclosure cx-set-disclosure-first cx-codex-advanced" id="codex-connection-advanced" open={advancedOpen || undefined}>
            <summary>
              {t("高级")}
              <span className="cx-set-summary-hint">{normalized.relayProfilesEnabled ? t("配置切换已开启") : t("配置切换已关闭")}</span>
            </summary>
            <CxRow
              detail={t("关闭后 Codex++ 不再写入 Codex 的 config.toml / auth.json；启动 Codex 时始终不会自动改这些文件。")}
              title={t("供应商配置切换")}
              tooltip={takenOverByGateway ? t("重新开启配置切换只恢复 Codex++ 写入 Codex 配置的能力，不会切换直连供应商或断开当前网关。") : undefined}
            >
              <label className="switch-row compact relay-master-switch relay-master-switch-compact">
                <input
                  aria-label={t("启用供应商配置切换")}
                  checked={normalized.relayProfilesEnabled}
                  onChange={(event) => {
                    const next = { ...normalized, relayProfilesEnabled: event.currentTarget.checked };
                    void saveRelaySettings(next);
                  }}
                  type="checkbox"
                />
                <span>{t("配置切换")}</span>
                <ToggleVisual />
              </label>
            </CxRow>
            <RelayProfileLinkContext.Provider value={openRelayProfileSection}>
              {advanced}
            </RelayProfileLinkContext.Provider>
          </details>
        </CxCard>
      </CxSection>
      )}
    </div>
  );
}

function ProviderCatalogScreen({
  form,
  query,
  onQueryChange: setQuery,
  onBack,
  onSelect,
}: {
  form: BackendSettings;
  query: string;
  onQueryChange: (value: string) => void;
  onBack: () => void;
  onSelect: (profile: RelayProfile) => void;
}) {
  const categories: Array<{ id: ProviderPreset["category"]; label: string }> = [
    { id: "official", label: t("账号登录") },
    { id: "cn_official", label: t("模型厂商") },
    { id: "aggregator", label: t("聚合/中转") },
    { id: "third_party", label: t("第三方平台") },
  ];
  // 赞助商排在「聚合/中转」最前面。
  const sponsors = useSponsorPresets();
  const sponsorIds = new Set(sponsors.map((preset) => preset.id));
  const filteredPresets = [...sponsors, ...PRESETS.filter((preset) => !sponsorIds.has(preset.id))].filter((preset) => {
    const haystack = `${preset.name} ${preset.baseUrl} ${preset.model}`.toLowerCase();
    return !query.trim() || haystack.includes(query.trim().toLowerCase());
  });
  const selectPreset = (preset?: ProviderPreset) => {
    const draft = createRelayProfile(form);
    const patch = preset ? createPresetPatch(preset) : { name: t("自定义供应商") };
    onSelect(applyRelayProfilePatchToFiles(draft, patch as unknown as Partial<RelayProfile>, { allowGenerateFiles: true }));
  };
  return (
    <div className="provider-catalog-screen provider-add-simple">
      <header className="provider-catalog-header" data-tauri-drag-region>
        <Button onClick={onBack} size="icon" variant="ghost" title={t("返回供应商列表")}>
          <ArrowLeft className="h-5 w-5" />
        </Button>
        <h2>{t("添加新供应商")}</h2>
        <span>Codex</span>
      </header>
      <main className="provider-add-simple-body provider-add-reference-layout">
        <div className="provider-add-search">
          <Search className="h-4 w-4" />
          <Input
            aria-label={t("搜索供应商")}
            onChange={(event) => setQuery(event.currentTarget.value)}
            placeholder={t("搜索名称或网址，例如 kimi、智谱、openrouter")}
            value={query}
          />
        </div>
        <button className="provider-add-custom" onClick={() => selectPreset()} type="button">
          <span className="provider-add-icon"><Sparkles className="h-4 w-4" /></span>
          <span className="provider-add-text">
            <strong>{t("自定义配置")}</strong>
            <small>{t("手动填写请求地址和 API Key，适合中转站和自建服务")}</small>
          </span>
          <ChevronRight className="provider-add-chevron h-4 w-4" />
        </button>
        {categories.map((category) => {
          const presets = filteredPresets.filter((preset) => preset.category === category.id);
          if (!presets.length) return null;
          return (
            <section className="provider-add-section" key={category.id}>
              <h3>{category.label}<span>{presets.length}</span></h3>
              <div className="provider-add-results">
                {presets.map((preset) => (
                  <button className="provider-add-result" key={preset.id} onClick={() => selectPreset(preset)} title={preset.baseUrl} type="button">
                    <span className="provider-add-icon">{providerInitial(preset.name)}</span>
                    <span className="provider-add-text">
                      <strong>{preset.name}</strong>
                      <small>{urlHost(preset.websiteUrl || preset.baseUrl)}</small>
                    </span>
                    <span className="provider-add-protocol">{preset.protocol === "chatCompletions" ? "Chat" : "Responses"}</span>
                  </button>
                ))}
              </div>
            </section>
          );
        })}
        {!filteredPresets.length ? <div className="provider-add-empty">{tf("没有匹配「{0}」的预设，可以用上面的「自定义配置」手动填写。", [query.trim()])}</div> : null}
        <p className="provider-add-hint">{t("选择供应商后会进入编辑页；API Key 和本地配置只在保存时写入当前 Codex。")}</p>
      </main>
    </div>
  );
}

function EnvConflictNotice({
  envConflicts,
  actions,
}: {
  envConflicts: EnvConflictsResult | null;
  actions: Actions;
}) {
  const conflicts = envConflicts?.conflicts ?? [];
  if (!conflicts.length) return null;
  const names = Array.from(new Set(conflicts.map((conflict) => conflict.name))).sort();
  return (
    <div className="env-conflict-notice">
      <div className="env-conflict-icon">
        <ShieldAlert className="h-4 w-4" />
      </div>
      <div className="env-conflict-body">
        <strong>{t("检测到 OPENAI 环境变量")}</strong>
        <p>{t("这些变量可能覆盖当前供应商写入的 config.toml / auth.json；CODEX_HOME 不会被清理。")}</p>
        <div className="env-conflict-tags">
          {conflicts.map((conflict) => (
            <span key={`${conflict.source}-${conflict.name}`}>
              {conflict.name}
              <small>{envConflictSourceLabel(conflict.source)}</small>
            </span>
          ))}
        </div>
      </div>
      <div className="env-conflict-actions">
        <Button onClick={() => void actions.removeEnvConflicts(names)} size="sm">
          <Trash2 className="h-4 w-4" />
          {t("删除")}
        </Button>
        <Button onClick={() => void actions.refreshEnvConflicts(false)} size="sm" variant="secondary">
          <RefreshCw className="h-4 w-4" />
          {t("检测")}
        </Button>
      </div>
    </div>
  );
}

const SSH_REMOTE_LOOPBACK_DISMISS_KEY = "codex-plus-ssh-remote-loopback-dismissed";

// Codex++ 不往 SSH 远端写配置；远端 Codex 若沿用本机 127.0.0.1 中转地址会连到服务器自己（issue #1195）。
// 这里只提示 ssh -R 反向转发的做法。「知道了」按端口 + 远端记住，组合变了再提示。
function SshRemoteLoopbackNotice({ notice }: { notice: SshRemoteLoopbackNoticeData | null }) {
  const signature = notice
    ? `${notice.ports.join(",")}|${notice.remotes.map((remote) => remote.label).join(",")}`
    : "";
  const [dismissed, setDismissed] = useState<string>(() => {
    try { return window.localStorage.getItem(SSH_REMOTE_LOOPBACK_DISMISS_KEY) ?? ""; } catch { return ""; }
  });
  if (!notice || !notice.remotes.length || dismissed === signature) return null;
  const dismiss = () => {
    setDismissed(signature);
    try { window.localStorage.setItem(SSH_REMOTE_LOOPBACK_DISMISS_KEY, signature); } catch { /* 存不了就只影响本次 */ }
  };
  return (
    <div className="env-conflict-notice ssh-remote-loopback-notice" role="note">
      <div className="env-conflict-icon">
        <ShieldAlert className="h-4 w-4" />
      </div>
      <div className="env-conflict-body">
        <strong>{t("SSH 远端连不上本机中转地址")}</strong>
        <p>{t("当前中转地址是本机回环地址，在 SSH 服务器上它指向服务器自己。Codex++ 不会往远端写配置；如果远端 Codex 用的也是这个地址，连接时需要把端口反向转发回本机（本机 Codex++ 要保持运行），或在远端单独配置可访问的上游地址。")}</p>
        <div className="env-conflict-tags">
          {notice.loopbackEndpoints.map((endpoint) => (
            <span key={endpoint.key}>
              {endpoint.url}
              <small>{endpoint.key}</small>
            </span>
          ))}
        </div>
        {notice.remotes.map((remote) => (
          <div className="ssh-remote-loopback-remote" key={remote.label}>
            <p>{tf("连接 {0} 时带上：", [remote.label])}</p>
            <code>{remote.forwardCommand}</code>
            <p>{t("或写进 ~/.ssh/config 对应 Host 下：")}</p>
            <code>{remote.sshConfigLines.join("\n")}</code>
          </div>
        ))}
      </div>
      <div className="env-conflict-actions">
        <Button onClick={dismiss} size="sm" variant="secondary">
          {t("知道了")}
        </Button>
      </div>
    </div>
  );
}

function envConflictSourceLabel(source: string): string {
  if (source === "process") return t("当前进程");
  if (source === "user") return t("用户环境");
  return source || t("环境变量");
}

function RelayProfileList({
  form,
  visibleProfiles = form.relayProfiles,
  onFormChange,
  onReorder,
  onEdit,
  onSwitchProfile,
  disabled = false,
  poolLink,
  actions,
}: {
  form: BackendSettings;
  /** 和网关供应商池的对应：每张卡片右侧显示「已在供应商池 / 加入供应商池」。 */
  poolLink?: CodexPoolLink;
  visibleProfiles?: RelayProfile[];
  onFormChange: (value: BackendSettings) => void;
  /// 拖拽排序落手专用：只走乐观更新路径；复制、删除等表单改动仍走 onFormChange。
  onReorder: (value: BackendSettings) => void;
  onEdit: (id: string) => void;
  onSwitchProfile: (id: string) => void;
  disabled?: boolean;
  actions: Actions;
}) {
  const sensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: { distance: 8 },
    }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    }),
  );
  const handleDragEnd = (event: DragEndEvent) => {
    const { active, over } = event;
    if (!over || active.id === over.id) return;
    const next = reorderRelayProfiles(form, String(active.id), String(over.id));
    if (next !== form) onReorder(next);
  };
  return (
    <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={handleDragEnd}>
      <SortableContext items={visibleProfiles.map((profile) => profile.id)} strategy={verticalListSortingStrategy}>
        <div className="relay-profile-list">
          {visibleProfiles.map((profile, index) => (
            <SortableRelayProfileCard
              poolLink={poolLink}
              actions={actions}
              form={form}
              index={index}
              key={profile.id}
              onEdit={onEdit}
              onFormChange={onFormChange}
              onSwitchProfile={onSwitchProfile}
              disabled={disabled}
              profile={profile}
            />
          ))}
        </div>
      </SortableContext>
    </DndContext>
  );
}

function SortableRelayProfileCard({
  form,
  profile,
  index,
  onFormChange,
  onEdit,
  onSwitchProfile,
  disabled = false,
  poolLink,
  actions,
}: {
  form: BackendSettings;
  poolLink?: CodexPoolLink;
  profile: RelayProfile;
  index: number;
  onFormChange: (value: BackendSettings) => void;
  onEdit: (id: string) => void;
  onSwitchProfile: (id: string) => void;
  disabled?: boolean;
  actions: Actions;
}) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id: profile.id });
  const active = profile.id === form.activeRelayId;
  // 网关托管的供应商：由「Agent 接入」管理，这里只能「使用」（重新接入）。
  const gatewayManaged = profile.id === CODEX_GATEWAY_RELAY_ID;
  const billing = useRelayBillingProbe(
    !gatewayManaged && relayBillingProbeEligible(profile) ? relayBillingCacheKey(profile) : "",
    () => invoke<CommandResult<RelayBillingProbe>>("probe_relay_billing", { profile }),
  );
  const style: CSSProperties = {
    transform: CSS.Transform.toString(transform),
    // 拖动中跟手、松手时 200ms 滑到新位置。
    transition: isDragging ? "none" : transition ?? "transform 200ms ease",
  };

  return (
    <div
      className={`relay-profile-card ${active ? "active" : ""} ${isDragging ? "dragging" : ""}`}
      data-gateway-managed={gatewayManaged || undefined}
      data-relay-profile-id={profile.id}
      key={profile.id}
      onKeyDown={(event) => {
        if (event.key === "Enter" && !gatewayManaged) onEdit(profile.id);
      }}
      // 点整行打开编辑（按钮、菜单自己拦住点击）。
      onClick={(event) => {
        if (gatewayManaged || (event.target as HTMLElement).closest("button, a, input, [role=menu]")) return;
        onEdit(profile.id);
      }}
      ref={setNodeRef}
      style={style}
      tabIndex={0}
    >
      <button
        aria-label={t("拖动排序")}
        className="relay-drag"
        title={t("拖动排序")}
        type="button"
        {...attributes}
        {...listeners}
      >
        <GripVertical className="h-4 w-4" />
      </button>
      <span className="relay-index" title={profile.name || t("未命名供应商")}>
        {providerInitial(profile.name)}
      </span>
      <span className="relay-summary">
        {/* 参照 CC Switch：接入方式、非 Responses 协议做成名字旁的标签，第二行只放地址。 */}
        <strong>
          <span className="relay-summary-name">{profile.name || t("未命名供应商")}</span>
          {gatewayManaged ? (
            <>
              <span className="relay-tag" data-tone="gateway">{t("新版网关")}</span>
              {codexGatewayKeepsLogin(profile) ? <span className="relay-tag" data-tone="official">{t("ChatGPT 登录")}</span> : null}
            </>
          ) : (
            <span className="relay-tag" data-tone={relayProfileTagTone(profile)}>{relayProfileModeLabel(profile)}</span>
          )}
          {!gatewayManaged && !isAggregateRelayProfile(profile) && profile.protocol !== "responses" ? (
            <span className="relay-tag" data-tone="protocol">{relayProtocolShortLabel(profile.protocol)}</span>
          ) : null}
          {active ? <span className="relay-active-badge">{t("使用中")}</span> : null}
          {!billing.probe && billing.settled ? (
            <RelayBillingRetry failure={billing.failure} loading={billing.loading} onRefresh={billing.refresh} />
          ) : null}
        </strong>
        <small>
          {gatewayManaged
            ? codexGatewayModelCount(profile)
              ? tf("经 Codex++ 本地网关，用供应商池里的 {0} 个模型；在「Agent 接入」里管理", [codexGatewayModelCount(profile)])
              : t("经 Codex++ 本地网关，用供应商池里的模型；在「Agent 接入」里管理")
            : profile.relayMode === "official"
            ? profile.officialMixApiKey
              ? profile.baseUrl || t("未填写 URL")
              : t("使用 Codex 当前登录的 ChatGPT 账号")
            : relayProfileConfigBrief(profile)}
        </small>
        {billing.probe ? (
          <RelayBillingLine failure={billing.failure} loading={billing.loading} onRefresh={billing.refresh} probe={billing.probe} />
        ) : null}
      </span>
      <span className="relay-card-actions">
        <Button
          className={`relay-use-button ${active ? "active" : ""}`}
          disabled={disabled}
          onClick={(event) => {
            event.stopPropagation();
            if (disabled) return;
            onSwitchProfile(profile.id);
          }}
          size="sm"
          title={
            disabled
              ? t("供应商切换中")
              : !form.relayProfilesEnabled
                ? t("供应商配置总开关已关闭，点击后按提示开启")
                : active
                  ? t("当前正在使用")
                  : gatewayManaged
                    ? t("重新接入 Codex++ 网关")
                    : t("设为当前")
          }
          variant={active ? "secondary" : "outline"}
        >
          <CheckCircle2 className="h-4 w-4" />
          {active ? t("使用中") : t("使用")}
        </Button>
        {gatewayManaged || !poolLink ? null : <PoolLinkControl link={poolLink} profileId={profile.id} />}
        {/* 次要操作收进三点菜单（原来是一排五个图标按钮）：多开、测试、编辑、复制、删除。点整行也是编辑。 */}
        {gatewayManaged ? null : (
        <span className="relay-card-extra" onClick={(event) => event.stopPropagation()}>
          <Menu label={t("供应商操作")}>
            <MenuItem onClick={() => onEdit(profile.id)}>{t("编辑")}</MenuItem>
            <MenuItem
              disabled={isAggregateRelayProfile(profile)}
              onClick={() => { if (!isAggregateRelayProfile(profile)) void actions.testRelayProfile(profile); }}
            >
              <span title={isAggregateRelayProfile(profile) ? t("聚合供应商会在真实对话中轮转成员，请测试成员供应商") : t("发送 hi 测试")}>{t("测试")}</span>
            </MenuItem>
            <MenuItem
              disabled={!multiLaunchSupported(profile) || actions.multiLaunchPending === profile.id}
              onClick={() => { if (multiLaunchSupported(profile)) void actions.launchMultiInstance(profile); }}
            >
              <span title={isAggregateRelayProfile(profile)
                ? t("聚合供应商暂不支持独立多开")
                : multiLaunchSupported(profile)
                  ? t("启动独立 Codex 实例")
                  : t("仅支持单一 Responses 或 Chat Completions 供应商，且不能配置模型路由")}>{t("用这个供应商多开")}</span>
            </MenuItem>
            <MenuItem onClick={() => onFormChange(duplicateRelayProfile(form, profile.id))}>{t("复制")}</MenuItem>
            <MenuItem
              danger
              disabled={form.relayProfiles.length <= 1}
              onClick={() => {
                void confirmRemoveRelayProfile(form, profile, actions, "relay").then((next) => {
                  if (next) onFormChange(next);
                });
              }}
            >
              {t("删除供应商")}
            </MenuItem>
          </Menu>
        </span>
        )}
      </span>
    </div>
  );
}

/// 网址只显示主机名；解析不了就原样返回。
function urlHost(value: string | undefined): string {
  const text = (value ?? "").trim();
  if (!text) return "";
  try {
    return new URL(text).host;
  } catch {
    return text.replace(/^https?:\/\//, "").replace(/\/.*$/, "");
  }
}

function ccsProviderSummary(result: CcsProvidersResult | null): string {
  if (!result) return t("读取 ~/.cc-switch/cc-switch.db");
  if (!isSuccessStatus(result.status)) return result.message || t("读取 cc-switch 供应商失败。");
  const count = result.providers.length;
  return count ? tf("发现 {0} 个 Codex 供应商", [count]) : t("未发现可导入供应商");
}
