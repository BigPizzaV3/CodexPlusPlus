/** 本地网关 · 供应商：网关自己的供应商池（可开关、排序、编辑、删除），右上角「添加供应商」打开目录弹窗。
 *
 * 与直连模式完全独立：这里的增删改只动 gateway.json；「导入…」是把直连模式的供应商复制一份过来。
 */
import { useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { createPortal } from "react-dom";
import { appConfirm } from "./app-dialog";
import { Check, ChevronRight, Copy, GripVertical, Plus, RefreshCw, Search, X } from "lucide-react";
import { t, tf } from "./i18n";
import {
  defaultProviderAgents,
  failureLabel,
  formatRemaining,
  gatewayPresets,
  groupModeLabel,
  groupStickyLabel,
  importCandidates,
  memberStatus,
  pendingImportCount,
  presetAdded,
  presetHost,
  protocolLabel,
  providerEnabled,
  providerInput,
  skippedAgents,
  type GatewayAgentId,
  type GatewayPayload,
  type GatewayPoolEntry,
  type GatewayPreset,
  type PresetSection,
  type ProviderDraft,
  type ProviderPrice,
  maskAccountName,
  type ImportCandidate,
  type ImportSource,
} from "./gateway-model";
import { AgentIcon, Avatar, KeyCount, MemberStatusLabel, QuotaBadge, QuotaWindows, Switch, type Notify, type Run } from "./gateway-ui";
import { ProviderFormDialog, type ProviderFormState } from "./ProviderFormDialog";
import { ProviderExtrasDialog, priceSummary } from "./ProviderExtrasDialog";
import { ProviderWindowsDialog, windowSummary } from "./ProviderWindowsDialog";
import { ProviderReasoningDialog, reasoningSummary } from "./ProviderReasoningDialog";
import { ProviderModelsDialog } from "./ProviderModelsDialog";
import { gatewayJump } from "./gateway-store";
import { ModelMatrix } from "./GatewayModelMatrix";
import { useEscapeLayer } from "./use-escape-layer";

/** 能在网关里直接登录的订阅。 */
type AccountKind = "chatgpt" | "chatgpt-api" | "copilot" | "grok";
type DeviceCode = { deviceCode: string; userCode: string; verificationUri: string; interval: number; expiresIn: number };

const ACCOUNT_LABELS: Record<AccountKind, string> = { chatgpt: "ChatGPT", "chatgpt-api": "ChatGPT API", copilot: "GitHub Copilot", grok: "SuperGrok" };

/** 浏览器登录的订阅各自的命令。 */
const BROWSER_LOGIN_COMMANDS: Record<Exclude<AccountKind, "copilot">, string> = {
  chatgpt: "login_gateway_chatgpt",
  "chatgpt-api": "login_gateway_chatgpt_api",
  grok: "login_gateway_grok",
};

/** 表单弹窗的用途：按预设新建、自定义新建、编辑已有供应商。 */
type FormState = ProviderFormState;

/** 供应商池上次用的视图（切页再回来保持）。 */
const providersViewMemo: { view: "list" | "models" } = { view: "list" };

export function ProvidersView({
  payload,
  busy,
  run,
  notify,
}: {
  payload: GatewayPayload | null;
  busy: string | null;
  run: Run;
  notify: Notify;
}) {
  const pool = payload?.pool ?? [];
  const [query, setQuery] = useState("");
  // 从别的页跳过来指定了供应商：展开那一家并滚到它。读完就清掉。
  const [expanded, setExpanded] = useState<string | null>(() => {
    const jumped = gatewayJump.provider;
    gatewayJump.provider = null;
    return jumped;
  });
  const jumpedTo = useRef(expanded);
  useEffect(() => {
    const id = jumpedTo.current;
    jumpedTo.current = null;
    if (id) document.querySelector(`[data-pool-id="${CSS.escape(id)}"]`)?.scrollIntoView({ block: "center" });
  }, []);
  const [adding, setAdding] = useState<false | "catalog" | "import">(false);
  const pendingImports = pendingImportCount(payload);
  // 「按供应商」是原来的列表；「按模型」是供应商 × 模型矩阵，看每个模型由哪几家提供。
  const [view, setViewState] = useState<"list" | "models">(providersViewMemo.view);
  const setView = (next: "list" | "models") => {
    providersViewMemo.view = next;
    setViewState(next);
  };
  const [dragging, setDragging] = useState<string | null>(null);
  // 编辑已有供应商的表单；新添加走 ProviderAdder。
  const [form, setForm] = useState<FormState | null>(null);
  const openCatalog = () => setAdding("catalog");
  // 拖动排序中的临时顺序；松手后提交，等新数据回来再清掉。
  const [order, setOrder] = useState<string[] | null>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const needle = query.trim().toLowerCase();
  const ordered = order ? order.map((id) => pool.find((entry) => entry.id === id)).filter((entry): entry is GatewayPoolEntry => Boolean(entry)) : pool;
  const visible = ordered.filter((entry) => !needle || entry.name.toLowerCase().includes(needle) || entry.models.some((model) => model.toLowerCase().includes(needle)));

  /** 按住把手上下拖动排序。用指针事件实现：桌面端 webview 会拦截 HTML5 拖放，原生 draggable 拖不动。 */
  const startDrag = (id: string, event: ReactPointerEvent) => {
    if (event.button !== 0) return;
    event.preventDefault();
    const initial = pool.map((entry) => entry.id);
    let current = initial;
    setExpanded(null);
    setDragging(id);
    setOrder(initial);
    // 落点按其它行的中线算：指针越过哪几行的中线，就排在它们后面。行高不同、列表正在重排时也不会来回跳。
    const move = (moveEvent: PointerEvent) => {
      const others = [...(listRef.current?.querySelectorAll<HTMLElement>("[data-pool-id]") ?? [])]
        .filter((row) => row.dataset.poolId !== id)
        .map((row) => {
          const rect = row.getBoundingClientRect();
          return { id: row.dataset.poolId ?? "", middle: rect.top + rect.height / 2 };
        })
        .sort((left, right) => left.middle - right.middle);
      if (!others.length) return;
      const above = others.filter((row) => row.middle < moveEvent.clientY);
      const next = current.filter((item) => item !== id);
      const index = above.length ? next.indexOf(above[above.length - 1].id) + 1 : next.indexOf(others[0].id);
      next.splice(Math.max(index, 0), 0, id);
      if (next.join("\n") === current.join("\n")) return;
      current = next;
      setOrder(next);
    };
    const finish = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", finish);
      window.removeEventListener("pointercancel", finish);
      document.body.classList.remove("gw-sorting");
      setDragging(null);
      if (current.join("\n") === initial.join("\n")) {
        setOrder(null);
        return;
      }
      void run("reorder", "reorder_gateway_providers", { ids: current }, t("调整顺序"), true).finally(() => setOrder(null));
    };
    document.body.classList.add("gw-sorting");
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", finish);
    window.addEventListener("pointercancel", finish);
  };

  const { login, pending: loginPending, dialog: loginDialog } = useAccountLogin(run, notify);
  const saveEdit = async (draft: ProviderDraft) => {
    if (form?.mode !== "edit") return;
    if (await run(`edit:${form.entry.id}`, "update_gateway_provider", { id: form.entry.id, input: providerInput(draft) }, t("保存供应商"))) setForm(null);
  };

  return (
    <>
      <div className="gw-provider-search-row">
        {pool.length ? (
          <div className="gw-models-tabs" role="tablist" aria-label={t("视图")}>
            <button aria-selected={view === "list"} data-active={view === "list"} onClick={() => setView("list")} role="tab" type="button">{t("按供应商")}</button>
            <button aria-selected={view === "models"} data-active={view === "models"} onClick={() => setView("models")} role="tab" type="button">{t("按模型")}</button>
          </div>
        ) : null}
        {view === "list" ? (
          <label className="gw-sheet-search">
            <Search className="h-4 w-4" />
            <input onChange={(event) => setQuery(event.currentTarget.value)} placeholder={t("查找供应商")} value={query} />
          </label>
        ) : <span className="gw-provider-search-gap" />}
        {pool.length ? (
          <button className="gw-primary-button" onClick={openCatalog} type="button">
            <Plus className="h-4 w-4" />
            {t("添加供应商")}
          </button>
        ) : null}
      </div>
      {pool.length && view === "models" ? (
        <ModelMatrix
          onOpenProvider={(id) => {
            // 点列头：回到按供应商的列表，展开并滚到那一家。
            setView("list");
            setExpanded(id);
            window.requestAnimationFrame(() => document.querySelector(`[data-pool-id="${CSS.escape(id)}"]`)?.scrollIntoView({ block: "center" }));
          }}
          payload={payload}
        />
      ) : null}
      {pool.length && view === "list" ? (
        <div className="gw-card gw-pool-card" ref={listRef}>
          {visible.map((entry) => (
            <PoolRow
              busy={busy}
              dragging={dragging === entry.id}
              entry={entry}
              expanded={expanded === entry.id}
              key={entry.id}
              onGripDown={(event) => startDrag(entry.id, event)}
              onEdit={() => setForm({ mode: "edit", entry })}
              loggingIn={Boolean(entry.account) && loginPending === entry.account}
              onRelogin={() => void login(entry.account as AccountKind)}
              onToggleExpand={() => setExpanded(expanded === entry.id ? null : entry.id)}
              payload={payload}
              run={run}
            />
          ))}
          {visible.length === 0 ? <div className="gw-empty">{t("没有匹配的供应商")}</div> : null}
        </div>
      ) : null}
      {payload !== null && pool.length === 0 ? (
        <div className="gw-card gw-pool-empty">
          <strong>{t("供应商池还是空的")}</strong>
          {pendingImports ? (
            <span>{tf("直连模式里有 {0} 个供应商，可以一次导入网关；之后两边各改各的。也可以登录订阅，或添加新的厂商 API、中转站。", [pendingImports])}</span>
          ) : (
            <span>{t("登录订阅，或添加厂商 API、中转站的 Key。所有接入的 Agent 共用这里的供应商。")}</span>
          )}
          <div className="gw-pool-empty-actions">
            {pendingImports ? (
              <button className="gw-primary-button" onClick={() => setAdding("import")} type="button">
                {tf("从直连模式导入 {0} 个", [pendingImports])}
              </button>
            ) : null}
            <button className={pendingImports ? "gw-soft-button" : "gw-primary-button"} onClick={openCatalog} type="button">
              <Plus className="h-4 w-4" />
              {t("添加供应商")}
            </button>
          </div>
        </div>
      ) : null}
      {adding ? <ProviderAdder busy={busy} notify={notify} onClose={() => setAdding(false)} payload={payload} run={run} startWith={adding === "import" ? "import" : "catalog"} /> : null}
      {loginDialog}
      {form ? <ProviderFormDialog busy={busy} form={form} onCancel={() => setForm(null)} onSubmit={(draft) => void saveEdit(draft)} /> : null}
    </>
  );
}

