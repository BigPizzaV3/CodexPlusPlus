/** 本地网关 · 智能体：每个 Agent 一行，接入后供应商池里的模型都出现在它自己的模型列表里，也可以在这里直接选。 */
import { useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode, type RefObject } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { Check, ChevronDown, ChevronRight, Copy, List, Search, Shuffle, Star, Waypoints, X } from "lucide-react";
import { t, tf } from "./i18n";
import { AgentDisconnectDialog } from "./AgentDisconnectDialog";
import { agentDiagnosticPresentation, canReconnectAgent, diagnosticErrorLabel, diagnosticEvidenceLabel } from "./gateway-agent-safety";
import {
  agentModels,
  agentPoolProviders,
  bindingFromDraft,
  draftFromBinding,
  modelDisplay,
  ownPickerSection,
  parseContextWindow,
  visibleOwnModels,
  pickerSections,
  providerEnabled,
  validateDraft,
  type BindingDraft,
  type GatewayAgentId,
  type GatewayAgentView,
  type GatewayPayload,
  type PickerItem,
  type PickerSection,
  maskAccountName,
  relativeTime,
  type ImageOption,
  isFavoriteModel,
  toggleFavoriteModel,
  type GatewayResult,
  type OwnModels,
} from "./gateway-model";
import { AppSelect } from "./AppSelect";
import { shortProviderName } from "./GatewayModelMatrix";
import { GatewayNavContext, publishGatewayPayload } from "./gateway-store";
import { useEscapeLayer } from "./use-escape-layer";
import { singleAgent, visibleAgents } from "./agent-focus-model";
import { useAgentFocus } from "./agent-focus-store";
import { CxBadge, CxCard, CxEmpty, CxSection, CxStatusIcon } from "./cx-layout";
import { AGENT_ICONS, AutoTag, Avatar, ModelViewGlyph, Switch, avatarOf, loadFavorites, saveFavorites, type Notify, type Run, AgentIcon } from "./gateway-ui";

