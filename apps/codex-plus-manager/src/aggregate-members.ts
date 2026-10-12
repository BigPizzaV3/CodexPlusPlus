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
