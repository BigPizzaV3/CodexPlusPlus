/**
 * Agent 的页面：侧栏只列 Agent，点开后它的全部页面都在内容区顶部的页签里，
 * 按分组（概览 / 供应商 / 增强 / 设置）用细分隔线隔开。
 */

export type PageGroup<R extends string> = { id: string; routes: R[] };

/** 当前 Agent 能看到的分组页面：不属于任何 Agent 的页面（概览）或属于当前 Agent 的页面。 */
export function visibleGroupRoutes<R extends string>(group: PageGroup<R>, toolOf: (route: R) => string | undefined, tool: string): R[] {
  return group.routes.filter((route) => {
    const owner = toolOf(route);
    return !owner || owner === tool;
  });
}

/** 某个 Agent 的页签分组：去掉一页都看不到的分组。 */
export function agentTabGroups<R extends string, G extends PageGroup<R>>(groups: G[], toolOf: (route: R) => string | undefined, tool: string): Array<{ group: G; routes: R[] }> {
  return groups
    .map((group) => ({ group, routes: visibleGroupRoutes(group, toolOf, tool) }))
    .filter((entry) => entry.routes.length > 0);
}

/** 点 Agent 时去哪一页：上次在这个 Agent 里看的页面（仍属于它的话），否则它的落脚页。 */
export function agentTarget<R extends string>(visible: R[], remembered: R | undefined, home: R): R {
  return remembered && visible.includes(remembered) ? remembered : home;
}
