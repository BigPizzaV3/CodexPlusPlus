/**
 * 旧路由 id → 现在的页面。这些页面已经并进别处，但旧 id 仍可能从拓展的 `codexPlusManager.navigate`、
 * `?previewRoute`、旧深链传进来：只在入口处换一次，内部不再出现旧 id。
 *
 * - `maintenance`：安装维护并进了 Codex 设置页；
 * - `about`：关于并进了设置页（滚到版本与更新）；
 * - `relayEnvironment`：旧版代理设置并进了「连接与供应商」页底部的「高级」（打开时展开）；
 * - `recommendations`：推荐内容并进了首页；
 * - `dreamSkin`：皮肤急救并进了增强页；
 * - `overview`：Agent 概览页去掉了，回到 Codex 的「连接与供应商」（主窗口里 `navigate` 会换成当前 Agent 的落脚页）。
 */
export const ROUTE_ALIASES = {
  maintenance: "agentSettings",
  about: "settings",
  relayEnvironment: "relay",
  recommendations: "home",
  dreamSkin: "enhance",
  overview: "relay",
} as const;

export type RouteAlias = keyof typeof ROUTE_ALIASES;

export function isRouteAlias(target: string): target is RouteAlias {
  return Object.prototype.hasOwnProperty.call(ROUTE_ALIASES, target);
}

/** 旧 id 换成现在的页面，其余原样返回。 */
export function resolveRouteAlias<T extends string>(target: T | RouteAlias): T | (typeof ROUTE_ALIASES)[RouteAlias] {
  return isRouteAlias(target) ? ROUTE_ALIASES[target] : (target as T);
}
