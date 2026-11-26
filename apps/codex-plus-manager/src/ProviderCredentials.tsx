/**
 * 供应商行上的「账号与分配」/「Key 分配」：Codex「连接与供应商」和 Claude「供应商」共用。
 * 这里只有轻量的判断和行上的文字；弹窗和登录在按需加载的 ProviderCredentialsHost 里。
 */
import { t, tf } from "./i18n";
import { credentialRotationLabel, type GatewayPayload, type GatewayPoolEntry } from "./gateway-model";

/** 这家有什么可管的：登录型账号总能加账号；Key 要有两个以上才有得分。 */
export function hasCredentialSettings(entry: GatewayPoolEntry): boolean {
  return Boolean(entry.account) || (entry.credentials?.length ?? 0) > 1;
}

/** 行菜单里那一项的名字。 */
export function credentialMenuLabel(entry: GatewayPoolEntry): string {
  return entry.account ? t("账号与分配") : t("Key 分配");
}

/** 行上的一句话：「3 个账号 · 额度优先」「2 个 Key · 依次」；只有一个时为空。 */
export function credentialSummary(entry: GatewayPoolEntry, payload: GatewayPayload | null): string {
  const count = entry.credentials?.length ?? 0;
  if (count < 2) return "";
  const rotation = entry.rotation ?? (payload?.gateway.keyPolicy?.mode === "roundRobin" ? "rotate" : "inOrder");
  return [entry.account ? tf("{0} 个账号", [count]) : tf("{0} 个 Key", [count]), t(credentialRotationLabel(rotation))].join(" · ");
}