/**
 * 订阅账号登录（新添加和「重新登录」共用）：返回登录函数、进行中的登录和它的弹窗 / 提示条。
 *
 * 登录要等用户在浏览器或 GitHub 上操作，最长 5–15 分钟，所以不占全局 busy（其它按钮照常可用），
 * 只用自己的 `pending`。关掉 Copilot 验证码弹窗、按 Esc 或点「取消登录」都会让后端中止这次登录
 * （释放回调端口、停止轮询）；开始新的登录时后端也会中止上一次。
 */
function useAccountLogin(run: Run, notify: Notify) {
  const [pending, setPending] = useState<AccountKind | null>(null);
  const [deviceCode, setDeviceCode] = useState<DeviceCode | null>(null);
  // 当前这次登录的序号：取消或开始新的登录后，旧登录的收尾不再改界面。
  const attempt = useRef(0);
  const cancel = () => {
    attempt.current += 1;
    setPending(null);
    setDeviceCode(null);
    void invoke("cancel_gateway_login").catch(() => undefined);
  };
  /** 登录订阅账号：ChatGPT / ChatGPT API / SuperGrok 走浏览器登录；Copilot 先显示验证码，再等你在 GitHub 上授权。
   * `skip` 是新添加时取消勾选的 Agent；重新登录已有账号时不传。 */
  const login = async (kind: AccountKind, skip?: GatewayAgentId[]) => {
    const id = ++attempt.current;
    const current = () => attempt.current === id;
    const title = tf("登录 {0}", [ACCOUNT_LABELS[kind]]);
    setPending(kind);
    setDeviceCode(null);
    try {
      if (kind === "copilot") {
        let code: DeviceCode;
        try {
          const result = await invoke<{ status: string; message: string } & Partial<DeviceCode>>("start_gateway_copilot_login");
          if (result.status !== "ok" || !result.userCode || !result.deviceCode) {
            await notify(tf("{0}失败", [title]), result.message, "failed");
            return;
          }
          code = { deviceCode: result.deviceCode, userCode: result.userCode, verificationUri: result.verificationUri ?? "https://github.com/login/device", interval: result.interval ?? 5, expiresIn: result.expiresIn ?? 900 };
        } catch (error) {
          await notify(t("调用失败"), String(error), "failed");
          return;
        }
        // 拿验证码期间已经取消了。
        if (!current()) return;
        setDeviceCode(code);
        await run("login:copilot", "finish_gateway_copilot_login", { code, skipAgents: skip }, title, false, { background: true });
        return;
      }
      void notify(title, t("已在浏览器打开登录页，登录完成后回到这里。5 分钟内有效。"), "info");
      await run(`login:${kind}`, BROWSER_LOGIN_COMMANDS[kind], skip ? { skipAgents: skip } : undefined, title, false, { background: true });
    } finally {
      if (current()) {
        setPending(null);
        setDeviceCode(null);
      }
    }
  };

  const dialog = deviceCode ? (
    <DeviceCodeDialog code={deviceCode} notify={notify} onCancel={cancel} />
  ) : pending ? (
    <LoginPendingBar label={ACCOUNT_LABELS[pending]} onCancel={cancel} />
  ) : null;
  return { login, pending, dialog };
}

