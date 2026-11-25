/**
 * 一家供应商里的多个 Key / 登录账号：添加、移除账号，分配方式，以及每个 Key / 账号单独的设置
 * （停用、权重、只给哪些模型、额度上限）。从 Codex「连接与供应商」和 Claude「供应商」的行菜单打开。
 *
 * 版式跟「从第三方导入」一样：上面是分配方式的选项卡，下面一行一个账号（额度条、设置摘要、开关），
 * 点行尾的箭头在行内展开它的设置，拖行首的把手调先后，最后一行是「添加账号」。
 */
import { useId, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { closestCenter, DndContext, KeyboardSensor, PointerSensor, useSensor, useSensors, type DragEndEvent } from "@dnd-kit/core";
import { arrayMove, SortableContext, sortableKeyboardCoordinates, useSortable, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { CSS as DndCSS } from "@dnd-kit/utilities";
import { ChevronDown, GripVertical, Plus, Trash2, X } from "lucide-react";
import { Input } from "@/components/ui/input";
import { t, tf } from "./i18n";
import {
  CREDENTIAL_ROTATIONS,
  credentialRotationDescription,
  credentialRotationLabel,
  maskAccountName,
  type CredentialRotation,
  type CredentialSettings,
  type CredentialView,
  type GatewayPoolEntry,
} from "./gateway-model";
import { Avatar, QuotaWindows, Switch } from "./gateway-ui";
import { useModalShell } from "./use-modal-shell";

/** 跟全局 Key 策略时的选项值。 */
const FOLLOW_GLOBAL = "global";
type RotationChoice = CredentialRotation | typeof FOLLOW_GLOBAL;

/** 每个凭据在表单里的样子：数字都先当文字编辑。 */
type Draft = { disabled: boolean; weight: string; models: string; cap: string };

function draftOf(settings: CredentialSettings): Draft {
  return {
    disabled: Boolean(settings.disabled),
    weight: settings.weight ? String(settings.weight) : "",
    models: (settings.models ?? []).join(", "),
    cap: settings.usedPercentLimit ? String(settings.usedPercentLimit) : "",
  };
}

/** 留空是 null；不是 min–max 的整数是 NaN。 */
function parseBounded(text: string, min: number, max: number): number | null {
  const value = text.trim();
  if (!value) return null;
  const number = Number(value);
  return Number.isInteger(number) && number >= min && number <= max ? number : Number.NaN;
}

export function settingsFromDraft(draft: Draft): CredentialSettings {
  return {
    disabled: draft.disabled,
    weight: parseBounded(draft.weight, 1, 1000),
    models: draft.models.split(/[,，\s]+/).map((model) => model.trim()).filter(Boolean),
    usedPercentLimit: parseBounded(draft.cap, 1, 100),
  };
}

function sameSettings(left: CredentialSettings, right: CredentialSettings): boolean {
  return Boolean(left.disabled) === Boolean(right.disabled)
    && (left.weight ?? null) === (right.weight ?? null)
    && (left.usedPercentLimit ?? null) === (right.usedPercentLimit ?? null)
    && (left.models ?? []).join("\n") === (right.models ?? []).join("\n");
}

/** 行上的设置摘要：「权重 3 · 只给 flash · 用到 90% 先不用」。只列和默认不同的。 */
function settingsSummary(settings: CredentialSettings, weighted: boolean): string {
  const weight = settings.weight;
  const cap = settings.usedPercentLimit;
  return [
    weighted && weight && weight !== 1 && !Number.isNaN(weight) ? tf("权重 {0}", [weight]) : "",
    settings.models?.length ? tf("只给 {0}", [settings.models.join("、")]) : "",
    cap && !Number.isNaN(cap) ? tf("用到 {0}% 先不用", [cap]) : "",
  ].filter(Boolean).join(" · ");
}

export type CredentialChanges = {
  /** undefined：分法没改；null：改回跟全局。 */
  rotation: CredentialRotation | null | undefined;
  settings: Array<{ id: string; settings: CredentialSettings }>;
  /** 拖动后的先后（全部凭据 id）；没拖过为 undefined。 */
  order?: string[];
};

/** 可拖动的一行：把手在行首，拖动时跟手，其余行让位。 */
function SortableRow({ id, draggable, label, off, open, children }: { id: string; draggable: boolean; label: string; off: boolean; open: boolean; children: (handle: ReactNode) => ReactNode }) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id, disabled: !draggable });
  const handle = draggable ? (
    <span {...attributes} {...listeners} aria-label={tf("拖动调整 {0} 的先后", [label])} className="pcd-grip" title={t("拖动调整先后：排在前面的先用")}>
      <GripVertical className="h-4 w-4" />
    </span>
  ) : null;
  return (
    <div
      className="pcd-row"
      data-dragging={isDragging || undefined}
      data-off={off || undefined}
      data-open={open || undefined}
      ref={setNodeRef}
      style={{ transform: DndCSS.Translate.toString(transform), transition: isDragging ? "none" : transition ?? "transform 200ms ease" }}
    >
      {children(handle)}
    </div>
  );
}

