/** 本地网关 · 智能体：每个 Agent 一行，接入后供应商池里的模型都出现在它自己的模型列表里，也可以在这里直接选。 */
import { useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode, type RefObject } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { appConfirm } from "./app-dialog";
import { Check, ChevronDown, ChevronRight, ChevronUp, Layers, List, Search, Shuffle, Star, Waypoints, X } from "lucide-react";
import { t, tf } from "./i18n";
import {
  agentModels,
  agentPoolProviders,
  bindingFromDraft,
  draftFromBinding,
  modelDisplay,
  parseContextWindow,
  pickerSections,
  providerEnabled,
  validateDraft,
  type BindingDraft,
  type GatewayAgentId,
  type GatewayAgentView,
  type GatewayPayload,
  type PickerItem,
  maskAccountName,
  type ImageOption,
  isFavoriteModel,
  toggleFavoriteModel,
  type GatewayResult,
} from "./gateway-model";
import { AppSelect } from "./AppSelect";
import { shortProviderName } from "./GatewayModelMatrix";
import { GatewayNavContext, publishGatewayPayload } from "./gateway-store";
import { useEscapeLayer } from "./use-escape-layer";
import { AGENT_ICONS, AutoTag, Avatar, ModelViewGlyph, Switch, avatarOf, loadFavorites, saveFavorites, type Notify, type Run, AgentIcon } from "./gateway-ui";

export function AgentsView({
  payload,
  busy,
  run,
  notify,
  onManage,
}: {
  payload: GatewayPayload | null;
  busy: string | null;
  run: Run;
  notify: Notify;
  onManage: () => void;
}) {
  const [expanded, setExpanded] = useState<string | null>(null);
  const agents = payload?.agents ?? [];
  const connected = agents.filter((agent) => agent.enabled);
  const others = agents.filter((agent) => !agent.enabled);
  const row = (agent: GatewayAgentView) => (
    <AgentRow
      agent={agent}
      busy={busy}
      expanded={expanded === agent.agent}
      key={agent.agent}
      notify={notify}
      onManage={onManage}
      onToggleExpand={() => setExpanded(expanded === agent.agent ? null : agent.agent)}
      payload={payload}
      run={run}
    />
  );
  return (
    <>
      <div className="gw-card gw-agent-card">
        {connected.length ? <div className="gw-subhead">{t("已接入")}<em>{connected.length}</em></div> : null}
        {connected.map(row)}
        {others.length ? <div className="gw-subhead">{t("未接入")}<em>{others.length}</em></div> : null}
        {others.map(row)}
        {!payload ? <div className="gw-empty">{t("正在读取…")}</div> : null}
      </div>
    </>
  );
}

/** 在别的页面（总览）里直接接入 Agent：弹窗里就是「Agent 接入」的列表，不用跳页。
 * 列表的样式挂在 `.shell` 下，所以弹窗就地渲染（固定定位），不挂到 body。 */
export function AgentConnectDialog({
  payload,
  busy,
  run,
  notify,
  onManage,
  onClose,
}: {
  payload: GatewayPayload | null;
  busy: string | null;
  run: Run;
  notify: Notify;
  onManage: () => void;
  onClose: () => void;
}) {
  // Esc 栈：里面的下拉、确认框开着时 Esc 先关它们，不会连带关掉这个弹窗。
  useEscapeLayer(true, onClose);
  return (
    <div className="gw-modal-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
      <div aria-label={t("接入 Agent")} className="gw-modal gw-agent-modal" role="dialog">
        <header>
          <strong>{t("接入 Agent")}</strong>
          <button aria-label={t("关闭")} className="gw-icon-button" onClick={onClose} type="button"><X className="h-4 w-4" /></button>
        </header>
        <div className="gw-modal-body gw-agent-modal-body">
          <AgentsView busy={busy} notify={notify} onManage={onManage} payload={payload} run={run} />
        </div>
      </div>
    </div>
  );
}

