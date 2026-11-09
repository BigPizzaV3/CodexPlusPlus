/**
 * 设置在管理器之外被改了（Codex 设置页、网关接入）时，把管理器里尚未保存的编辑叠到最新设置上。
 *
 * - 没有本地编辑：结果就是最新设置，界面立即反映外部改动；
 * - 有本地编辑：只保留与上次读到的值不同的字段，其余取最新值；
 * - revision 一律取最新，之后的自动保存不会撞上 settings_revision_conflict。
 *
 * 三个参数应先经过同一套规范化，否则规范化差异会被误判成本地编辑。
 */
export function rebaseSettingsEdits<T extends object>(form: T, previous: T, latest: T): T {
  const merged = { ...latest } as Record<string, unknown>;
  const local = form as Record<string, unknown>;
  const base = previous as Record<string, unknown>;
  for (const key of Object.keys(local)) {
    if (key === "revision") continue;
    if (JSON.stringify(local[key]) !== JSON.stringify(base[key])) merged[key] = local[key];
  }
  return merged as T;
}

type AuthAwareProfile = { id?: unknown; authContents?: unknown } & Record<string, unknown>;
type AuthAwareSettings = { relayProfiles?: AuthAwareProfile[] } & Record<string, unknown>;

/**
 * 保存撞上 settings_revision_conflict 自动重试前的凭据保护：
 * 后台刷新登录 token 只会改 relayProfiles[].authContents。rebase 是顶层键粒度，
 * 用户只要动过供应商列表，整个数组都会取用户旧快照，把已轮换的 refresh token
 * 写回去。这里按 profile id 细化：外部只动了 authContents、而用户没有自己改
 * authContents 的 profile，采用外部最新 token；用户粘贴过新凭据的 profile 不动。
 */
export function mergeRotatedAuthContents<T extends AuthAwareSettings>(
  merged: T,
  previous: T,
  latest: T,
): T {
  const mergedProfiles = merged.relayProfiles;
  const previousProfiles = previous.relayProfiles;
  const latestProfiles = latest.relayProfiles;
  if (
    !Array.isArray(mergedProfiles)
    || !Array.isArray(previousProfiles)
    || !Array.isArray(latestProfiles)
  ) {
    return merged;
  }
  const baseById = new Map(
    previousProfiles
      .filter((profile) => typeof profile?.id === "string")
      .map((profile) => [profile.id as string, profile] as const),
  );
  const latestById = new Map(
    latestProfiles
      .filter((profile) => typeof profile?.id === "string")
      .map((profile) => [profile.id as string, profile] as const),
  );
  let changed = false;
  const nextProfiles = mergedProfiles.map((profile) => {
    if (typeof profile?.id !== "string") return profile;
    const base = baseById.get(profile.id);
    const fresh = latestById.get(profile.id);
    if (!base || !fresh) return profile;
    if (profile.authContents === base.authContents && fresh.authContents !== base.authContents) {
      changed = true;
      return { ...profile, authContents: fresh.authContents };
    }
    return profile;
  });
  return changed ? { ...merged, relayProfiles: nextProfiles } : merged;
}
