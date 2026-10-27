/** 本地网关：让 Claude Code、Grok 经 Codex++ 使用统一的供应商池，支持多 Key 轮换与路由组。 */
import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { appConfirm } from "./app-dialog";
import { AlertTriangle, ChevronDown, ChevronUp, Plus, RefreshCw, Waypoints, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { t, tf } from "./i18n";
import {
  agentName,
  connectedAgents,
  groupModeDescription,
  groupModeLabel,
  groupModelId,
  memberLabel,
  memberStatus,
  modelDisplay,
  routeMembers,
  routeNarrative,
  slugify,
  type AgentActivityView,
  type GatewayAgentId,
  type GatewayGroupEntry,
  type GatewayPayload,
  type GatewayResult,
  type GroupMember,
  type GroupMode,
  type MemberStatus,
  type RouteRecordView,
  maskAccountName,
} from "./gateway-model";
import { AgentsView, Floating, ModelList } from "./GatewayAgents";
import { AppSelect } from "./AppSelect";
import { ProvidersView } from "./GatewayProviders";
import { SessionsView } from "./GatewaySessions";
import { UsageView } from "./GatewayUsage";
import { AGENT_ICONS, APP_LOGO_URL, Avatar, SectionLabel, Switch, type Notify, type Run, AgentIcon } from "./gateway-ui";
import { TopbarActions } from "./TopbarActions";
import "./GatewayPage.css";

type Tab = "agents" | "sessions" | "providers" | "routing" | "usage" | "gateway";
export type GatewayTab = Tab;

const memo: { payload: GatewayPayload | null; tab: Tab } = { payload: null, tab: "agents" };

/** 路由图里各 Agent 连线的色相。 */
const AGENT_HUES: Record<string, number> = { claude: 18, grok: 210, codex: 250 };
/** 路由页实时刷新间隔。 */
const LIVE_REFRESH_MS = 2500;

function hasPayload(result: GatewayResult): result is GatewayResult & GatewayPayload {
  return Boolean(result.gateway && Array.isArray(result.pool));
}

function statusText(status: MemberStatus): string {
  return status.args.length ? tf(status.text, status.args) : t(status.text);
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
    }
  };

  const run: Run = async (key, command, args, title, silent = false) => {
    setBusy(key);
    try {
      const result = await invoke<GatewayResult>(command, args);
      accept(result);
      if (result.status !== "ok") {
        await notify(tf("{0}失败", [title]), result.message, "failed");
        return false;
      }
      if (!silent) await notify(title, result.message, "ok");
      return true;
    } catch (error) {
      await notify(t("调用失败"), String(error), "failed");
      return false;
    } finally {
      setBusy(null);
    }
  };

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
      void invoke<GatewayResult>("load_gateway")
        .then((result) => {
          if (!cancelled) accept(result);
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

  // 路由页实时刷新：静默读取，不占用 busy，也不弹错误。
  useEffect(() => {
    if (tab !== "routing") return;
    let cancelled = false;
    const timer = window.setInterval(() => {
      void invoke<GatewayResult>("load_gateway")
        .then((result) => {
          if (!cancelled) accept(result);
        })
        .catch(() => undefined);
    }, LIVE_REFRESH_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab]);

  const tabs: Array<{ id: Tab; label: string }> = [
    { id: "agents", label: t("智能体") },
    { id: "sessions", label: t("会话") },
    { id: "providers", label: t("供应商") },
    { id: "routing", label: t("路由") },
    { id: "usage", label: t("用量") },
    { id: "gateway", label: t("网关") },
  ];

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
        <TopbarActions>{refreshButton}</TopbarActions>
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
    </div>
  );
}

/* —————————————————————— 路由 —————————————————————— */

function RoutingView({ payload, busy, run }: { payload: GatewayPayload | null; busy: string | null; run: Run }) {
  const agents = connectedAgents(payload);
  const [editing, setEditing] = useState<GatewayGroupEntry | "new" | null>(null);
  const remove = async (group: GatewayGroupEntry) => {
    const ok = await appConfirm(tf("删除路由组「{0}」？引用它的其它组会去掉这个成员。", [group.name]), { title: t("删除路由组"), confirmText: t("删除"), danger: true });
    if (ok) await run(`group:${group.id}`, "delete_gateway_group", { id: group.id }, t("删除路由组"));
  };
  return (
    <>
      <SectionLabel>
        {t("路由")}
        <i className="gw-dot gw-dot-live" data-tone={payload?.helperRunning ? "on" : "off"} />
        <em>{payload?.helperRunning ? t("实时，按网关的实际决策") : t("网关没有运行")}</em>
      </SectionLabel>
      {agents.length === 0 ? (
        <div className="gw-card gw-empty">
          <strong>{t("还没有 Agent 经网关")}</strong>
          <span>{t("在「智能体」里为 Claude Code 或 Grok 选好模型并打开开关，这里会实时显示每个请求怎么走。")}</span>
        </div>
      ) : (
        <RouteCard agents={agents} payload={payload} />
      )}

      <SectionLabel
        aside={(
          <>
            <em>{t("Agent 可以把路由组当作一个模型来选")}</em>
            <button className="gw-text-button strong" disabled={busy !== null} onClick={() => setEditing("new")} type="button">{t("新建路由组")}</button>
          </>
        )}
      >
        {t("路由组")}
      </SectionLabel>
      <div className="gw-card">
        {(payload?.groups ?? []).map((group) => {
          const joiner = group.mode === "inOrder" ? " → " : " · ";
          const icons = group.members
            .map((member) => modelDisplay(payload, member.model))
            .filter((view) => !view.group && view.sub)
            .slice(0, 3);
          return (
            <div className="gw-group-row" key={group.id}>
              <span className="gw-avatar-stack">
                {icons.length ? icons.map((view, index) => <Avatar key={`${view.sub}-${index}`} name={view.sub} size="sm" />) : <Waypoints className="h-4 w-4" />}
              </span>
              <div className="gw-group-main">
                <div>
                  <strong>{group.name}</strong>
                  <code className="gw-chip">{groupModelId(group.id)}</code>
                </div>
                <small>{group.members.length ? group.members.map((member) => memberLabel(payload, member.model)).join(joiner) : t("没有成员")}</small>
              </div>
              <span className="gw-pill gw-pill-outline">{t(groupModeLabel(group.mode))}</span>
              <span className="gw-row-buttons">
                <button className="gw-text-button" disabled={busy !== null} onClick={() => setEditing(group)} type="button">{t("编辑")}</button>
                <button className="gw-text-button" disabled={busy !== null} onClick={() => void remove(group)} type="button">{t("删除")}</button>
              </span>
            </div>
          );
        })}
        {payload && payload.groups.length === 0 ? (
          <div className="gw-empty">
            <strong>{t("还没有路由组")}</strong>
            <span>{t("把几个模型编成一组，Agent 在模型列表里把它当一个模型来选：按顺序、轮换或按权重回答，组里也可以再放组。")}</span>
          </div>
        ) : null}
      </div>
      {editing ? (
        <GroupEditor
          busy={busy}
          group={editing === "new" ? null : editing}
          onCancel={() => setEditing(null)}
          onSave={async (input) => {
            if (await run("group:save", "save_gateway_group", { group: input }, t("保存路由组"))) setEditing(null);
          }}
          payload={payload}
        />
      ) : null}
    </>
  );
}

/** 新建 / 编辑路由组：名称、模式、成员（池里的模型或别的组）。 */
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
  onSave: (input: { id: string; name: string; mode: GroupMode; members: GroupMember[] }) => void;
  onCancel: () => void;
}) {
  const [name, setName] = useState(group?.name ?? "");
  const [mode, setMode] = useState<GroupMode>(group?.mode ?? "inOrder");
  const [members, setMembers] = useState<GroupMember[]>(group?.members.map((member) => ({ ...member })) ?? []);
  const [picking, setPicking] = useState(false);
  const addRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !picking) onCancel();
    };
    document.addEventListener("keydown", escape);
    return () => document.removeEventListener("keydown", escape);
  }, [onCancel, picking]);
  const move = (index: number, delta: number) => {
    const next = [...members];
    const [item] = next.splice(index, 1);
    next.splice(Math.max(0, Math.min(next.length, index + delta)), 0, item);
    setMembers(next);
  };
  const error = !name.trim() ? t("请填写路由组名称") : members.length === 0 ? t("路由组至少要有一个成员") : "";
  const modes: Array<{ value: GroupMode; label: string }> = [
    { value: "inOrder", label: t("依次") },
    { value: "rotate", label: t("轮换") },
    { value: "weighted", label: t("按权重") },
  ];
  return createPortal(
    <div className="gw-modal-backdrop gw-portal" onMouseDown={(event) => { if (event.target === event.currentTarget) onCancel(); }}>
      <form
        className="gw-modal gw-group-modal"
        onSubmit={(event) => {
          event.preventDefault();
          if (!error) onSave({ id: group?.id ?? "", name: name.trim(), mode, members });
        }}
      >
        <header>
          <span className="gw-avatar gw-avatar-plus"><Waypoints className="h-4 w-4" /></span>
          <strong>{group ? tf("编辑 {0}", [group.name]) : t("新建路由组")}</strong>
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
            <span>{t("成员")} <small>{mode === "inOrder" ? t("从上到下依次尝试") : t("答不了时按列表顺序换下一个")}</small></span>
            <div className="gw-member-list">
              {members.map((member, index) => {
                const view = modelDisplay(payload, member.model);
                return (
                  <div className="gw-member-edit" key={`${member.model}-${index}`}>
                    {view.group ? <Waypoints className="gw-picker-glyph h-4 w-4" /> : <Avatar name={view.sub || view.title} size="sm" />}
                    <span className="gw-member-edit-name">
                      <strong>{view.title}</strong>
                      <small>{view.group ? view.sub : view.sub || t("池里还没有提供它的供应商")}</small>
                    </span>
                    {mode === "weighted" ? (
                      <label className="gw-inline-field">
                        <span>{t("权重")}</span>
                        <input
                          className="gw-inline-input gw-weight-input"
                          min={1}
                          onChange={(event) => {
                            const next = [...members];
                            next[index] = { ...member, weight: Math.max(1, Number(event.currentTarget.value) || 1) };
                            setMembers(next);
                          }}
                          type="number"
                          value={member.weight}
                        />
                      </label>
                    ) : null}
                    <button aria-label={t("上移")} className="gw-icon-button" disabled={index === 0} onClick={() => move(index, -1)} type="button"><ChevronUp className="h-4 w-4" /></button>
                    <button aria-label={t("下移")} className="gw-icon-button" disabled={index === members.length - 1} onClick={() => move(index, 1)} type="button"><ChevronDown className="h-4 w-4" /></button>
                    <button aria-label={t("移除")} className="gw-icon-button" onClick={() => setMembers(members.filter((_, position) => position !== index))} type="button"><X className="h-4 w-4" /></button>
                  </div>
                );
              })}
              <button className="gw-add-row" onClick={() => setPicking(!picking)} ref={addRef} type="button">
                <Plus className="h-4 w-4" />
                {t("添加成员")}
              </button>
              <Floating anchorRef={addRef} className="gw-model-popover" onClose={() => setPicking(false)} open={picking} width={520}>
                <ModelList
                  exclude={[]}
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

function RouteCard({ payload, agents }: { payload: GatewayPayload | null; agents: GatewayAgentId[] }) {
  const activity = (payload?.activity?.agents ?? []).filter((item): item is AgentActivityView => agents.includes(item.agent as GatewayAgentId));
  const latest = activity
    .filter((item) => item.last)
    .sort((left, right) => (right.last?.finishedAtMs ?? 0) - (left.last?.finishedAtMs ?? 0))[0];
  const records = activity.map((item) => item.last).filter((record): record is RouteRecordView => Boolean(record));
  const members = routeMembers(payload, records);
  const totals = activity.reduce((sum, item) => ({ requests: sum.requests + item.requests, rerouted: sum.rerouted + item.rerouted, errors: sum.errors + item.errors }), { requests: 0, rerouted: 0, errors: 0 });
  const lastGroup = latest?.last?.groupId ? payload?.groups.find((group) => group.id === latest.last?.groupId) : undefined;
  const summary = latest?.last?.planned[0]
    ? tf("{0} 排在第一个。", [maskAccountName(latest.last.planned[0].relayName)])
    : t("还没有请求经过这里。在 Agent 里发一条消息试试。");

  return (
    <div className="gw-card gw-route-card">
      <div className="gw-route-head">
        <strong>{t("供应商池")}</strong>
        <span> · {tf("{0} 个供应商 · {1} 个 Agent", [members.length, agents.length])}</span>
        <p>{lastGroup ? t(groupModeDescription(lastGroup.mode)) : t("按模型名选供应商：Agent 请求哪个模型，就发给池里提供它的供应商；同一个模型有多家时按池里的顺序依次尝试，每家再按自己的 Key 轮换。")}</p>
      </div>
      <RouteDiagram
        activeAgent={latest?.agent ?? null}
        agents={agents.map((agent) => ({ id: agent, binding: payload?.gateway.bindings[agent] }))}
        hubPill={lastGroup ? t(groupModeLabel(lastGroup.mode)) : t("按模型")}
        members={members.map((member) => ({ ...member, status: memberStatus(payload, member.relayId) }))}
        payload={payload}
      />
      <div className="gw-route-summary">
        <p>{summary}</p>
        <dl>
          <div><dt>{totals.requests}</dt><dd>{t("个请求")}</dd></div>
          <div><dt>{totals.rerouted}</dt><dd>{t("次换上游")}</dd></div>
          <div><dt>{totals.errors}</dt><dd>{t("个错误交给了 Agent")}</dd></div>
        </dl>
      </div>
      {latest?.last ? (
        <div className="gw-route-trace">
          <h4>{t("上一个请求是怎么路由的")}</h4>
          <ul>
            {routeNarrative(latest.agent, latest.last, payload).map((line, index) => (
              <li data-tone={line.tone} key={index}>{tf(line.text, line.args.map((arg) => (typeof arg === "string" && /^[一-龥]/.test(arg) ? t(arg) : arg)))}</li>
            ))}
          </ul>
        </div>
      ) : null}
    </div>
  );
}

const AGENT_NODE_H = 76;
const AGENT_NODE_GAP = 12;
const MEMBER_ROW_H = 46;
const MEMBER_ROW_GAP = 8;
const HUB_H = 118;
const LINK_W = 64;

/** 路由图：左边 Agent，中间网关，右边成员；最近一次请求走过的线用虚线高亮。 */
function RouteDiagram({
  payload,
  agents,
  members,
  activeAgent,
  hubPill,
}: {
  payload: GatewayPayload | null;
  agents: Array<{ id: GatewayAgentId; binding: GatewayPayload["gateway"]["bindings"][GatewayAgentId] }>;
  members: Array<{ relayId: string; name: string; model: string; status: MemberStatus }>;
  activeAgent: string | null;
  hubPill: string;
}) {
  const agentsH = agents.length * AGENT_NODE_H + Math.max(0, agents.length - 1) * AGENT_NODE_GAP;
  const membersH = members.length * MEMBER_ROW_H + Math.max(0, members.length - 1) * MEMBER_ROW_GAP;
  const height = Math.max(agentsH, membersH, HUB_H);
  const agentTop = (height - agentsH) / 2;
  const memberTop = (height - membersH) / 2;
  const middle = height / 2;
  const agentY = (index: number) => agentTop + index * (AGENT_NODE_H + AGENT_NODE_GAP) + AGENT_NODE_H / 2;
  const memberY = (index: number) => memberTop + index * (MEMBER_ROW_H + MEMBER_ROW_GAP) + MEMBER_ROW_H / 2;
  const curve = (fromY: number, toY: number) => `M0 ${fromY} C${LINK_W / 2} ${fromY}, ${LINK_W / 2} ${toY}, ${LINK_W} ${toY}`;
  const hue = AGENT_HUES[activeAgent ?? ""] ?? 232;
  const activeStyle = { "--gw-hue": hue } as CSSProperties;

  return (
    <div className="gw-diagram" style={{ height }}>
      <div className="gw-diagram-col" style={{ paddingTop: agentTop, gap: AGENT_NODE_GAP }}>
        {agents.map((agent) => (
          <div className="gw-node-agent" key={agent.id} style={{ height: AGENT_NODE_H, "--gw-hue": AGENT_HUES[agent.id] ?? 232 } as CSSProperties}>
            <AgentIcon agent={agent.id} className="gw-node-agent-icon" />
            <strong><i className="gw-dot" data-tone="agent" />{agentName(agent.id)}</strong>
            <small>{agent.binding ? modelDisplay(payload, agent.binding.model, agent.binding.poolExclude ?? []).title : ""}</small>
          </div>
        ))}
      </div>
      <svg aria-hidden className="gw-links" height={height} width={LINK_W}>
        {agents.map((agent, index) => (
          <path className={agent.id === activeAgent ? "active" : ""} d={curve(agentY(index), middle)} key={agent.id} style={agent.id === activeAgent ? activeStyle : undefined} />
        ))}
      </svg>
      <div className="gw-diagram-col" style={{ paddingTop: (height - HUB_H) / 2 }}>
        <div className="gw-hub" style={{ height: HUB_H }}>
          <img alt="" src={APP_LOGO_URL} />
          <strong>{t("Codex++ 网关")}</strong>
          <small>{`127.0.0.1:${payload?.port ?? ""}`}</small>
          <span className="gw-pill gw-pill-solid">{hubPill}</span>
        </div>
      </div>
      <svg aria-hidden className="gw-links" height={height} width={LINK_W}>
        {members.map((member, index) => {
          const active = member.status.state === "answered" || member.status.state === "busy";
          return <path className={active ? "active" : ""} d={curve(middle, memberY(index))} key={member.relayId} style={active ? activeStyle : undefined} />;
        })}
      </svg>
      <div className="gw-diagram-col" style={{ paddingTop: memberTop, gap: MEMBER_ROW_GAP }}>
        {members.map((member) => (
          <div className="gw-member" data-state={member.status.state} key={member.relayId} style={{ height: MEMBER_ROW_H }}>
            <i className="gw-radio" />
            <Avatar name={member.name} size="sm" />
            <strong>{maskAccountName(member.name)}</strong>
            {member.model ? <code className="gw-chip">{member.model}</code> : null}
            <span className="gw-status" data-state={member.status.state}>{statusText(member.status)}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

/* —————————————————————— 网关 —————————————————————— */

function GatewayView({ payload, busy, run }: { payload: GatewayPayload | null; busy: string | null; run: Run }) {
  if (!payload) return null;
  const mode = payload.gateway.keyPolicy.mode;
  const restoreOnExit = payload.gateway.restoreOnExit;
  const trayPanel = payload.gateway.trayPanel !== false;
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
            <small>{t("网关只在 Codex++ 运行时可用。打开后退出时把 Agent 切回直连，下次启动再自动指回网关。")}</small>
          </div>
          <span />
          <Switch disabled={busy !== null} label={t("退出 Codex++ 时恢复直连")} on={restoreOnExit} onToggle={() => saveOptions(mode, !restoreOnExit)} />
        </div>
        <div className="gw-setting-row">
          <div>
            <strong>{t("点击托盘图标弹出快捷面板")}</strong>
            <small>{t("面板里能直接给各 Agent 换模型、看最近的请求；右键仍是原来的菜单，双击打开主窗口。关掉恢复原来的点击行为。")}</small>
          </div>
          <span />
          <Switch
            disabled={busy !== null}
            label={t("点击托盘图标弹出快捷面板")}
            on={trayPanel}
            onToggle={() => void (async () => {
              if (await run("tray-panel", "save_gateway_tray_panel", { enabled: !trayPanel }, t("保存网关选项"), true)) await invoke("refresh_tray_mode");
            })()}
          />
        </div>
      </div>
    </>
  );
}
