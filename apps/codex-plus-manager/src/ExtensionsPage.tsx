// 拓展页（用户脚本、脚本市场、内置拓展）与启动急救页。
import { invoke } from "@tauri-apps/api/core";
import { Blocks, Check, Download, Loader2, Palette, PowerOff, RefreshCw, RotateCcw, Settings, Sparkles, Trash2, AppWindow, Upload } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { isGitHubRepositoryHomepage } from "./github-repository";
import { DshPluginsPanel } from "./DshPluginsPanel";
import { CommunityExtensionsPanel, type CommunitySort } from "./CommunityExtensionsPanel";
import { ExtensionCard, githubAvatar } from "./ExtensionCard";
import { CuratedPackagesSection } from "./CuratedPackages";
import { ExtensionPublishDialog } from "./ExtensionPublishDialog";
import { MyPublishedPanel } from "./MyPublishedPanel";
import { AppSelect } from "./AppSelect";
import { ExtensionConfigForm } from "./extension-config-form";
import { ExtensionManagerPanel } from "./ExtensionManagerPanel";
import { AgentGlyph } from "./HubOverview";
import { TopbarActions } from "./TopbarActions";
import { agentVisible, type AgentFocus } from "./agent-focus-model";
import { defaultMarketSort, parseMarketTime, relativeAge, sortMarketScripts, type LocalScriptState, type ScriptMarketSort } from "./script-market-model";
import { extensionEnabledForHost, extensionsForHost, extensionStateForHost, type ExtensionHost } from "./extension-host-model";
import { Menu, MenuItem } from "./ManagementPages";
import {
  builtinSettingValue,
  enhancementItem,
  isEnhancementItemMasterGated,
  managerBuiltinCards,
  managerStandaloneGroup,
  type EnhancementSettingItem,
} from "./enhancement-catalog";
import { getLanguage, t, tf } from "@/i18n";
import { CxBadge, CxBanner, CxCard, CxEmpty, CxFilters, CxRow, CxSection, CxStatusIcon, CxToolbar } from "./cx-layout";
import { Puzzle } from "lucide-react";
import * as AgentCx from "./cx-layout"; // cx-pages:agents：Agent 页面在函数内解构，避免和别的分组的具名导入重名
import type { Actions, BackendSettings, BuiltinExtension, ScriptMarketItem, ScriptMarketResult, SettingsResult, UserScriptInventory } from "./app-types";
import { AutoSaveStatus, enhancementPlatform, ToggleVisual, useEnhanceAutoSave } from "./app-shared";

const SCRIPT_MARKET_REPOSITORY_URL = "https://github.com/BigPizzaV3/CodexPlusPlusScriptMarket";