/** 「添加供应商」整套流程：目录弹窗 → 填写表单或订阅登录，顶部可选给哪些 Agent 用。
 * 供应商池和总览都用它；流程结束（添加成功、取消或登录结束）时调 `onClose`。 */
export function ProviderAdder({
  payload,
  busy,
  run,
  notify,
  onClose,
  startWith = "catalog",
}: {
  payload: GatewayPayload | null;
  busy: string | null;
  run: Run;
  notify: Notify;
  onClose: () => void;
  /** 「import」：引导卡片点进来，直接打开从直连模式导入。 */
  startWith?: "catalog" | "import";
}) {
  // 这次添加给哪些 Agent 用：按各 Agent 的「添加供应商时自动加入」预选。
  const [picked, setPicked] = useState<GatewayAgentId[]>(() => defaultProviderAgents(payload));
  const skipAgents = skippedAgents(payload, picked);
  const [catalogOpen, setCatalogOpen] = useState(startWith === "catalog");
  const [importOnly, setImportOnly] = useState(startWith === "import");
  const [form, setForm] = useState<FormState | null>(null);
  const [loggingIn, setLoggingIn] = useState(false);
  const { login, dialog } = useAccountLogin(run, notify);
  const agentPicks = <AgentPicks onChange={setPicked} payload={payload} picked={picked} />;
  const submit = async (draft: ProviderDraft) => {
    if (!form || form.mode === "edit") return;
    const input = providerInput(draft);
    if (await run("add:create", "create_gateway_provider", { input, source: form.mode === "preset" ? `preset:${form.preset.key}` : "custom", skipAgents }, t("添加供应商"))) onClose();
  };
  return (
    <>
      {catalogOpen ? (
        <ProviderCatalog
          agentPicks={agentPicks}
          busy={busy}
          notify={notify}
          onClose={onClose}
          onCustom={() => { setCatalogOpen(false); setForm({ mode: "custom" }); }}
          onLogin={(kind) => {
            setCatalogOpen(false);
            setLoggingIn(true);
            void login(kind, skipAgents).finally(onClose);
          }}
          onPreset={(preset) => { setCatalogOpen(false); setForm({ mode: "preset", preset }); }}
          payload={payload}
          run={run}
          skipAgents={skipAgents}
        />
      ) : null}
      {importOnly ? (
        <ImportDialog
          agentPicks={agentPicks}
          busy={busy}
          onClose={onClose}
          onLogin={() => {
            setImportOnly(false);
            setLoggingIn(true);
            void login("chatgpt", skipAgents).finally(onClose);
          }}
          payload={payload}
          run={run}
          skipAgents={skipAgents}
        />
      ) : null}
      {loggingIn ? dialog : null}
      {form ? <ProviderFormDialog agentPicks={agentPicks} busy={busy} form={form} onCancel={onClose} onSubmit={(draft) => void submit(draft)} /> : null}
    </>
  );
}