export function ProviderCredentialsDialog({
  entry,
  busy,
  globalRoundRobin,
  loggingIn = false,
  onAddAccount,
  onRemoveAccount,
  onSave,
  onCancel,
}: {
  entry: GatewayPoolEntry;
  busy: boolean;
  /** 全局 Key 策略是「每次轮换」：跟全局时实际是轮换，否则是依次。 */
  globalRoundRobin: boolean;
  /** 正在登录要加进来的账号。 */
  loggingIn?: boolean;
  /** 再登录一个账号加进这一家；Key 供应商不给。 */
  onAddAccount?: () => void;
  /** 移除一个账号（只剩一个时不显示）。 */
  onRemoveAccount?: (item: CredentialView) => void;
  onSave: (changes: CredentialChanges) => void;
  onCancel: () => void;
}) {
  const credentials: CredentialView[] = entry.credentials ?? [];
  const accounts = Boolean(entry.account);
  // 只有一个时没什么可分的：只列出来，能再加。
  const several = credentials.length > 1;
  const initialRotation: RotationChoice = entry.rotationSet && entry.rotation ? entry.rotation : FOLLOW_GLOBAL;
  const [rotation, setRotation] = useState<RotationChoice>(initialRotation);
  const [drafts, setDrafts] = useState<Record<string, Draft>>(() => Object.fromEntries(credentials.map((item) => [item.id, draftOf(item.settings)])));
  const [expanded, setExpanded] = useState<string | null>(null);
  const initialOrder = credentials.map((item) => item.id);
  const [order, setOrder] = useState<string[]>(initialOrder);
  // 登录、移除后列表会变：新来的排在最后，已不在的去掉。
  const ordered = [
    ...order.map((id) => credentials.find((item) => item.id === id)).filter((item): item is CredentialView => Boolean(item)),
    ...credentials.filter((item) => !order.includes(item.id)),
  ];
  const orderChanged = ordered.map((item) => item.id).join("\n") !== initialOrder.join("\n");
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );
  const finishDrag = (event: DragEndEvent) => {
    const ids = ordered.map((item) => item.id);
    const from = ids.indexOf(String(event.active.id));
    const to = event.over ? ids.indexOf(String(event.over.id)) : -1;
    if (from >= 0 && to >= 0 && from !== to) setOrder(arrayMove(ids, from, to));
  };
  const globalRotation: CredentialRotation = globalRoundRobin ? "rotate" : "inOrder";
  const weighted = (rotation === FOLLOW_GLOBAL ? globalRotation : rotation) === "weighted";
  const parsed = ordered.map((item) => ({ item, settings: settingsFromDraft(drafts[item.id] ?? draftOf(item.settings)) }));
  const invalidWeight = parsed.some(({ settings }) => Number.isNaN(settings.weight));
  const invalidCap = parsed.some(({ settings }) => Number.isNaN(settings.usedPercentLimit));
  const allDisabled = parsed.length > 0 && parsed.every(({ settings }) => settings.disabled);
  const error = invalidWeight ? t("权重要填 1–1000 的整数，留空当 1")
    : invalidCap ? t("额度上限要填 1–100 的整数，留空表示不限")
    : allDisabled ? t("至少要留一个不停用；整家不用请关掉这家供应商") : "";
  const changed = parsed.filter(({ item, settings }) => !sameSettings(item.settings, settings));
  const dirty = rotation !== initialRotation || changed.length > 0 || orderChanged;
  const titleId = useId();
  const { backdropProps, dialogProps, requestClose } = useModalShell<HTMLFormElement>({ onClose: onCancel, dirty, labelledBy: titleId });
  const update = (id: string, patch: Partial<Draft>) => setDrafts((current) => ({ ...current, [id]: { ...(current[id] ?? draftOf({})), ...patch } }));
  const choices: Array<{ value: RotationChoice; label: string; hint: string }> = [
    { value: FOLLOW_GLOBAL, label: t("跟随全局"), hint: tf("现在是「{0}」，跟本地服务的 Key 策略走", [t(credentialRotationLabel(globalRotation))]) },
    ...CREDENTIAL_ROTATIONS.map((value) => ({ value, label: t(credentialRotationLabel(value)), hint: t(credentialRotationDescription(value)) })),
  ];

  return createPortal(
    <div className="gw-modal-backdrop gw-portal" {...backdropProps}>
      <form
        {...dialogProps}
        className="gw-modal pcd-modal"
        onSubmit={(event) => {
          event.preventDefault();
          if (error || !dirty) return;
          onSave({
            rotation: rotation === initialRotation ? undefined : rotation === FOLLOW_GLOBAL ? null : rotation,
            settings: changed.map(({ item, settings }) => ({ id: item.id, settings })),
            order: orderChanged ? ordered.map((item) => item.id) : undefined,
          });
        }}
      >
        <header>
          <Avatar baseUrl={entry.baseUrl} name={entry.name} />
          <span className="pcd-title">
            <strong id={titleId}>{accounts ? t("账号与分配") : t("Key 分配")}</strong>
            <small>{maskAccountName(entry.name)}</small>
          </span>
          <button aria-label={t("关闭")} className="gw-icon-button" onClick={() => void requestClose()} type="button"><X className="h-4 w-4" /></button>
        </header>

        <div className="gw-modal-body">
          {several ? (
            <section className="pcd-section">
              <h4>{t("分配方式")}<small>{t("同一个对话固定用一个，它答不了才换下一个")}</small></h4>
              <div aria-label={t("分配方式")} className="pcd-modes" role="radiogroup">
                {choices.map((choice) => (
                  <button aria-checked={rotation === choice.value} className="tpi-mode" disabled={busy} key={choice.value} onClick={() => setRotation(choice.value)} role="radio" type="button">
                    <strong>{choice.label}</strong>
                    <small>{choice.hint}</small>
                  </button>
                ))}
              </div>
            </section>
          ) : null}

          <section className="pcd-section">
            <h4>
              {accounts ? t("账号") : "Key"}
              <span className="pcd-count">{credentials.length}</span>
              {several ? <small>{t("拖动调先后，排在前面的先用；停用的不再分请求，凭据保留")}</small> : null}
            </h4>
            <div className="pcd-list">
              <DndContext collisionDetection={closestCenter} onDragEnd={finishDrag} onDragStart={() => setExpanded(null)} sensors={sensors}>
              <SortableContext items={ordered.map((item) => item.id)} strategy={verticalListSortingStrategy}>
              {parsed.map(({ item, settings }, index) => {
                const draft = drafts[item.id] ?? draftOf(item.settings);
                const name = item.kind === "account" ? maskAccountName(item.label) : item.label;
                const open = expanded === item.id;
                const summary = settingsSummary(settings, weighted);
                return (
                  <SortableRow draggable={several} id={item.id} key={item.id} label={name} off={draft.disabled} open={open}>
                    {(handle) => (<>
                    <div className="pcd-row-main">
                      {handle}
                      <span className="pcd-index">{index + 1}</span>
                      <span className="pcd-copy">
                        <span className="pcd-name">
                          {item.kind === "key" ? <code>{name}</code> : <strong>{name}</strong>}
                          {item.plan ? <em className="gw-import-badge">{item.plan}</em> : null}
                          {draft.disabled ? <em className="gw-import-badge">{t("已停用")}</em> : null}
                        </span>
                        {item.quota?.windows.length ? <QuotaWindows compact quota={item.quota} /> : null}
                        {summary ? <small className="pcd-summary">{summary}</small> : null}
                      </span>
                      {several ? (
                        <>
                          <Switch label={tf("使用 {0}", [name])} on={!draft.disabled} onToggle={() => update(item.id, { disabled: !draft.disabled })} />
                          <button
                            aria-expanded={open}
                            aria-label={tf("{0} 的设置", [name])}
                            className="gw-icon-button pcd-expand"
                            onClick={() => setExpanded(open ? null : item.id)}
                            title={t("权重、只给哪些模型、额度上限")}
                            type="button"
                          >
                            <ChevronDown className="h-4 w-4" />
                          </button>
                        </>
                      ) : null}
                    </div>
                    {open ? (
                      <div className="pcd-settings">
                        {weighted ? (
                          <label className="pcd-field pcd-field-narrow">
                            <span>{t("权重")}</span>
                            <Input inputMode="numeric" onChange={(event) => update(item.id, { weight: event.currentTarget.value })} placeholder="1" value={draft.weight} />
                          </label>
                        ) : null}
                        <label className="pcd-field">
                          <span>{t("只给这些模型")}</span>
                          <Input onChange={(event) => update(item.id, { models: event.currentTarget.value })} placeholder={t("全部模型")} value={draft.models} />
                        </label>
                        {item.kind === "account" ? (
                          <label className="pcd-field pcd-field-narrow">
                            <span>{t("额度用到 %")}</span>
                            <Input inputMode="numeric" onChange={(event) => update(item.id, { cap: event.currentTarget.value })} placeholder={t("不限")} value={draft.cap} />
                          </label>
                        ) : null}
                        {onRemoveAccount && item.kind === "account" ? (
                          <button className="gw-text-button pcd-remove" disabled={busy} onClick={() => onRemoveAccount(item)} type="button">
                            <Trash2 className="h-3.5 w-3.5" />{t("移除这个账号")}
                          </button>
                        ) : null}
                      </div>
                    ) : null}
                    </>)}
                  </SortableRow>
                );
              })}
              </SortableContext>
              </DndContext>
              {onAddAccount ? (
                <button className="pcd-add" disabled={busy || loggingIn} onClick={onAddAccount} type="button">
                  <Plus className="h-4 w-4" />
                  <span>
                    <strong>{loggingIn ? t("登录中…") : t("添加账号")}</strong>
                    <small>{t("再登录一个账号加进这一家，轮着用各个账号的额度")}</small>
                  </span>
                </button>
              ) : null}
            </div>
          </section>
        </div>

        <footer className="gw-import-foot">
          <span className={error ? "pcd-error" : "pcd-hint"}>
            {error || (expanded ? t("模型用逗号分开，留空表示都能用") : "")}
          </span>
          {several ? (
            <button className="gw-primary-button" disabled={busy || Boolean(error) || !dirty} type="submit">{busy ? t("保存中") : t("保存")}</button>
          ) : (
            <button className="gw-primary-button" onClick={() => void requestClose()} type="button">{t("完成")}</button>
          )}
        </footer>
      </form>
    </div>,
    document.body,
  );
}
