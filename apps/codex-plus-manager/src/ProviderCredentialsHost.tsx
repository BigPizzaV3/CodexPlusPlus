/**
 * 「账号与分配」弹窗和账号登录（浏览器 / 验证码 / 选登录方式）。按需加载：登录相关的代码在本地服务那个大模块里。
 * 打开过一次就一直挂着，关掉弹窗后进行中的登录提示条照样在。
 */
import { useState } from "react";
import { appConfirm } from "./app-dialog";
import { t, tf } from "./i18n";
import { maskAccountName, type CredentialView, type GatewayPayload } from "./gateway-model";
import type { Notify, Run } from "./gateway-ui";
import { useAccountLogin, type AccountKind } from "./GatewayProviders";
import { ProviderCredentialsDialog, type CredentialChanges } from "./ProviderCredentialsDialog";

export function ProviderCredentialsHost({
  payload,
  run,
  notify,
  openId,
  onClose,
}: {
  payload: GatewayPayload | null;
  run: Run;
  notify: Notify;
  /** 打开的是哪一家；null 时只留着登录提示。 */
  openId: string | null;
  onClose: () => void;
}) {
  const { login, pending, dialog: loginDialog } = useAccountLogin(payload, run, notify);
  const [saving, setSaving] = useState(false);
  // 登录、移除后列表会变：每次从最新的数据里取这一家。
  const entry = openId ? payload?.pool.find((item) => item.id === openId) ?? null : null;
  const save = async (changes: CredentialChanges) => {
    if (!entry) return;
    setSaving(true);
    try {
      if (changes.rotation !== undefined && !(await run(`rotation:${entry.id}`, "set_gateway_provider_rotation", { id: entry.id, rotation: changes.rotation }, t("保存分配方式"), true))) return;
      for (const change of changes.settings) {
        if (!(await run(`credential:${entry.id}`, "set_gateway_credential_settings", { id: entry.id, credentialId: change.id, settings: change.settings }, t("保存分配方式"), true))) return;
      }
      onClose();
    } finally {
      setSaving(false);
    }
  };
  const remove = async (item: CredentialView) => {
    if (!entry) return;
    const ok = await appConfirm(tf("从「{0}」移除账号「{1}」？", [maskAccountName(entry.name), maskAccountName(item.label)]), { title: t("移除账号"), confirmText: t("移除"), danger: true });
    if (ok) await run(`account-remove:${entry.id}`, "remove_gateway_provider_account", { id: entry.id, credentialId: item.id }, t("移除账号"));
  };
  return (
    <>
      {entry ? (
        <ProviderCredentialsDialog
          busy={saving}
          entry={entry}
          globalRoundRobin={payload?.gateway.keyPolicy?.mode === "roundRobin"}
          key={entry.id}
          loggingIn={Boolean(entry.account) && pending === entry.account}
          onAddAccount={entry.account ? () => void login(entry.account as AccountKind, undefined, entry.id) : undefined}
          onCancel={onClose}
          onRemoveAccount={entry.account ? (item) => void remove(item) : undefined}
          onSave={(changes) => void save(changes)}
        />
      ) : null}
      {loginDialog}
    </>
  );
}