function PoolRow({
  entry,
  payload,
  busy,
  run,
  expanded,
  dragging,
  onToggleExpand,
  onGripDown,
  onEdit,
  onRelogin,
  loggingIn,
}: {
  entry: GatewayPoolEntry;
  payload: GatewayPayload | null;
  busy: string | null;
  run: Run;
  expanded: boolean;
  dragging: boolean;
  onToggleExpand: () => void;
  onGripDown: (event: ReactPointerEvent) => void;
  onEdit: () => void;
  onRelogin: () => void;
  /** 这个账号类型正在登录（浏览器 / 设备码），按钮显示进行中。 */
  loggingIn: boolean;
}) {
  const enabled = providerEnabled(entry);
  const status = memberStatus(payload, entry.id);
  const failures = (payload?.health.keys ?? []).filter((key) => key.relayId === entry.id && key.state !== "ok");
  const breaker = payload?.health.providers.some((item) => item.relayId === entry.id && item.breakerOpen);
  const tone = !enabled ? "off" : status.state === "cooling" ? "error" : "on";
  const sub = (entry.account
    ? [t("订阅登录"), entry.lentToCodex ? t("Codex 正用它登录") : "", tf("{0} 个模型", [entry.models.length])]
    : [protocolLabel(entry.protocol), presetHost(entry.baseUrl), tf("{0} 个模型", [entry.models.length])]
  ).filter(Boolean).join(" · ");
  // 价格、代理各开各的弹窗。
  const [extrasOpen, setExtrasOpen] = useState<"price" | "proxy" | null>(null);
  const [modelsOpen, setModelsOpen] = useState(false);
  const [windowsOpen, setWindowsOpen] = useState(false);
  const [reasoningOpen, setReasoningOpen] = useState(false);
  const reasoning = reasoningSummary(entry);
  const saveReasoning = async (modelMetadata: string) => {
    if (await run(`reasoning:${entry.id}`, "save_gateway_provider_metadata", { id: entry.id, modelMetadata }, t("保存思考等级"))) setReasoningOpen(false);
  };
  const windows = windowSummary(entry);
  const saveWindows = async (modelWindows: Record<string, number>, fallback: number | null) => {
    if (await run(`windows:${entry.id}`, "save_gateway_provider_windows", { id: entry.id, windows: modelWindows, fallback }, t("保存上下文窗口"))) setWindowsOpen(false);
  };
  const newModels = entry.upstreamNew?.length ?? 0;
  const saveModels = async (models: string[], defaultModel: string) => {
    if (await run(`models:${entry.id}`, "save_gateway_provider_models", { id: entry.id, models, defaultModel }, t("保存模型列表"))) setModelsOpen(false);
  };
  const price = priceSummary(entry);
  // ChatGPT 订阅账号和 Copilot 能查额度（ChatGPT API 账号按量计费，没有额度窗口）。
  const quotaSupported = entry.account === "chatgpt" || entry.account === "copilot";
  const saveExtras = async (nextPrice: ProviderPrice | null, modelPrices: Record<string, ProviderPrice> | null, proxy: string) => {
    const title = extrasOpen === "proxy" ? t("保存代理") : t("保存价格");
    if (await run(`extras:${entry.id}`, "save_gateway_provider_extras", { id: entry.id, price: nextPrice, modelPrices, proxy }, title)) setExtrasOpen(null);
  };
  const remove = async () => {
    const ok = await appConfirm(tf("从网关删除「{0}」？直连模式里的供应商不受影响。", [maskAccountName(entry.name)]), { title: t("删除供应商"), confirmText: t("删除"), danger: true });
    if (ok) await run(`remove:${entry.id}`, "delete_gateway_provider", { id: entry.id }, t("删除供应商"));
  };
  return (
    <div className="gw-pool-item" data-dragging={dragging} data-off={!enabled} data-pool-id={entry.id}>
      {/* 整行点击展开；把手、开关、按钮各管各的。 */}
      <div
        className="gw-pool-row"
        onClick={(event) => {
          if ((event.target as HTMLElement).closest("button, a, input, .gw-grip")) return;
          onToggleExpand();
        }}
      >
        <span
          aria-label={t("拖动排序")}
          className="gw-grip"
          onPointerDown={onGripDown}
          title={t("拖动排序：同一个模型有多家时按这个顺序尝试")}
        >
          <GripVertical className="h-4 w-4" />
        </span>
        <Avatar baseUrl={entry.baseUrl} name={entry.name} size="lg" />
        <div className="gw-pool-main">
          <strong>{maskAccountName(entry.name)}</strong>
          <small>{sub}</small>
        </div>
        <span className="gw-pool-status">
          {newModels ? (
            <button className="gw-new-models" onClick={() => setModelsOpen(true)} title={t("后台检查发现上游有新模型，点开查看")} type="button">
              {tf("+{0} 新模型", [newModels])}
            </button>
          ) : null}
          <i className="gw-dot" data-tone={tone} />
          {/* 订阅账号空闲时显示套餐（和剩余额度），其它供应商显示 Key 数。 */}
          {!enabled ? t("已关闭") : status.state === "idle" && entry.account && entry.plan ? entry.plan : <MemberStatusLabel status={status} />}
          {enabled && entry.quota ? <QuotaBadge quota={entry.quota} /> : null}
        </span>
        <Switch
          disabled={busy !== null}
          label={tf("{0} 参与调度", [maskAccountName(entry.name)])}
          on={enabled}
          onToggle={() => void run(`toggle:${entry.id}`, "set_gateway_provider_enabled", { id: entry.id, enabled: !enabled }, t("切换供应商"), true)}
        />
        <button aria-expanded={expanded} aria-label={t("详情")} className="gw-chevron" data-open={expanded} onClick={onToggleExpand} type="button">
          <ChevronRight className="h-4 w-4" />
        </button>
      </div>
      {expanded ? (
        <div className="gw-agent-detail gw-pool-detail">
          <div className="gw-detail-row">
            <span className="gw-detail-label">{t("模型")}</span>
            <div className="gw-detail-value gw-chip-row">
              {entry.models.map((model) => <code className="gw-chip" key={model}>{`${entry.id}/${model}`}</code>)}
              {entry.models.length === 0 ? <span className="gw-muted">{t("未配置模型")}</span> : null}
              <button className="gw-text-button" disabled={busy !== null} onClick={() => setModelsOpen(true)} type="button">{t("编辑")}</button>
            </div>
          </div>
          <div className="gw-detail-row">
            <span className="gw-detail-label">{entry.account ? t("套餐") : "Key"}</span>
            <div className="gw-detail-value gw-chip-row">
              {entry.account ? <span>{entry.plan || t("未知")}</span> : <KeyCount count={entry.keyCount} />}
              {failures.map((key) => (
                <span className="gw-key-failure" key={key.keyId}>
                  {`${key.keyId}${t("：")}${[t(failureLabel(key.lastFailure)) || t(key.state === "disabled" ? "已停用" : "冷却中"), formatRemaining(key.remainingMs)].filter(Boolean).join(" · ")}`}
                </span>
              ))}
              {failures.length || breaker ? (
                <button className="gw-text-button" disabled={busy !== null} onClick={() => void run(`reset:${entry.id}`, "reset_gateway_key_health", { relayId: entry.id }, t("重置 Key 状态"), true)} type="button">
                  {t("重置")}
                </button>
              ) : null}
            </div>
          </div>
          {quotaSupported ? (
            <div className="gw-detail-row">
              <span className="gw-detail-label">{t("额度")}</span>
              <div className="gw-detail-value gw-chip-row">
                {entry.quota ? <QuotaWindows quota={entry.quota} /> : <span className="gw-muted">{t("还没查到，每 10 分钟查一次，用过一次也会更新")}</span>}
                <button
                  className="gw-text-button gw-sync-button"
                  disabled={busy !== null}
                  onClick={() => void run(`quota:${entry.id}`, "refresh_gateway_quota", { id: entry.id }, t("刷新额度"), true)}
                  type="button"
                >
                  <RefreshCw className={busy === `quota:${entry.id}` ? "h-3.5 w-3.5 animate-spin" : "h-3.5 w-3.5"} />
                  {t("刷新")}
                </button>
              </div>
            </div>
          ) : null}
          <div className="gw-detail-row">
            <span className="gw-detail-label">{t("价格")}</span>
            <div className="gw-detail-value gw-chip-row">
              <span className={price ? undefined : "gw-muted"}>{price || t("未填写，不估算费用")}</span>
              <button className="gw-text-button" disabled={busy !== null} onClick={() => setExtrasOpen("price")} type="button">{t("编辑")}</button>
              {/* 一键按公开价格表更新所有模型的价格，不用打开弹窗。 */}
              <button
                className="gw-text-button gw-sync-button"
                disabled={busy !== null || entry.models.length === 0}
                onClick={() => void run(`prices:${entry.id}`, "sync_gateway_provider_prices", { id: entry.id }, t("同步价格"))}
                title={t("按公开价格表（models.dev）更新所有模型的价格")}
                type="button"
              >
                <RefreshCw className={busy === `prices:${entry.id}` ? "h-3.5 w-3.5 animate-spin" : "h-3.5 w-3.5"} />
                {t("同步")}
              </button>
            </div>
          </div>
          <div className="gw-detail-row">
            <span className="gw-detail-label">{t("上下文窗口")}</span>
            <div className="gw-detail-value gw-chip-row">
              <span className={windows ? undefined : "gw-muted"}>{windows || t("未填写，用模型自带资料")}</span>
              <button className="gw-text-button" disabled={busy !== null || entry.models.length === 0} onClick={() => setWindowsOpen(true)} type="button">{t("编辑")}</button>
            </div>
          </div>
          <div className="gw-detail-row">
            <span className="gw-detail-label">{t("思考等级")}</span>
            <div className="gw-detail-value gw-chip-row">
              <span className={reasoning ? undefined : "gw-muted"}>{reasoning || t("按内置资料")}</span>
              <button className="gw-text-button" disabled={busy !== null || entry.models.length === 0} onClick={() => setReasoningOpen(true)} type="button">{t("编辑")}</button>
            </div>
          </div>
          <div className="gw-detail-row">
            <span className="gw-detail-label">{t("代理")}</span>
            <div className="gw-detail-value gw-chip-row">
              {entry.proxy ? <code className="gw-chip">{entry.proxy}</code> : <span className="gw-muted">{t("沿用系统代理")}</span>}
              <button className="gw-text-button" disabled={busy !== null} onClick={() => setExtrasOpen("proxy")} type="button">{t("编辑")}</button>
            </div>
          </div>
          <div className="gw-detail-row gw-detail-foot">
            <span className="gw-detail-label" />
            <div className="gw-detail-value">
              {entry.account ? (
                <button className="gw-text-button" disabled={busy !== null || loggingIn} onClick={onRelogin} type="button">
                  {loggingIn ? t("登录中…") : t("重新登录")}
                  <ChevronRight className="h-3.5 w-3.5" />
                </button>
              ) : (
                <button className="gw-text-button" onClick={onEdit} type="button">
                  {t("编辑")}
                  <ChevronRight className="h-3.5 w-3.5" />
                </button>
              )}
              <button className="gw-danger-link" disabled={busy !== null} onClick={() => void remove()} type="button">
                {t("删除")}
              </button>
            </div>
          </div>
        </div>
      ) : null}
      {modelsOpen ? (
        <ProviderModelsDialog busy={busy !== null} entry={entry} onCancel={() => setModelsOpen(false)} onSave={(models, defaultModel) => void saveModels(models, defaultModel)} />
      ) : null}
      {reasoningOpen ? (
        <ProviderReasoningDialog busy={busy !== null} entry={entry} onCancel={() => setReasoningOpen(false)} onSave={(modelMetadata) => void saveReasoning(modelMetadata)} />
      ) : null}
      {windowsOpen ? (
        <ProviderWindowsDialog busy={busy !== null} entry={entry} onCancel={() => setWindowsOpen(false)} onSave={(modelWindows, fallback) => void saveWindows(modelWindows, fallback)} />
      ) : null}
      {extrasOpen ? (
        <ProviderExtrasDialog
          section={extrasOpen}
          busy={busy !== null}
          entry={entry}
          onCancel={() => setExtrasOpen(null)}
          onSave={(nextPrice, modelPrices, proxy) => void saveExtras(nextPrice, modelPrices, proxy)}
        />
      ) : null}
    </div>
  );
}

