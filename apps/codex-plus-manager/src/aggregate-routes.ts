/**
 * @description 聚合供应商路由规则纯函数：priority 清洗、规范化、校验（供 App.tsx 与单测共用）
 * @author Albert_Luo
 * @email 480199976@qq.com
 * @date 2026-08-05
 */

export type AggregateRouteLike =
  | { pattern: string; profileId: string; priority: number }
  | { pattern: string; relayId: string; priority: number };

export type AggregateRouteNormalized = {
  pattern: string;
  profileId: string;
  priority: number;
};

export type AggregateRouteValidationIssue =
  | { code: "emptyPattern"; pattern: string }
  | { code: "invalidPriority"; pattern: string }
  | { code: "notMember"; pattern: string };

export type NormalizeAggregateRoutesOptions = {
  /** 只保留 pattern trim 后非空的规则（默认 false：保留空 pattern 供校验提示） */
  dropEmptyPattern?: boolean;
  /** 提供时，只保留目标 profileId 在成员集合中的规则 */
  memberIds?: ReadonlySet<string>;
  /** 是否清洗 priority（默认 true，与保存方向一致） */
  clampPriority?: boolean;
};

export function clampAggregateRoutePriority(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(999, Math.round(value)));
}

export function normalizeAggregateRoutes(
  routes: AggregateRouteLike[],
  options: NormalizeAggregateRoutesOptions = {},
): AggregateRouteNormalized[] {
  const { dropEmptyPattern = false, memberIds, clampPriority = true } = options;
  return routes
    .map((route) => ({
      pattern: route.pattern.trim(),
      profileId: "profileId" in route ? route.profileId : route.relayId,
      priority: clampPriority ? clampAggregateRoutePriority(route.priority) : route.priority,
    }))
    .filter((route) => {
      if (dropEmptyPattern && !route.pattern) return false;
      if (memberIds && !memberIds.has(route.profileId)) return false;
      return true;
    });
}

export function validateAggregateRoutes(
  routes: AggregateRouteLike[],
  memberIds: ReadonlySet<string>,
): AggregateRouteValidationIssue[] | null {
  const issues: AggregateRouteValidationIssue[] = [];
  for (const route of routes) {
    const pattern = route.pattern.trim();
    const profileId = "profileId" in route ? route.profileId : route.relayId;
    if (!pattern && !profileId.trim()) continue;
    if (!pattern) {
      issues.push({ code: "emptyPattern", pattern: "" });
      continue;
    }
    if (!Number.isInteger(route.priority) || route.priority < 0 || route.priority > 999) {
      issues.push({ code: "invalidPriority", pattern });
      continue;
    }
    if (!memberIds.has(profileId)) {
      issues.push({ code: "notMember", pattern });
    }
  }
  return issues.length ? issues : null;
}