/// 「启动急救」页：把所有能让 Codex 恢复启动 / 恢复正常显示的开关放在一处——
/// 增强总开关（关掉后 Codex 不再注入，没法在 Codex 里再打开）、总开关关掉后仍生效的独立模式补丁、
/// 本地拓展总开关与「下次启动跳过拓展」、恢复默认皮肤。其余内置拓展的开关、语音输入与下一步建议的
/// 服务配置都只在 Codex 里的 Codex++ 设置页里改。旧的 dreamSkin 路由也落到这一页。
export function EnhanceScreen({
  dirty,
  form,
  onFormChange: applyForm,
  settings,
  launchPending,
  skinRestartPending,
  actions,
}: {
  dirty: boolean;
  form: BackendSettings;
  onFormChange: (value: BackendSettings) => void;
  settings: SettingsResult | null;
  launchPending: boolean;
  /** 恢复默认皮肤（或网页一键换肤）已保存，重启 Codex 才完整生效。 */
  skinRestartPending: boolean;
  actions: Actions;
}) {
  const { CxBadge, CxBanner, CxCard, CxRow, CxSection, CxStatusIcon } = AgentCx;
  const standaloneGroup = managerStandaloneGroup();
  const standaloneCards = useMemo(() => managerBuiltinCards(managerStandaloneGroup().id, enhancementPlatform), []);
  // 说明里写了要重启 Codex 的开关：改动时记下改之前的值，页面顶部汇总成一条「重启 Codex 生效」。
  const restartItems = useMemo(() => standaloneCards
    .flatMap((card) => (card.item ? [{ key: card.item.key, title: card.item.title, description: card.item.description }] : []))
    .filter((item) => RESTART_HINT.test(t(item.description))), [standaloneCards]);
  const restartOriginal = useRef(new Map<string, unknown>());
  const [restartBusy, setRestartBusy] = useState(false);
  const onFormChange = (next: BackendSettings) => {
    for (const item of restartItems) {
      const before = builtinSettingValue(form, item.key);
      if (before !== builtinSettingValue(next, item.key) && !restartOriginal.current.has(item.key)) restartOriginal.current.set(item.key, before);
    }
    applyForm(next);
  };
  const restartPending = restartItems.filter((item) => restartOriginal.current.has(item.key)
    && restartOriginal.current.get(item.key) !== builtinSettingValue(form, item.key));
  // 皮肤的待重启状态在 App 里（重启后清掉）；「稍后」只在本页收起这一项。
  const [skinDismissed, setSkinDismissed] = useState(false);
  const skinPending = skinRestartPending && !skinDismissed;
  const restartLabels = [
    ...restartPending.map((item) => t(item.title).replace(EXPERIMENTAL_TITLE, "")),
    ...(skinPending ? [t("恢复默认皮肤")] : []),
  ];
  const [, setRestartDismissed] = useState(0);
  const clearRestartPending = () => { restartOriginal.current.clear(); setSkinDismissed(true); setRestartDismissed((value) => value + 1); };
  const autoSave = useEnhanceAutoSave(dirty, form, actions.autoSaveSettings);
  const masterEnabled = form.enhancementsEnabled;
  const masterItem = enhancementItem("enhancementsEnabled");
  const inventory = settings?.user_scripts;
  const localEnabled = inventory?.enabled !== false;
  const skipNextStart = Boolean(inventory?.skip_next_start);
  const skinActive = masterEnabled && form.codexAppDreamSkinEnabled && !form.codexAppDreamSkinPaused;
  const skinState = !masterEnabled
    ? t("增强总开关已关闭")
    : !form.codexAppDreamSkinEnabled
      ? t("皮肤未启用")
      : form.codexAppDreamSkinPaused
        ? t("已恢复原始外观")
        : t("皮肤已启用");
  const themeName = form.codexAppDreamSkinThemeConfig?.name?.trim() || t("未记录");
  return (
    <div className="settings-page enhance-page sectioned-page cx-enhance-page">
      <div className="cx-enhance-bar" id="enhance-page-controls">
        <span className="cx-toolbar-spacer" />
        <span className="enhance-save-actions cx-enhance-save">
          <AutoSaveStatus state={autoSave} />
        </span>
        {/* 总开关：一个紧凑开关，说明放悬停。 */}
        <label className="feature-toggle enhance-master-switch cx-enhance-master-switch" title={t(masterItem.description)}>
          <input
            checked={form.enhancementsEnabled}
            onChange={(event) => onFormChange({ ...form, enhancementsEnabled: event.currentTarget.checked })}
            type="checkbox"
          />
          <span><strong>{t(masterItem.title)}</strong></span>
          <ToggleVisual />
        </label>
      </div>
      <div className="sectioned-body cx-enhance-body">
      {restartLabels.length ? (
        <CxBanner
          detail={restartLabels.join("、")}
          icon={<RotateCcw />}
          title={tf("{0} 项改动重启 Codex 后生效", [restartLabels.length])}
          tone="warn"
        >
          <button className="cx-secondary" onClick={clearRestartPending} type="button">{t("稍后")}</button>
          <button
            className="cx-primary"
            disabled={restartBusy || launchPending}
            onClick={() => void (async () => {
              setRestartBusy(true);
              try {
                if (await actions.restart()) clearRestartPending();
              } finally {
                setRestartBusy(false);
              }
            })()}
            type="button"
          >
            {restartBusy ? <Loader2 className="cx-dash-spin" /> : <RotateCcw />}
            {t("重启 Codex")}
          </button>
        </CxBanner>
      ) : null}
      {masterEnabled ? null : (
        <CxBanner icon={<PowerOff />} title={t("内置拓展已全部暂停")} detail={t("独立模式里的补丁仍然生效；服务配置会保留。")} tone="warn">
          <button className="cx-primary" onClick={() => onFormChange({ ...form, enhancementsEnabled: true })} type="button">{t("全部启用")}</button>
        </CxBanner>
      )}
      <CxCard>
        <div className="feature-toggle cx-enhance-moved" data-enhance-moved="codex">
          <span>
            <strong>{t("Codex 打不开或显示异常时在这里处理")}</strong>
            <small>{t("这里只放能让 Codex 恢复启动的开关：增强总开关、独立模式补丁、本地拓展和皮肤。各项内置拓展的开关、语音输入与下一步建议的服务配置在 Codex 左侧栏的「Codex++」设置页里。")}</small>
          </span>
        </div>
      </CxCard>
      <FeatureGroup id="enhance-group-standalone" title={t(standaloneGroup.title)} detail={t(standaloneGroup.description ?? "")}>
        {standaloneCards.map((card) => (card.item ? (
          <EnhancementSettingControl key={card.id} item={card.item} form={form} onFormChange={onFormChange} masterEnabled={masterEnabled} />
        ) : null))}
      </FeatureGroup>
      {/* 本地拓展的整体开关立即生效（不走设置表单的自动保存），和「万物皆拓展」页的三点菜单是同一份状态。 */}
      <CxSection className="cx-feature-group" id="enhance-group-extensions" title={t("本地拓展")}>
        <CxCard className="cx-feature-card">
          <FeatureToggle
            checked={localEnabled}
            detail={t("关掉后 Codex 不再注入任何本地拓展；各拓展的启停设置不会改变")}
            disabled={!inventory}
            onChange={(value) => void actions.setUserScriptsEnabled(value, "codex")}
            title={t("启用本地拓展")}
          />
          <FeatureToggle
            checked={skipNextStart}
            detail={t("拓展导致黑屏或卡死时，下一次启动先跳过拓展；各拓展的启停设置不会改变")}
            disabled={!inventory}
            onChange={(value) => void actions.setUserScriptsSkipNextStart(value)}
            title={t("下次启动跳过拓展")}
          />
        </CxCard>
      </CxSection>
      {/* 皮肤的界面已移到 Codex 里（Codex++ → 设置 → 皮肤，见 96-dream-skin-page.js），这里只留恢复默认皮肤。 */}
      <CxSection className="enhance-skin-section" id="enhance-group-skin" title={t("皮肤")}>
        <CxCard>
          <CxRow
            badges={<CxBadge tone={skinActive ? "ok" : "neutral"}>{skinState}</CxBadge>}
            detail={<span role="status">{tf("当前主题：{0}", [themeName])}</span>}
            icon={<CxStatusIcon tone={skinActive ? "ok" : "off"}><Palette /></CxStatusIcon>}
            title={t("当前皮肤")}
            tooltip={t("皮肤已移到 Codex 里：打开 Codex 左侧的 Codex++ → 设置 → 皮肤，可以换主题、装社区和市场主题、编辑主题。")}
          />
          <CxRow
            detail={t("皮肤让 Codex 显示异常或打不开时使用：关掉皮肤并清掉当前窗口里的皮肤，主题库不受影响；重启 Codex 后完整生效。")}
            icon={<CxStatusIcon tone="warn"><RotateCcw /></CxStatusIcon>}
            title={t("恢复默认皮肤")}
          >
            <button className="cx-secondary" onClick={() => { setSkinDismissed(false); void actions.restoreDreamSkin(); }} type="button">
              <RotateCcw />
              {t("恢复默认皮肤")}
            </button>
          </CxRow>
        </CxCard>
        <p className="dream-skin-about">
          {t("项目来源：Fei-Away/Codex-Dream-Skin · 原作者 Fei-Away · MIT License · 第三方图片需自行确认授权")}
        </p>
      </CxSection>
      </div>
    </div>
  );
}

type BooleanSettingKey = { [K in keyof BackendSettings]-?: BackendSettings[K] extends boolean ? K : never }[keyof BackendSettings];

