/** 本地网关：让 Claude Code、Grok 经 Codex++ 使用统一的供应商池，支持多 Key 轮换与模型组。 */
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { appConfirm } from "./app-dialog";
import { AlertTriangle, Asterisk, ChevronDown, ChevronRight, ChevronUp, Plus, RefreshCw, Search, Shuffle, Waypoints, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { t, tf } from "./i18n";
import {
  agentName,
  formatElapsed,
  groupModeDescription,
  groupModeLabel,
  groupModelId,
  groupStickyDescription,
  groupStickyLabel,
  isPattern,
  memberLabel,
  parseWeight,
  dispatchStatus,
  modelDispatch,
  modelDisplay,
  patternMatches,
  prettyModelName,
  speedOf,
  slugify,
  type GatewayGroupEntry,
  type GatewayPayload,
  type GatewayResult,
  type GroupMember,
  type GroupMode,
  type GroupSticky,
  agentPoolProviders,
  maskAccountName,
  type DispatchRow,
  type MemberStatus,
  memberTarget,
  type AutoGroupEntry,
} from "./gateway-model";
import { AgentsView, Floating, ModelList } from "./GatewayAgents";
import { AppSelect } from "./AppSelect";
import { ProvidersView } from "./GatewayProviders";
import { SessionsView } from "./GatewaySessions";
import { UsageView } from "./GatewayUsage";
import { Avatar, MemberStatusLabel, QuotaBadge, SectionLabel, Switch, makeRun, type Notify, type Run } from "./gateway-ui";
import { shortProviderName } from "./GatewayModelMatrix";
import { TopbarActions } from "./TopbarActions";
import { GatewayNavContext } from "./gateway-store";
import { useEscapeLayer } from "./use-escape-layer";
import { gatewayGeneration, publishGatewayPayload, subscribeGatewayPayload } from "./gateway-store";
import "./GatewayPage.css";

type Tab = "agents" | "sessions" | "providers" | "routing" | "usage" | "gateway";
export type GatewayTab = Tab;

const memo: { payload: GatewayPayload | null; tab: Tab } = { payload: null, tab: "agents" };

/** 组编辑器的模型选择不排除任何供应商；模块级常量，避免每次渲染都换新数组让列表重算。 */
const NO_EXCLUDE: string[] = [];

/** 路由页刷新供应商状态的间隔。 */
const LIVE_REFRESH_MS = 2500;

function hasPayload(result: GatewayResult): result is GatewayResult & GatewayPayload {
  return Boolean(result.gateway && Array.isArray(result.pool));
}

/**
 * 传入 `tab` 时由外层侧栏决定当前页签（不再显示页内标签条），页内跳转通过 `onTabChange` 交还外层；
 * 不传时保持原来的自管页签。
 */
export function GatewayPage({
  notify,
  tab: controlledTab,
  onTabChange,
  extraSettings,
}: {
  notify: Notify;
  tab?: Tab;
  onTabChange?: (tab: Tab) => void;
  /** 「网关设置」末尾附加的内容：主窗口传进来的新旧共用本地服务设置。 */
  extraSettings?: ReactNode;
}) {
  const [payload, setPayload] = useState<GatewayPayload | null>(memo.payload);
  const [busy, setBusy] = useState<string | null>(memo.payload ? null : "load");
  const [ownTab, setTabState] = useState<Tab>(memo.tab);
  const tab = controlledTab ?? ownTab;
  const setTab = (next: Tab) => {
    if (controlledTab) {
      onTabChange?.(next);
      return;
    }
    memo.tab = next;
    setTabState(next);
  };

  const accept = (result: GatewayResult) => {
    if (hasPayload(result)) {
      memo.payload = result;
      setPayload(result);
      // 侧栏状态卡、总览共用的数据也立刻更新，不等它们下次轮询。
      publishGatewayPayload(result);
    }
  };

  const run: Run = makeRun(setBusy, accept, notify);
  // 别处改了网关数据（如模型下拉里收藏）也同步过来，不然这里还拿着旧数据，下次保存会把新改的盖掉。
  useEffect(
    () =>
      subscribeGatewayPayload((next) => {
        memo.payload = next;
        setPayload(next);
      }),
    [],
  );

  const refresh = async (silent = true) => {
    setBusy("load");
    try {
      const result = await invoke<GatewayResult>("load_gateway");
      accept(result);
      if (result.status !== "ok") await notify(t("读取失败"), result.message, "failed");
      else if (!silent) await notify(t("已刷新"), result.message, "ok");
    } catch (error) {
      await notify(t("调用失败"), String(error), "failed");
    } finally {
      setBusy(null);
    }
  };

  useEffect(() => {
    void refresh();
    // 只在进入本页时读一次，之后由本页操作维护。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 窗口回到前台时静默读一次：别的工具可能趁这时改了 Agent 的配置（被接管提示要跟上）。
  useEffect(() => {
    let cancelled = false;
    const onFocus = () => {
      const started = gatewayGeneration();
      void invoke<GatewayResult>("load_gateway")
        .then((result) => {
          // 读的过程中保存过：这份是旧的，不用。
          if (!cancelled && started === gatewayGeneration()) accept(result);
        })
        .catch(() => undefined);
    };
    window.addEventListener("focus", onFocus);
    return () => {
      cancelled = true;
      window.removeEventListener("focus", onFocus);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 路由页定时刷新供应商状态：静默读取，不占用 busy，也不弹错误。
  useEffect(() => {
    if (tab !== "routing") return;
    let cancelled = false;
    const timer = window.setInterval(() => {
      const started = gatewayGeneration();
      void invoke<GatewayResult>("load_gateway")
        .then((result) => {
          if (!cancelled && started === gatewayGeneration()) accept(result);
        })
        .catch(() => undefined);
    }, LIVE_REFRESH_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab]);

  // 与侧栏同序：供应商 → Agent → 路由。
  const tabs: Array<{ id: Tab; label: string }> = [
    { id: "providers", label: t("供应商") },
    { id: "agents", label: t("智能体") },
    { id: "routing", label: t("路由") },
    { id: "sessions", label: t("会话") },
    { id: "usage", label: t("用量") },
    { id: "gateway", label: t("网关") },
  ];

  // 模型下拉里「我的组」旁的「管理」→ 路由页。
  const nav = { openModelGroups: () => setTab("routing") };

  const refreshButton = (
    <button aria-label={t("刷新")} className="gw-icon-button" disabled={busy !== null} onClick={() => void refresh(false)} title={t("刷新")} type="button">
      <RefreshCw className={`h-4 w-4 ${busy === "load" ? "animate-spin" : ""}`} />
    </button>
  );

  return (
    <div className="mg-page gateway-page">
      {/* 页面标题由外层框架显示，这里只放分段标签和刷新。 */}
      {controlledTab ? (
        // 由外层侧栏控制时不显示页内工具条，刷新按钮放进外壳顶栏。
        // 用量页签有自己的刷新（读用量和余额），这里的只刷新网关状态，放在一起会让人以为刷新了用量。
        <TopbarActions>{tab === "usage" ? null : refreshButton}</TopbarActions>
      ) : (
        <div className="gw-toolbar">
          <nav aria-label={t("本地网关")} className="gw-tabs" role="tablist">
            {tabs.map((item) => (
              <button aria-selected={tab === item.id} data-active={tab === item.id} key={item.id} onClick={() => setTab(item.id)} role="tab" type="button">
                {item.label}
              </button>
            ))}
          </nav>
          {refreshButton}
        </div>
      )}
      <GatewayNavContext.Provider value={nav}>
      <div className="mg-content gw-content">
        {payload && !payload.helperRunning && payload.agents.some((agent) => agent.enabled) ? (
          <p className="gw-alert" role="alert">
            <AlertTriangle className="h-4 w-4" />
            <span>{t("本地网关当前没有运行，走网关的 Agent 会连不上。请重新保存一次绑定或重启 Codex++。")}</span>
          </p>
        ) : null}
        {tab === "agents" ? <AgentsView busy={busy} notify={notify} onManage={() => setTab("providers")} payload={payload} run={run} /> : null}
        {tab === "sessions" ? <SessionsView notify={notify} /> : null}
        {tab === "providers" ? <ProvidersView busy={busy} notify={notify} payload={payload} run={run} /> : null}
        {tab === "routing" ? <RoutingView busy={busy} payload={payload} run={run} /> : null}
        {tab === "usage" ? <UsageView busy={busy} notify={notify} payload={payload} run={run} /> : null}
        {tab === "gateway" ? <GatewayView busy={busy} payload={payload} run={run} /> : null}
        {tab === "gateway" && extraSettings ? extraSettings : null}
      </div>
      </GatewayNavContext.Provider>
    </div>
  );
}

/* —————————————————————— 路由 —————————————————————— */

/** 卡片上路由方式的选项（顺序与组编辑器一致）。 */
const MODE_OPTIONS: GroupMode[] = ["inOrder", "rotate", "weighted", "usage", "fast", "manual"];
const STICKY_OPTIONS: GroupSticky[] = ["cache", "session", "off"];

/** 路由页：你建的组和自动组，每张卡片上直接改路由方式、会话固定、隐藏成员，手动模式下点成员锁定。 */
function RoutingView({ payload, busy, run }: { payload: GatewayPayload | null; busy: string | null; run: Run }) {
  const [editing, setEditing] = useState<GatewayGroupEntry | "new" | null>(null);
  const [query, setQuery] = useState("");
  const [showHidden, setShowHidden] = useState(false);
  const autoOn = payload?.gateway.autoModels !== false;
  const needle = query.trim().toLowerCase();
  const hit = (group: GatewayGroupEntry) =>
    !needle || group.name.toLowerCase().includes(needle) || group.members.some((member) => member.model.toLowerCase().includes(needle));
  const mine = (payload?.groups ?? []).filter(hit);
  const autos = (payload?.autoGroups ?? []).filter(hit);
  const visibleAutos = autos.filter((group) => !group.hidden);
  const hiddenAutos = autos.filter((group) => group.hidden);

  const saveMine = (group: GatewayGroupEntry, patch: Partial<GatewayGroupEntry>) => {
    const next = { ...group, ...patch };
    void run(`group:${group.id}`, "save_gateway_group", {
      group: { id: next.id, name: next.name, mode: next.mode, sticky: next.sticky ?? "cache", members: next.members, hideMembers: next.hideMembers ?? false, pick: next.pick ?? "" },
    }, t("保存组"), true);
  };
  const saveAuto = (group: AutoGroupEntry, patch: Partial<AutoGroupEntry>) => {
    const next = { ...group, ...patch };
    void run(`auto:${group.id}`, "save_gateway_auto_group", {
      model: group.name,
      settings: { mode: next.mode, sticky: next.sticky ?? "cache", hidden: next.hidden, hideMembers: next.hideMembers ?? false, pick: next.pick ?? "" },
    }, t("保存自动组"), true);
  };
  const remove = async (group: GatewayGroupEntry) => {
    const ok = await appConfirm(tf("删除组「{0}」？引用它的其它组会去掉这个成员。", [group.name]), { title: t("删除组"), confirmText: t("删除"), danger: true });
    if (ok) await run(`group:${group.id}`, "delete_gateway_group", { id: group.id }, t("删除组"));
  };

  return (
    <>
      <div className="gw-models-toolbar">
        <label className="gw-sheet-search gw-models-search">
          <Search className="h-4 w-4" />
          <input aria-label={t("搜索组或模型")} onChange={(event) => setQuery(event.target.value)} placeholder={t("搜索组或模型")} value={query} />
        </label>
        <div className="gw-models-auto" title={t("多家都有的模型自动成一个组：Agent 选不带供应商的模型名时走它。关掉后只列出你建的组，不带供应商的模型名按供应商池的顺序依次尝试。")}>
          <span>{t("自动建组")}</span>
          <Switch
            disabled={busy !== null || !payload}
            label={t("自动建组")}
            on={autoOn}
            onToggle={() => void run("auto-models", "save_gateway_auto_models", { enabled: !autoOn }, t("自动建组"), true)}
          />
        </div>
        <button className="gw-primary-button" disabled={busy !== null || !payload} onClick={() => setEditing("new")} type="button">
          <Plus className="h-4 w-4" />
          {t("新建组")}
        </button>
      </div>
      <p className="gw-models-hint">{t("组让 Agent 把几个模型当成一个来选：按什么方式先试哪个、同一会话要不要固定、出错换谁，都在卡片上改。")}</p>

      {!payload ? <div className="gw-card gw-empty">{t("正在读取…")}</div> : null}

      {payload ? (
        <SectionLabel aside={<em>{t("Agent 里以 group/<id> 出现")}</em>}>
          {t("我的组")}<em>{mine.length}</em>
        </SectionLabel>
      ) : null}
      {payload && mine.length === 0 ? (
        <div className="gw-card gw-empty">
          <strong>{needle ? t("没有匹配的组") : t("还没有自己建的组")}</strong>
          {needle ? null : <span>{t("比如把两家的同一个模型编成「依次」组：第一家出错自动换第二家；或者把几个便宜模型编成「轮换」组分摊用量。")}</span>}
        </div>
      ) : null}
      {mine.map((group) => (
        <GroupCard
          busy={busy}
          group={group}
          key={group.id}
          onChange={(patch) => saveMine(group, patch)}
          onDelete={() => void remove(group)}
          onEdit={() => setEditing(group)}
          payload={payload}
        />
      ))}

      {payload ? (
        <SectionLabel aside={<em>{autoOn ? t("多家都有的模型自动成组，Agent 选不带供应商的模型名时走它") : t("自动建组已关闭")}</em>}>
          {t("自动组")}<em>{visibleAutos.length}</em>
        </SectionLabel>
      ) : null}
      {payload && !autoOn ? (
        <div className="gw-card gw-empty">
          <strong>{t("自动建组已关闭")}</strong>
          <span>{t("打开后，多家都有的模型会自动成一个组；不带供应商的模型名现在按供应商池的顺序依次尝试。")}</span>
        </div>
      ) : null}
      {payload && autoOn && autos.length === 0 ? (
        <div className="gw-card gw-empty">
          <strong>{needle ? t("没有匹配的组") : t("还没有多家都有的模型")}</strong>
          {needle ? null : <span>{t("两家以上的供应商都提供同一个模型时，这里会自动多出一个组。")}</span>}
        </div>
      ) : null}
      {autoOn
        ? visibleAutos.map((group) => (
          <GroupCard
            auto
            busy={busy}
            group={group}
            key={group.id}
            onChange={(patch) => saveAuto(group, patch)}
            onHide={() => saveAuto(group, { hidden: true })}
            onReset={group.customized ? () => void run(`auto:${group.id}`, "save_gateway_auto_group", { model: group.name, settings: {} }, t("恢复默认"), true) : undefined}
            payload={payload}
          />
        ))
        : null}
      {autoOn && hiddenAutos.length ? (
        <div className="gw-hidden-autos">
          <button className="gw-text-button" onClick={() => setShowHidden(!showHidden)} type="button">
            {showHidden ? t("收起已隐藏的") : tf("已隐藏 {0} 个", [hiddenAutos.length])}
          </button>
          {showHidden
            ? hiddenAutos.map((group) => (
              <div className="gw-hidden-auto" key={group.id}>
                <strong>{prettyModelName(group.name)}</strong>
                <code>{group.name}</code>
                <button className="gw-text-button strong" disabled={busy !== null} onClick={() => saveAuto(group, { hidden: false })} type="button">{t("恢复")}</button>
              </div>
            ))
            : null}
        </div>
      ) : null}

      {editing ? (
        <GroupEditor
          busy={busy}
          group={editing === "new" ? null : editing}
          onCancel={() => setEditing(null)}
          onSave={async (input) => {
            // 编辑器不管「隐藏成员」和手动选中的成员：沿用卡片上已有的。
            const previous = editing === "new" ? null : editing;
            const group = { ...input, hideMembers: previous?.hideMembers ?? false, pick: previous?.pick ?? "" };
            if (await run("group:save", "save_gateway_group", { group }, t("保存组"))) setEditing(null);
          }}
          payload={payload}
        />
      ) : null}
    </>
  );
}

/** 一张组卡片：标题（名字、Agent 里怎么选它）、路由方式、会话固定、成员列表（状态、额度、耗时）、隐藏成员。 */
function GroupCard({
  group,
  payload,
  busy,
  auto = false,
  onChange,
  onEdit,
  onDelete,
  onHide,
  onReset,
}: {
  group: GatewayGroupEntry;
  payload: GatewayPayload | null;
  busy: string | null;
  auto?: boolean;
  onChange: (patch: Partial<GatewayGroupEntry>) => void;
  onEdit?: () => void;
  onDelete?: () => void;
  onHide?: () => void;
  onReset?: () => void;
}) {
  const manual = group.mode === "manual";
  const picked = manual ? (group.members.find((member) => member.model.toLowerCase() === (group.pick ?? "").toLowerCase()) ?? group.members[0])?.model : "";
  const disabled = busy !== null;
  return (
    <section className="gw-card gw-route-group" data-auto={auto}>
      <header className="gw-route-group-head">
        <span className="gw-route-group-icon">{auto ? <Shuffle className="h-4 w-4" /> : <Waypoints className="h-4 w-4" />}</span>
        <div className="gw-route-group-title">
          <strong>{auto ? prettyModelName(group.name) : group.name}</strong>
          <code className="gw-chip" title={t("Agent 里选这个名字就走这个组")}>{auto ? group.name : groupModelId(group.id)}</code>
          {auto ? <span className="gw-pill gw-pill-outline">{t("自动")}</span> : null}
        </div>
        <div className="gw-route-group-controls">
          <AppSelect
            disabled={disabled}
            label={t("路由方式")}
            onChange={(mode) => onChange({ mode })}
            options={MODE_OPTIONS.map((mode) => ({ value: mode, label: t(groupModeLabel(mode)), hint: t(groupModeDescription(mode)) }))}
            size="sm"
            value={group.mode}
          />
          <AppSelect
            disabled={disabled || manual}
            label={t("会话固定")}
            onChange={(sticky) => onChange({ sticky })}
            options={STICKY_OPTIONS.map((sticky) => ({ value: sticky, label: t(groupStickyLabel(sticky)), hint: t(groupStickyDescription(sticky)) }))}
            size="sm"
            title={manual ? t("手动模式只用选中的那个成员，不需要会话固定") : undefined}
            value={group.sticky ?? "cache"}
          />
        </div>
      </header>
      <ol className="gw-route-members" data-manual={manual}>
        {group.members.map((member, index) => (
          <RouteMember
            disabled={disabled}
            index={index}
            key={`${member.model}-${index}`}
            manual={manual}
            member={member}
            mode={group.mode}
            onPick={() => onChange({ pick: member.model })}
            payload={payload}
            picked={picked === member.model}
          />
        ))}
        {group.members.length === 0 ? <li className="gw-route-member-empty">{t("没有成员")}</li> : null}
      </ol>
      <footer className="gw-route-group-foot">
        <label className="gw-route-hide" title={t("打开后，组里的模型不再单独出现在 Agent 的模型列表里，只能通过这个组来选；直接写名字仍然能用。")}>
          <Switch disabled={disabled} label={t("隐藏成员")} on={group.hideMembers === true} onToggle={() => onChange({ hideMembers: !group.hideMembers })} />
          <span>{t("隐藏成员")}</span>
        </label>
        <span className="gw-route-group-mode">{t(groupModeDescription(group.mode))}</span>
        <span className="gw-row-buttons">
          {onEdit ? <button className="gw-text-button" disabled={disabled} onClick={onEdit} type="button">{t("编辑")}</button> : null}
          {onReset ? <button className="gw-text-button" disabled={disabled} onClick={onReset} title={t("回到依次、缓存期内固定、不隐藏成员")} type="button">{t("恢复默认")}</button> : null}
          {onHide ? <button className="gw-text-button" disabled={disabled} onClick={onHide} title={t("这个自动组不再出现在 Agent 里，不带供应商的模型名按供应商池的顺序依次尝试")} type="button">{t("隐藏")}</button> : null}
          {onDelete ? <button className="gw-text-button" disabled={disabled} onClick={onDelete} type="button">{t("删除")}</button> : null}
        </span>
      </footer>
    </section>
  );
}

/** 组里的一个成员：先后 / 手动选中、供应商与模型、状态、额度、平均耗时。 */
function RouteMember({
  member,
  index,
  mode,
  manual,
  picked,
  payload,
  disabled,
  onPick,
}: {
  member: GroupMember;
  index: number;
  mode: GroupMode;
  manual: boolean;
  picked: boolean;
  payload: GatewayPayload | null;
  disabled: boolean;
  onPick: () => void;
}) {
  const target = memberTarget(payload, member.model);
  const view = modelDisplay(payload, member.model);
  const status = target ? dispatchStatus(payload, target.entry.id, target.model) : null;
  const speed = target ? speedOf(payload, target.entry.id, target.model) : null;
  const quota = target?.entry.quota ?? null;
  const body = (
    <>
      <span className="gw-route-member-mark">
        {manual ? <i className="gw-radio-dot" data-on={picked} /> : <i>{mode === "inOrder" || mode === "usage" || mode === "fast" ? index + 1 : "·"}</i>}
      </span>
      {target ? <Avatar baseUrl={target.entry.baseUrl} name={target.entry.name} size="sm" /> : view.group ? <Waypoints className="gw-picker-glyph h-4 w-4" /> : <Shuffle className="gw-picker-glyph h-4 w-4" />}
      <span className="gw-route-member-name">
        <strong>{target ? maskAccountName(shortProviderName(target.entry.name)) : view.title}</strong>
        <small>{target ? prettyModelName(target.model) : member.model}</small>
      </span>
      {mode === "weighted" ? <span className="gw-route-member-weight">{tf("权重 {0}", [member.weight])}</span> : null}
      {status ? <span className="gw-route-member-status" data-state={status.state}><MemberStatusLabel status={status} /></span> : null}
      {quota ? <QuotaBadge quota={quota} /> : null}
      {speed !== null ? <small className="gw-route-member-speed" title={t("最近的平均首包耗时")}>{formatElapsed(speed)}</small> : null}
    </>
  );
  return (
    <li className="gw-route-member" data-picked={picked} data-state={status?.state ?? "idle"}>
      {manual ? (
        <button className="gw-route-member-row" disabled={disabled || picked} onClick={onPick} title={picked ? t("正在用这一个") : t("改成只用这一个")} type="button">{body}</button>
      ) : (
        <div className="gw-route-member-row">{body}</div>
      )}
    </li>
  );
}


/** 新建 / 编辑模型组：名称、模式、成员（池里的模型或别的组）。 */
function GroupEditor({
  payload,
  group,
  busy,
  onSave,
  onCancel,
}: {
  payload: GatewayPayload | null;
  group: GatewayGroupEntry | null;
  busy: string | null;
  onSave: (input: { id: string; name: string; mode: GroupMode; sticky: GroupSticky; members: GroupMember[] }) => void;
  onCancel: () => void;
}) {
  const [name, setName] = useState(group?.name ?? "");
  const [mode, setMode] = useState<GroupMode>(group?.mode ?? "inOrder");
  const [sticky, setSticky] = useState<GroupSticky>(group?.sticky ?? "cache");
  const [pattern, setPattern] = useState("");
  const [members, setMembers] = useState<GroupMember[]>(group?.members.map((member) => ({ ...member })) ?? []);
  const [picking, setPicking] = useState(false);
  const addRef = useRef<HTMLButtonElement>(null);
  // Esc 栈：「添加成员」浮层、模式下拉开着时 Esc 先关它们。
  useEscapeLayer(true, onCancel);
  // 权重输入框正在编辑的原始文字（允许暂时为空），失焦时再规范成 ≥ 1 的整数。
  const [weightText, setWeightText] = useState<Record<number, string>>({});
  const move = (index: number, delta: number) => {
    const next = [...members];
    const [item] = next.splice(index, 1);
    next.splice(Math.max(0, Math.min(next.length, index + delta)), 0, item);
    setMembers(next);
  };
  const error = !name.trim() ? t("请填写模型组名称") : members.length === 0 ? t("模型组至少要有一个成员") : "";
  const modes: Array<{ value: GroupMode; label: string }> = [
    { value: "inOrder", label: t("依次") },
    { value: "rotate", label: t("轮换") },
    { value: "weighted", label: t("按权重") },
    { value: "usage", label: t("额度优先") },
    { value: "fast", label: t("速度优先") },
    { value: "manual", label: t("手动") },
  ];
  const stickies: GroupSticky[] = ["cache", "session", "off"];
  const patternText = pattern.trim();
  const patternHits = isPattern(patternText) ? patternMatches(payload, patternText) : [];
  const addPattern = () => {
    if (!isPattern(patternText)) return;
    if (!members.some((member) => member.model === patternText)) setMembers([...members, { model: patternText, weight: 1 }]);
    setPattern("");
  };
  const memberHint = mode === "inOrder"
    ? t("从上到下依次尝试")
    : mode === "usage" || mode === "fast"
      ? t("额度 / 速度相同时按列表顺序")
      : t("答不了时按列表顺序换下一个");
  return createPortal(
    <div className="gw-modal-backdrop gw-portal" onMouseDown={(event) => { if (event.target === event.currentTarget) onCancel(); }}>
      <form
        className="gw-modal gw-group-modal"
        onSubmit={(event) => {
          event.preventDefault();
          if (!error) onSave({ id: group?.id ?? "", name: name.trim(), mode, sticky, members });
        }}
      >
        <header>
          <span className="gw-avatar gw-avatar-plus"><Waypoints className="h-4 w-4" /></span>
          <strong>{group ? tf("编辑 {0}", [group.name]) : t("新建模型组")}</strong>
          <button aria-label={t("关闭")} className="gw-icon-button" onClick={onCancel} type="button"><X className="h-4 w-4" /></button>
        </header>
        <div className="gw-modal-body">
          <label className="gw-field">
            <span>{t("名称")} <small>{group ? groupModelId(group.id) : name.trim() ? groupModelId(slugify(name)) : ""}</small></span>
            <Input autoFocus onChange={(event) => setName(event.currentTarget.value)} placeholder={t("例如 日常编码")} value={name} />
          </label>
          <div className="gw-field">
            <span>{t("模式")}</span>
            <div className="gw-tabs gw-mode-tabs" role="radiogroup">
              {modes.map((item) => (
                <button aria-checked={mode === item.value} data-active={mode === item.value} key={item.value} onClick={() => setMode(item.value)} role="radio" type="button">
                  {item.label}
                </button>
              ))}
            </div>
            <small className="gw-muted">{t(groupModeDescription(mode))}</small>
          </div>
          <div className="gw-field">
            <span>{t("会话固定")}</span>
            <div className="gw-tabs gw-mode-tabs" role="radiogroup">
              {stickies.map((item) => (
                <button aria-checked={sticky === item} data-active={sticky === item} key={item} onClick={() => setSticky(item)} role="radio" type="button">
                  {t(groupStickyLabel(item))}
                </button>
              ))}
            </div>
            <small className="gw-muted">{t(groupStickyDescription(sticky))}</small>
          </div>
          <div className="gw-field">
            <span>{t("成员")} <small>{memberHint}</small></span>
            <div className="gw-member-list">
              {members.map((member, index) => {
                const view = modelDisplay(payload, member.model);
                const wildcard = isPattern(member.model);
                const hits = wildcard ? patternMatches(payload, member.model) : [];
                return (
                  <div className="gw-member-edit" data-pattern={wildcard} key={`${member.model}-${index}`}>
                    {wildcard ? <Asterisk className="gw-picker-glyph h-4 w-4" /> : view.group ? <Waypoints className="gw-picker-glyph h-4 w-4" /> : <Avatar name={view.sub || view.title} size="sm" />}
                    <span className="gw-member-edit-name">
                      <strong>{wildcard ? member.model : view.title}</strong>
                      {wildcard ? (
                        <small title={hits.join("\n")}>{hits.length ? tf("通配符 · 现在匹配 {0} 个模型", [hits.length]) : t("通配符 · 现在没有匹配的模型")}</small>
                      ) : (
                        <small>{view.group ? view.sub : view.sub || t("池里还没有提供它的供应商")}</small>
                      )}
                    </span>
                    {mode === "weighted" ? (
                      <label className="gw-inline-field">
                        <span>{t("权重")}</span>
                        <input
                          className="gw-inline-input gw-weight-input"
                          min={1}
                          onBlur={() => {
                            setWeightText((previous) => {
                              const next = { ...previous };
                              delete next[index];
                              return next;
                            });
                          }}
                          onChange={(event) => {
                            const text = event.currentTarget.value;
                            setWeightText((previous) => ({ ...previous, [index]: text }));
                            // 能读成数的就先记下；删空、输入到一半时保留原值，失焦后显示规范值。
                            const parsed = parseWeight(text);
                            if (parsed !== null) {
                              const next = [...members];
                              next[index] = { ...member, weight: parsed };
                              setMembers(next);
                            }
                          }}
                          type="number"
                          value={weightText[index] ?? String(member.weight)}
                        />
                      </label>
                    ) : null}
                    <button aria-label={t("上移")} className="gw-icon-button" disabled={index === 0} onClick={() => move(index, -1)} type="button"><ChevronUp className="h-4 w-4" /></button>
                    <button aria-label={t("下移")} className="gw-icon-button" disabled={index === members.length - 1} onClick={() => move(index, 1)} type="button"><ChevronDown className="h-4 w-4" /></button>
                    <button aria-label={t("移除")} className="gw-icon-button" onClick={() => setMembers(members.filter((_, position) => position !== index))} type="button"><X className="h-4 w-4" /></button>
                  </div>
                );
              })}
              <div className="gw-add-line">
                <button className="gw-add-row" onClick={() => setPicking(!picking)} ref={addRef} type="button">
                  <Plus className="h-4 w-4" />
                  {t("添加成员")}
                </button>
                <span className="gw-pattern-input">
                  <Asterisk aria-hidden className="h-3.5 w-3.5" />
                  <input
                    aria-label={t("通配符成员")}
                    onChange={(event) => setPattern(event.currentTarget.value)}
                    onKeyDown={(event) => {
                      if (event.key === "Enter") {
                        event.preventDefault();
                        addPattern();
                      }
                    }}
                    placeholder={t("通配符，如 openrouter/*:free")}
                    value={pattern}
                  />
                  {patternText ? (
                    <button className="gw-text-button strong" disabled={!isPattern(patternText)} onClick={addPattern} title={patternHits.slice(0, 20).join("\n")} type="button">
                      {isPattern(patternText) ? tf("添加（{0} 个）", [patternHits.length]) : t("要带 *")}
                    </button>
                  ) : null}
                </span>
              </div>
              <small className="gw-muted">{t("通配符按「供应商 id/模型」或模型名匹配，* 代表任意字符；上游新增的模型会自动算进来。")}</small>
              <Floating anchorRef={addRef} className="gw-model-popover" onClose={() => setPicking(false)} open={picking} width={520}>
                <ModelList
                  exclude={NO_EXCLUDE}
                  onPick={(item) => {
                    setPicking(false);
                    if (!members.some((member) => member.model === item.model)) setMembers([...members, { model: item.model, weight: 1 }]);
                  }}
                  payload={payload}
                  selectedModel=""
                  skipGroup={group?.id}
                />
              </Floating>
            </div>
          </div>
        </div>
        <footer>
          <span className="gw-error">{error}</span>
          <Button onClick={onCancel} size="sm" type="button" variant="ghost">{t("取消")}</Button>
          <Button disabled={busy !== null || Boolean(error)} size="sm" type="submit">{busy ? t("保存中") : t("保存")}</Button>
        </footer>
      </form>
    </div>,
    document.body,
  );
}

/* —————————————————————— 网关 —————————————————————— */

function GatewayView({ payload, busy, run }: { payload: GatewayPayload | null; busy: string | null; run: Run }) {
  if (!payload) return null;
  const mode = payload.gateway.keyPolicy.mode;
  const restoreOnExit = payload.gateway.restoreOnExit;
  const modelSuffix = payload.gateway.modelSuffix !== false;
  const saveOptions = (keyRotation: string, restore: boolean) => void run("options", "save_gateway_options", { keyRotation, restoreOnExit: restore }, t("保存网关选项"), true);
  return (
    <>
      <SectionLabel>{t("网关")}</SectionLabel>
      <div className="gw-card">
        <div className="gw-setting-row">
          <div>
            <strong>{t("监听地址")}</strong>
            <small>{t("只监听本机；各 Agent 用自己的路径和 token 接入。")}</small>
          </div>
          <code>{`http://127.0.0.1:${payload.port}`}</code>
          <span className="gw-pill" data-tone={payload.helperRunning ? "ok" : "warning"}>{payload.helperRunning ? t("运行中") : t("未运行")}</span>
        </div>
        {payload.agents.map((agent) => {
          const binding = payload.gateway.bindings[agent.agent];
          return (
            <div className="gw-setting-row" key={agent.agent}>
              <div>
                <strong>{tf("{0} 接入地址", [agent.name])}</strong>
                <small>{agent.enabled ? t("已写入该 Agent 的配置") : t("开启后写入该 Agent 的配置")}</small>
              </div>
              <code title={agent.baseUrl}>{agent.baseUrl}</code>
              {binding?.token ? (
                <button className="gw-text-button" disabled={busy !== null} onClick={() => void run(`rotate:${agent.agent}`, "rotate_gateway_token", { agent: agent.agent }, t("更换 token"))} title={t("换一个新的本地 token，旧 token 立即失效")} type="button">
                  {t("更换 token")}
                </button>
              ) : <span />}
            </div>
          );
        })}
      </div>

      <SectionLabel>{t("网关选项")}</SectionLabel>
      <div className="gw-card">
        <div className="gw-setting-row">
          <div>
            <strong>{t("多 Key 轮换")}</strong>
            <small>{mode === "roundRobin" ? t("每个请求换下一个 Key，用量更平均，但上游缓存命中会变低。") : t("同一会话固定一个 Key，出错才换；保住上游的提示词缓存。")}</small>
          </div>
          <span />
          <AppSelect
            disabled={busy !== null}
            label={t("多 Key 轮换")}
            onChange={(next) => saveOptions(next, restoreOnExit)}
            options={[
              { value: "sticky", label: t("会话粘性（推荐）"), hint: t("同一会话固定一个 Key，出错才换") },
              { value: "roundRobin", label: t("逐请求轮换"), hint: t("每个请求换下一个 Key") },
            ]}
            value={mode}
          />
        </div>
        <div className="gw-setting-row">
          <div>
            <strong>{t("退出 Codex++ 时恢复直连")}</strong>
            <small>{restoreOnExit
              ? t("网关只在 Codex++ 运行时可用。退出时把 Agent 切回直连，下次启动再自动指回网关。")
              : t("Agent 一直指着网关：退出 Codex++ 后连不上，再打开就恢复。安装更新前照样会切回直连。")}</small>
          </div>
          <span />
          <Switch disabled={busy !== null} label={t("退出 Codex++ 时恢复直连")} on={restoreOnExit} onToggle={() => saveOptions(mode, !restoreOnExit)} />
        </div>
        <div className="gw-setting-row">
          <div>
            <strong>{t("模型名后带供应商")}</strong>
            <small>{modelSuffix
              ? t("多家都有的模型，在 Codex 的模型列表和输入框里显示成「GPT-6 Astra（ChatGPT Pro）」，自动组显示「（自动）」。")
              : t("只显示模型名「GPT-6 Astra」，看不出是哪一家；Codex 选择器里仍按供应商分组。")}</small>
          </div>
          <span />
          <Switch
            disabled={busy !== null}
            label={t("模型名后带供应商")}
            on={modelSuffix}
            onToggle={() => void run("model-suffix", "save_gateway_model_suffix", { enabled: !modelSuffix }, t("保存网关选项"), true)}
          />
        </div>
      </div>
    </>
  );
}