function AgentRow({
  agent,
  payload,
  busy,
  expanded,
  onToggleExpand,
  onManage,
  run,
  notify,
}: {
  agent: GatewayAgentView;
  payload: GatewayPayload | null;
  busy: string | null;
  expanded: boolean;
  onToggleExpand: () => void;
  onManage: () => void;
  run: Run;
  notify: Notify;
}) {
  const binding = payload?.gateway.bindings[agent.agent as GatewayAgentId];
  const [draft, setDraft] = useState<BindingDraft>(() => draftFromBinding(binding));
  const [pickerOpen, setPickerOpen] = useState(false);
  // 后端保存后（或别处改动后）用最新绑定重置表单。
  const bindingKey = JSON.stringify(binding ?? null);
  useEffect(() => {
    setDraft(draftFromBinding(binding));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bindingKey]);

  const isClaude = agent.agent === "claude";
  const connected = agent.enabled;
  const save = (next: BindingDraft) => {
    setDraft(next);
    const error = validateDraft(next, payload);
    if (error) {
      void notify(t("还不能接入"), t(error), "warning");
      return Promise.resolve(false);
    }
    return run(`save:${agent.agent}`, "save_gateway_binding", { agent: agent.agent, binding: bindingFromDraft(next, binding) }, t("保存网关绑定"), connected);
  };
  const disconnect = async () => {
    const ok = await appConfirm(tf("断开 {0} 与 Codex++ 的连接？会还原它接入前的设置。", [agent.name]), { title: t("断开并还原"), confirmText: t("断开并还原"), danger: true });
    if (ok) await run(`disable:${agent.agent}`, "disable_gateway_binding", { agent: agent.agent }, t("断开并还原"));
  };
  // 在选择器里选模型：已接入时直接改默认模型，未接入时选完即接入。
  const pick = (item: PickerItem) => {
    setPickerOpen(false);
    void save({ ...draft, model: item.model });
  };
  const toggle = () => {
    if (connected) {
      void disconnect();
      return;
    }
    const error = validateDraft(draft, payload);
    if (error) {
      setPickerOpen(true);
      void notify(t("还不能接入"), t(error), "warning");
      return;
    }
    void save(draft);
  };

  const running = Boolean(payload?.helperRunning);
  // 配置被别的工具改掉了：请求不再经过 Codex++，「已接入」名不副实。
  const drift = connected ? agent.drift ?? null : null;
  const tone = connected ? (running && !drift ? "on" : "error") : "off";
  const modelCount = agentModels(payload, draft.poolExclude).length;
  const reconnect = () =>
    run(`save:${agent.agent}`, "save_gateway_binding", { agent: agent.agent, binding: bindingFromDraft(draft, binding) }, t("重新接入"));
  // 状态标签：短词；出问题时才写一句说明。
  const badge = drift ? t("被改动") : connected ? (running ? t("已接入") : t("网关未运行")) : t("未接入");
  const note = drift
    ? tf("配置被 {0} 改了，{1} 现在不经过 Codex++", [drift.by || t("其它工具"), agent.name])
    : connected && isClaude
    ? "opus · sonnet · haiku"
    : "";

  return (
    <div className="gw-agent" data-expanded={expanded}>
      {/* 已接入时整行点击展开详情，模型下拉和开关各管各的。 */}
      <div
        className="gw-agent-row"
        data-clickable={connected}
        onClick={(event) => {
          if (!connected || (event.target as HTMLElement).closest("button, a, input")) return;
          onToggleExpand();
        }}
      >
        <span className="gw-agent-icon-box"><AgentIcon agent={agent.agent} className="gw-agent-icon" /></span>
        <div className="gw-agent-main">
          <div className="gw-agent-title">
            <strong>{agent.name}</strong>
            {agent.version ? <span className="gw-version">{agent.version}</span> : null}
          </div>
          <small className="gw-agent-meta" title={drift?.endpoint ? tf("现在连的是 {0}", [drift.endpoint]) : undefined}>
            <span className="gw-agent-badge" data-tone={tone}>{badge}</span>
            {connected && !drift ? (
              <span className="gw-agent-count" title={tf("{0} 个模型", [modelCount])}>
                <Layers aria-hidden="true" />
                {modelCount}
              </span>
            ) : null}
            {note ? <span>{note}</span> : null}
            {drift ? (
              <button className="gw-link gw-drift-reconnect" disabled={busy !== null} onClick={() => void reconnect()} type="button">
                {t("重新接入")}
              </button>
            ) : null}
          </small>
        </div>
        <ModelPicker
          agent={agent}
          connected={connected}
          draft={draft}
          onDisconnect={() => {
            setPickerOpen(false);
            if (connected) void disconnect();
          }}
          onOpenChange={setPickerOpen}
          onPick={pick}
          open={pickerOpen}
          payload={payload}
        />
        <Switch disabled={busy !== null} label={tf("{0} 接入 Codex++", [agent.name])} on={connected} onToggle={toggle} />
        <button
          aria-expanded={expanded}
          aria-label={isClaude ? t("档位") : t("详情")}
          className="gw-chevron"
          data-open={expanded}
          disabled={!connected}
          onClick={onToggleExpand}
          type="button"
        >
          <ChevronRight className="h-4 w-4" />
        </button>
      </div>
      {expanded && connected ? (
        <>
          <AgentDetail agent={agent} busy={busy} draft={draft} onDisconnect={() => void disconnect()} onSave={(next) => void save(next)} payload={payload} run={run} />
          <div className="gw-collapse-bar">
            <button className="gw-text-button" onClick={onToggleExpand} type="button">
              {t("收起")}
              <ChevronUp className="h-3.5 w-3.5" />
            </button>
            <button className="gw-text-button gw-collapse-manage" onClick={onManage} type="button">
              <List className="h-4 w-4" />
              {t("管理")}
            </button>
          </div>
        </>
      ) : null}
    </div>
  );
}