/** 添加时给哪些 Agent 用：一排可切换的 Agent，默认按各自的「添加供应商时自动加入」。 */
function AgentPicks({ payload, picked, onChange }: { payload: GatewayPayload | null; picked: GatewayAgentId[]; onChange: (next: GatewayAgentId[]) => void }) {
  const agents = payload?.agents ?? [];
  if (!agents.length) return null;
  return (
    <div className="gw-agent-picks">
      <span>{t("添加后给这些 Agent 用")}</span>
      {agents.map((agent) => {
        const on = picked.includes(agent.agent);
        return (
          <button
            aria-pressed={on}
            data-on={on}
            key={agent.agent}
            onClick={() => onChange(on ? picked.filter((item) => item !== agent.agent) : [...picked, agent.agent])}
            type="button"
          >
            <AgentIcon agent={agent.agent} className="gw-agent-pick-icon" />
            {agent.name}
            {on ? <Check className="h-3.5 w-3.5" /> : null}
          </button>
        );
      })}
      {picked.length === 0 ? <em>{t("都不选也能添加，之后在 Agent 的「挑选」里再打开")}</em> : null}
    </div>
  );
}

type CatalogItem = { key: string; name: string; added?: boolean; icon?: ReactNode; onClick: () => void };

/** 「添加供应商」目录：订阅、供应商（厂商 API）、中转、本机；「导入…」从直连模式复制。 */
function ProviderCatalog({
  payload,
  busy,
  run,
  notify,
  onClose,
  onPreset,
  onCustom,
  onLogin,
  agentPicks,
  skipAgents,
}: {
  payload: GatewayPayload | null;
  busy: string | null;
  run: Run;
  notify: Notify;
  onClose: () => void;
  onPreset: (preset: GatewayPreset) => void;
  onCustom: () => void;
  onLogin: (kind: AccountKind) => void;
  agentPicks: ReactNode;
  skipAgents: GatewayAgentId[];
}) {
  const [query, setQuery] = useState("");
  const [importing, setImporting] = useState(false);
  const presets = useMemo(() => gatewayPresets(), []);
  const needle = query.trim().toLowerCase();
  const hit = (...values: string[]) => !needle || values.some((value) => value.toLowerCase().includes(needle));
  const presetItems = (section: PresetSection): CatalogItem[] => presets
    .filter((preset) => preset.section === section && hit(preset.name, preset.baseUrl))
    .map((preset) => ({ key: preset.key, name: preset.name, added: presetAdded(payload, preset), onClick: () => onPreset(preset) }));
  const loggedIn = (kind: AccountKind) => Boolean(payload?.pool.some((entry) => entry.account === kind));
  const subscriptions: CatalogItem[] = (
    [
      { kind: "chatgpt", name: "ChatGPT", icon: "openai" },
      { kind: "chatgpt-api", name: "ChatGPT API", icon: "openai" },
      { kind: "copilot", name: "GitHub Copilot", icon: "githubcopilot" },
      { kind: "grok", name: "SuperGrok", icon: "grok" },
    ] as Array<{ kind: AccountKind; name: string; icon: string }>
  )
    .filter((item) => hit(item.name, t("订阅")))
    .map((item) => ({
      key: `login:${item.kind}`,
      name: item.name,
      added: loggedIn(item.kind),
      onClick: () => onLogin(item.kind),
    }));
  const local: CatalogItem[] = [
    ...presetItems("local"),
    ...(hit(t("自定义")) ? [{ key: "custom", name: t("自定义"), icon: <span className="gw-avatar gw-avatar-plus" data-size="sm"><Plus className="h-3.5 w-3.5" /></span>, onClick: onCustom }] : []),
  ];
  const sections: Array<{ key: string; title: string; hint: string; items: CatalogItem[] }> = [
    { key: "sponsor", title: t("赞助商"), hint: t("支持 Codex++ 的服务商"), items: presetItems("sponsor") },
    { key: "subscription", title: t("订阅"), hint: t("登录即可，无需密钥；与 Codex 自己的登录互不影响"), items: subscriptions },
    { key: "vendor", title: t("供应商"), hint: t("模型厂商的 API"), items: presetItems("vendor") },
    { key: "relay", title: t("中转"), hint: t("一个密钥用多家模型"), items: presetItems("relay") },
    { key: "local", title: t("本机"), hint: t("本机运行的模型服务，或任意兼容地址"), items: local },
  ].filter((section) => section.items.length);

  // Esc 栈：从这里打开的「导入…」在上面一层，Esc 先关导入，不会把整个添加流程关掉。
  useEscapeLayer(true, onClose);

  return createPortal(
    <div className="gw-modal-backdrop gw-portal" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
      <div className="gw-modal gw-catalog-modal" role="dialog" aria-label={t("添加供应商")}>
        <header>
          <strong>{t("添加供应商")}</strong>
          <label className="gw-sheet-search">
            <Search className="h-4 w-4" />
            <input autoFocus onChange={(event) => setQuery(event.currentTarget.value)} placeholder={t("查找供应商…")} value={query} />
          </label>
          <button className="gw-text-button" onClick={() => setImporting(true)} type="button">{t("导入…")}</button>
          <button aria-label={t("关闭")} className="gw-icon-button" onClick={onClose} type="button"><X className="h-4 w-4" /></button>
        </header>
        <div className="gw-modal-body gw-catalog">
          {sections.map((section) => (
            <section className="gw-catalog-section" key={section.key}>
              <h4>{section.title} <span>{section.hint}</span></h4>
              <div className="gw-catalog-grid">
                {section.items.map((item) => (
                  <button className="gw-catalog-item" disabled={busy !== null} key={item.key} onClick={item.onClick} type="button">
                    {item.icon ?? <Avatar name={item.name} size="sm" />}
                    <span>{item.name}</span>
                    {item.added ? <i className="gw-dot" data-tone="added" title={t("已在供应商池里")} /> : null}
                  </button>
                ))}
              </div>
            </section>
          ))}
          {sections.length === 0 ? <div className="gw-empty">{t("没有匹配的供应商")}</div> : null}
        </div>
        <footer className="gw-catalog-foot">{agentPicks}</footer>
      </div>
      {importing ? (
        <ImportDialog
          agentPicks={agentPicks}
          busy={busy}
          onClose={() => setImporting(false)}
          onLogin={() => { setImporting(false); onLogin("chatgpt"); }}
          payload={payload}
          run={run}
          skipAgents={skipAgents}
        />
      ) : null}
    </div>,
    document.body,
  );
}

