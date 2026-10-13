/**
 * @description 聚合成员候选纯函数：API 供应商与官方登录账号都可作为成员（供 App.tsx 与单测共用）
 * @author Albert_Luo
 * @email 480199976@qq.com
 * @date 2026-10-12
 */

export type AggregateMemberProfileLike = {
  id: string;
  relayMode: string;
  officialMixApiKey?: boolean;
  aggregate?: unknown;
  baseUrl?: string;
  apiKey?: string;
  authContents?: string;
  protocol?: string;
};

/** 聚合供应商本身不能当成员，也不能有 aggregate 配置残留。 */
export function isAggregateProfile(profile: Pick<AggregateMemberProfileLike, "relayMode" | "aggregate">): boolean {
  return profile.relayMode === "aggregate" || !!profile.aggregate;
}

/** API 成员：Base URL 与 Key 都填写。 */
export function isApiMemberProfile(profile: Pick<AggregateMemberProfileLike, "baseUrl" | "apiKey">): boolean {
  return Boolean((profile.baseUrl ?? "").trim() && (profile.apiKey ?? "").trim());
}

/** authContents 里是否有 ChatGPT OAuth tokens（纯官方登录 profile 的凭据来源）。 */
export function authContentsHaveChatgptTokens(contents: string | undefined): boolean {
  const trimmed = (contents ?? "").trim();
  if (!trimmed) return false;
  try {
    const value = JSON.parse(trimmed) as { tokens?: Record<string, unknown> } | null;
    const tokens = value && typeof value === "object" ? value.tokens : undefined;
    if (!tokens || typeof tokens !== "object") return false;
    return [tokens.access_token, tokens.refresh_token].some(
      (token) => typeof token === "string" && token.trim().length > 0,
    );
  } catch {
    return /"tokens"\s*:/.test(trimmed) && /"(access_token|refresh_token)"\s*:/.test(trimmed);
  }
}

/** 官方登录成员：纯官方 profile 且快照里有 ChatGPT 登录凭据；混合 API Key 走 API 分支。 */
export function isOfficialLoginMemberProfile(profile: AggregateMemberProfileLike): boolean {
  return (
    profile.relayMode === "official"
    && !profile.officialMixApiKey
    && (profile.protocol ?? "responses") === "responses"
    && authContentsHaveChatgptTokens(profile.authContents)
  );
}

/** 可勾选的聚合成员：API 供应商或已完成官方登录的账号。 */
export function aggregateMemberCandidates<T extends AggregateMemberProfileLike>(
  profiles: T[],
  aggregateId: string,
): T[] {
  return profiles.filter(
    (profile) =>
      profile.id !== aggregateId
      && !isAggregateProfile(profile)
      && (isApiMemberProfile(profile) || isOfficialLoginMemberProfile(profile)),
  );
}

/**
 * 默认供应商规范化：只保留仍是当前聚合成员的值，其余（空值、已移出的成员、
 * 配置漂移）一律清空为「未设置」，由请求期落回原有报错。
 */
export function normalizeAggregateDefaultRelayId(
  value: string | null | undefined,
  memberIds: ReadonlySet<string>,
): string {
  const id = (value ?? "").trim();
  return id && memberIds.has(id) ? id : "";
}

/** 成员列表规范化只关心"这家供应商还在不在"，不关心它当前是否可勾选。 */
export type AggregateMemberLike = { profileId: string; weight: number };

/**
 * 聚合成员规范化：**只按"供应商是否还在"决定去留**。
 *
 * 这里必须和"可勾选候选"分开，否则会出事故：
 * `isApiMemberProfile` / `isOfficialLoginMemberProfile` 要求 URL+Key 齐备、官方登录
 * 快照里有 tokens。供应商在编辑中、Key 被临时清空、官方登录快照还没回填时，这些
 * 判定会瞬间返回 false。早先成员规范化直接拿候选集合过滤，于是**任何一次保存**都会
 * 把"一时不就绪"的成员从聚合里静默删掉：用户新增一个供应商、或改了别的设置触发一次
 * 保存，正在用的聚合就报「成员供应商不存在」——看起来就是供应商被删了。
 *
 * 现在的口径：
 *   - 请求里带了成员、且该 profile 确实还在 settings 里 → 保留（不就绪由后端按成员
 *     归因报错，其他成员的模型照常可用）；
 *   - profile 已经不在了（真被删除） → 丢掉；
 *   - 无法判定（没有 profileIds：例如列表页拿到的分页子集） → 保留，宁可留着让用户
 *     自己取消勾选，也不能替用户删配置；
 *   - 新增成员由 UI 从候选里勾选，仍然受候选限制。
 */
export function normalizeAggregateMembers<T extends AggregateMemberLike>(
  members: readonly T[] | null | undefined,
  profileIds: ReadonlySet<string> | null,
): T[] {
  const seen = new Set<string>();
  const normalized: T[] = [];
  for (const member of members ?? []) {
    const profileId = String(member?.profileId ?? "").trim();
    if (!profileId || seen.has(profileId)) continue;
    if (profileIds && !profileIds.has(profileId)) continue;
    seen.add(profileId);
    normalized.push({ ...member, profileId });
  }
  return normalized;
}
