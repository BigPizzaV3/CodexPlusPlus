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
};

export type NativeAuthResult = NativeAuthState & { status: string; message: string };

export function nativeAuthSelection(state: NativeAuthState): string {
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