/**
 * 导入：把直连模式里 Codex、Claude Code、Grok 的供应商复制一份进网关（之后两边互不影响），或用 OpenCode 里保存的 Key。
 * 多选，默认勾上还没导入的，一次导完；聚合供应商搬成「成员进池 + 同名模型组」；官方登录账号不能复制，直接去登录。
 */
function ImportDialog({
  payload,
  busy,
  run,
  onClose,
  onLogin,
  skipAgents,
  agentPicks,
}: {
  payload: GatewayPayload | null;
  busy: string | null;
  run: Run;
  onClose: () => void;
  onLogin: () => void;
  skipAgents: GatewayAgentId[];
  agentPicks?: ReactNode;
}) {
  useEscapeLayer(true, onClose);
  const candidates = importCandidates(payload);
  const keyOf = (item: ImportCandidate) => `${item.source}:${item.id}`;
  const [picked, setPicked] = useState<Set<string>>(() => new Set(candidates.filter((item) => !item.imported).map(keyOf)));
  const toggle = (key: string) => {
    const next = new Set(picked);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    setPicked(next);
  };
  const chosen = candidates.filter((item) => picked.has(keyOf(item)));
  const submit = async () => {
    const items = chosen.map((item) => ({ source: item.source, id: item.id }));
    if (await run("import:batch", "import_gateway_providers", { items, skipAgents }, t("导入供应商"))) onClose();
  };
  const codexEntry = (id: string) => payload?.codexProviders.find((entry) => entry.id === id);
  const aggregate = (id: string) => payload?.codexAggregates?.find((item) => item.id === id);
  const subOf = (item: ImportCandidate) => {
    switch (item.source) {
      case "codex": {
        const entry = codexEntry(item.id);
        return entry ? `${protocolLabel(entry.protocol)} · ${presetHost(entry.baseUrl)}` : "";
      }
      case "aggregate": {
        const info = aggregate(item.id);
        return info ? tf("{0} 个成员 · {1} · {2}，成员一起导入", [info.members, t(groupModeLabel(info.mode)), t(groupStickyLabel(info.sticky))]) : "";
      }
      case "claude":
        return t("Anthropic 协议，原样转发");
      case "grok":
        return t("Grok 页里配置的供应商");
      default:
        return t("opencode auth login 保存的 Key，按模型自动选协议");
    }
  };
  const sections: Array<{ key: ImportSource; title: string }> = [
    { key: "codex", title: t("Codex 的供应商") },
    { key: "aggregate", title: t("模型组（来自 Codex 的聚合供应商）") },
    { key: "claude", title: t("Claude Code 的供应商") },
    { key: "grok", title: t("Grok 的供应商") },
    { key: "opencode", title: t("OpenCode 里保存的 Key") },
  ];
  const official = payload?.codexOfficial ?? [];
  const chatgptLoggedIn = Boolean(payload?.pool.some((entry) => entry.account === "chatgpt"));
  const pending = candidates.filter((item) => !item.imported);
  const allPicked = pending.length > 0 && pending.every((item) => picked.has(keyOf(item)));
  return createPortal(
    <div className="gw-modal-backdrop gw-portal" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
      <div className="gw-modal gw-import-modal">
        <header>
          <strong>{t("从直连模式导入")}</strong>
          {pending.length ? (
            <button className="gw-text-button" onClick={() => setPicked(allPicked ? new Set() : new Set(pending.map(keyOf)))} type="button">
              {allPicked ? t("全不选") : t("全选未导入的")}
            </button>
          ) : null}
          <button aria-label={t("关闭")} className="gw-icon-button" onClick={onClose} type="button"><X className="h-4 w-4" /></button>
        </header>
        <div className="gw-modal-body">
          <p className="gw-muted gw-import-note">{t("复制一份进网关，之后两边各改各的，互不影响。")}</p>
          {sections.map((section) => {
            const items = candidates.filter((item) => item.source === section.key);
            if (!items.length) return null;
            return (
              <section key={section.key}>
                <h4 className="gw-import-title">{section.title}</h4>
                {items.map((item) => {
                  const key = keyOf(item);
                  const on = picked.has(key);
                  return (
                    <button aria-pressed={on} className="gw-import-item" data-on={on} disabled={busy !== null} key={key} onClick={() => toggle(key)} type="button">
                      <span className="gw-import-check">{on ? <Check className="h-3.5 w-3.5" /> : null}</span>
                      <Avatar name={item.name} size="sm" />
                      <span>
                        <strong>{maskAccountName(item.name)}</strong>
                        <small>{subOf(item)}</small>
                      </span>
                      {item.imported ? <em className="gw-import-badge">{t("已在池里")}</em> : null}
                    </button>
                  );
                })}
              </section>
            );
          })}
          {official.length ? (
            <section>
              <h4 className="gw-import-title">{t("官方登录账号")}</h4>
              {official.map((item) => (
                <div className="gw-import-item gw-import-login" key={item.id}>
                  <span className="gw-import-check" />
                  <Avatar name={item.name} size="sm" />
                  <span>
                    <strong>{maskAccountName(item.name)}</strong>
                    <small>{t("不能复制：两边共用登录会互相把对方刷失效，需要在网关里单独登录一次")}</small>
                  </span>
                  {chatgptLoggedIn ? (
                    <em className="gw-import-badge">{t("网关已登录 ChatGPT")}</em>
                  ) : (
                    <button className="gw-text-button strong" disabled={busy !== null} onClick={onLogin} type="button">{t("去登录")}</button>
                  )}
                </div>
              ))}
            </section>
          ) : null}
          {candidates.length === 0 && official.length === 0 ? <p className="gw-muted gw-import-empty">{t("直连模式里没有可导入的供应商")}</p> : null}
        </div>
        <footer className="gw-import-foot">
          {agentPicks}
          <button className="gw-primary-button" disabled={busy !== null || chosen.length === 0} onClick={() => void submit()} type="button">
            {busy === "import:batch" ? t("导入中…") : tf("导入 {0} 个", [chosen.length])}
          </button>
        </footer>
      </div>
    </div>,
    document.body,
  );
}

