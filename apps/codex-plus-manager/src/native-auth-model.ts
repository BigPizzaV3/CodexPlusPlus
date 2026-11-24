export type SavedAuthAccount = {
  id: string;
  name: string;
  email: string;
  workspaceId: string;
  source: string;
};

export type NativeAuthState = {
  managed: boolean;
  selection: { mode: "none" } | { mode: "account"; accountRef: string } | null;
  actualAccountRef: string | null;
  actualLabel: string;
  accounts: SavedAuthAccount[];
  drift: string | null;
  restartBlockedReason?: string | null;
  /** 选的是「本地账号」（用 API Key 解锁 ChatGPT 专属入口）。 */
  localLogin?: boolean;
  /** 「本地账号」现在不能用的原因（官方登录供应商等）。 */
  localLoginBlockedReason?: string | null;
};

export type NativeAuthResult = NativeAuthState & { status: string; message: string };

/** 下拉的值："local" 本地账号、"none" 不登录、"account:<id>" ChatGPT 账号、"" 尚未指定。 */
export function nativeAuthSelection(state: NativeAuthState): string {
  if (state.localLogin) return "local";
  if (state.selection?.mode === "none") return "none";
  if (state.selection?.mode === "account") return `account:${state.selection.accountRef}`;
  return "";
}

export function savedAuthLabel(account: SavedAuthAccount): string {
  const label = account.name || account.email || account.id;
  const email = account.email && account.email !== label ? ` · ${account.email}` : "";
  const workspace = account.workspaceId ? ` · ${account.workspaceId}` : "";
  return `${label}${email}${workspace}`;
}

/** 下拉的值换成 `set_codex_login_identity` 的参数。 */
export function loginIdentityArgs(value: string): { kind: "account" | "local" | "none"; accountRef: string | null } {
  if (value.startsWith("account:")) return { kind: "account", accountRef: value.slice("account:".length) };
  return { kind: value === "local" ? "local" : "none", accountRef: null };
}