/// 按设置项清单渲染一个增强设置：bool 是开关卡，select 是带下拉的卡片。
/// 受总开关控制的项在增强关闭时禁用；独立模式补丁（masterGated: false）始终可改，
/// 并按总开关状态切换说明（descriptionWhenEnhancementsOn）。
function EnhancementSettingControl({
  item,
  form,
  onFormChange,
  masterEnabled,
  onToggle,
}: {
  item: EnhancementSettingItem;
  form: BackendSettings;
  onFormChange: (value: BackendSettings) => void;
  masterEnabled: boolean;
  /** 需要在写入前插入确认等步骤时覆盖默认写入。 */
  onToggle?: (value: boolean) => void;
}) {
  const disabled = isEnhancementItemMasterGated(item) && !masterEnabled;
  const detail = t(masterEnabled && item.descriptionWhenEnhancementsOn ? item.descriptionWhenEnhancementsOn : item.description);
  if (item.control === "select") {
    const key = item.key as keyof BackendSettings;
    return (
      <div className={`feature-toggle ${disabled ? "disabled" : ""}`}>
        <span>
          <strong>{t(item.title)}</strong>
          <small title={detail}>{detail}</small>
        </span>
        <AppSelect<string>
          ariaLabel={t(item.title)}
          value={String(form[key] ?? "")}
          disabled={disabled}
          onChange={(value) => onFormChange({ ...form, [key]: value })}
          options={(item.options ?? []).map((option) => ({ value: option.value, label: t(option.label) }))}
        />
      </div>
    );
  }
  const key = item.key as BooleanSettingKey;
  return (
    <FeatureToggle
      title={t(item.title)}
      detail={detail}
      checked={form[key]}
      disabled={disabled}
      onChange={onToggle ?? ((value) => onFormChange({ ...form, [key]: value }))}
    />
  );
}