/** 展开的详情：在 Agent 里怎么切模型、模型列表（可挑选供应商）、Claude 档位、其它选项、改动的文件。 */
function AgentDetail({
  agent,
  payload,
  draft,
  busy,
  run,
  onSave,
  onDisconnect,
}: {
  agent: GatewayAgentView;
  payload: GatewayPayload | null;
  draft: BindingDraft;
  busy: string | null;
  run: Run;
  onSave: (next: BindingDraft) => void;
  onDisconnect: () => void;
}) {
  const isClaude = agent.agent === "claude";
  const [selecting, setSelecting] = useState(false);
  const [windowText, setWindowText] = useState(draft.contextWindow);
  const selectRef = useRef<HTMLButtonElement>(null);
  useEffect(() => setWindowText(draft.contextWindow), [draft.contextWindow]);
  const providers = (payload?.pool ?? []).filter(providerEnabled);
  const used = agentPoolProviders(payload, draft.poolExclude);
  const groups = (payload?.groups ?? []).filter((group) => group.members.length);
  const tiers: Array<[keyof BindingDraft, string]> = [
    ["opus", "Opus"],
    ["sonnet", "Sonnet"],
    ["haiku", t("小模型")],
    ["subagent", t("子代理")],
  ];

  return (
    <div className="gw-agent-detail">
      {/* 只有 Claude Code 的档位切换需要说明；其它 Agent 在自己的模型列表里直接选，不用再讲一遍。 */}
      {isClaude ? (
        <div className="gw-detail-row">
          <span className="gw-detail-label">{tf("在 {0} 里", [agent.name])}</span>
          <div className="gw-detail-value">
            <span>{t("在 Claude Code 的")} <code className="gw-code">/model</code> {t("里切换 opus · sonnet · haiku 档，各档用的模型在下面设置。")}</span>
          </div>
        </div>
      ) : null}
      <div className="gw-detail-row">
        <span className="gw-detail-label">{t("用哪些供应商")}</span>
        <div className="gw-detail-value gw-chip-row">
          {used.map((entry) => (
            <span className="gw-model-chip" key={entry.id}>
              <Avatar baseUrl={entry.baseUrl} name={entry.name} size="sm" />
              {maskAccountName(entry.name)}
              <em>{entry.models.length}</em>
            </span>
          ))}
          {groups.length ? (
            <span className="gw-model-chip">
              <Waypoints className="h-3.5 w-3.5" />
              {t("模型组")}
              <em>{groups.length}</em>
            </span>
          ) : null}
          {used.length === 0 ? <span className="gw-muted">{t("没有可用的供应商")}</span> : null}
          <button className="gw-text-button" onClick={() => setSelecting(!selecting)} ref={selectRef} type="button">
            {t("挑选")}
            <ChevronRight className="h-3.5 w-3.5" />
          </button>
          <Floating anchorRef={selectRef} onClose={() => setSelecting(false)} open={selecting} width={300}>
            <div className="gw-select-list">
              <div className="gw-popover-heading">{t("给这个 Agent 用的供应商")}</div>
              {providers.map((entry) => {
                const on = !draft.poolExclude.includes(entry.id);
                return (
                  <label className="gw-select-item" key={entry.id}>
                    <input
                      checked={on}
                      disabled={busy !== null}
                      onChange={() => onSave({ ...draft, poolExclude: on ? [...draft.poolExclude, entry.id] : draft.poolExclude.filter((id) => id !== entry.id) })}
                      type="checkbox"
                    />
                    <Avatar baseUrl={entry.baseUrl} name={entry.name} size="sm" />
                    <span>{maskAccountName(entry.name)}</span>
                    <em>{entry.models.length}</em>
                  </label>
                );
              })}
              {providers.length === 0 ? <div className="gw-empty">{t("供应商池是空的")}</div> : null}
            </div>
          </Floating>
        </div>
      </div>
      {isClaude ? (
        <div className="gw-detail-row">
          <span className="gw-detail-label">{t("档位")}</span>
          <div className="gw-detail-value gw-chip-row">
            {tiers.map(([key, label]) => (
              <TierPicker
                key={key}
                label={label}
                onPick={(model) => onSave({ ...draft, [key]: model })}
                payload={payload}
                exclude={draft.poolExclude}
                value={String(draft[key])}
              />
            ))}
          </div>
        </div>
      ) : null}
      {/* 目前只有 Codex 自己会发生图请求；别的 Agent 有了生图工具再显示。 */}
      {agent.agent === "codex" ? <ImageModelRow agent={agent.agent} busy={busy} payload={payload} run={run} /> : null}
      {/* Codex「替我审批」固定请求 codex-auto-review：默认改写成主模型，可单独指定，避免落到 ChatGPT 订阅额度上（#2703）。 */}
      {/* Codex 给会话起名时另开临时线程（首次用内置轻量模型，重新起名用会话当前模型），默认照原模型路由，可单独指定。 */}
      {agent.agent === "codex" ? (
        <div className="gw-detail-row">
          <span className="gw-detail-label">{t("起名模型")}</span>
          <div className="gw-detail-value gw-chip-row" title={t("Codex 给会话起名、重新起名的请求发给这个模型；不指定时照 Codex 自己选的模型路由")}>
            <TierPicker
              emptyLabel={t("不单独指定")}
              exclude={draft.poolExclude}
              label={t("起名")}
              onPick={(model) => onSave({ ...draft, titleModel: model })}
              payload={payload}
              value={draft.titleModel}
            />
          </div>
        </div>
      ) : null}
      {agent.agent === "codex" ? (
        <div className="gw-detail-row">
          <span className="gw-detail-label">{t("审批模型")}</span>
          <div className="gw-detail-value gw-chip-row" title={t("Codex「替我审批」的请求（codex-auto-review）发给这个模型；同默认模型时按「未知模型名」规则处理")}>
            <TierPicker
              exclude={draft.poolExclude}
              label={t("审批")}
              onPick={(model) => onSave({ ...draft, autoReviewModel: model })}
              payload={payload}
              value={draft.autoReviewModel}
            />
          </div>
        </div>
      ) : null}
      <div className="gw-detail-row">
        <span className="gw-detail-label">{t("其它")}</span>
        <div className="gw-detail-value gw-chip-row">
          <div className="gw-inline-field">
            <span>{t("未知模型名")}</span>
            <AppSelect
              disabled={busy !== null}
              label={t("未知模型名")}
              menuWidth={240}
              onChange={(next) => onSave({ ...draft, unknownModel: next as BindingDraft["unknownModel"] })}
              options={[
                { value: "mapToDefault", label: t("改写成默认模型"), hint: t("池里没有的模型名换成主模型") },
                { value: "passthrough", label: t("原样转发"), hint: t("按客户端给的名字发给上游") },
              ]}
              value={draft.unknownModel}
            />
          </div>
          {/* 窗口按模型在供应商池里填；这里只给池里没写窗口的模型兜底。 */}
          <label className="gw-inline-field" title={t("供应商池里没写窗口的模型用这个；按模型的窗口在供应商池的详情里填")}>
            <span>{t("默认窗口")}</span>
            <input
              className="gw-inline-input"
              onBlur={() => {
                const parsed = parseContextWindow(windowText);
                if (windowText !== draft.contextWindow && (parsed === null || (Number.isFinite(parsed) && parsed >= 1000))) onSave({ ...draft, contextWindow: windowText });
              }}
              onChange={(event) => setWindowText(event.currentTarget.value)}
              placeholder={t("例如 200k")}
              value={windowText}
            />
          </label>
          {isClaude ? (
            <label className="gw-inline-field gw-inline-check" title={t("非 claude-* 模型名时 Claude Code 默认不发 effort，打开后照常发送。")}>
              <input checked={draft.claudeEnableEffort} disabled={busy !== null} onChange={(event) => onSave({ ...draft, claudeEnableEffort: event.currentTarget.checked })} type="checkbox" />
              <span>{t("强制启用推理强度")}</span>
            </label>
          ) : null}
          <label className="gw-inline-field gw-inline-check" title={t("关掉后，新添加的供应商默认不给这个 Agent 用；添加时仍可手动勾选。")}>
            <input checked={draft.autoJoinProviders} disabled={busy !== null} onChange={(event) => onSave({ ...draft, autoJoinProviders: event.currentTarget.checked })} type="checkbox" />
            <span>{t("添加供应商时自动加入")}</span>
          </label>
        </div>
      </div>
      <div className="gw-detail-row gw-detail-foot">
        <span className="gw-detail-label gw-detail-file">
          {t("改动了")} <code>{agent.configPath ?? ""}</code>
        </span>
        <div className="gw-detail-value">
          <button className="gw-text-button" onClick={() => void run(`reveal:${agent.agent}`, "reveal_gateway_agent_config", { agent: agent.agent }, t("查看"), true)} type="button">
            {t("查看")}
            <ChevronRight className="h-3.5 w-3.5" />
          </button>
          <button className="gw-danger-link" disabled={busy !== null} onClick={onDisconnect} type="button">{t("断开并还原")}</button>
        </div>
      </div>
    </div>
  );
}