export function AgentsView({
  payload,
  busy,
  run,
  notify,
  onManage,
  onOpenCodexProviderSettings,
}: {
  payload: GatewayPayload | null;
  busy: string | null;
  run: Run;
  notify: Notify;
  onManage: () => void;
  onOpenCodexProviderSettings?: () => void;
}) {
  // 没勾选的 Agent 不列；它们已经接入的话照常工作，到设置里「我用的 Agent」重新勾选就能再管理。
  const focus = useAgentFocus();
  const solo = singleAgent(focus);
  const agents = visibleAgents(focus, payload?.agents ?? []);
  const [expanded, setExpanded] = useState<string | null>(solo);
  const connected = agents.filter((agent) => agent.enabled);
  const others = agents.filter((agent) => !agent.enabled);
  const [othersOpen, setOthersOpen] = useState<boolean | null>(null);
  const othersShown = othersOpen ?? connected.length === 0;
  const row = (agent: GatewayAgentView) => (
    <AgentRow
      agent={agent}
      busy={busy}
      expanded={expanded === agent.agent}
      key={agent.agent}
      notify={notify}
      onManage={onManage}
      onOpenCodexProviderSettings={onOpenCodexProviderSettings}
      onToggleExpand={() => setExpanded(expanded === agent.agent ? null : agent.agent)}
      payload={payload}
      run={run}
    />
  );
  // 参照 Cursor 的 Customize：「已接入」「未接入」两节，每个 Agent 一行；选模型 + 打开开关就算接入，详情在行下展开。
  // 单 Agent 模式只有一行，不分节，直接展开它的设置。
  const list = (items: GatewayAgentView[]) => <CxCard className="gw-agent-card">{items.map(row)}</CxCard>;
  return (
    <div className="gw-agents cx-page">
      {!payload ? <CxCard><CxEmpty title={t("正在读取…")} /></CxCard> : null}
      {solo && agents.length ? list(agents) : null}
      {!solo && connected.length ? <CxSection count={connected.length} title={t("已接入")}>{list(connected)}</CxSection> : null}
      {/* 未接入的默认折叠，点标题展开；一个都没接入时直接展开，新用户不用再点一下。 */}
      {!solo && others.length ? (
        <CxSection
          className="gw-agents-others"
          title={(
            <button aria-expanded={othersShown} className="cx-section-toggle" onClick={() => setOthersOpen(!othersShown)} type="button">
              <ChevronRight aria-hidden="true" className="h-3.5 w-3.5" />
              {t("未接入")}
              <span className="cx-section-count">{others.length}</span>
            </button>
          )}
        >
          {othersShown ? list(others) : null}
        </CxSection>
      ) : null}
    </div>
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
  onOpenCodexProviderSettings,
  onClose,
}: {
  payload: GatewayPayload | null;
  busy: string | null;
  run: Run;
  notify: Notify;
  onManage: () => void;
  onOpenCodexProviderSettings?: () => void;
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
          <AgentsView busy={busy} notify={notify} onManage={onManage} onOpenCodexProviderSettings={onOpenCodexProviderSettings} payload={payload} run={run} />
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
  onOpenCodexProviderSettings,
  run,
  notify,
}: {
  agent: GatewayAgentView;
  payload: GatewayPayload | null;
  busy: string | null;
  expanded: boolean;
  onToggleExpand: () => void;
  onManage: () => void;
  onOpenCodexProviderSettings?: () => void;
  run: Run;
  notify: Notify;
}) {
  const binding = payload?.gateway.bindings[agent.agent as GatewayAgentId];
  const [draft, setDraft] = useState<BindingDraft>(() => draftFromBinding(binding));
  const [pickerOpen, setPickerOpen] = useState(false);
  const [disconnectTarget, setDisconnectTarget] = useState<PickerItem | "disconnect" | null>(null);
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
  const disconnect = () => setDisconnectTarget("disconnect");
  // 在选择器里选模型：已接入时直接改默认模型，未接入时选完即接入。
  const pick = (item: PickerItem) => {
    setPickerOpen(false);
    void save({ ...draft, model: item.model });
  };
  // 已接入时先预览还原差异，确认后才断开并选择直连模型。
  const pickOwn = async (item: PickerItem) => {
    setPickerOpen(false);
    if (connected || agent.requiresDisconnectPreview) {
      setDisconnectTarget(item);
      return;
    }
    await run(`own:${agent.agent}`, "use_gateway_own_model", { agent: agent.agent, model: item.model }, t("切换模型"));
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
  const drift = connected ? agent.drift ?? null : null;
  const diagnostic = agentDiagnosticPresentation(agent, running);
  const modelCount = agentModels(payload, draft.poolExclude).length;
  const reconnectAllowed = canReconnectAgent(agent, payload?.relayProfilesEnabled);
  const reconnect = () =>
    run(`save:${agent.agent}`, "save_gateway_binding", { agent: agent.agent, binding: bindingFromDraft(draft, binding) }, t("重新接入"));
  const note = diagnostic.reconnect
    ? drift?.by ? tf("配置被 {0} 改了", [drift.by]) : t("当前文件与接入配置不一致")
    : connected && isClaude
    ? "opus · sonnet · haiku"
    : "";

  return (
    <div className="gw-agent" data-expanded={expanded}>
      {/* 已接入时整行点击展开详情，模型下拉和开关各管各的。 */}
      <div
        className="gw-agent-row cx-row"
        data-clickable={connected}
        onClick={(event) => {
          if (!connected || (event.target as HTMLElement).closest("button, a, input")) return;
          onToggleExpand();
        }}
      >
        <div className="cx-row-main">
          <span className="cx-row-icon gw-agent-icon-box">
            <CxStatusIcon tone={!connected ? undefined : diagnostic.tone === "error" ? "danger" : "ok"}>
              {/* 包一层：CxStatusIcon 把直接子级的 <i> 当状态点，Claude / Grok 的遮罩图标正是 <i>。 */}
              <span className="gw-agent-glyph"><AgentIcon agent={agent.agent} className="gw-agent-icon" /></span>
            </CxStatusIcon>
          </span>
          <span className="cx-row-copy">
            <span className="cx-row-title">
              <strong>{agent.name}</strong>
              {agent.version ? <CxBadge>{agent.version}</CxBadge> : null}
            </span>
            <span className="cx-row-detail gw-agent-meta" title={drift?.endpoint ? tf("现在连的是 {0}", [drift.endpoint]) : undefined}>
              {/* 「等待请求验证」读起来像出了问题：还没见到请求只是还没用过，写「还没收到请求」。 */}
              <span className="gw-agent-badge" data-tone={diagnostic.tone}>{agentStatusText(agent, diagnostic.badge, diagnostic.tone)}</span>
              {connected && !diagnostic.reconnect ? <span>{tf("{0} 个模型", [modelCount])}</span> : null}
              {note ? <span>{note}</span> : null}
              {diagnostic.reconnect ? (
                reconnectAllowed ? (
                  <button className="gw-link gw-drift-reconnect" disabled={busy !== null} onClick={() => void reconnect()} type="button">
                    {t("重新接入")}
                  </button>
                ) : (
                  <>
                    <span>{t("供应商配置切换已关闭，在供应商配置页打开后才能切换")}</span>
                    {onOpenCodexProviderSettings ? (
                      <button className="gw-link gw-drift-reconnect" onClick={onOpenCodexProviderSettings} type="button">
                        {t("打开供应商页")}
                      </button>
                    ) : null}
                  </>
                )
              ) : null}
            </span>
          </span>
        </div>
        <div className="cx-row-controls">
          <ModelPicker
            agent={agent}
            connected={connected}
            draft={draft}
            onDisconnect={() => {
              setPickerOpen(false);
              if (connected || agent.requiresDisconnectPreview) void disconnect();
            }}
            onOpenChange={setPickerOpen}
            onPick={pick}
            onPickOwn={(item) => void pickOwn(item)}
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
      </div>
      {expanded && connected ? (
        <AgentDetail agent={agent} busy={busy} draft={draft} onDisconnect={() => void disconnect()} onManage={onManage} onSave={(next) => void save(next)} payload={payload} run={run} />
      ) : null}
      {disconnectTarget ? (
        <AgentDisconnectDialog
          agent={agent}
          busy={busy !== null}
          onApply={(previewToken) => disconnectTarget === "disconnect"
            ? run(`disable:${agent.agent}`, "disable_gateway_binding", { agent: agent.agent, previewToken }, t("断开并还原"))
            : run(`own:${agent.agent}`, "use_gateway_own_model", { agent: agent.agent, model: disconnectTarget.model, previewToken }, t("断开并直连"))}
          onFullSwitch={(previewToken) => run(`disable:${agent.agent}`, "disable_gateway_binding_full_switch", { agent: agent.agent, previewToken }, t("断开并还原"))}
          onClose={() => setDisconnectTarget(null)}
          ownModel={disconnectTarget === "disconnect" ? null : disconnectTarget}
        />
      ) : null}
    </div>
  );
}

/**
 * 展开的详情（行下展开，不跳页）：接入状态、用哪些供应商、Claude 档位，常用的就这几行；
 * 起名 / 审批 / 生图模型、未知模型名、默认窗口这些默认值大多数人不用改，收进「高级」。底部是改动的文件、查看差异、断开并还原。
 */
function AgentDetail({
  agent,
  payload,
  draft,
  busy,
  run,
  onSave,
  onDisconnect,
  onManage,
}: {
  agent: GatewayAgentView;
  payload: GatewayPayload | null;
  draft: BindingDraft;
  busy: string | null;
  run: Run;
  onSave: (next: BindingDraft) => void;
  onDisconnect: () => void;
  /** 去供应商池（弹窗里是关掉弹窗再去）。 */
  onManage: () => void;
}) {
  const isClaude = agent.agent === "claude";
  const [selecting, setSelecting] = useState(false);
  const [windowText, setWindowText] = useState(draft.contextWindow);
  // 通用 Agent 有的只能在自己界面里填 Key（如 Zed 存在钥匙串里）：给出地址和本 Agent 的网关 token 一键复制。
  const [copied, setCopied] = useState<"url" | "token" | null>(null);
  const token = payload?.gateway.bindings[agent.agent as GatewayAgentId]?.token ?? "";
  const copy = (kind: "url" | "token", text: string) => {
    void navigator.clipboard.writeText(text).then(() => {
      setCopied(kind);
      window.setTimeout(() => setCopied((current) => (current === kind ? null : current)), 1500);
    }).catch(() => undefined);
  };
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

  const usedModels = used.reduce((sum, entry) => sum + entry.models.length, 0);
  const usedNames = used.map((entry) => maskAccountName(shortProviderName(entry.name)));
  const providerHint = used.length
    ? [
      tf("{0} 家 · {1} 个模型", [String(used.length), String(usedModels)]),
      groups.length ? tf("{0} 个模型组", [String(groups.length)]) : "",
    ].filter(Boolean).join(" · ")
    : t("没有可用的供应商");

  // 每行一样的排法（参照设置页）：左边名字 + 一行说明，右边一个控件；开关一律用开关，下拉一律同宽。
  return (
    <div className="gw-agent-detail">
      {/* 接入诊断只在出问题时出现（配置被改、读不到、网关没运行、上游报错），并自动展开详情；
          正常时 Agent 名字下面那行状态就够了，不单独占一行。 */}
      {agentDiagnosticPresentation(agent, Boolean(payload?.helperRunning)).tone === "error" ? (
        <AgentDiagnostics agent={agent} helperRunning={Boolean(payload?.helperRunning)} />
      ) : null}
      {agent.hint ? (
        <SetLine hint={t(agent.hint)} title={tf("在 {0} 里", [agent.name])}>
          {agent.enabled ? (
            <>
              <button className="gw-text-button" onClick={() => copy("url", `${agent.baseUrl}/v1`)} type="button">
                {copied === "url" ? <Check className="h-3.5 w-3.5" /> : <Copy className="h-3.5 w-3.5" />}
                {t("复制地址")}
              </button>
              {token ? (
                <button className="gw-text-button" onClick={() => copy("token", token)} type="button">
                  {copied === "token" ? <Check className="h-3.5 w-3.5" /> : <Copy className="h-3.5 w-3.5" />}
                  {t("复制 token")}
                </button>
              ) : null}
            </>
          ) : null}
        </SetLine>
      ) : null}
      {/* 供应商不再一家一个长标签铺三行：一排头像 + 「几家 · 几个模型」，名字在悬停提示里，点「挑选」改。 */}
      <SetLine hint={providerHint} title={t("用哪些供应商")} tooltip={usedNames.join("、")}>
        {used.length ? (
          <span className="gw-avatar-stack gw-set-line-avatars">
            {used.slice(0, 5).map((entry) => <Avatar baseUrl={entry.baseUrl} key={entry.id} name={entry.name} size="sm" />)}
            {used.length > 5 ? <span className="gw-avatar gw-avatar-more" data-size="sm">+{used.length - 5}</span> : null}
          </span>
        ) : null}
        <button className="gw-secondary-button gw-set-line-button" onClick={() => setSelecting(!selecting)} ref={selectRef} type="button">
          {t("挑选")}
          <ChevronDown className="h-3.5 w-3.5" />
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
            <button className="gw-text-button gw-select-manage" onClick={() => { setSelecting(false); onManage(); }} type="button">
              <List className="h-3.5 w-3.5" />
              {t("管理供应商池")}
            </button>
          </div>
        </Floating>
      </SetLine>
      {/* 只有 Claude Code 的档位切换需要说明，放在悬停提示里；其它 Agent 在自己的模型列表里直接选。 */}
      {isClaude ? (
        <SetLine
          className="gw-set-line-tiers"
          hint={t("在 /model 里切换的几档")}
          title={t("档位")}
          tooltip={`${t("在 Claude Code 的")} /model ${t("里切换 opus · sonnet · haiku 档，各档用的模型在下面设置。")}`}
        >
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
        </SetLine>
      ) : null}
      <details className="gw-detail-advanced">
        <summary>
          <ChevronRight aria-hidden="true" className="h-3.5 w-3.5" />
          {t("高级")}
        </summary>
        {agent.ownModels ? <OwnModelsRow agent={agent} busy={busy} payload={payload} run={run} /> : null}
        {/* 目前只有 Codex 自己会发生图请求；别的 Agent 有了生图工具再显示。 */}
        {agent.agent === "codex" ? <ImageModelRow agent={agent.agent} busy={busy} payload={payload} run={run} /> : null}
        {/* Codex 给会话起名时另开临时线程（首次用内置轻量模型，重新起名用会话当前模型），默认照原模型路由，可单独指定。 */}
        {agent.agent === "codex" ? (
          <SetLine hint={t("给会话起名用；不指定时跟 Codex 自己选的一样")} title={t("起名模型")} tooltip={t("Codex 给会话起名、重新起名的请求发给这个模型；不指定时照 Codex 自己选的模型路由")}>
            <TierPicker
              emptyLabel={t("不单独指定")}
              exclude={draft.poolExclude}
              label=""
              onPick={(model) => onSave({ ...draft, titleModel: model })}
              payload={payload}
              value={draft.titleModel}
            />
          </SetLine>
        ) : null}
        {/* Codex「替我审批」固定请求 codex-auto-review：默认改写成主模型，可单独指定，避免落到 ChatGPT 订阅额度上（#2703）。 */}
        {agent.agent === "codex" ? (
          <SetLine hint={t("「替我审批」的请求发给它")} title={t("审批模型")} tooltip={t("Codex「替我审批」的请求（codex-auto-review）发给这个模型；同默认模型时按「未知模型名」规则处理")}>
            <TierPicker
              exclude={draft.poolExclude}
              label=""
              onPick={(model) => onSave({ ...draft, autoReviewModel: model })}
              payload={payload}
              value={draft.autoReviewModel}
            />
          </SetLine>
        ) : null}
        <SetLine hint={t("池里没有的模型名怎么处理")} title={t("未知模型名")}>
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
        </SetLine>
        {/* 窗口按模型在供应商池里填；这里只给池里没写窗口的模型兜底。 */}
        <SetLine hint={t("供应商池里没写窗口的模型用这个")} title={t("默认窗口")} tooltip={t("供应商池里没写窗口的模型用这个；按模型的窗口在供应商池的详情里填")}>
          <input
            aria-label={t("默认窗口")}
            className="gw-inline-input"
            onBlur={() => {
              const parsed = parseContextWindow(windowText);
              if (windowText !== draft.contextWindow && (parsed === null || (Number.isFinite(parsed) && parsed >= 1000))) onSave({ ...draft, contextWindow: windowText });
            }}
            onChange={(event) => setWindowText(event.currentTarget.value)}
            placeholder={t("例如 200k")}
            value={windowText}
          />
        </SetLine>
        {isClaude ? (
          <SetLine hint={t("非 claude-* 模型名时也照常发送 effort")} title={t("强制启用推理强度")} tooltip={t("非 claude-* 模型名时 Claude Code 默认不发 effort，打开后照常发送。")}>
            <Switch disabled={busy !== null} label={t("强制启用推理强度")} on={draft.claudeEnableEffort} onToggle={() => onSave({ ...draft, claudeEnableEffort: !draft.claudeEnableEffort })} />
          </SetLine>
        ) : null}
        <SetLine hint={t("新加的供应商默认给这个 Agent 用")} title={t("添加供应商时自动加入")} tooltip={t("关掉后，新添加的供应商默认不给这个 Agent 用；添加时仍可手动勾选。")}>
          <Switch disabled={busy !== null} label={t("添加供应商时自动加入")} on={draft.autoJoinProviders} onToggle={() => onSave({ ...draft, autoJoinProviders: !draft.autoJoinProviders })} />
        </SetLine>
      </details>
      <div className="gw-detail-row gw-detail-foot">
        <span className="gw-detail-label gw-detail-file" title={agent.configPath ?? ""}>
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

/**
 * Agent 名字下面那行状态：请求到过就写「最近请求 5 分钟前」，比「请求已到达」有用；
 * 还没见到请求写「还没收到请求」（原来的「等待请求验证」读起来像出了问题）；出问题时照旧写原因。
 */
function agentStatusText(agent: GatewayAgentView, badge: string, tone: string): string {
  const lastSeen = agent.diagnostics?.lastSeenMs;
  if (tone === "on" && lastSeen) {
    const when = relativeTime(lastSeen, Date.now());
    return tf("最近请求 {0}", [when.args.length ? tf(when.text, when.args.map(String)) : t(when.text)]);
  }
  if (badge === "等待请求验证") return t("还没收到请求");
  return t(badge);
}

/** 详情里的一行：左边名字 + 一行说明，右边控件（参照设置页的行）。长说明放 tooltip。 */
function SetLine({ title, hint, tooltip, className = "", children }: { title: ReactNode; hint?: ReactNode; tooltip?: string; className?: string; children?: ReactNode }) {
  return (
    <div className={`gw-set-line ${className}`.trim()} title={tooltip}>
      <span className="gw-set-line-copy">
        <strong>{title}</strong>
        {hint ? <small>{hint}</small> : null}
      </span>
      {children ? <div className="gw-set-line-control">{children}</div> : null}
    </div>
  );
}

/** 接入状态一行：请求数、最近请求；地址、配置证据、最近错误收进「诊断详情」，有错误时默认展开。 */
function AgentDiagnostics({ agent, helperRunning }: { agent: GatewayAgentView; helperRunning: boolean }) {
  const diagnostics = agent.diagnostics;
  const presentation = agentDiagnosticPresentation(agent, helperRunning);
  const lastSeen = diagnostics?.lastSeenMs
    ? new Date(diagnostics.lastSeenMs).toLocaleString()
    : "";
  const pending = presentation.tone === "pending"
    ? diagnostics?.state === "awaitingTraffic"
      ? t("配置已指向网关；尚未观察到请求，请在 Agent 里发送一条消息。")
      : t("尚无足够信息确认请求路径。")
    : "";
  return (
    <div className="gw-set-line gw-set-line-diagnostics">
      <div className="gw-set-line-copy gw-agent-diagnostics">
        <strong>{t("接入诊断")}</strong>
        <div className="gw-agent-diagnostic-summary">
          {diagnostics ? <span>{tf("请求 {0} · 失败 {1} · 进行中 {2}", [diagnostics.requests, diagnostics.errors, diagnostics.inFlight])}</span> : null}
          {lastSeen ? <span>{tf("最近请求：{0}", [lastSeen])}</span> : null}
          {!diagnostics && pending ? <span>{pending}</span> : null}
        </div>
        {diagnostics ? (
          <details className="gw-agent-diagnostic-more" open={presentation.tone === "error" || undefined}>
            <summary>{t("诊断详情")}</summary>
            {pending ? <p>{pending}</p> : null}
            {diagnostics.state === "configUnreadable" ? <p>{t("未能读取配置，暂时无法判断请求会经过哪里。")}</p> : null}
            <div className="gw-agent-diagnostic-endpoints">
              <p><span>{t("当前地址")}</span><code>{diagnostics.endpoint || t("未读取到地址")}</code></p>
              <p><span>{t("预期地址")}</span><code>{diagnostics.expectedEndpoint}</code></p>
            </div>
            {diagnostics.recentError ? (
              <p className="gw-agent-diagnostic-error">
                {tf("最近一次网关错误：{0}", [`${t(diagnosticErrorLabel(diagnostics.recentError.kind))}${diagnostics.recentError.status ? ` · HTTP ${diagnostics.recentError.status}` : ""}`])}
              </p>
            ) : null}
            {diagnostics.configEvidence.length ? (
              <dl className="gw-agent-diagnostic-evidence">
                {diagnostics.configEvidence.map((evidence, index) => (
                  <div key={`${evidence.path}:${evidence.field}:${index}`}>
                    <dt><code>{evidence.path}</code><span>{evidence.field}</span></dt>
                    <dd>
                      <span>{t(diagnosticEvidenceLabel(evidence.code))}</span>
                      {evidence.endpoint ? <code>{evidence.endpoint}</code> : null}
                    </dd>
                  </div>
                ))}
              </dl>
            ) : null}
          </details>
        ) : null}
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

/** 模型列表：左侧全部 / 收藏 / 按供应商跳转，右侧分组列出模型；顶部是「默认（不经 Codex++）」与「断开」。
 * 传了 `own` 与 `onPickOwn` 时，池里的模型后面再列一组 Agent 自带的模型（直连，不经 Codex++）。 */
export function ModelList({
  payload,
  exclude,
  selectedModel,
  head,
  skipGroup,
  onPick,
  own,
  ownAgent,
  onPickOwn,
}: {
  payload: GatewayPayload | null;
  exclude: string[];
  selectedModel: string;
  head?: ReactNode;
  skipGroup?: string;
  onPick: (item: PickerItem) => void;
  own?: OwnModels | null;
  /** Agent 的显示名，自带模型的说明里用。 */
  ownAgent?: string;
  onPickOwn?: (item: PickerItem) => void;
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
  const ownList = onPickOwn ? own : null;
  const sections = useMemo(() => {
    const all = withOwnSection(pickerSections(payload, query, avatarOf, exclude, skipGroup), ownPickerSection(ownList, query, avatarOf));
    // 手输的模型 ID（custom）不属于哪一家，按供应商筛选时也留着。
    if (sectionOnly) return all.filter((section) => section.key === sectionOnly || section.key === "custom");
    if (!favoritesOnly) return all;
    // 收藏的是池里的模型；Agent 自带的同名模型不算。
    return all
      .filter((section) => section.key !== "own")
      .map((section) => ({ ...section, items: section.items.filter((item) => isFavoriteModel(favorites, item.model, payload)) }))
      .filter((section) => section.items.length);
  }, [payload, query, exclude, favoritesOnly, sectionOnly, favorites, skipGroup, ownList]);
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
  const isSelected = (item: PickerItem) => {
    const current = item.direct ? ownList?.current ?? "" : selectedModel;
    return Boolean(current) && item.model.toLowerCase() === current.toLowerCase();
  };
  const choose = (item: PickerItem) => (item.direct && onPickOwn ? onPickOwn(item) : onPick(item));

  const filters = useMemo(
    () => withOwnSection(pickerSections(payload, "", avatarOf, exclude, skipGroup), ownPickerSection(ownList, "", avatarOf)).filter((section) => section.key !== "custom"),
    [payload, exclude, skipGroup, ownList],
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
                  data-direct={item.direct ? true : undefined}
                  data-selected={selected}
                  key={item.model}
                  onClick={() => choose(item)}
                  onKeyDown={(event) => { if (event.key === "Enter") choose(item); }}
                  role="option"
                  tabIndex={0}
                >
                  <span className="gw-mp-text">
                    <strong title={item.model}>{item.label}</strong>
                    {item.custom ? <small>{t("自定义模型 ID")}</small> : item.sub && item.sub !== section.title ? <small>{item.sub}</small> : null}
                  </span>
                  {item.direct ? (
                    <em className="gw-mp-direct" title={tf("{0} 用自己的登录或 Key 直接向 {1} 要这个模型，不经 Codex++。选它会断开 Codex++ 并还原接入前的设置。", [ownAgent ?? "Agent", item.direct])}>
                      {t("直连，不经 Codex++")}
                    </em>
                  ) : null}
                  {item.direct ? null : <button
                    aria-label={favorite ? t("取消收藏") : t("收藏")}
                    className="gw-mp-star"
                    data-on={favorite}
                    onClick={(event) => { event.stopPropagation(); toggleFavorite(item.model); }}
                    type="button"
                  >
                    <Star className="h-3.5 w-3.5" />
                  </button>}
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

/** 把 Agent 自带模型那一组放在池里的模型后面、手输的模型 ID 前面。 */
function withOwnSection(sections: PickerSection[], own: PickerSection | null): PickerSection[] {
  if (!own) return sections;
  const custom = sections.findIndex((section) => section.key === "custom");
  return custom < 0 ? [...sections, own] : [...sections.slice(0, custom), own, ...sections.slice(custom)];
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
    <>
      <SetLine
        hint={value === "" && !auto ? t("池里还没有能生图的模型：登录 ChatGPT 账号，或给供应商加上生图模型") : t("Codex 的生图请求发给谁")}
        title={t("生图模型")}
      >
        <AppSelect
          className="gw-image-select"
          disabled={busy !== null}
          label={t("生图模型")}
          menuWidth={320}
          onChange={(next) => void run(`image:${agent}`, "save_gateway_image_model", { agent, model: next }, t("保存生图模型"), true)}
          options={items}
          value={value}
        />
      </SetLine>
      {value !== "off" ? (
        <SetLine
          hint={t("上游只给图片链接时，下载下来再交给 Codex")}
          title={t("图片链接转成图片数据")}
          tooltip={t("上游只返回图片链接时，网关把图下载下来转成图片数据再交给 Agent（Codex 只认图片数据）。关掉后原样转发链接。")}
        >
          <Switch
            disabled={busy !== null}
            label={t("图片链接转成图片数据")}
            on={inlineUrls}
            onToggle={() => void run(`image-inline:${agent}`, "save_gateway_image_inline", { agent, enabled: !inlineUrls }, t("保存生图选项"), true)}
          />
        </SetLine>
      ) : null}
    </>
  );
}

/** Agent 详情里的「自带模型」：模型下拉里列不列 Agent 自己的模型（直连，不经 Codex++）。 */
function OwnModelsRow({ agent, payload, busy, run }: { agent: GatewayAgentView; payload: GatewayPayload | null; busy: string | null; run: Run }) {
  const own = agent.ownModels;
  if (!own) return null;
  const shown = !(payload?.gateway.hideOwnModels ?? []).includes(agent.agent);
  return (
    <SetLine
      hint={tf("直连 {0} · {1} 个，不经 Codex++", [own.vendor, String(own.models.length)])}
      title={tf("显示 {0} 自带的模型", [agent.name])}
      tooltip={tf("{0} 用自己的登录或 Key 直接向 {1} 要的模型，不经 Codex++。关掉后模型下拉只列网关的模型；{0} 现在用着的那个照样显示。", [agent.name, own.vendor])}
    >
      <Switch
        disabled={busy !== null}
        label={tf("在模型下拉里显示 {0} 自带的模型", [agent.name])}
        on={shown}
        onToggle={() => void run(`own-shown:${agent.agent}`, "save_gateway_own_models_shown", { agent: agent.agent, shown: !shown }, t("保存自带模型的显示设置"), true)}
      />
    </SetLine>
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
  onPickOwn,
  onDisconnect,
}: {
  agent: GatewayAgentView;
  payload: GatewayPayload | null;
  draft: BindingDraft;
  connected: boolean;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onPick: (item: PickerItem) => void;
  onPickOwn: (item: PickerItem) => void;
  onDisconnect: () => void;
}) {
  const buttonRef = useRef<HTMLButtonElement>(null);
  const display = modelDisplay(payload, draft.model, draft.poolExclude);
  // 没接入、Agent 用着自带的某个模型：按钮上显示它（直连）。
  const own = connected ? null : ownCurrent(agent);
  const showDefault = !connected && !draft.model && !own;
  const selectedModel = connected || draft.model ? draft.model.trim() || agentModels(payload, draft.poolExclude)[0] || "" : "";
  const head = (
    <>
      {/* 用默认 = 断开网关并还原接入前的配置，只放这一个入口。 */}
      <button
        className="gw-mp-action"
        data-active={!connected && !own}
        onClick={onDisconnect}
        title={connected ? tf("断开 Codex++，还原 {0} 接入之前的设置", [agent.name]) : tf("{0} 自带的默认值", [agent.name])}
        type="button"
      >
        <AgentIcon agent={agent.agent} className="gw-popover-agent" />
        {t("用默认（不经 Codex++）")}
        {!connected && !own ? <Check className="h-3.5 w-3.5" /> : null}
      </button>
    </>
  );
  return (
    <div className="gw-picker">
      <button aria-expanded={open} aria-haspopup="listbox" className="gw-picker-button" onClick={() => onOpenChange(!open)} ref={buttonRef} type="button">
        {showDefault || own ? <AgentIcon agent={agent.agent} className="gw-picker-agent" /> : <ModelViewGlyph view={display} />}
        <span title={own ? tf("{0} 自带的模型，直连 {1}，不经 Codex++", [agent.name, agent.ownModels?.vendor ?? ""]) : undefined}>
          {own ?? (showDefault ? t("默认") : display.title || t("选择模型"))}
        </span>
        {showDefault || own ? null : <AutoTag view={display} />}
        <ChevronDown className="h-3.5 w-3.5" />
      </button>
      <Floating anchorRef={buttonRef} className="gw-model-popover" onClose={() => onOpenChange(false)} open={open} width={560}>
        <ModelList
          exclude={draft.poolExclude}
          head={head}
          onPick={onPick}
          onPickOwn={onPickOwn}
          own={visibleOwnModels(agent, payload)}
          ownAgent={agent.name}
          payload={payload}
          selectedModel={selectedModel}
        />
      </Floating>
    </div>
  );
}

/** 没接入时 Agent 现在用的自带模型的显示名；没设时为 null。 */
export function ownCurrent(agent: GatewayAgentView): string | null {
  const own = agent.ownModels;
  const current = own?.current.trim();
  if (!own || !current) return null;
  return own.models.find((model) => model.id.toLowerCase() === current.toLowerCase())?.label ?? current;
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
        {label ? <em>{label}</em> : null}
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