export function UserScriptsScreen({ host, focus = null, onHostChange, settings, market, onOpenRescue, onOpenPluginMarket, onOpenClaude, actions }: { host: ExtensionHost; focus?: AgentFocus; onHostChange: (host: ExtensionHost) => void; settings: SettingsResult | null; market: ScriptMarketResult | null; onOpenRescue: () => void; onOpenPluginMarket?: () => void; onOpenClaude: () => void; actions: Actions }) {
  const codexHost = host === "codex";
  const [reloading, setReloading] = useState(false);
  const reload = async () => {
    if (reloading) return;
    setReloading(true);
    try {
      await actions.reloadUserScripts();
    } finally {
      setReloading(false);
    }
  };
  const inventory = settings?.user_scripts;
  const scripts = extensionsForHost(inventory?.scripts ?? [], host);
  const marketScripts = market?.market.scripts ?? [];
  const marketPackages = market?.market.packages ?? [];
  // 参照 Cursor 的 Customize：一个搜索框管所有筛选（精选 / 社区 / 本地 / 我发布的 / dsh 插件），
  // 一排筛选胶囊切换来源，列表按「已安装」「可安装」（本地是「已启用」「未启用」）分节。
  const [search, setSearch] = useState("");
  const [communitySort, setCommunitySort] = useState<CommunitySort>("stars");
  const [codexPane, setPane] = useState<"market" | "community" | "local" | "mine" | "dsh">("market");
  const pane = codexHost ? codexPane : "local";
  // 直接打开本页（命令面板、深链）时市场可能还没读过，自动读一次，不再要用户点「刷新」。
  useEffect(() => {
    if (codexHost && !market) void actions.refreshScriptMarket();
  }, [codexHost]);
  const query = search.trim().toLocaleLowerCase();
  const filteredMarketScripts = useMemo(() => {
    if (!query) return marketScripts;
    return marketScripts.filter((script) => {
      const haystack = [
        script.name,
        script.author,
        script.description,
        script.version,
        script.homepage,
        ...script.tags,
      ]
        .filter(Boolean)
        .join(" ")
        .toLocaleLowerCase();
      return haystack.includes(query);
    });
  }, [query, marketScripts]);
  // 用户没手动选排序时跟随数据：清单里每个脚本都标了更新时间就默认「最近更新」，否则保持清单原顺序。
  const [marketSortChoice, setMarketSortChoice] = useState<ScriptMarketSort | null>(null);
  const marketSort = marketSortChoice ?? defaultMarketSort(marketScripts);
  const sortedMarketScripts = useMemo(
    () => sortMarketScripts(filteredMarketScripts, marketSort),
    [filteredMarketScripts, marketSort],
  );
  const hasMarketUpdateTimes = marketScripts.some((script) => parseMarketTime(script.updatedAt) !== null);
  // market_id -> 本地脚本 key。只用 user 来源的脚本：内置脚本走的是 builtin: 前缀，
  // 后端 delete_user_script 只接受 user:，映射过去会直接报错。
  const installedKeyByMarketId = useMemo(() => {
    const map = new Map<string, string>();
    for (const script of scripts) {
      if (script.source !== "user" || !script.market_id) continue;
      map.set(script.market_id, script.key);
    }
    return map;
  }, [scripts]);
  const localEnabled = codexHost ? inventory?.enabled !== false : inventory?.claude_enabled === true;
  const skipNextStart = Boolean(inventory?.skip_next_start);
  const localScripts = (query
    ? scripts.filter((script) => [script.name, script.key, script.package?.id, script.package?.description].filter(Boolean).join(" ").toLocaleLowerCase().includes(query))
    : scripts);
  const enabledScripts = localScripts.filter((script) => extensionEnabledForHost(script, host));
  const idleScripts = localScripts.filter((script) => !extensionEnabledForHost(script, host));
  const scriptRow = (script: (typeof scripts)[number]) => <ScriptRow key={`${host}:${script.key}`} host={host} script={script} globalEnabled={localEnabled} actions={actions} />;
  const marketRow = (script: ScriptMarketItem) => (
    <MarketScriptCard actions={actions} installedKey={installedKeyByMarketId.get(script.id)} key={script.id} script={script} />
  );
  const switchLabel = codexHost ? t("启用本地拓展") : t("启用 Claude 拓展");
  const panes = [
    { id: "market" as const, label: t("精选"), count: marketScripts.length + marketPackages.length },
    { id: "community" as const, label: t("社区") },
    { id: "local" as const, label: t("本地"), count: scripts.length },
    { id: "mine" as const, label: t("我发布的") },
    // dsh 插件不是又一个拓展来源，而是把 dsh 插件导入成 Codex 的 MCP / Skill。
    { id: "dsh" as const, label: t("导入 dsh 插件") },
  ];
  const sortControl = pane === "market" ? (
    <AppSelect
      ariaLabel={t("拓展市场排序")}
      className="script-market-sort"
      onChange={(value) => setMarketSortChoice(value)}
      options={[
        { value: "default", label: t("默认顺序") },
        {
          value: "updated",
          label: t("最近更新"),
          title: hasMarketUpdateTimes ? undefined : t("市场清单暂未提供更新时间"),
        },
        { value: "name", label: t("名称") },
      ]}
      size="sm"
      title={t("拓展市场排序")}
      value={marketSort}
    />
  ) : pane === "community" ? (
    <AppSelect
      ariaLabel={t("社区拓展排序")}
      className="script-market-sort"
      onChange={(value) => setCommunitySort(value)}
      options={[
        { value: "stars", label: t("星数") },
        { value: "updated", label: t("最近更新") },
        { value: "name", label: t("名称") },
      ]}
      size="sm"
      title={t("社区拓展排序")}
      value={communitySort}
    />
  ) : null;
  return (
    <div className="tp-page cx-page user-scripts-page">
      <TopbarActions>
        {codexHost ? <button className="cx-secondary" disabled={reloading} onClick={() => void reload()} title={t("应用本地拓展及开关；旧版脚本可能需要刷新 Codex 页面")} type="button">
          <RefreshCw className={reloading ? "animate-spin" : ""} />
          {t("热重载拓展")}
        </button> : <button className="cx-secondary" onClick={onOpenClaude} type="button">
          <AppWindow />
          {t("Claude 补丁版")}
        </button>}
        {/* 不常用的整体开关收进三点菜单；关着时页面顶部会有一条提示带「启用」。 */}
        <Menu label={t("更多操作")}>
          <MenuItem onClick={() => void (async () => { await actions.refreshScriptMarket(false, false, true); await actions.refreshCurrent(); })()}>{t("刷新列表")}</MenuItem>
          <MenuItem checked={localEnabled} disabled={!inventory} onClick={() => void actions.setUserScriptsEnabled(!localEnabled, host)}>{switchLabel}</MenuItem>
          {codexHost ? <MenuItem checked={skipNextStart} disabled={!inventory} onClick={() => void actions.setUserScriptsSkipNextStart(!skipNextStart)}>{t("下次启动跳过拓展")}</MenuItem> : null}
          {/* 这两个整体开关在「启动急救」页里也有一份（Codex 打不开时从那里一起处理）。 */}
          {codexHost ? <MenuItem onClick={onOpenRescue}>{t("打开启动急救")}</MenuItem> : null}
        </Menu>
      </TopbarActions>
      <ExtensionsHero host={host} focus={focus} onHostChange={onHostChange} />
      {codexHost && onOpenPluginMarket ? (
        <p className="cx-aggregate-note ext-crosslink">
          <span>{t("这里是 Codex++ 拓展；Codex 官方目录里的插件在「Codex 插件」页。")}</span>
          <button className="cx-link" onClick={onOpenPluginMarket} type="button">{t("打开 Codex 插件")}</button>
        </p>
      ) : null}
      <CxToolbar onQuery={setSearch} placeholder={codexHost ? t("搜索拓展：名称、作者、描述或标签") : t("搜索适配 Claude 的拓展")} query={search}>
        {codexHost ? <ExtensionCreatorCard /> : null}
      </CxToolbar>
      {/* 顶部最多一条状态提示：整体关闭优先，其次是「下次启动跳过」。 */}
      {inventory && !localEnabled ? (
        <CxBanner
          detail={codexHost ? t("拓展开着，但「启用本地拓展」整体开关关着，暂不注入") : t("重启 Claude 补丁版后应用")}
          icon={<PowerOff />}
          title={codexHost ? t("本地拓展已整体关闭") : t("Claude 拓展已关闭")}
          tone="warn"
        >
          <button className="cx-secondary" onClick={() => void actions.setUserScriptsEnabled(true, host)} type="button">{t("启用")}</button>
        </CxBanner>
      ) : codexHost && skipNextStart ? (
        <CxBanner detail={t("拓展导致黑屏或卡死时，下一次启动先跳过拓展；各拓展的启停设置不会改变")} icon={<PowerOff />} title={t("下次启动会跳过拓展")} tone="warn">
          <button className="cx-secondary" onClick={() => void actions.setUserScriptsSkipNextStart(false)} type="button">{t("取消跳过")}</button>
        </CxBanner>
      ) : null}
      {codexHost ? <CxFilters items={panes} label={t("拓展")} onChange={setPane} trailing={sortControl} value={pane} /> : null}
      {pane === "market" ? (
        <CuratedPackagesSection
          blockedInstalled={market?.market.blockedInstalled ?? []}
          emptyText={market?.status === "failed" ? market.message : t("正在读取拓展市场…")}
          hasScripts={marketScripts.length > 0}
          installedScripts={sortedMarketScripts.filter((script) => script.installed).map(marketRow)}
          onInstalled={async () => { await actions.reloadUserScripts(); await actions.refreshScriptMarket(false, true); }}
          onOpenUrl={(url) => void actions.openExternalUrl(url)}
          packages={marketPackages}
          search={search}
        >
          {sortedMarketScripts.filter((script) => !script.installed).map(marketRow)}
        </CuratedPackagesSection>
      ) : pane === "community" ? (
        <CommunityExtensionsPanel onCurated={actions.refreshScriptMarket} search={search} sort={communitySort} onInstalled={actions.reloadUserScripts} onOpenUrl={(url) => void actions.openExternalUrl(url)} />
      ) : pane === "mine" ? (
        <MyPublishedPanel onOpenUrl={(url) => void actions.openExternalUrl(url)} search={search} />
      ) : pane === "dsh" ? (
        <DshPluginsPanel search={search} />
      ) : (
        <>
          {enabledScripts.length ? (
            <CxSection count={enabledScripts.length} title={t("已启用")}>
              <CxCard>{enabledScripts.map(scriptRow)}</CxCard>
            </CxSection>
          ) : null}
          {idleScripts.length ? (
            <CxSection count={idleScripts.length} title={t("未启用")}>
              <CxCard>{idleScripts.map(scriptRow)}</CxCard>
            </CxSection>
          ) : null}
          {!localScripts.length ? (
            <CxCard>
              <CxEmpty title={query ? t("没有匹配的拓展。") : codexHost ? t("还没有本地拓展。") : t("还没有适配 Claude 的本地拓展。")}>
                {query ? <button className="cx-secondary" onClick={() => setSearch("")} type="button">{t("清除搜索")}</button>
                  : codexHost ? <button className="cx-secondary" onClick={() => setPane("market")} type="button">{t("浏览精选")}</button> : null}
              </CxEmpty>
            </CxCard>
          ) : null}
          {codexHost && inventory?.builtins?.length ? <BuiltinExtensionsSummary builtins={inventory.builtins} /> : null}
        </>
      )}
    </div>
  );
}