/** 挂到 body 的浮层：跟随锚点定位，下方放不下时翻到上方；点外面或按 Esc 关闭。 */
export function Floating({
  anchorRef,
  open,
  onClose,
  width,
  children,
  className = "",
}: {
  anchorRef: RefObject<HTMLElement | null>;
  open: boolean;
  onClose: () => void;
  width: number;
  children: ReactNode;
  className?: string;
}) {
  const panelRef = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState<CSSProperties | null>(null);
  useLayoutEffect(() => {
    if (!open) return;
    const place = () => {
      const rect = anchorRef.current?.getBoundingClientRect();
      if (!rect) return;
      const panelWidth = Math.min(width, window.innerWidth - 24);
      const left = Math.max(12, Math.min(rect.right - panelWidth, window.innerWidth - panelWidth - 12));
      const below = window.innerHeight - rect.bottom - 18;
      const above = rect.top - 18;
      const upward = below < 320 && above > below;
      const maxHeight = Math.min(520, upward ? above : below);
      setPosition(upward
        ? { left, width: panelWidth, bottom: window.innerHeight - rect.top + 6, maxHeight }
        : { left, width: panelWidth, top: rect.bottom + 6, maxHeight });
    };
    place();
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [open, anchorRef, width]);
  useEffect(() => {
    if (!open) return;
    const close = (event: MouseEvent) => {
      const node = event.target as Node;
      if (!anchorRef.current?.contains(node) && !panelRef.current?.contains(node)) onClose();
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [open, onClose, anchorRef]);
  // Esc 只关这个浮层（它总在外面的弹窗之后打开，位于 Esc 栈顶）。
  useEscapeLayer(open, onClose);
  if (!open || !position) return null;
  return createPortal(
    <div className={`gw-popover gw-portal ${className}`} ref={panelRef} style={position}>
      {children}
    </div>,
    document.body,
  );
}

/** 模型列表：左侧全部 / 收藏 / 按供应商跳转，右侧分组列出模型；顶部是「默认（不经 Codex++）」与「断开」。 */
export function ModelList({
  payload,
  exclude,
  selectedModel,
  head,
  skipGroup,
  onPick,
}: {
  payload: GatewayPayload | null;
  exclude: string[];
  selectedModel: string;
  head?: ReactNode;
  skipGroup?: string;
  onPick: (item: PickerItem) => void;
}) {
  const nav = useContext(GatewayNavContext);
  const [query, setQuery] = useState("");
  // 筛选：全部 / 只看收藏 / 只看某一家（或自动组、模型组）。再点一次选中的筛选回到全部。
  const [scope, setScope] = useState<"all" | "favorites" | { section: string }>("all");
  const favoritesOnly = scope === "favorites";
  const sectionOnly = typeof scope === "object" ? scope.section : null;
  // 收藏存在网关里（Codex 的选择器也要显示）；旧版存在本窗口的 localStorage，网关里还没有时沿用并迁过去。
  const stored = payload?.gateway.favoriteModels;
  const [favorites, setFavorites] = useState<string[]>(() => (stored && stored.length ? stored : loadFavorites()));
  useEffect(() => {
    if (stored && stored.length) {
      setFavorites(stored);
      return;
    }
    const legacy = loadFavorites();
    if (payload && legacy.length) void persistFavorites(legacy);
    // 只在网关里的收藏变化时同步。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [stored?.join("\n")]);
  const listRef = useRef<HTMLDivElement>(null);
  const sections = useMemo(() => {
    const all = pickerSections(payload, query, avatarOf, exclude, skipGroup);
    // 手输的模型 ID（custom）不属于哪一家，按供应商筛选时也留着。
    if (sectionOnly) return all.filter((section) => section.key === sectionOnly || section.key === "custom");
    if (!favoritesOnly) return all;
    return all
      .map((section) => ({ ...section, items: section.items.filter((item) => isFavoriteModel(favorites, item.model, payload)) }))
      .filter((section) => section.items.length);
  }, [payload, query, exclude, favoritesOnly, sectionOnly, favorites, skipGroup]);
  const toggleFavorite = (model: string) => {
    const next = toggleFavoriteModel(favorites, model, payload);
    setFavorites(next);
    void persistFavorites(next);
  };
  const pickScope = (next: "all" | "favorites" | { section: string }) => {
    const same = typeof next === "object" ? sectionOnly === next.section : scope === next;
    setScope(same ? "all" : next);
    if (listRef.current) listRef.current.scrollTop = 0;
  };
  const isSelected = (item: PickerItem) => Boolean(selectedModel) && item.model.toLowerCase() === selectedModel.toLowerCase();

  const filters = useMemo(
    () => pickerSections(payload, "", avatarOf, exclude, skipGroup).filter((section) => section.key !== "custom"),
    [payload, exclude, skipGroup],
  );
  const total = sections.reduce((sum, section) => sum + section.items.length, 0);

  // 排版：搜索 + 一排横向筛选在上；列表每个模型两行（模型名 / 供应商）；「默认」「断开」这类操作在底部。
  return (
    <div className="gw-mp">
      <div className="gw-mp-top">
        <label className="gw-mp-search">
          <Search className="h-4 w-4" />
          <input autoFocus onChange={(event) => setQuery(event.currentTarget.value)} placeholder={t("筛选，或输入任意模型 ID…")} value={query} />
          <em>{tf("{0} 个", [total])}</em>
        </label>
        <div className="gw-mp-filters">
          <button data-active={scope === "all"} onClick={() => { setScope("all"); setQuery(""); if (listRef.current) listRef.current.scrollTop = 0; }} type="button">{t("全部")}</button>
          <button data-active={favoritesOnly} onClick={() => pickScope("favorites")} type="button"><Star className="h-3 w-3" />{t("收藏")}</button>
          {filters.map((section) => (
            <button data-active={sectionOnly === section.key} key={section.key} onClick={() => pickScope({ section: section.key })} title={section.key === "group" || section.key === "auto" ? t(section.title) : section.title} type="button">
              {section.key === "group" ? <Waypoints className="h-3 w-3" /> : section.key === "auto" ? <Shuffle className="h-3 w-3" /> : <Avatar name={section.title} size="sm" />}
              <span>{section.key === "group" || section.key === "auto" ? t(section.title) : section.title.split(" · ")[0]}</span>
            </button>
          ))}
        </div>
      </div>
      <div className="gw-mp-list" ref={listRef}>
        {sections.length === 0 ? (
          <div className="gw-empty">
            {favoritesOnly ? t("还没有收藏的模型，点模型右侧的星标收藏。") : payload?.pool.length ? t("没有匹配的模型") : t("供应商池是空的，先到「供应商」里添加。")}
          </div>
        ) : null}
        {sections.map((section) => (
          <section className="gw-mp-section" data-section={section.key} key={section.key}>
            <div className="gw-mp-heading">
              {section.key === "group" ? <Waypoints className="h-3.5 w-3.5" /> : section.key === "auto" ? <Shuffle className="h-3.5 w-3.5" /> : section.key === "custom" ? null : <Avatar name={section.title} size="sm" />}
              <span>{section.key === "group" || section.key === "custom" || section.key === "auto" ? t(section.title) : section.title}</span>
              {section.key === "group" && nav.openModelGroups ? (
                <button className="gw-mp-manage" onClick={nav.openModelGroups} type="button">{t("管理")}</button>
              ) : null}
              <em>{section.items.length}</em>
            </div>
            {section.items.map((item) => {
              const selected = isSelected(item);
              const favorite = isFavoriteModel(favorites, item.model, payload);
              return (
                <div
                  aria-selected={selected}
                  className="gw-mp-item"
                  data-selected={selected}
                  key={item.model}
                  onClick={() => onPick(item)}
                  onKeyDown={(event) => { if (event.key === "Enter") onPick(item); }}
                  role="option"
                  tabIndex={0}
                >
                  <span className="gw-mp-text">
                    <strong title={item.model}>{item.label}</strong>
                    {item.custom ? <small>{t("自定义模型 ID")}</small> : item.sub && item.sub !== section.title ? <small>{item.sub}</small> : null}
                  </span>
                  <button
                    aria-label={favorite ? t("取消收藏") : t("收藏")}
                    className="gw-mp-star"
                    data-on={favorite}
                    onClick={(event) => { event.stopPropagation(); toggleFavorite(item.model); }}
                    type="button"
                  >
                    <Star className="h-3.5 w-3.5" />
                  </button>
                  {selected ? <Check className="gw-mp-check h-4 w-4" /> : null}
                </div>
              );
            })}
          </section>
        ))}
      </div>
      {head ? <div className="gw-mp-foot">{head}</div> : null}
    </div>
  );
}

/** 一项生图选项的显示名：「ChatGPT Pro · gpt-image」。 */
function imageOptionLabel(option: ImageOption): string {
  return `${maskAccountName(shortProviderName(option.providerName))} · ${option.model}`;
}

/** Agent 详情里的「生图模型」：这个 Agent 的生图请求经网关发给谁。 */
function ImageModelRow({ agent, payload, busy, run }: { agent: string; payload: GatewayPayload | null; busy: string | null; run: Run }) {
  const value = payload?.gateway.imageModels?.[agent] ?? "";
  const inlineUrls = !(payload?.gateway.imageKeepUrls ?? []).includes(agent);
  const options = payload?.imageOptions ?? [];
  const auto = payload?.imageAuto ?? null;
  const known = value === "" || value === "off" || options.some((option) => option.id === value);
  const items = [
    { value: "", label: auto ? tf("自动 · {0}", [imageOptionLabel(auto)]) : t("自动"), hint: t("ChatGPT 账号优先，其次按池里的顺序每家试一次，优先用请求里写的模型") },
    ...options.map((option) => ({
      value: option.id,
      label: imageOptionLabel(option),
      hint: option.vendorPicks ? t("ChatGPT 后端自己选生图模型") : t("只发给这一家，失败不换"),
    })),
    ...(known ? [] : [{ value, label: tf("{0}（已不可用）", [value]), hint: t("所在的供应商已删除或关闭") }]),
    { value: "off", label: t("关闭"), hint: t("生图请求直接报错") },
  ];
  return (
    <div className="gw-detail-row">
      <span className="gw-detail-label">{t("生图模型")}</span>
      <div className="gw-detail-value gw-chip-row">
        <AppSelect
          className="gw-image-select"
          disabled={busy !== null}
          label={t("生图模型")}
          menuWidth={320}
          onChange={(next) => void run(`image:${agent}`, "save_gateway_image_model", { agent, model: next }, t("保存生图模型"), true)}
          options={items}
          value={value}
        />
        {value === "" && !auto ? <span className="gw-muted">{t("池里还没有能生图的模型：登录 ChatGPT 账号，或给供应商加上生图模型")}</span> : null}
        {value !== "off" ? (
          <label className="gw-inline-field gw-inline-check" title={t("上游只返回图片链接时，网关把图下载下来转成图片数据再交给 Agent（Codex 只认图片数据）。关掉后原样转发链接。")}>
            <input
              checked={inlineUrls}
              disabled={busy !== null}
              onChange={(event) => void run(`image-inline:${agent}`, "save_gateway_image_inline", { agent, enabled: event.currentTarget.checked }, t("保存生图选项"), true)}
              type="checkbox"
            />
            <span>{t("图片链接转成图片数据")}</span>
          </label>
        ) : null}
      </div>
    </div>
  );
}

/** 收藏写进网关（同时留一份在本窗口，后端旧版时也不丢），并把新数据推给其它页面。失败不打扰用户。 */
async function persistFavorites(models: string[]) {
  saveFavorites(models);
  try {
    publishGatewayPayload(await invoke<GatewayResult>("save_gateway_favorites", { models }));
  } catch {
    // 后端没有这个命令（旧版）或保存失败：本窗口里的收藏照样生效。
  }
}

/** Agent 行右侧的模型下拉。 */
function ModelPicker({
  agent,
  payload,
  draft,
  connected,
  open,
  onOpenChange,
  onPick,
  onDisconnect,
}: {
  agent: GatewayAgentView;
  payload: GatewayPayload | null;
  draft: BindingDraft;
  connected: boolean;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onPick: (item: PickerItem) => void;
  onDisconnect: () => void;
}) {
  const buttonRef = useRef<HTMLButtonElement>(null);
  const display = modelDisplay(payload, draft.model, draft.poolExclude);
  const showDefault = !connected && !draft.model;
  const selectedModel = connected || draft.model ? draft.model.trim() || agentModels(payload, draft.poolExclude)[0] || "" : "";
  const head = (
    <>
      {/* 用默认 = 断开网关并还原接入前的配置，只放这一个入口。 */}
      <button
        className="gw-mp-action"
        data-active={!connected}
        onClick={onDisconnect}
        title={connected ? tf("断开 Codex++，还原 {0} 接入之前的设置", [agent.name]) : tf("{0} 自带的默认值", [agent.name])}
        type="button"
      >
        <AgentIcon agent={agent.agent} className="gw-popover-agent" />
        {t("用默认（不经 Codex++）")}
        {!connected ? <Check className="h-3.5 w-3.5" /> : null}
      </button>
    </>
  );
  return (
    <div className="gw-picker">
      <button aria-expanded={open} aria-haspopup="listbox" className="gw-picker-button" onClick={() => onOpenChange(!open)} ref={buttonRef} type="button">
        {showDefault ? <AgentIcon agent={agent.agent} className="gw-picker-agent" /> : <ModelViewGlyph view={display} />}
        <span>{showDefault ? t("默认") : display.title || t("选择模型")}</span>
        {showDefault ? null : <AutoTag view={display} />}
        <ChevronDown className="h-3.5 w-3.5" />
      </button>
      <Floating anchorRef={buttonRef} className="gw-model-popover" onClose={() => onOpenChange(false)} open={open} width={560}>
        <ModelList
          exclude={draft.poolExclude}
          head={head}
          onPick={onPick}
          payload={payload}
          selectedModel={selectedModel}
        />
      </Floating>
    </div>
  );
}

/** Claude 档位的小下拉：前缀是档位名，空值表示同默认模型。 */
function TierPicker({
  label,
  value,
  payload,
  exclude,
  onPick,
  emptyLabel,
}: {
  label: string;
  value: string;
  payload: GatewayPayload | null;
  exclude: string[];
  onPick: (model: string) => void;
  /** 没选时显示的文字；默认「同默认模型」。 */
  emptyLabel?: string;
}) {
  const empty = emptyLabel ?? t("同默认模型");
  const [open, setOpen] = useState(false);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const display = value ? modelDisplay(payload, value, exclude) : null;
  const head = (
    <button className="gw-mp-action" data-active={!value} onClick={() => { setOpen(false); onPick(""); }} title={t("跟随这一行选的模型")} type="button">
      <span className="gw-tier-default">=</span>
      {empty}
      {!value ? <Check className="h-3.5 w-3.5" /> : null}
    </button>
  );
  return (
    <>
      <button aria-expanded={open} className="gw-tier-button" onClick={() => setOpen(!open)} ref={buttonRef} type="button">
        <em>{label}</em>
        {display ? <ModelViewGlyph view={display} /> : null}
        <span>{display ? display.title : empty}</span>
        {display ? <AutoTag view={display} /> : null}
        <ChevronDown className="h-3.5 w-3.5" />
      </button>
      <Floating anchorRef={buttonRef} className="gw-model-popover" onClose={() => setOpen(false)} open={open} width={520}>
        <ModelList
          exclude={exclude}
          head={head}
          onPick={(item) => {
            setOpen(false);
            onPick(item.model);
          }}
          payload={payload}
          selectedModel={value}
        />
      </Floating>
    </>
  );
}
