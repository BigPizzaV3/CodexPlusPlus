/** 本地网关 · 智能体：每个 Agent 一行，接入后供应商池里的模型都出现在它自己的模型列表里，也可以在这里直接选。 */
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode, type RefObject } from "react";
import { createPortal } from "react-dom";
import { appConfirm } from "./app-dialog";
import { Check, ChevronDown, ChevronRight, ChevronUp, LayoutGrid, List, Search, Star, Unplug, Waypoints } from "lucide-react";
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
} from "./gateway-model";
import { AppSelect } from "./AppSelect";
import { AGENT_ICONS, Avatar, Switch, avatarOf, loadFavorites, saveFavorites, type Notify, type Run, AgentIcon } from "./gateway-ui";

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
      <p className="gw-lead">
        {t("打开「接入」后，Codex++ 中配置的模型会作为")} <strong>Codex++</strong> {t("供应商出现在 Agent 的模型列表里；也可在此直接选模型。")}
      </p>
      <div className="gw-card gw-agent-card">
        {connected.map(row)}
        {others.length ? <div className="gw-subhead">{t("未设置")}</div> : null}
        {others.map(row)}
        {!payload ? <div className="gw-empty">{t("正在读取…")}</div> : null}
      </div>
    </>
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
  const status = drift
    ? tf("配置被 {0} 改了，{1} 现在不经过 Codex++", [drift.by || t("其它工具"), agent.name])
    : connected
    ? running
      ? isClaude
        ? t("已接入 · 在 /model 里切换 opus · sonnet · haiku 档")
        : tf("已接入 · {0} 的模型列表里有 {1} 个模型", [agent.name, agent.models.length])
      : t("已接入 · 网关没有运行")
    : tf("未接入 · 用 {0} 自己的设置", [agent.name]);

  return (
    <div className="gw-agent" data-expanded={expanded}>
      <div className="gw-agent-row">
        <AgentIcon agent={agent.agent} className="gw-agent-icon" />
        <div className="gw-agent-main">
          <div className="gw-agent-title">
            <strong>{agent.name}</strong>
            {agent.version ? <span className="gw-version">{agent.version}</span> : null}
          </div>
          <small title={drift?.endpoint ? tf("现在连的是 {0}", [drift.endpoint]) : undefined}>
            <i className="gw-dot" data-tone={tone} />
            {status}
            {drift ? (
              <button className="gw-link gw-drift-reconnect" disabled={busy !== null} onClick={() => void reconnect()} type="button">
                {t("重新接入")}
              </button>
            ) : null}
          </small>
        </div>
        {connected ? (
          expanded ? (
            <button className="gw-soft-button" onClick={onToggleExpand} type="button">{t("收起")}</button>
          ) : (
            <button aria-expanded={false} className="gw-link" onClick={onToggleExpand} type="button">
              {isClaude ? t("档位") : tf("模型 {0}", [modelCount])}
              <ChevronRight className="h-3.5 w-3.5" />
            </button>
          )
        ) : <span />}
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
      <div className="gw-detail-row">
        <span className="gw-detail-label">{tf("在 {0} 里", [agent.name])}</span>
        <div className="gw-detail-value">
          {isClaude ? (
            <span>{t("在 Claude Code 的")} <code className="gw-code">/model</code> {t("里切换 opus · sonnet · haiku 档，各档用的模型在下面设置。")}</span>
          ) : (
            <span>{tf("在 {0} 的模型列表里选任何一个模型。", [agent.name])}</span>
          )}
        </div>
      </div>
      <div className="gw-detail-row">
        <span className="gw-detail-label">{t("模型列表")}</span>
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
              {t("路由组")}
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
      <div className="gw-detail-row">
        <span className="gw-detail-label">{t("其它")}</span>
        <div className="gw-detail-value gw-chip-row">
          <div className="gw-inline-field">
            <span>{t("未知模型名")}</span>
            <AppSelect
              disabled={busy !== null}
              label={t("未知模型名")}
              onChange={(next) => onSave({ ...draft, unknownModel: next as BindingDraft["unknownModel"] })}
              options={[
                { value: "mapToDefault", label: t("改写成默认模型"), hint: t("池里没有的模型名换成主模型") },
                { value: "passthrough", label: t("原样转发"), hint: t("按客户端给的名字发给上游") },
              ]}
              value={draft.unknownModel}
            />
          </div>
          <label className="gw-inline-field">
            <span>{t("上下文窗口")}</span>
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
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", escape);
    };
  }, [open, onClose, anchorRef]);
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
  const [query, setQuery] = useState("");
  const [favoritesOnly, setFavoritesOnly] = useState(false);
  const [favorites, setFavorites] = useState<string[]>(loadFavorites);
  const listRef = useRef<HTMLDivElement>(null);
  const sections = useMemo(() => {
    const all = pickerSections(payload, query, avatarOf, exclude, skipGroup);
    if (!favoritesOnly) return all;
    return all
      .map((section) => ({ ...section, items: section.items.filter((item) => favorites.includes(item.model)) }))
      .filter((section) => section.items.length);
  }, [payload, query, exclude, favoritesOnly, favorites, skipGroup]);
  const toggleFavorite = (model: string) => {
    const next = favorites.includes(model) ? favorites.filter((item) => item !== model) : [...favorites, model];
    setFavorites(next);
    saveFavorites(next);
  };
  const jump = (key: string) => {
    setFavoritesOnly(false);
    window.requestAnimationFrame(() => {
      const node = listRef.current?.querySelector<HTMLElement>(`[data-section="${CSS.escape(key)}"]`);
      if (node && listRef.current) listRef.current.scrollTop = node.offsetTop - 6;
    });
  };
  const isSelected = (item: PickerItem) => Boolean(selectedModel) && item.model.toLowerCase() === selectedModel.toLowerCase();

  return (
    <>
      <label className="gw-popover-search">
        <Search className="h-4 w-4" />
        <input autoFocus onChange={(event) => setQuery(event.currentTarget.value)} placeholder={t("筛选，或输入任意模型 ID…")} value={query} />
      </label>
      <div className="gw-popover-body">
        <div className="gw-popover-rail">
          <button aria-label={t("全部")} data-active={!favoritesOnly} onClick={() => { setFavoritesOnly(false); setQuery(""); if (listRef.current) listRef.current.scrollTop = 0; }} type="button">
            <LayoutGrid className="h-4 w-4" />
          </button>
          <button aria-label={t("收藏")} data-active={favoritesOnly} onClick={() => setFavoritesOnly(!favoritesOnly)} type="button">
            <Star className="h-4 w-4" />
          </button>
          <span className="gw-rail-divider" />
          {pickerSections(payload, "", avatarOf, exclude, skipGroup).filter((section) => section.key !== "custom").map((section) => (
            <button aria-label={section.title} key={section.key} onClick={() => jump(section.key)} title={section.key === "group" ? t("路由组") : section.title} type="button">
              {section.key === "group" ? <Waypoints className="h-4 w-4" /> : <Avatar name={section.title} size="sm" />}
            </button>
          ))}
        </div>
        <div className="gw-popover-list" ref={listRef}>
          {!favoritesOnly && !query ? head : null}
          {sections.length === 0 ? (
            <div className="gw-empty">
              {favoritesOnly ? t("还没有收藏的模型，点模型右侧的星标收藏。") : payload?.pool.length ? t("没有匹配的模型") : t("供应商池是空的，先到「供应商」里添加。")}
            </div>
          ) : null}
          {sections.map((section) => (
            <div data-section={section.key} key={section.key}>
              <div className="gw-popover-heading">{section.key === "group" || section.key === "custom" ? t(section.title) : section.title}</div>
              {section.items.map((item) => {
                const selected = isSelected(item);
                const favorite = favorites.includes(item.model);
                return (
                  <div
                    aria-selected={selected}
                    className="gw-popover-item"
                    data-selected={selected}
                    key={item.model}
                    onClick={() => onPick(item)}
                    onKeyDown={(event) => { if (event.key === "Enter") onPick(item); }}
                    role="option"
                    tabIndex={0}
                  >
                    {section.key === "group" ? <Waypoints className="gw-picker-glyph h-4 w-4" /> : <Avatar name={item.sub} size="sm" />}
                    <span className="gw-popover-item-label">{item.label}</span>
                    <span className="gw-via">{item.custom ? t("自定义") : t("经 Codex++")}</span>
                    <small>{item.sub}</small>
                    <button
                      aria-label={favorite ? t("取消收藏") : t("收藏")}
                      className="gw-star"
                      data-on={favorite}
                      onClick={(event) => { event.stopPropagation(); toggleFavorite(item.model); }}
                      type="button"
                    >
                      <Star className="h-3.5 w-3.5" />
                    </button>
                    {selected ? <Check className="gw-popover-check h-4 w-4" /> : <span className="gw-popover-check" />}
                  </div>
                );
              })}
            </div>
          ))}
        </div>
      </div>
    </>
  );
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
      <div className="gw-popover-item" data-selected={!connected} onClick={onDisconnect} role="option" aria-selected={!connected} tabIndex={0} onKeyDown={(event) => { if (event.key === "Enter") onDisconnect(); }}>
        <AgentIcon agent={agent.agent} className="gw-popover-agent" />
        <span className="gw-popover-item-label">{t("默认")}</span>
        <span className="gw-via gw-via-off">{t("不经 Codex++")}</span>
        <small>{tf("{0} 自带的默认值", [agent.name])}</small>
        <span className="gw-star" />
        {!connected ? <Check className="gw-popover-check h-4 w-4" /> : <span className="gw-popover-check" />}
      </div>
      {connected ? (
        <div className="gw-popover-item" onClick={onDisconnect} role="option" aria-selected={false} tabIndex={0} onKeyDown={(event) => { if (event.key === "Enter") onDisconnect(); }}>
          <Unplug className="gw-picker-glyph h-4 w-4" />
          <span className="gw-popover-item-label">{t("断开 Codex++（还原配置）")}</span>
          <small>{tf("还原 {0} 接入 Codex++ 之前的设置", [agent.name])}</small>
        </div>
      ) : null}
    </>
  );
  return (
    <div className="gw-picker">
      <button aria-expanded={open} aria-haspopup="listbox" className="gw-picker-button" onClick={() => onOpenChange(!open)} ref={buttonRef} type="button">
        {showDefault ? (
          <AgentIcon agent={agent.agent} className="gw-picker-agent" />
        ) : display.group ? (
          <Waypoints className="gw-picker-glyph h-4 w-4" />
        ) : (
          <Avatar name={display.sub || display.title} size="sm" />
        )}
        <span>{showDefault ? t("默认") : display.title || t("选择模型")}</span>
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
}: {
  label: string;
  value: string;
  payload: GatewayPayload | null;
  exclude: string[];
  onPick: (model: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const display = value ? modelDisplay(payload, value, exclude) : null;
  const head = (
    <div className="gw-popover-item" data-selected={!value} onClick={() => { setOpen(false); onPick(""); }} role="option" aria-selected={!value} tabIndex={0} onKeyDown={(event) => { if (event.key === "Enter") { setOpen(false); onPick(""); } }}>
      <span className="gw-picker-glyph gw-tier-default">=</span>
      <span className="gw-popover-item-label">{t("同默认模型")}</span>
      <small>{t("跟随这一行选的模型")}</small>
      <span className="gw-star" />
      {!value ? <Check className="gw-popover-check h-4 w-4" /> : <span className="gw-popover-check" />}
    </div>
  );
  return (
    <>
      <button aria-expanded={open} className="gw-tier-button" onClick={() => setOpen(!open)} ref={buttonRef} type="button">
        <em>{label}</em>
        {display ? (display.group ? <Waypoints className="h-3.5 w-3.5" /> : <Avatar name={display.sub || display.title} size="sm" />) : null}
        <span>{display ? display.title : t("同默认模型")}</span>
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