/// 市场脚本更新时间的相对描述：刚刚 / N 小时前 / N 天前 / N 个月前 / N 年前。
function formatRelativeAge(time: number, now = Date.now()): string {
  const age = relativeAge(time, now);
  switch (age.unit) {
    case "justNow":
      return t("刚刚");
    case "hours":
      return tf("{0} 小时前", [age.value]);
    case "days":
      return tf("{0} 天前", [age.value]);
    case "months":
      return tf("{0} 个月前", [age.value]);
    case "years":
      return tf("{0} 年前", [age.value]);
  }
}

function MarketScriptCard({
  script,
  actions,
  installedKey,
}: {
  script: ScriptMarketItem;
  actions: Actions;
  /// 已安装市场脚本在本地清单里的 key（`user:market-*.js`）。后端按 id 生成文件名，
  /// 前端不重算，直接取自 user_scripts 清单；为空表示未安装或不可删除。
  installedKey?: string;
}) {
  // 「未安装」是默认状态：满屏同一个徽标只是噪音，只给已安装 / 可更新打标。
  const status = script.updateAvailable ? t("可更新") : script.installed ? tf("已安装 {0}", [script.installedVersion]) : null;
  const isGitHubHomepage = script.homepage ? isGitHubRepositoryHomepage(script.homepage) : false;
  const updatedTime = parseMarketTime(script.updatedAt);
  // 头像：清单给的图标，否则主页仓库作者的 GitHub 头像；主页就是市场仓库时退回名字首字，免得全是同一个头像。
  const homepageRepo = isGitHubHomepage ? script.homepage.replace(/^https?:\/\/github\.com\//, "").split(/[/?#]/).slice(0, 2).join("/") : "";
  const icon = script.icon || (homepageRepo && !SCRIPT_MARKET_REPOSITORY_URL.includes(homepageRepo) ? githubAvatar(homepageRepo) : "");
  const updatedText = updatedTime !== null ? tf("{0}更新", [formatRelativeAge(updatedTime)]) : "";
  const meta = [script.author || t("未知作者"), `v${script.version}`, updatedText].filter(Boolean).join(" · ");
  // 已安装且是最新版时不再给按钮（重新安装收进菜单），未安装 / 可更新时右侧一个按钮。
  const showInstall = !script.installed || script.updateAvailable;
  return (
    <ExtensionCard
      description={script.description}
      icon={icon}
      link={script.homepage ? { url: script.homepage, text: isGitHubHomepage ? tf("在 GitHub 上支持作者：{0}", [script.name]) : t("主页") } : null}
      menu={
        <>
          {script.installed && !script.updateAvailable ? (
            <MenuItem onClick={() => void actions.installMarketScript(script.id)}>
              <span className="ext-row-menu-item"><Download size={14} />{t("重新安装")}</span>
            </MenuItem>
          ) : null}
          {installedKey ? (
            <MenuItem danger onClick={() => void actions.deleteUserScript(installedKey)}>
              <span className="ext-row-menu-item"><Trash2 size={14} />{t("卸载")}</span>
            </MenuItem>
          ) : null}
        </>
      }
      meta={meta}
      name={script.name}
      onOpenUrl={(url) => void actions.openExternalUrl(url)}
      primary={
        showInstall ? (
          <button className="cx-secondary" onClick={() => void actions.installMarketScript(script.id)} type="button">
            <Download />
            {script.updateAvailable ? t("更新") : t("安装")}
          </button>
        ) : null
      }
      status={status ? { text: status, highlight: script.updateAvailable } : null}
      tags={script.tags}
    />
  );
}

// 增强页的一组开关：卡片外的小标题（分组说明收进悬停）+ 一张卡，卡里一行一项。
function FeatureGroup({ title, detail, children, id, hidden = false }: { title: string; detail: string; children: ReactNode; id?: string; hidden?: boolean }) {
  const { CxCard, CxSection } = AgentCx;
  if (hidden) return null;
  return (
    <CxSection className="cx-feature-group" id={id} title={<span title={detail || undefined}>{title}</span>}>
      <CxCard className="cx-feature-card">{children}</CxCard>
    </CxSection>
  );
}

function FeatureToggle({
  title,
  detail,
  checked,
  disabled = false,
  onChange,
}: {
  title: string;
  detail: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (value: boolean) => void;
}) {
  const { CxBadge } = AgentCx;
  // 「（实验）」从标题里拿出来做成小标签；说明里写了要重启 Codex 的，标题旁加「需重启」。
  const experimental = EXPERIMENTAL_TITLE.test(title);
  const needsRestart = RESTART_HINT.test(detail);
  return (
    <label className={`feature-toggle ${disabled ? "disabled" : ""}`}>
      <input
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.currentTarget.checked)}
        type="checkbox"
      />
      <span>
        <strong>
          {experimental ? title.replace(EXPERIMENTAL_TITLE, "") : title}
          {experimental ? <CxBadge tone="warn">{t("实验")}</CxBadge> : null}
          {needsRestart ? <CxBadge>{t("需重启")}</CxBadge> : null}
        </strong>
        <small title={detail}>{detail}</small>
      </span>
      <ToggleVisual />
    </label>
  );
}