/** Copilot 设备码登录：显示验证码，等你在 GitHub 页面里输入并授权。关掉弹窗（含 Esc）就是取消登录。 */
function DeviceCodeDialog({ code, notify, onCancel }: { code: DeviceCode; notify: Notify; onCancel: () => void }) {
  useEscapeLayer(true, onCancel);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(code.userCode);
      await notify(t("已复制"), code.userCode, "ok");
    } catch (error) {
      await notify(t("复制失败"), String(error), "failed");
    }
  };
  return createPortal(
    <div className="gw-modal-backdrop gw-portal">
      <div className="gw-modal gw-device-modal">
        <header>
          <strong>{t("登录 GitHub Copilot")}</strong>
          <button aria-label={t("取消登录")} className="gw-icon-button" onClick={onCancel} title={t("取消登录")} type="button"><X className="h-4 w-4" /></button>
        </header>
        <div className="gw-modal-body">
          <p className="gw-muted">{tf("已在浏览器打开 {0}，输入下面的验证码并授权。授权完成后这里会自动继续。", [code.verificationUri])}</p>
          <button className="gw-device-code" onClick={() => void copy()} title={t("复制验证码")} type="button">
            <span>{code.userCode}</span>
            <Copy className="h-4 w-4" />
          </button>
          <p className="gw-muted">{tf("验证码 {0} 分钟内有效。需要这个 GitHub 账号有 Copilot 订阅。", [Math.round(code.expiresIn / 60)])}</p>
        </div>
        <footer>
          <span className="gw-error" />
          <button className="gw-soft-button" onClick={onCancel} type="button">{t("取消登录")}</button>
        </footer>
      </div>
    </div>,
    document.body,
  );
}

/** 浏览器登录进行中的提示条：不挡操作（固定在窗口底部），可随时取消。 */
function LoginPendingBar({ label, onCancel }: { label: string; onCancel: () => void }) {
  return createPortal(
    <div aria-live="polite" className="gw-portal gw-login-pending" role="status">
      <RefreshCw aria-hidden="true" className="h-3.5 w-3.5 animate-spin" />
      <span>{tf("正在等待 {0} 登录，在浏览器里完成后会自动继续", [label])}</span>
      <button className="gw-text-button strong" onClick={onCancel} type="button">{t("取消登录")}</button>
    </div>,
    document.body,
  );
}
