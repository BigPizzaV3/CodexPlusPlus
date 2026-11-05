/** 本地网关 · 供应商：网关自己的供应商池（可开关、排序、编辑、删除），右上角「添加供应商」打开目录弹窗。
 *
 * 与直连模式完全独立：这里的增删改只动 gateway.json；「导入…」是把直连模式的供应商复制一份过来。
 */
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { closestCenter, DndContext, KeyboardSensor, PointerSensor, useSensor, useSensors, type DragEndEvent } from "@dnd-kit/core";
import { SortableContext, sortableKeyboardCoordinates, useSortable, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { CSS as DndCSS } from "@dnd-kit/utilities";
import { invoke } from "@tauri-apps/api/core";
import { createPortal } from "react-dom";
import { appConfirm } from "./app-dialog";
import { Check, ChevronRight, Copy, GripVertical, Pencil, Plus, RefreshCw, Search, X, Zap } from "lucide-react";
import { t, tf } from "./i18n";
import { providerPoolSections, reorderProviderSlots } from "./provider-pool-layout";
import { ProviderBalancePoolBadge, RelayBillingPoolBadge, RelayBillingPoolLabel, RelayBillingRetry, useProviderBalance, useRelayBillingProbe } from "./RelayBillingLine";
import { relayBillingGatewayCacheKey, type RelayBillingProbe } from "./relay-billing";
import {
  glanceAgents,
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
  subscriptionBlockReason,
  maskAccountText,
  type GatewayAgentId,
  type GatewayPayload,
  type GatewayPoolEntry,
  type GatewayPreset,
  type PresetSection,
  type ProviderDraft,
  type ProviderPrice,
  maskAccountName,
  isDirectProxy,
  keyLabel,
  type ImportCandidate,
  type ImportSource,
  type LocalLogin,
  type SubscriptionLoginMethod,
  type SubscriptionLoginStart,
  type SubscriptionView,
} from "./gateway-model";
import { AgentIcon, Avatar, KeyCount, MemberStatusLabel, QuotaWindows, Switch, type Notify, type Run } from "./gateway-ui";
import { ProviderFormDialog, type ProviderFormState } from "./ProviderFormDialog";
import { ProviderExtrasDialog, ProviderLimitsDialog, ProviderNewApiAccessDialog, keyLimitSummary, priceSummary } from "./ProviderExtrasDialog";
import { ProviderWindowsDialog, windowSummary, withUnlistedWindows } from "./ProviderWindowsDialog";
import { ProviderReasoningDialog, reasoningSummary } from "./ProviderReasoningDialog";
import { ProviderModelsDialog } from "./ProviderModelsDialog";
import { ProviderSyncDialog } from "./ProviderSyncDialog";
import { ProviderMetaSyncDialog } from "./ProviderMetaSyncDialog";
import type { MetaSyncKind } from "./provider-meta-sync-model";
import { ProviderKeyList } from "./ProviderKeyList";
import { ProviderTestDialog } from "./ProviderTestDialog";
import { gatewayJump } from "./gateway-store";
import { useEscapeLayer } from "./use-escape-layer";
import { singleAgent, visibleAgents } from "./agent-focus-model";
import { useAgentFocus } from "./agent-focus-store";

/** 能在网关里直接登录的订阅。 */
type AccountKind = "chatgpt" | "chatgpt-api" | "copilot" | "grok" | `sub:${string}`;
type DeviceCode = { deviceCode: string; userCode: string; verificationUri: string; interval: number; expiresIn: number; title?: string; hint?: string };

const ACCOUNT_LABELS: Record<string, string> = { chatgpt: "ChatGPT", "chatgpt-api": "ChatGPT API", copilot: "GitHub Copilot", grok: "SuperGrok" };

/** 私有接口订阅（`sub:<id>`）的信息；内置的四种为 null。 */
function subscriptionOf(payload: GatewayPayload | null, kind: AccountKind): SubscriptionView | null {
  if (!kind.startsWith("sub:")) return null;
  return payload?.subscriptions?.find((item) => item.id === kind.slice(4)) ?? null;
}

function accountLabel(payload: GatewayPayload | null, kind: AccountKind): string {
  return ACCOUNT_LABELS[kind] ?? subscriptionOf(payload, kind)?.name ?? kind;
}

const LOGIN_METHOD_LABELS: Record<SubscriptionLoginMethod, string> = {
  browser: "浏览器登录",
  deviceCode: "验证码登录",
  importLocal: "导入本机客户端的登录",
  paste: "粘贴凭据",
};

/** 浏览器登录的订阅各自的命令。 */
const BROWSER_LOGIN_COMMANDS: Record<string, string> = {
  chatgpt: "login_gateway_chatgpt",
  "chatgpt-api": "login_gateway_chatgpt_api",
  grok: "login_gateway_grok",
};

/** 表单弹窗的用途：按预设新建、自定义新建、编辑已有供应商。 */
type FormState = ProviderFormState;

/** 详情里模型超过 MODEL_CHIP_COLLAPSE 个时先只列前 MODEL_CHIP_PREVIEW 个，其余点开再看。 */
const MODEL_CHIP_PREVIEW = 6;
const MODEL_CHIP_COLLAPSE = 8;

/** 供应商池上次用的视图（切页再回来保持）。 */

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
  const [dragging, setDragging] = useState<string | null>(null);
  // 拖动用 dnd-kit：行跟手移动、松手时有位移动画；按住把手稍微一动（4px）才算拖，单击照样展开。
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );
  useEffect(() => {
    document.body.classList.toggle("gw-sorting", dragging !== null);
    return () => document.body.classList.remove("gw-sorting");
  }, [dragging]);
  // 编辑已有供应商的表单；新添加走 ProviderAdder。
  const [form, setForm] = useState<FormState | null>(null);
  const openCatalog = () => setAdding("catalog");
  // 拖动排序中的临时顺序；松手后提交，等新数据回来再清掉。
  const [order, setOrder] = useState<string[] | null>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const needle = query.trim().toLowerCase();
  const ordered = order ? order.map((id) => pool.find((entry) => entry.id === id)).filter((entry): entry is GatewayPoolEntry => Boolean(entry)) : pool;
  // 开启 / 关闭两组只是展示上的分组，不改池里的调度顺序（搜索也匹配地址和模型）。
  const { active: onList, inactive: offList } = providerPoolSections(ordered, query);
  const visible = ordered.filter((entry) => onList.includes(entry) || offList.includes(entry));
  // 关掉的供应商默认折叠在「已关闭」里；搜索时、或者一家都没开时照常全列出来。从别的页跳到某家时先展开，免得它正好藏在折叠里。
  const [offOpen, setOffOpen] = useState(Boolean(expanded));
  const collapseOff = onList.length > 0 && offList.length > 0 && !needle;
  const renderRow = (entry: GatewayPoolEntry) => (
    <PoolRow
      busy={busy}
      dragging={dragging === entry.id}
      entry={entry}
      expanded={expanded === entry.id}
      key={entry.id}
      onEdit={() => setForm({ mode: "edit", entry })}
      loggingIn={Boolean(entry.account) && loginPending === entry.account}
      onRelogin={() => void login(entry.account as AccountKind)}
      onToggleExpand={() => setExpanded(expanded === entry.id ? null : entry.id)}
      payload={payload}
      run={run}
    />
  );

  /** 组内排序回填到原来的槽位：开启 / 关闭两组交错时，另一组的位置不变，折叠着也不会被挤到队尾。 */
  const finishDrag = (event: DragEndEvent, group: GatewayPoolEntry[]) => {
    setDragging(null);
    const ids = group.map((entry) => entry.id);
    const from = ids.indexOf(String(event.active.id));
    const to = ids.indexOf(String(event.over?.id ?? ""));
    if (from < 0 || to < 0 || from === to) return;
    const initial = pool.map((entry) => entry.id);
    const groupOrder = ids.slice();
    groupOrder.splice(from, 1);
    groupOrder.splice(to, 0, ids[from]);
    const next = reorderProviderSlots(initial, groupOrder);
    if (next.join("\n") === initial.join("\n")) return;
    setOrder(next);
    void run("reorder", "reorder_gateway_providers", { ids: next }, t("调整顺序"), true).finally(() => setOrder(null));
  };
  /** 一组可拖动排序的行：两组各自一个拖动区，只在组内换位置。 */
  const sortableList = (group: GatewayPoolEntry[], className: string, extra?: ReactNode) => (
    <DndContext collisionDetection={closestCenter} onDragCancel={() => setDragging(null)} onDragEnd={(event) => finishDrag(event, group)} onDragStart={(event) => { setExpanded(null); setDragging(String(event.active.id)); }} sensors={sensors}>
      <SortableContext items={group.map((entry) => entry.id)} strategy={verticalListSortingStrategy}>
        <div className={className} data-sorting={dragging !== null || order !== null}>
          {group.map(renderRow)}
          {extra}
        </div>
      </SortableContext>
    </DndContext>
  );

  const { login, importLocal, pending: loginPending, dialog: loginDialog } = useAccountLogin(payload, run, notify);
  const localLogins = useLocalLogins(payload?.gateway.localLoginDiscovery === true, pool.map((entry) => entry.id).join("\n"));
  // 本次打开页面里点过关闭就不再显示（下次进来、或池子变了扫出新的还会提示）。
  const [localDismissed, setLocalDismissed] = useState("");
  const localKey = localLogins.map((item) => `${item.id}:${item.account}`).join("\n");
  // 编辑弹窗里打开的上下文窗口弹窗：按编辑中的模型列表列出（null 表示没打开）。
  const [formWindows, setFormWindows] = useState<string[] | null>(null);
  // 编辑中的供应商取最新数据（窗口刚保存过，弹窗里要显示新的）。
  const editingEntry = form?.mode === "edit" ? pool.find((entry) => entry.id === form.entry.id) ?? form.entry : null;
  const closeForm = () => {
    setForm(null);
    setFormWindows(null);
  };
  const saveEdit = async (draft: ProviderDraft) => {
    if (form?.mode !== "edit") return;
    if (await run(`edit:${form.entry.id}`, "update_gateway_provider", { id: form.entry.id, input: providerInput(draft) }, t("保存供应商"))) closeForm();
  };
  const saveFormWindows = async (modelWindows: Record<string, number>, fallback: number | null) => {
    if (!editingEntry || !formWindows) return;
    const windows = withUnlistedWindows(editingEntry, formWindows, modelWindows);
    if (await run(`windows:${editingEntry.id}`, "save_gateway_provider_windows", { id: editingEntry.id, windows, fallback }, t("保存上下文窗口"))) setFormWindows(null);
  };

  return (
    <>
      <div className="gw-provider-search-row">
        {/* 只有按供应商的列表：每个模型由哪几家提供，在「路由」的自动组里看。 */}
        <label className="gw-sheet-search">
          <Search className="h-4 w-4" />
          <input onChange={(event) => setQuery(event.currentTarget.value)} placeholder={t("查找供应商")} value={query} />
        </label>
        {pool.length ? (
          <button className="gw-primary-button" onClick={openCatalog} type="button">
            <Plus className="h-4 w-4" />
            {t("添加供应商")}
          </button>
        ) : null}
      </div>
      {pool.length ? (
        // 开启的一组在上面；关掉的收进「已关闭」，两组各自拖动排序。
        <div className="gw-pool-lists" ref={listRef}>
          {sortableList(collapseOff ? onList : visible, "gw-card gw-pool-card", visible.length === 0 ? <div className="gw-empty">{t("没有匹配的供应商")}</div> : null)}
          {collapseOff ? (
            <>
              <button aria-expanded={offOpen} className="gw-pool-off-toggle" onClick={() => setOffOpen(!offOpen)} type="button">
                <ChevronRight className="h-3.5 w-3.5" />
                {t("已关闭")}
                <span className="cx-section-count">{offList.length}</span>
              </button>
              {offOpen ? sortableList(offList, "gw-card gw-pool-card gw-pool-off-card") : null}
            </>
          ) : null}
        </div>
      ) : null}
      {localLogins.length && localDismissed !== localKey ? (
        <LocalLoginsCard busy={busy !== null} found={localLogins} onDismiss={() => setLocalDismissed(localKey)} onImport={importLocal} pending={loginPending} />
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
      {form ? (
        <ProviderFormDialog
          busy={busy}
          form={form}
          onCancel={closeForm}
          onEditWindows={(models) => setFormWindows(models)}
          onSubmit={(draft) => void saveEdit(draft)}
          windowsEntry={editingEntry ?? undefined}
        />
      ) : null}
      {/* 编辑弹窗里的「上下文窗口」：复用详情里的窗口弹窗，叠在编辑弹窗上，存完回到编辑弹窗。 */}
      {editingEntry && formWindows ? (
        <ProviderWindowsDialog
          busy={busy !== null}
          entry={{ ...editingEntry, models: formWindows }}
          onCancel={() => setFormWindows(null)}
          onSave={(modelWindows, fallback) => void saveFormWindows(modelWindows, fallback)}
        />
      ) : null}
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
function useAccountLogin(payload: GatewayPayload | null, run: Run, notify: Notify) {
  const [pending, setPending] = useState<AccountKind | null>(null);
  const [deviceCode, setDeviceCode] = useState<DeviceCode | null>(null);
  // 私有接口订阅：先选登录方式（或粘贴凭据），确认后才开始。
  const [choosing, setChoosing] = useState<{ item: SubscriptionView; skip?: GatewayAgentId[] } | null>(null);
  // 当前这次登录的序号：取消或开始新的登录后，旧登录的收尾不再改界面。
  const attempt = useRef(0);
  // 私有接口订阅的 `login()` 要等选登录方式 → 登录这一整段结束才返回：调用方（添加供应商）
  // 在返回后就关掉整个界面，提前返回会把选择弹窗一起销毁，登录页根本拉不起来（#2879）。
  const chooserDone = useRef<(() => void) | null>(null);
  const settleChooser = () => {
    const done = chooserDone.current;
    chooserDone.current = null;
    done?.();
  };
  const cancel = () => {
    attempt.current += 1;
    setPending(null);
    setDeviceCode(null);
    setChoosing(null);
    settleChooser();
    void invoke("cancel_gateway_login").catch(() => undefined);
  };
  /** 私有接口订阅的登录：开始 → （验证码弹窗 / 浏览器）→ 等完成后加进池里。 */
  const loginSubscription = async (item: SubscriptionView, method: SubscriptionLoginMethod, input: string, skip?: GatewayAgentId[]) => {
    const kind: AccountKind = `sub:${item.id}`;
    const id = ++attempt.current;
    const current = () => attempt.current === id;
    const title = tf("登录 {0}", [item.name]);
    // 选择弹窗这一步结束了，登录本身结束时再通知 `login()` 的调用方。
    const done = chooserDone.current;
    chooserDone.current = null;
    setChoosing(null);
    setPending(kind);
    setDeviceCode(null);
    try {
      let start: SubscriptionLoginStart;
      try {
        const result = await invoke<{ status: string; message: string; start?: SubscriptionLoginStart | null }>("start_gateway_subscription_login", { kind: item.id, method, input });
        if (result.status !== "ok") {
          await notify(tf("{0}失败", [title]), result.message, "failed");
          return;
        }
        if (!result.start) {
          await notify(tf("{0}失败", [title]), t("登录响应缺少必要状态，请重试。"), "failed");
          return;
        }
        start = result.start;
      } catch (error) {
        await notify(t("调用失败"), String(error), "failed");
        return;
      }
      if (!current()) return;
      if (start.userCode) {
        setDeviceCode({ deviceCode: "", userCode: start.userCode, verificationUri: start.openUrl ?? "", interval: 5, expiresIn: 300, title, hint: start.message });
      } else if (start.openUrl) {
        void notify(title, start.message || t("已在浏览器打开登录页，登录完成后回到这里。5 分钟内有效。"), "info");
      }
      await run(`login:${kind}`, "finish_gateway_subscription_login", { kind: item.id, start, skipAgents: skip }, title, false, { background: true });
    } finally {
      if (current()) {
        setPending(null);
        setDeviceCode(null);
      }
      done?.();
    }
  };
  /** 登录订阅账号：ChatGPT / ChatGPT API / SuperGrok 走浏览器登录；Copilot 先显示验证码，再等你在 GitHub 上授权。
   * `skip` 是新添加时取消勾选的 Agent；重新登录已有账号时不传。 */
  const login = async (kind: AccountKind, skip?: GatewayAgentId[]) => {
    const subscription = subscriptionOf(payload, kind);
    if (kind.startsWith("sub:")) {
      if (!subscription) {
        await notify(t("登录失败"), t("当前版本不支持这个订阅"), "failed");
        return;
      }
      settleChooser();
      return new Promise<void>((resolve) => {
        chooserDone.current = resolve;
        setChoosing({ item: subscription, skip });
      });
    }
    const id = ++attempt.current;
    const current = () => attempt.current === id;
    const title = tf("登录 {0}", [accountLabel(payload, kind)]);
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

  const dialog = choosing ? (
    <SubscriptionLoginDialog
      item={choosing.item}
      onCancel={() => {
        setChoosing(null);
        settleChooser();
      }}
      onStart={(method, input) => void loginSubscription(choosing.item, method, input, choosing.skip)}
    />
  ) : deviceCode ? (
    <DeviceCodeDialog code={deviceCode} notify={notify} onCancel={cancel} />
  ) : pending ? (
    <LoginPendingBar label={accountLabel(payload, pending)} onCancel={cancel} />
  ) : null;
  /** 直接导入本机已登录的订阅（供应商页的「发现本机登录」提示用），不再弹登录方式选择。 */
  const importLocal = (id: string) => {
    const item = payload?.subscriptions?.find((entry) => entry.id === id);
    if (item) void loginSubscription(item, "importLocal", "");
  };
  return { login, importLocal, pending, dialog };
}

/** 本机已登录、还没进池的订阅账号：开关开着时扫描一次，池子变了再扫。 */
function useLocalLogins(enabled: boolean, poolKey: string) {
  const [found, setFound] = useState<LocalLogin[]>([]);
  useEffect(() => {
    if (!enabled) {
      setFound([]);
      return;
    }
    let alive = true;
    invoke<{ status: string; logins?: LocalLogin[] }>("scan_gateway_local_logins")
      .then((result) => { if (alive) setFound(result.status === "ok" ? result.logins ?? [] : []); })
      .catch(() => { if (alive) setFound([]); });
    return () => { alive = false; };
  }, [enabled, poolKey]);
  return found;
}

/**
 * 发现本机已登录的订阅账号：列表下面一行小字（「发现 2 个本机已登录的订阅账号：Cursor、Kiro · 导入」），
 * 点开才列出账号逐个导入；不再整块卡片压在供应商列表上面。× 关掉这次的提示，长期不想看到在「网关」页关掉。
 */
function LocalLoginsCard({ found, busy, pending, onImport, onDismiss }: { found: LocalLogin[]; busy: boolean; pending: string | null; onImport: (id: string) => void; onDismiss: () => void }) {
  const [open, setOpen] = useState(false);
  const names = found.map((item) => maskAccountName(item.name)).join("、");
  return (
    <div className="gw-local-logins">
      <div className="gw-local-logins-line">
        <button aria-expanded={open} className="gw-pool-off-toggle gw-local-logins-toggle" onClick={() => setOpen(!open)} title={t("来自这些官方客户端的登录文件，导入时复制一份登录凭据进网关。不想看到这条提示，可以在「网关」页关掉「发现本机已登录的订阅账号」。")} type="button">
          <ChevronRight className="h-3.5 w-3.5" />
          {tf("发现 {0} 个本机已登录的订阅账号：{1}", [found.length, names])}
          <span className="gw-local-logins-action">{t("导入")}</span>
        </button>
        <button aria-label={t("关闭")} className="gw-icon-button gw-local-logins-dismiss" onClick={onDismiss} title={t("这次不再提示")} type="button"><X className="h-3.5 w-3.5" /></button>
      </div>
      {open ? (
        <div className="gw-card gw-local-logins-card">
          {found.map((item) => (
            <div className="gw-setting-row" key={`${item.id}:${item.account}`}>
              <div>
                <strong>{maskAccountName(item.name)}</strong>
                <small>{item.account ? maskAccountName(item.account) : t("本机官方客户端已登录")}</small>
              </div>
              <span />
              <button className="gw-text-button strong" disabled={busy || pending !== null} onClick={() => onImport(item.id)} type="button">
                {pending === `sub:${item.id}` ? t("导入中…") : t("导入")}
              </button>
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
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
  const { login, dialog } = useAccountLogin(payload, run, notify);
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
  onEdit: () => void;
  onRelogin: () => void;
  /** 这个账号类型正在登录（浏览器 / 设备码），按钮显示进行中。 */
  loggingIn: boolean;
}) {
  const enabled = providerEnabled(entry);
  // 拖动中跟手移动，松手时 200ms 滑到新位置；拖着的那一行本身不加过渡，免得跟不上指针。
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id: entry.id });
  const status = memberStatus(payload, entry.id);
  const failures = (payload?.health.keys ?? []).filter((key) => key.relayId === entry.id && key.state !== "ok");
  const breaker = payload?.health.providers.some((item) => item.relayId === entry.id && item.breakerOpen);
  const tone = !enabled ? "off" : status.state === "cooling" ? "error" : "on";
  const billing = useRelayBillingProbe(
    relayBillingGatewayCacheKey(entry),
    () => invoke<Partial<RelayBillingProbe> & { status: string }>("probe_gateway_provider_billing", { id: entry.id }),
  );
  // DeepSeek、Kimi 等有余额接口的厂商：卡片余额和「用量」页、托盘面板同一来源（#2793）。
  const vendorBalance = useProviderBalance(entry.id, enabled && Boolean(entry.balanceApi));
  // new-api 的访问令牌（#2793）：识别成 new-api 或已经配过时，详情里才有这一行。
  const [accessOpen, setAccessOpen] = useState(false);
  const showAccess = !entry.account && (billing.probe?.platform === "newapi" || Boolean(entry.newapiAccess));
  const saveAccess = async (token: string | null, userId: string, clear: boolean) => {
    const title = clear ? t("清除访问令牌") : t("保存访问令牌");
    if (await run(`newapi-access:${entry.id}`, "save_gateway_provider_newapi_access", { id: entry.id, token, userId, clear }, title)) setAccessOpen(false);
  };
  const sub = (entry.account
    ? [t("订阅登录"), tf("{0} 个模型", [entry.models.length])]
    : [protocolLabel(entry.protocol), presetHost(entry.baseUrl), tf("{0} 个模型", [entry.models.length])]
  ).filter(Boolean).join(" · ");
  // 价格、代理各开各的弹窗。
  const [extrasOpen, setExtrasOpen] = useState<"price" | "proxy" | null>(null);
  const [limitsOpen, setLimitsOpen] = useState(false);
  const [modelsOpen, setModelsOpen] = useState(false);
  const [windowsOpen, setWindowsOpen] = useState(false);
  // 生图模型单独一行，「模型」里只列能聊天的。
  const imageModels = entry.imageModels ?? [];
  const chatModels = entry.models.filter((model) => !imageModels.some((image) => image.toLowerCase() === model.toLowerCase()));
  const [allModels, setAllModels] = useState(false);
  const modelsCollapsible = chatModels.length > MODEL_CHIP_COLLAPSE;
  const shownModels = modelsCollapsible && !allModels ? chatModels.slice(0, MODEL_CHIP_PREVIEW) : chatModels;
  const [reasoningOpen, setReasoningOpen] = useState(false);
  // 「测试」弹窗：null 时用当前选中的 Key，给了标识就只用那个 Key；undefined 表示没打开。
  const [testKey, setTestKey] = useState<string | null | undefined>(undefined);
  const reasoning = reasoningSummary(entry);
  const saveReasoning = async (modelMetadata: string) => {
    if (await run(`reasoning:${entry.id}`, "save_gateway_provider_metadata", { id: entry.id, modelMetadata }, t("保存思考等级"))) setReasoningOpen(false);
  };
  const windows = windowSummary(entry);
  const saveWindows = async (modelWindows: Record<string, number>, fallback: number | null) => {
    if (await run(`windows:${entry.id}`, "save_gateway_provider_windows", { id: entry.id, windows: modelWindows, fallback }, t("保存上下文窗口"))) setWindowsOpen(false);
  };
  // 从上游同步上下文窗口 / 思考等级（#2807）：先列差异，勾选后才写入。订阅账号的模型接口不带这些资料。
  const [metaSync, setMetaSync] = useState<MetaSyncKind | null>(null);
  const metaSyncable = !entry.account && entry.models.length > 0;
  const applyWindowsSync = async (picks: Array<{ model: string; window: number }>) => {
    if (await run(`windows-sync:${entry.id}`, "apply_gateway_provider_windows_sync", { id: entry.id, picks }, t("从上游同步上下文窗口"))) setMetaSync(null);
  };
  const applyReasoningSync = async (picks: Array<{ model: string; levels: string[]; defaultLevel: string | null }>) => {
    if (await run(`reasoning-sync:${entry.id}`, "apply_gateway_provider_reasoning_sync", { id: entry.id, picks }, t("从上游同步思考等级"))) setMetaSync(null);
  };
  const newModels = entry.upstreamNew?.length ?? 0;
  const [syncOpen, setSyncOpen] = useState(false);
  const applySync = async (remove: string[], add: string[]) => {
    if (await run(`sync-models:${entry.id}`, "apply_gateway_provider_models_sync", { id: entry.id, remove, add }, t("从上游同步"))) setSyncOpen(false);
  };
  const saveModels = async (models: string[], defaultModel: string) => {
    if (await run(`models:${entry.id}`, "save_gateway_provider_models", { id: entry.id, models, defaultModel }, t("保存模型列表"))) setModelsOpen(false);
  };
  const price = priceSummary(entry);
  // ChatGPT 订阅账号、Copilot 和 SuperGrok 能查额度（ChatGPT API 账号按量计费，没有额度窗口）。
  const quotaSupported = entry.account === "chatgpt" || entry.account === "copilot" || entry.account === "grok";
  const testBlocked = subscriptionBlockReason(entry);
  const saveExtras = async (nextPrice: ProviderPrice | null, modelPrices: Record<string, ProviderPrice> | null, proxy: string) => {
    const title = extrasOpen === "proxy" ? t("保存代理") : t("保存价格");
    if (await run(`extras:${entry.id}`, "save_gateway_provider_extras", { id: entry.id, price: nextPrice, modelPrices, proxy }, title)) setExtrasOpen(null);
  };
  const keyLimits = keyLimitSummary(entry);
  const saveLimits = async (rpm: number | null, concurrency: number | null) => {
    if (await run(`limits:${entry.id}`, "save_gateway_provider_limits", { id: entry.id, rpm, concurrency }, t("保存限速"))) setLimitsOpen(false);
  };
  const remove = async () => {
    const ok = await appConfirm(tf("从网关删除「{0}」？直连模式里的供应商不受影响。", [maskAccountName(entry.name)]), { title: t("删除供应商"), confirmText: t("删除"), danger: true });
    if (ok) await run(`remove:${entry.id}`, "delete_gateway_provider", { id: entry.id }, t("删除供应商"));
  };
  return (
    <div className="gw-pool-item" data-dragging={dragging || isDragging} data-expanded={expanded} data-off={!enabled} data-pool-id={entry.id} ref={setNodeRef} style={{ transform: DndCSS.Translate.toString(transform), transition: isDragging ? "none" : transition ?? "transform 200ms ease" }}>
      {/* 整行点击展开；把手、开关、按钮各管各的。 */}
      <div
        className="gw-pool-row"
        onClick={(event) => {
          if ((event.target as HTMLElement).closest("button, a, input, .gw-grip")) return;
          onToggleExpand();
        }}
      >
        <span
          {...attributes}
          {...listeners}
          aria-label={t("拖动排序")}
          className="gw-grip"
          title={t("拖动排序：同一个模型有多家时按这个顺序尝试")}
        >
          <GripVertical className="h-4 w-4" />
        </span>
        <Avatar baseUrl={entry.baseUrl} name={entry.name} size="lg" />
        <div className="gw-pool-main">
          <strong>{maskAccountName(entry.name)}</strong>
          <small>{sub}</small>
          {quotaSupported ? entry.quota
            ? <QuotaWindows compact maxAgeSeconds={entry.subscriptionPolicy?.maxAgeSeconds} quota={entry.quota} />
            : <small>{t("额度待刷新")}</small> : null}
        </div>
        <span className="gw-pool-status">
          {newModels ? (
            <button className="gw-new-models" onClick={() => setModelsOpen(true)} title={t("后台检查发现上游有新模型，点开查看")} type="button">
              {tf("+{0} 新模型", [newModels])}
            </button>
          ) : null}
          <i className="gw-dot" data-tone={tone} />
          {/* 订阅账号空闲时显示套餐（和剩余额度），中转站（sub2api / new-api）显示倍率（和余额），其它供应商显示 Key 数。 */}
          {!enabled ? t("已关闭") : status.state === "idle" && entry.account && entry.plan ? entry.plan
            : status.state === "idle" && billing.probe?.rate ? <RelayBillingPoolLabel probe={billing.probe} />
            // 只有一个 Key（订阅登录就是一个账号）时「🔑 1」没信息量，写「可用」。
            : status.text === "{0} 个 Key" && Number(status.args[0]) <= 1 ? t("可用") : <MemberStatusLabel status={status} />}
          {enabled && billing.probe ? <RelayBillingPoolBadge failure={billing.failure} loading={billing.loading} onRefresh={billing.refresh} probe={billing.probe} /> : null}
          {vendorBalance.balance ? <ProviderBalancePoolBadge balance={vendorBalance.balance} loading={vendorBalance.loading} onRefresh={vendorBalance.refresh} /> : null}
          {enabled && !billing.probe && billing.settled ? (
            <RelayBillingRetry failure={billing.failure} loading={billing.loading} onRefresh={billing.refresh} />
          ) : null}
        </span>
        <span className="gw-pool-actions">
          {/* 普通订阅没有可编辑的连接；ChatGPT 订阅可编辑独立的额度策略。 */}
          {entry.account && entry.account !== "chatgpt" ? (
            <span aria-hidden="true" className="gw-pool-edit-slot" />
          ) : (
            <button
              aria-label={tf("编辑 {0}", [maskAccountName(entry.name)])}
              className="gw-icon-button gw-pool-edit"
              onClick={onEdit}
              title={entry.account === "chatgpt" ? t("编辑订阅额度策略") : t("编辑名称、地址、协议与模型")}
              type="button"
            >
              <Pencil className="h-4 w-4" />
            </button>
          )}
          <Switch
            disabled={busy !== null}
            label={tf("{0} 参与调度", [maskAccountName(entry.name)])}
            on={enabled}
            onToggle={() => void run(`toggle:${entry.id}`, "set_gateway_provider_enabled", { id: entry.id, enabled: !enabled }, t("切换供应商"), true)}
          />
        </span>
        <button aria-expanded={expanded} aria-label={t("详情")} className="gw-chevron" data-open={expanded} onClick={onToggleExpand} type="button">
          <ChevronRight className="h-4 w-4" />
        </button>
      </div>
      {expanded ? (
        <div className="gw-agent-detail gw-pool-detail">
          {/* 模型单独一块：标题行带数量和操作，下面是模型名标签（Agent 里的完整名「供应商/模型」在悬停提示里）。 */}
          <div className="gw-detail-row gw-detail-models">
            <span className="gw-detail-label">{t("模型")}<span className="gw-detail-count">{chatModels.length}</span></span>
            <div className="gw-detail-value gw-chip-row">
              {shownModels.map((model) => <code className="gw-chip" key={model} title={`${entry.id}/${model}`}>{model}</code>)}
              {chatModels.length === 0 ? <span className="gw-muted">{t("未配置模型")}</span> : null}
              {modelsCollapsible ? (
                <button className="gw-text-button" onClick={() => setAllModels(!allModels)} type="button">
                  {allModels ? t("收起") : tf("显示全部 {0} 个模型", [chatModels.length])}
                </button>
              ) : null}
            </div>
            <span className="gw-detail-actions">
              <EditIcon disabled={busy !== null} label={t("编辑模型列表")} onClick={() => setModelsOpen(true)} />
              {/* 从上游同步（#2785）：先拉上游列出差异，勾选要删、要加的再写入；拉取失败不改列表。 */}
              <button
                className="gw-text-button gw-sync-button"
                disabled={busy !== null}
                onClick={() => setSyncOpen(true)}
                title={t("用已保存的 Key 或登录拉取上游模型，先列出差异，勾选后再改列表")}
                type="button"
              >
                <RefreshCw className={busy === `sync-models:${entry.id}` ? "h-3.5 w-3.5 animate-spin" : "h-3.5 w-3.5"} />
                {t("从上游同步")}
              </button>
            </span>
          </div>
          {imageModels.length ? (
            <div className="gw-detail-row">
              <span className="gw-detail-label">{t("生图模型")}</span>
              <div className="gw-detail-value gw-chip-row">
                {imageModels.map((model) => <code className="gw-chip" key={model}>{model}</code>)}
              </div>
              <span className="gw-detail-actions gw-detail-hint" title={entry.account === "chatgpt" ? t("ChatGPT 后端自己选生图模型；在「Agent 接入」里选用哪个") : t("Agent 不和它们对话，不在模型列表里；在「Agent 接入」里选用哪个")}>
                {t("在「Agent 接入」里选用")}
              </span>
            </div>
          ) : null}
          <div className="gw-detail-row">
            <span className="gw-detail-label">{entry.account ? t("套餐") : "Key"}</span>
            <div className="gw-detail-value gw-chip-row">
              {entry.account ? <span>{entry.plan || t("未知")}</span> : entry.keys?.length ? <ProviderKeyList busy={busy} entry={entry} onTest={(keyId) => setTestKey(keyId)} run={run} /> : <KeyCount count={entry.keyCount} />}
              {failures.map((key) => (
                <span className="gw-key-failure" key={key.keyId}>
                  {`${keyLabel(entry.keys, key.keyId)}${t("：")}${[t(failureLabel(key.lastFailure)) || t(key.state === "disabled" ? "已停用" : "冷却中"), formatRemaining(key.remainingMs)].filter(Boolean).join(" · ")}`}
                </span>
              ))}
            </div>
            <span className="gw-detail-actions">
              {failures.length || breaker ? (
                <button className="gw-text-button" disabled={busy !== null} onClick={() => void run(`reset:${entry.id}`, "reset_gateway_key_health", { relayId: entry.id }, t("重置 Key 状态"), true)} type="button">
                  {t("重置")}
                </button>
              ) : null}
            </span>
          </div>
          {showAccess ? (
            <div className="gw-detail-row">
              <span className="gw-detail-label">{t("访问令牌")}</span>
              <div className="gw-detail-value gw-chip-row">
                {entry.newapiAccess ? (
                  <code className="gw-chip">{entry.newapiAccess.masked}</code>
                ) : (
                  <span className="gw-muted">{t("未填写：Key 不限额度时查不到账户余额")}</span>
                )}
                {entry.newapiAccess?.userId ? <span className="gw-muted">{tf("用户 ID {0}", [entry.newapiAccess.userId])}</span> : null}
              </div>
              <span className="gw-detail-actions">
                <EditIcon disabled={busy !== null} label={t("编辑访问令牌")} onClick={() => setAccessOpen(true)} />
              </span>
            </div>
          ) : null}
          {quotaSupported ? (
            <div className="gw-detail-row">
              <span className="gw-detail-label">{t("额度")}</span>
              <div className="gw-detail-value gw-chip-row">
                {entry.quota ? <QuotaWindows maxAgeSeconds={entry.subscriptionPolicy?.maxAgeSeconds} quota={entry.quota} /> : <span className="gw-muted">{t("尚未取得官方用量")}</span>}
              </div>
              <span className="gw-detail-actions">
                <button
                  className="gw-text-button gw-sync-button"
                  disabled={busy !== null}
                  onClick={() => void run(`quota:${entry.id}`, "refresh_gateway_quota", { id: entry.id }, t("刷新额度"), true)}
                  type="button"
                >
                  <RefreshCw className={busy === `quota:${entry.id}` ? "h-3.5 w-3.5 animate-spin" : "h-3.5 w-3.5"} />
                  {t("刷新")}
                </button>
              </span>
            </div>
          ) : null}
          {entry.account === "chatgpt" ? (
            <div className="gw-detail-row">
              <span className="gw-detail-label">{t("订阅额度策略")}</span>
              <div className="gw-detail-value">
                {t("按官方窗口限制")}
              </div>
              <span className="gw-detail-actions"><EditIcon disabled={busy !== null} label={t("编辑订阅额度策略")} onClick={onEdit} /></span>
            </div>
          ) : null}
          {/* 价格、窗口、思考等级、代理、限速不常改：收进「更多设置」，展开详情先看到模型、Key、额度。 */}
          <details className="gw-detail-advanced gw-pool-more">
            <summary>
              <ChevronRight aria-hidden="true" className="h-3.5 w-3.5" />
              {t("更多设置")}
              <span className="gw-pool-more-hint">{t("价格、上下文窗口、思考等级、代理、限速")}</span>
            </summary>
          <div className="gw-detail-row">
            <span className="gw-detail-label">{t("价格")}</span>
            <div className="gw-detail-value gw-chip-row">
              <span className={price ? undefined : "gw-muted"}>{price || t("未填写，不估算费用")}</span>
            </div>
            <span className="gw-detail-actions">
              <EditIcon disabled={busy !== null} label={t("编辑价格")} onClick={() => setExtrasOpen("price")} />
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
            </span>
          </div>
          <div className="gw-detail-row">
            <span className="gw-detail-label">{t("上下文窗口")}</span>
            <div className="gw-detail-value gw-chip-row">
              <span className={windows ? undefined : "gw-muted"}>{windows || t("未填写，用模型自带资料")}</span>
            </div>
            <span className="gw-detail-actions">
              <EditIcon disabled={busy !== null || entry.models.length === 0} label={t("编辑上下文窗口")} onClick={() => setWindowsOpen(true)} />
              {metaSyncable ? (
                <MetaSyncButton
                  busy={busy}
                  busyKey={`windows-sync:${entry.id}`}
                  onClick={() => setMetaSync("windows")}
                  title={t("读取上游 /models 返回的上下文窗口，先列出差异，勾选后再写入")}
                />
              ) : null}
            </span>
          </div>
          <div className="gw-detail-row">
            <span className="gw-detail-label">{t("思考等级")}</span>
            <div className="gw-detail-value gw-chip-row">
              <span className={reasoning ? undefined : "gw-muted"}>{reasoning || t("按内置资料")}</span>
            </div>
            <span className="gw-detail-actions">
              <EditIcon disabled={busy !== null || entry.models.length === 0} label={t("编辑思考等级")} onClick={() => setReasoningOpen(true)} />
              {metaSyncable ? (
                <MetaSyncButton
                  busy={busy}
                  busyKey={`reasoning-sync:${entry.id}`}
                  onClick={() => setMetaSync("reasoning")}
                  title={t("读取上游 /models 返回的思考等级，先列出差异，勾选后再写入")}
                />
              ) : null}
            </span>
          </div>
          <div className="gw-detail-row">
            <span className="gw-detail-label">{t("代理")}</span>
            <div className="gw-detail-value gw-chip-row">
              {isDirectProxy(entry.proxy) ? <span>{t("不使用代理（直连）")}</span> : entry.proxy ? <code className="gw-chip">{entry.proxy}</code> : <span className="gw-muted">{t("沿用系统代理")}</span>}
            </div>
            <span className="gw-detail-actions">
              <EditIcon disabled={busy !== null} label={t("编辑代理")} onClick={() => setExtrasOpen("proxy")} />
            </span>
          </div>
          <div className="gw-detail-row">
            <span className="gw-detail-label">{t("限速")}</span>
            <div className="gw-detail-value gw-chip-row">
              <span className={keyLimits ? undefined : "gw-muted"} title={t("按每个 Key 计；达到上限时先换别的 Key 或下一家，都满了就排队等待（最多 2 分钟）。")}>
                {keyLimits || t("不限")}
              </span>
            </div>
            <span className="gw-detail-actions">
              <EditIcon disabled={busy !== null} label={t("编辑限速")} onClick={() => setLimitsOpen(true)} />
            </span>
          </div>
          </details>
          <div className="gw-detail-row gw-detail-foot">
            <span className="gw-detail-label" />
            <div className="gw-detail-value">
              {/* 普通供应商的「编辑」在标题栏；这里是测试和订阅账号的重新登录。 */}
              <span className="gw-detail-foot-actions">
                <button
                  className="gw-text-button gw-sync-button"
                  disabled={busy !== null || Boolean(testBlocked)}
                  onClick={() => setTestKey(null)}
                  title={testBlocked ? t(maskAccountText(testBlocked)) : t("发一条很短的请求，看这家供应商能不能用、多快回复；不计入路由与统计")}
                  type="button"
                >
                  <Zap className="h-3.5 w-3.5" />
                  {t("测试")}
                </button>
                {entry.account ? (
                  <button className="gw-text-button" disabled={busy !== null || loggingIn} onClick={onRelogin} type="button">
                    {loggingIn ? t("登录中…") : t("重新登录")}
                    <ChevronRight className="h-3.5 w-3.5" />
                  </button>
                ) : null}
              </span>
              <button className="gw-danger-link" disabled={busy !== null} onClick={() => void remove()} type="button">
                {t("删除")}
              </button>
            </div>
          </div>
        </div>
      ) : null}
      {testKey !== undefined ? <ProviderTestDialog entry={entry} keyId={testKey} onEditSubscription={onEdit} onClose={() => setTestKey(undefined)} /> : null}
      {metaSync ? (
        <ProviderMetaSyncDialog
          busy={busy !== null}
          entry={entry}
          kind={metaSync}
          onApplyReasoning={(picks) => void applyReasoningSync(picks)}
          onApplyWindows={(picks) => void applyWindowsSync(picks)}
          onCancel={() => setMetaSync(null)}
        />
      ) : null}
      {syncOpen ? (
        <ProviderSyncDialog busy={busy !== null} entry={entry} onApply={(remove, add) => void applySync(remove, add)} onCancel={() => setSyncOpen(false)} />
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
      {accessOpen ? (
        <ProviderNewApiAccessDialog
          busy={busy !== null}
          entry={entry}
          onCancel={() => setAccessOpen(false)}
          onClear={() => void saveAccess(null, "", true)}
          onSave={(token, userId) => void saveAccess(token, userId, false)}
        />
      ) : null}
      {limitsOpen ? (
        <ProviderLimitsDialog busy={busy !== null} entry={entry} onCancel={() => setLimitsOpen(false)} onSave={(rpm, concurrency) => void saveLimits(rpm, concurrency)} />
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

/** 详情里「上下文窗口」「思考等级」的「从上游同步」（#2807）。 */
function MetaSyncButton({ busy, busyKey, title, onClick }: { busy: string | null; busyKey: string; title: string; onClick: () => void }) {
  return (
    <button className="gw-text-button gw-sync-button" disabled={busy !== null} onClick={onClick} title={title} type="button">
      <RefreshCw className={busy === busyKey ? "h-3.5 w-3.5 animate-spin" : "h-3.5 w-3.5"} />
      {t("从上游同步")}
    </button>
  );
}

/** 详情里各项的「编辑」：铅笔图标按钮，文案放在无障碍名和悬停提示里（#2782）。 */
function EditIcon({ label, disabled, onClick }: { label: string; disabled: boolean; onClick: () => void }) {
  return (
    <button aria-label={label} className="gw-icon-button gw-inline-icon" disabled={disabled} onClick={onClick} title={label} type="button">
      <Pencil className="h-3.5 w-3.5" />
    </button>
  );
}

/** 添加时给哪些 Agent 用：一排可切换的 Agent，默认按各自的「添加供应商时自动加入」。 */
function AgentPicks({ payload, picked, onChange }: { payload: GatewayPayload | null; picked: GatewayAgentId[]; onChange: (next: GatewayAgentId[]) => void }) {
  // 只列内置的三个和已接入的通用 Agent；装了但没接入的不在这里出现。
  // 没勾选的 Agent 不列（保持各自的默认）；单 Agent 模式下只有一个可选，整行不显示。
  const focus = useAgentFocus();
  const agents = visibleAgents(focus, glanceAgents(payload?.agents));
  if (!agents.length || singleAgent(focus)) return null;
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
      ...(payload?.subscriptions ?? []).map((item) => ({ kind: `sub:${item.id}` as AccountKind, name: item.name, icon: item.id })),
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
          <strong>{code.title ?? t("登录 GitHub Copilot")}</strong>
          <button aria-label={t("取消登录")} className="gw-icon-button" onClick={onCancel} title={t("取消登录")} type="button"><X className="h-4 w-4" /></button>
        </header>
        <div className="gw-modal-body">
          <p className="gw-muted">{tf("已在浏览器打开 {0}，输入下面的验证码并授权。授权完成后这里会自动继续。", [code.verificationUri])}</p>
          <button className="gw-device-code" onClick={() => void copy()} title={t("复制验证码")} type="button">
            <span>{code.userCode}</span>
            <Copy className="h-4 w-4" />
          </button>
          <p className="gw-muted">{code.hint ?? tf("验证码 {0} 分钟内有效。需要这个 GitHub 账号有 Copilot 订阅。", [Math.round(code.expiresIn / 60)])}</p>
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

/** 私有接口订阅登录前的弹窗：选登录方式，粘贴凭据时填内容；附带订阅的说明与风险提示。 */
function SubscriptionLoginDialog({
  item,
  onCancel,
  onStart,
}: {
  item: SubscriptionView;
  onCancel: () => void;
  onStart: (method: SubscriptionLoginMethod, input: string) => void;
}) {
  useEscapeLayer(true, onCancel);
  const [method, setMethod] = useState<SubscriptionLoginMethod>(item.methods[0] ?? "browser");
  const [input, setInput] = useState("");
  const ready = method !== "paste" || input.trim().length > 0;
  return createPortal(
    <div className="gw-modal-backdrop gw-portal" onMouseDown={(event) => { if (event.target === event.currentTarget) onCancel(); }}>
      <div className="gw-modal gw-device-modal" role="dialog" aria-label={tf("登录 {0}", [item.name])}>
        <header>
          <strong>{tf("登录 {0}", [item.name])}</strong>
          <button aria-label={t("关闭")} className="gw-icon-button" onClick={onCancel} type="button"><X className="h-4 w-4" /></button>
        </header>
        <div className="gw-modal-body">
          {item.note ? <p className="gw-muted">{t(item.note)}</p> : null}
          <p className="gw-muted">{t("通过模拟官方客户端调用未公开的接口，可能随时失效，也有账号被限制的风险。")}</p>
          {item.methods.length > 1 ? (
            <div className="gw-chip-row" role="radiogroup">
              {item.methods.map((value) => (
                <label className="gw-inline-field gw-inline-check" key={value}>
                  <input checked={method === value} name="sub-login-method" onChange={() => setMethod(value)} type="radio" />
                  <span>{t(LOGIN_METHOD_LABELS[value])}</span>
                </label>
              ))}
            </div>
          ) : null}
          {method === "paste" ? (
            <textarea autoFocus className="gw-textarea" onChange={(event) => setInput(event.currentTarget.value)} placeholder={t("粘贴 token、Cookie 或导出的登录文件内容")} rows={4} value={input} />
          ) : null}
        </div>
        <footer>
          <span className="gw-error" />
          <button className="gw-soft-button" onClick={onCancel} type="button">{t("取消")}</button>
          <button className="gw-primary-button" disabled={!ready} onClick={() => onStart(method, input.trim())} type="button">{t("开始登录")}</button>
        </footer>
      </div>
    </div>,
    document.body,
  );
}

/** 浏览器登录进行中的提示条：不挡操作（固定在窗口底部），可随时取消。 */
function LoginPendingBar({ label, onCancel }: { label: string; onCancel: () => void }) {
  const [copied, setCopied] = useState<"idle" | "copied" | "failed">("idle");
  // 复制登录链接（#2823）：想用无痕窗口或别的浏览器登录时，不用抢在默认浏览器打开的瞬间去地址栏复制。
  const copyLink = async () => {
    try {
      const result = await invoke<{ status: string; url?: string | null }>("gateway_login_url");
      if (!result.url) throw new Error("no url");
      await navigator.clipboard.writeText(result.url);
      setCopied("copied");
    } catch {
      setCopied("failed");
    }
  };
  return createPortal(
    <div aria-live="polite" className="gw-portal gw-login-pending" role="status">
      <RefreshCw aria-hidden="true" className="h-3.5 w-3.5 animate-spin" />
      <span>{tf("正在等待 {0} 登录，在浏览器里完成后会自动继续", [label])}</span>
      <button className="gw-text-button" onClick={() => void copyLink()} title={t("复制后可以在无痕窗口或其他浏览器里打开")} type="button">
        {copied === "copied" ? t("已复制") : copied === "failed" ? t("复制失败") : t("复制登录链接")}
      </button>
      <button className="gw-text-button strong" onClick={onCancel} type="button">{t("取消登录")}</button>
    </div>,
    document.body,
  );
}