const EXPERIMENTAL_TITLE = /\s*(?:（实验性?）|\(experimental\))$/i;
const RESTART_HINT = /重启 ?Codex|重新启动 ?Codex|下次启动 ?Codex|restart(?:ing)? Codex|relaunch/i;

/// 本地脚本合并状态的文案与悬停说明。
function localScriptStateLabel(state: LocalScriptState, host: ExtensionHost = "codex"): { label: string; hint: string } {
  if (host === "claude") {
    if (state === "off") return { label: t("已关闭"), hint: t("该拓展已在 Claude 中关闭") };
    if (state === "globalOff") return { label: t("整体已关闭"), hint: t("Claude 拓展整体开关已关闭") };
    return { label: t("重启后应用"), hint: t("重启 Claude 补丁版后应用") };
  }
  switch (state) {
    case "off":
      return { label: t("已关闭"), hint: t("该拓展已关闭，不会注入 Codex") };
    case "globalOff":
      return { label: t("整体已关闭"), hint: t("拓展开着，但「启用本地拓展」整体开关关着，暂不注入") };
    case "loaded":
      return { label: t("已加载"), hint: t("已在 Codex 里运行") };
    case "loading":
      return { label: t("加载中"), hint: t("正在 Codex 里加载") };
    case "failed":
      return { label: t("加载失败"), hint: t("拓展在 Codex 里运行出错") };
    case "unknown":
      return { label: t("状态待确认"), hint: t("请检查实际效果") };
    case "notLoaded":
      return { label: t("未加载"), hint: t("Codex 未运行或尚未热重载拓展") };
  }
}

type ExtensionCreatorStatus = {
  status: "connected" | "not_connected" | "disconnected" | "conflict";
  message: string;
  configPath?: string;
  extensionsDirectory?: string;
};

/// 「对话造拓展」：把拓展创作 MCP 接入 Codex 后，用户在 Codex 对话里描述需求，agent 直接写出拓展并热重载。
/// 拓展页顶部的理念：Codex++ 的增强能力都以拓展的形式开放，下面列出拓展能改到的地方（放在悬停里）。
const extensionCapabilities = [
  "界面与页面", "斜杠命令", "消息与面板", "模型工具", "Codex 钩子", "网关中间件", "常驻后台", "会话历史", "拓展间服务", "多语言",
] as const;

function ExtensionsHero({ host, focus = null, onHostChange }: { host: ExtensionHost; focus?: AgentFocus; onHostChange: (host: ExtensionHost) => void }) {
  // 页面标题就是「万物皆拓展」：这里只留一句理念（能改到的地方放在悬停里），宿主切换放在右侧；只有一个宿主时不显示切换。
  const hosts = (["codex", "claude"] as const).filter((target) => agentVisible(focus, target));
  return (
    <section className="ext-hero" aria-label={t("万物皆拓展")}>
      <p className="ext-hero-lead" title={host === "codex" ? `${t("Codex++ 构建于万物皆拓展的架构之上：界面、命令、模型能用的工具、agent 循环的钩子、发往模型的请求与回复，都能由拓展接管。在 Codex 对话里说一句，就能造一个属于自己的拓展。")}\n${extensionCapabilities.map((label) => t(label)).join(" · ")}` : undefined}>
        {host === "codex"
          ? t("界面、命令、工具、钩子、模型请求都能由拓展接管；在 Codex 对话里说一句就能造一个。")
          : t("给 Claude 补丁版装拓展：只显示声明了适配 Claude 的拓展。")}
      </p>
      {hosts.length > 1 ? (
        <div className="tp-tabs cx-pills ext-host-switch" role="group" aria-label={t("拓展宿主")}>
          {hosts.map((target) => (
            <button key={target} type="button" aria-pressed={host === target} data-active={host === target} onClick={() => onHostChange(target)}>
              <AgentGlyph agent={target} />
              {target === "codex" ? "Codex" : "Claude"}
            </button>
          ))}
        </div>
      ) : null}
    </section>
  );
}

/// 工具条右侧的主按钮「对话造拓展」：一键把拓展创作 MCP 接入 Codex；接入后按钮变成「已接入」，再点一次取消接入。
/// 说明放在悬停里，不再单独占一张卡。
function ExtensionCreatorCard() {
  const [state, setState] = useState<ExtensionCreatorStatus | null>(null);
  const [busy, setBusy] = useState<"connect" | "disconnect" | null>(null);
  const [error, setError] = useState("");
  const request = async (action: "status" | "connect" | "disconnect") => {
    setBusy(action === "status" ? null : action);
    setError("");
    try {
      setState(await invoke<ExtensionCreatorStatus>("extension_creator_request", { action }));
    } catch (failure) {
      setError(String(failure));
    } finally {
      setBusy(null);
    }
  };
  useEffect(() => {
    void request("status");
  }, []);
  const connected = state?.status === "connected";
  const hint = error || (connected
    ? t("新开一个 Codex 对话，说「帮我做个拓展：…」即可；不造拓展时可取消接入，省下工具说明占用的上下文")
    : state?.status === "disconnected" ? t("已取消接入，新开的 Codex 对话不再加载拓展创作工具；要造拓展时再接入")
    : state?.status === "conflict" ? state.message : t("在 Codex 对话里描述需求，agent 直接写出拓展并热重载"));
  return (
    <button
      aria-pressed={connected}
      className={connected ? "cx-secondary ext-creator" : "cx-primary ext-creator"}
      data-error={error ? "true" : undefined}
      data-status={state?.status ?? "loading"}
      disabled={busy !== null || !state || (!connected && state.status === "conflict")}
      onClick={() => void request(connected ? "disconnect" : "connect")}
      title={`${hint}${connected ? `\n${t("再点一次取消接入")}` : ""}${state?.extensionsDirectory ? `\n${state.extensionsDirectory}` : ""}`}
      type="button"
    >
      {connected ? <Check /> : <Sparkles />}
      {busy === "connect" ? t("接入中…") : busy === "disconnect" ? t("取消中…") : connected ? t("对话造拓展已接入") : t("对话造拓展")}
    </button>
  );
}

