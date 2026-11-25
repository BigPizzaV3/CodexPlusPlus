/**
 * 一家供应商里的多个 Key / 登录账号：添加、移除账号，分配方式，以及每个 Key / 账号单独的设置
 * （停用、权重、只给哪些模型、额度上限）。从 Codex「连接与供应商」和 Claude「供应商」的行菜单打开。
 */
import { useId, useState } from "react";
import { createPortal } from "react-dom";
import { Plus, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { t, tf } from "./i18n";
import { AppSelect } from "./AppSelect";
import {
  CREDENTIAL_ROTATIONS,
  credentialRotationDescription,
  credentialRotationLabel,
  maskAccountName,
  type CredentialRotation,
  type CredentialSettings,
  type CredentialView,
  type GatewayPoolEntry,
  type ProviderQuota,
} from "./gateway-model";
import { Avatar, Switch } from "./gateway-ui";
import { useModalShell } from "./use-modal-shell";
import "./ProviderFormDialog.css";

/** 账号额度的一句话：「5 小时已用 32% · 每周已用 10%」。 */
function quotaText(quota: ProviderQuota | null | undefined): string {
  return (quota?.windows ?? []).map((window) => {
    const minutes = window.windowMinutes;
    const name = minutes === 300 ? t("5 小时") : minutes === 10080 ? t("每周") : minutes ? tf("{0} 小时", [Math.round(minutes / 60)]) : t("本期");
    return tf("{0}已用 {1}%", [name, Math.round(Number(window.usedPercent) || 0)]);
  }).join(" · ");
}

/** 跟全局 Key 策略时选择框里的值。 */
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

export type CredentialChanges = {
  /** undefined：分法没改；null：改回跟全局。 */
  rotation: CredentialRotation | null | undefined;
  settings: Array<{ id: string; settings: CredentialSettings }>;
};

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
  // 只有一个账号时没什么可分的：只列出来，能再加。
  const several = credentials.length > 1;
  const initialRotation: RotationChoice = entry.rotationSet && entry.rotation ? entry.rotation : FOLLOW_GLOBAL;
  const [rotation, setRotation] = useState<RotationChoice>(initialRotation);
  const [drafts, setDrafts] = useState<Record<string, Draft>>(() => Object.fromEntries(credentials.map((item) => [item.id, draftOf(item.settings)])));
  const effective: CredentialRotation = rotation === FOLLOW_GLOBAL ? (globalRoundRobin ? "rotate" : "inOrder") : rotation;
  const parsed = credentials.map((item) => ({ item, settings: settingsFromDraft(drafts[item.id] ?? draftOf(item.settings)) }));
  const invalidWeight = parsed.some(({ settings }) => Number.isNaN(settings.weight));
  const invalidCap = parsed.some(({ settings }) => Number.isNaN(settings.usedPercentLimit));
  const allDisabled = parsed.length > 0 && parsed.every(({ settings }) => settings.disabled);
  const error = invalidWeight ? t("权重要填 1–1000 的整数，留空当 1")
    : invalidCap ? t("额度上限要填 1–100 的整数，留空表示不限")
    : allDisabled ? t("至少要留一个不停用；整家不用请关掉这家供应商") : "";
  const changed = parsed.filter(({ item, settings }) => !sameSettings(item.settings, settings));
  const dirty = rotation !== initialRotation || changed.length > 0;
  const titleId = useId();
  const { backdropProps, dialogProps, requestClose } = useModalShell<HTMLFormElement>({ onClose: onCancel, dirty, labelledBy: titleId });
  const update = (id: string, patch: Partial<Draft>) => setDrafts((current) => ({ ...current, [id]: { ...(current[id] ?? draftOf({})), ...patch } }));
  const globalLabel = tf("跟随全局（{0}）", [t(credentialRotationLabel(globalRoundRobin ? "rotate" : "inOrder"))]);

  return createPortal(
    <div className="gw-modal-backdrop gw-portal" {...backdropProps}>
      <form
        {...dialogProps}
        className="pf-modal pf-extras-modal"
        onSubmit={(event) => {
          event.preventDefault();
          if (error) return;
          onSave({
            rotation: rotation === initialRotation ? undefined : rotation === FOLLOW_GLOBAL ? null : rotation,
            settings: changed.map(({ item, settings }) => ({ id: item.id, settings })),
          });
        }}
      >
        <header className="pf-head">
          <Avatar baseUrl={entry.baseUrl} name={entry.name} />
          <div>
            <strong id={titleId}>{entry.account ? tf("{0} 的账号", [maskAccountName(entry.name)]) : tf("{0} 的 Key 分配", [maskAccountName(entry.name)])}</strong>
            <small>{entry.account ? tf("{0} 个账号", [credentials.length]) : tf("{0} 个 Key", [credentials.length])}</small>
          </div>
          <button aria-label={t("关闭")} className="pf-icon" onClick={() => void requestClose()} type="button"><X /></button>
        </header>

        <div className="pf-body">
          {several ? <section className="pf-section">
            <h4>{t("分配方式")}<small>{t("同一个对话固定用一个，它答不了才换下一个")}</small></h4>
            <AppSelect<RotationChoice>
              ariaLabel={t("分配方式")}
              onChange={setRotation}
              options={[
                { value: FOLLOW_GLOBAL, label: globalLabel, hint: t("跟「本地服务」里的 Key 策略走") },
                ...CREDENTIAL_ROTATIONS.map((value) => ({ value, label: t(credentialRotationLabel(value)), hint: t(credentialRotationDescription(value)) })),
              ]}
              value={rotation}
            />
          </section> : null}

          <section className="pf-section">
            <div className="pf-credential-title">
              <h4>{entry.account ? t("账号") : "Key"}{several ? <small>{t("排在前面的先用；停用的不再分请求，凭据保留")}</small> : null}</h4>
              {onAddAccount ? (
                <Button disabled={busy || loggingIn} onClick={onAddAccount} size="sm" title={t("再登录一个账号，加进这一家，轮着用各个账号的额度")} type="button" variant="outline">
                  <Plus className="h-3.5 w-3.5" />{loggingIn ? t("登录中…") : t("添加账号")}
                </Button>
              ) : null}
            </div>
            {parsed.map(({ item }) => {
              const draft = drafts[item.id] ?? draftOf(item.settings);
              const name = item.kind === "account" ? maskAccountName(item.label) : item.label;
              return (
                <div className="pf-credential" data-off={draft.disabled} key={item.id}>
                  <div className="pf-credential-head">
                    <span className="pf-credential-name">
                      {item.kind === "key" ? <code className="pf-mono">{name}</code> : <strong>{name}</strong>}
                      {item.plan ? <small>{item.plan}</small> : null}
                    </span>
                    <span className="pf-credential-actions">
                      {several ? <Switch label={tf("使用 {0}", [name])} on={!draft.disabled} onToggle={() => update(item.id, { disabled: !draft.disabled })} /> : null}
                      {several && onRemoveAccount && item.kind === "account" ? (
                        <button aria-label={tf("移除 {0}", [name])} className="pf-icon" disabled={busy} onClick={() => onRemoveAccount(item)} title={t("移除这个账号")} type="button"><X /></button>
                      ) : null}
                    </span>
                  </div>
                  {item.quota?.windows.length ? <small className="pf-credential-quota">{quotaText(item.quota)}</small> : null}
                  {several ? <div className="pf-credential-fields">
                    {effective === "weighted" ? (
                      <label className="pf-field">
                        <span>{t("权重")}</span>
                        <Input inputMode="numeric" onChange={(event) => update(item.id, { weight: event.currentTarget.value })} placeholder="1" value={draft.weight} />
                      </label>
                    ) : null}
                    <label className="pf-field">
                      <span>{t("只给这些模型")}</span>
                      <Input onChange={(event) => update(item.id, { models: event.currentTarget.value })} placeholder={t("全部模型")} value={draft.models} />
                    </label>
                    {item.kind === "account" ? (
                      <label className="pf-field">
                        <span>{t("额度用到")} <small>%</small></span>
                        <Input inputMode="numeric" onChange={(event) => update(item.id, { cap: event.currentTarget.value })} placeholder={t("不限")} value={draft.cap} />
                      </label>
                    ) : null}
                  </div> : null}
                </div>
              );
            })}
            {several ? <p className="pf-note">{entry.account ? t("模型用逗号分开，留空表示这家的模型都能用。额度用到上限的账号先不用，额度恢复后自动回来。") : t("模型用逗号分开，留空表示这家的模型都能用。")}</p> : (
              <p className="pf-note">{t("再加一个账号，这一家就会轮着用各个账号的额度；怎么分、每个账号用到多少先不用，都在这里设。")}</p>
            )}
          </section>
        </div>

        <footer className="pf-foot">
          <span className="pf-error">{error}</span>
          <Button onClick={() => void requestClose()} size="sm" type="button" variant="ghost">{several ? t("取消") : t("关闭")}</Button>
          {several ? <Button disabled={busy || Boolean(error) || !dirty} size="sm" type="submit">{busy ? t("保存中") : t("保存")}</Button> : null}
        </footer>
      </form>
    </div>,
    document.body,
  );
}