/// 拓展包 locale/<语言>.json 里的本地化名称与描述：先找当前界面语言，再找 en，最后用清单原值。
function localizedPackageText(
  locales: Record<string, { title?: string | null; description?: string | null }> | undefined,
  field: "title" | "description",
  fallback: string,
): string {
  if (!locales) return fallback;
  for (const lang of [getLanguage(), "en"]) {
    const hit = Object.keys(locales).find((key) => key.toLowerCase() === lang || key.toLowerCase().startsWith(`${lang}-`));
    const value = hit ? locales[hit]?.[field] : null;
    if (typeof value === "string" && value.trim()) return value;
  }
  return fallback;
}

/// 本地拓展列表底部的内置拓展摘要：Codex++ 自带的功能都是内置拓展，开关与参数统一在 Codex 里的 Codex++ 设置页管理，
/// 这里只给个数和被顶替的项（只读，不可点），免得两处各有一套开关。
function BuiltinExtensionsSummary({ builtins }: { builtins: BuiltinExtension[] }) {
  const enabled = builtins.filter((builtin) => builtin.enabled && !builtin.replacedBy).length;
  const replaced = builtins.filter((builtin) => builtin.replacedBy);
  return (
    <CxSection title={t("内置")}>
      <CxCard>
        <CxRow
          badges={<CxBadge tone={enabled ? "ok" : "neutral"}>{tf("已启用 {0} / 共 {1} 项", [String(enabled), String(builtins.length)])}</CxBadge>}
          detail={replaced.length
            ? tf("已被顶替：{0}", [replaced.map((builtin) => `${t(builtin.name)} → ${String(builtin.replacedBy).replace(/^user:/, "")}`).join("、")])
            : t("Codex++ 自带的功能，开关与参数在 Codex 里的「Codex++」设置页管理")}
          icon={<Blocks />}
          title={t("内置拓展")}
          tooltip={tf("Codex++ 自带的功能都是内置拓展：已启用 {0} / 共 {1} 项。可以被声明 replaces 的拓展顶替。", [String(enabled), String(builtins.length)])}
        />
      </CxCard>
    </CxSection>
  );
}

/// 本地脚本一行：名称 / 来源 / 开关 + 状态 / 操作。
/// 以前「启用 / 关闭」和 loaded / disabled 各占一列、还另有启用按钮，三处说同一件事（#2781）；
/// 现在开关直接放进状态列，状态只补充开关之外的信息（是否真正加载、失败原因）。
function ScriptRow({ host, script, globalEnabled, actions }: { host: ExtensionHost; script: NonNullable<UserScriptInventory["scripts"]>[number]; globalEnabled: boolean; actions: Actions }) {
  const codexHost = host === "codex";
  const enabled = extensionEnabledForHost(script, host);
  const isPackage = script.kind === "package";
  const source = script.market_id
    ? tf("市场 · {0}", [script.version || t("未知版本")])
    : isPackage
      ? tf("拓展包 · {0}", [script.package?.version || t("未知版本")])
      : script.source === "builtin" ? t("内置") : t("用户");
  // 拓展包是文件夹：后端整个移进废纸篓（可找回，#2919），单文件脚本直接删除。
  const canDelete = codexHost && script.source === "user";
  const state = extensionStateForHost(script, host, globalEnabled);
  const { label, hint } = localScriptStateLabel(state, host);
  const stateHint = codexHost && state === "failed" && script.error ? `${hint}\n${script.error}` : hint;
  const configSchema = codexHost ? script.package?.configSchema ?? [] : [];
  // 拓展自己的设置页（settings.page）优先，没有时用按清单 config 生成的通用表单。
  const settingsPage = codexHost && !!script.package?.settingsPage;
  const hasSettings = settingsPage || configSchema.length > 0;
  const [configOpen, setConfigOpen] = useState(false);
  // 拓展自己的管理器面板（manager.page），启用时才能打开。
  const managerPage = codexHost && enabled ? script.package?.managerPage ?? null : null;
  const [panelOpen, setPanelOpen] = useState(false);
  // 用户自己的拓展包可以一键发布到 GitHub（单文件脚本、内置拓展没有这个入口）。
  const canPublish = codexHost && isPackage && script.source === "user" && !!script.package?.id;
  const [publishOpen, setPublishOpen] = useState(false);
  const displayName = localizedPackageText(script.package?.locales, "title", script.name);
  const displayDescription = localizedPackageText(script.package?.locales, "description", script.package?.description ?? "");
  // 拓展声明的能力，排成一行小标签；警示类（资源超限）用警告色。
  const pkg = script.package;
  const joined = (parts: unknown[]) => parts.filter((part): part is string => typeof part === "string" && part !== "").join("、");
  type CapabilityTag = { text: string; tone?: "warn"; title?: string };
  const capabilityCandidates: Array<CapabilityTag | null> = codexHost ? [
    pkg?.claudeDesktop ? { text: "Claude Code" } : null,
    pkg?.hookEvents?.length ? { text: tf("Codex 钩子：{0}", [pkg.hookEvents.join("、")]) } : null,
    pkg?.tools?.length ? { text: tf("模型工具：{0}", [pkg.tools.join("、")]) } : null,
    pkg?.backend ? { text: t("常驻后台") } : null,
    pkg?.services?.length ? { text: tf("提供服务：{0}", [pkg.services.join("、")]) } : null,
    pkg?.providers?.length ? { text: tf("模型供应商：{0}", [pkg.providers.map((provider) => provider.name || provider.id).join("、")]) } : null,
    pkg?.toolPipeline && (pkg.toolPipeline.preExecute || pkg.toolPipeline.execute || pkg.toolPipeline.postExecute)
      ? { text: tf("工具管道：{0}", [joined([pkg.toolPipeline.preExecute && t("改参数"), pkg.toolPipeline.execute && t("代为执行"), pkg.toolPipeline.postExecute && t("改结果")])]) }
      : null,
    pkg?.gatewayRequest || pkg?.gatewayResponse || pkg?.gatewayError
      ? { text: tf("网关中间件：{0}", [joined([!!pkg.gatewayRequest && t("请求"), !!pkg.gatewayResponse && t("响应流"), !!pkg.gatewayError && t("失败重试")])]) }
      : null,
    pkg?.codex?.skills?.length ? { text: tf("技能：{0}", [pkg.codex.skills.join("、")]) } : null,
    pkg?.codex?.agents?.length ? { text: tf("子 agent：{0}", [pkg.codex.agents.join("、")]) } : null,
    pkg?.codex?.mcpServers?.length ? { text: tf("MCP 服务器：{0}", [pkg.codex.mcpServers.join("、")]) } : null,
    pkg?.codex?.instructions ? { text: t("全局指令") } : null,
    pkg?.codex?.prompts?.length || pkg?.codex?.resources?.length
      ? { text: tf("提示词 / 资源：{0}", [[...(pkg.codex.prompts ?? []), ...(pkg.codex.resources ?? [])].join("、")]) }
      : null,
    pkg?.schedules?.length ? { text: tf("定时任务：{0}", [pkg.schedules.map((item) => item.at ? `${item.method} ${item.at}` : tf("{0} 每 {1} 分钟", [item.method, String(item.everyMinutes ?? "")])).join("、")]) } : null,
    pkg?.http ? { text: tf("HTTP 路由：/ext/{0}/", [pkg.id]) } : null,
    pkg?.skippedAssets?.length
      ? { text: tf("资源超出大小上限未加载：{0}", [pkg.skippedAssets.join("、")]), tone: "warn" as const, title: pkg.skippedAssets.join("\n") }
      : null,
  ] : [
    pkg?.claudeDesktop ? { text: "Claude Code" } : null,
    pkg?.backend ? { text: t("常驻后台") } : null,
  ];
  const capabilityTags = capabilityCandidates.filter((tag): tag is CapabilityTag => tag !== null);
  // 一行只放一句说明：有描述用描述，没有就列能力；完整的能力清单放进悬停。警示类能力单独打橙色标签。
  const plainTags = capabilityTags.filter((tag) => tag.tone !== "warn");
  const warnTags = capabilityTags.filter((tag) => tag.tone === "warn");
  const detailText = displayDescription || plainTags.map((tag) => tag.text).join(" · ");
  const detailHover = [displayDescription, ...plainTags.map((tag) => tag.text)].filter(Boolean).join("\n");
  // 状态点：已加载 = 绿，加载失败 = 红，整体关闭 / 未加载 = 橙，关着 = 灰。
  const stateTone = state === "loaded" ? "ok" : state === "failed" ? "danger" : state === "off" ? "off" : "warn";
  // 开关本身已经说明「开 / 关」，状态文字只在开关之外还有话说时显示（未加载、失败、整体关闭）。
  const quietState = state === "loaded" || state === "off";
  return (
    <>
    <CxRow
      badges={
        <>
          <CxBadge>{source}</CxBadge>
          {warnTags.map((tag) => <CxBadge key={tag.text} title={tag.title ?? tag.text} tone="warn">{t("资源超限")}</CxBadge>)}
        </>
      }
      className="ext-local-row"
      detail={detailText ? <span title={detailHover}>{detailText}</span> : undefined}
      icon={<CxStatusIcon tone={stateTone}><Puzzle /></CxStatusIcon>}
      muted={!enabled}
      title={displayName}
    >
      <span className="user-script-state-label" data-state={state} hidden={quietState || undefined} title={stateHint}>{label}</span>
      {hasSettings ? (
        <button
          type="button"
          className="mg-icon-button"
          title={configOpen ? t("收起设置") : t("设置")}
          aria-label={tf("{0} 设置", [displayName])}
          aria-expanded={configOpen}
          data-active={configOpen || undefined}
          onClick={() => setConfigOpen((open) => !open)}
        >
          <Settings size={16} />
        </button>
      ) : null}
      <label className="relay-bare-switch" title={enabled ? t("关闭该拓展") : t("启用该拓展")}>
        <input
          aria-label={tf("启用 {0}", [script.name])}
          checked={enabled}
          onChange={() => void actions.setUserScriptEnabled(script.key, !enabled, host)}
          type="checkbox"
        />
        <ToggleVisual />
      </label>
      {canPublish || canDelete || managerPage ? (
        <Menu label={tf("{0} 的更多操作", [displayName])}>
          {managerPage ? (
            <MenuItem onClick={() => setPanelOpen((open) => !open)}>
              <span className="user-script-menu-item"><AppWindow className="h-4 w-4" />{panelOpen ? t("收起面板") : t("打开面板")}</span>
            </MenuItem>
          ) : null}
          {canPublish ? (
            <MenuItem onClick={() => setPublishOpen(true)}>
              <span className="user-script-menu-item"><Upload className="h-4 w-4" />{t("发布到 GitHub")}</span>
            </MenuItem>
          ) : null}
          {canDelete ? (
            <MenuItem danger onClick={() => void actions.deleteUserScript(script.key)}>
              <span className="user-script-menu-item"><Trash2 className="h-4 w-4" />{t("删除")}</span>
            </MenuItem>
          ) : null}
        </Menu>
      ) : <span className="user-script-menu-spacer" aria-hidden="true" />}
    </CxRow>
    {publishOpen && script.package?.id ? (
      <ExtensionPublishDialog id={script.package.id} onClose={() => setPublishOpen(false)} onOpenUrl={(url) => void actions.openExternalUrl(url)} />
    ) : null}
    {panelOpen && managerPage ? (
      <div className="cx-row-expand"><ExtensionManagerPanel
        scriptKey={script.key}
        config={script.package?.config ?? {}}
        onSaveConfig={(values) => actions.setExtensionConfig(script.key, values)}
        onToast={(message) => actions.showExtensionNotice(managerPage.title || displayName, message)}
      /></div>
    ) : null}
    {configOpen && settingsPage ? (
      <div className="cx-row-expand"><ExtensionManagerPanel
        kind="settings"
        scriptKey={script.key}
        config={script.package?.config ?? {}}
        onSaveConfig={(values) => actions.setExtensionConfig(script.key, values, { silent: true })}
        onToast={(message) => actions.showExtensionNotice(displayName, message)}
      /></div>
    ) : null}
    {configOpen && !settingsPage && configSchema.length ? (
      <div className="cx-row-expand"><ExtensionConfigForm
        fields={configSchema}
        values={script.package?.config ?? {}}
        onCancel={() => setConfigOpen(false)}
        onSave={async (values) => {
          if (await actions.setExtensionConfig(script.key, values)) setConfigOpen(false);
        }}
      /></div>
    ) : null}
    </>
  );
}
