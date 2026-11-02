/**
 * 管理器也「万物皆拓展」：拓展包的 `manager` 页面可以进侧栏（`nav`），也可以顶替管理器的内置页面（`replaces`）。
 * 被顶替的页面上方保留「查看内置页面」，拓展出错时也回得去。
 *
 * 这里只放纯函数（从拓展清单算出侧栏入口、顶替关系，拓展页面能读的数据怎么映射到管理器命令），
 * 由 App.tsx 与 ExtensionManagerPanel 使用。允许的页面 / 数据名与 Rust 侧
 * `MANAGER_REPLACEABLE_PAGES` / `MANAGER_DATA_SOURCES`（crates/codex-plus-core/src/user_scripts.rs）一致，测试守住。
 */

export type ManagerNavGroup = "gateway" | "management" | "tools";

export type ManagerPageSummary = {
  title: string;
  nav?: ManagerNavGroup | null;
  icon?: string | null;
  replaces?: string[];
  data?: string[];
};

export type ManagerPageScript = {
  key: string;
  name: string;
  enabled: boolean;
  error?: string;
  package?: { managerPage?: ManagerPageSummary | null } | null;
};

export type ManagerExtensionPage = {
  key: string;
  title: string;
  nav: ManagerNavGroup | null;
  icon: string | null;
  replaces: string[];
  data: string[];
};

/** 启用中、清单无误的拓展页面。用户脚本总开关关着时没有（和页面侧拓展一样）。 */
export function managerExtensionPages(inventory: { enabled?: boolean; scripts?: ManagerPageScript[] } | null | undefined): ManagerExtensionPage[] {
  if (!inventory || inventory.enabled === false) return [];
  return (inventory.scripts ?? [])
    .filter((script) => script.enabled && !script.error && script.package?.managerPage)
    .map((script) => {
      const page = script.package!.managerPage!;
      return {
        key: script.key,
        title: page.title || script.name,
        nav: page.nav ?? null,
        icon: page.icon ?? null,
        replaces: page.replaces ?? [],
        data: page.data ?? [],
      };
    });
}

/** 管理器路由 → 顶替它的拓展页面（先声明的拓展优先，与内置拓展的 replaces 一致）。 */
export function managerPageOverrides(pages: ManagerExtensionPage[]): Record<string, ManagerExtensionPage> {
  const overrides: Record<string, ManagerExtensionPage> = {};
  for (const page of pages) {
    for (const route of page.replaces) {
      if (!overrides[route]) overrides[route] = page;
    }
  }
  return overrides;
}

/** 某个侧栏分组里的拓展入口。 */
export function managerNavPages(pages: ManagerExtensionPage[], group: ManagerNavGroup): ManagerExtensionPage[] {
  return pages.filter((page) => page.nav === group);
}

/**
 * 拓展页面经 `codexPlusManager.data(name, params)` 能读的数据：名字 → 管理器命令与参数。
 * 都是只读、不含凭据的数据；拓展要在清单 `manager.data` 里声明才能读。
 */
export const MANAGER_DATA_SOURCES: Record<string, { command: string; args: (params: Record<string, unknown>) => Record<string, unknown> }> = {
  requestFlow: { command: "load_request_flow", args: (params) => ({ limit: numberParam(params.limit) }) },
  requestLogs: {
    command: "load_request_logs",
    args: (params) => ({
      limit: numberParam(params.limit),
      sinceMs: numberParam(params.sinceMs),
      agent: stringParam(params.agent),
      relayId: stringParam(params.relayId),
      model: stringParam(params.model),
    }),
  },
  gatewayUsage: { command: "load_gateway_usage", args: (params) => ({ range: stringParam(params.range) ?? "today" }) },
  localSessions: {
    command: "list_local_sessions",
    args: (params) => ({ request: { offset: numberParam(params.offset) ?? 0, limit: numberParam(params.limit) ?? 50 } }),
  },
  gatewayProviders: publicData("gatewayProviders"),
  gatewayRouting: publicData("gatewayRouting"),
  gatewayHealth: publicData("gatewayHealth"),
  gatewayQuota: publicData("gatewayQuota"),
  installedSkills: publicData("installedSkills"),
  installedPlugins: publicData("installedPlugins"),
  agentCollaboration: publicData("agentCollaboration"),
};

function publicData(name: string) {
  return {
    command: "extension_manager_data",
    args: (params: Record<string, unknown>) => ({
      name,
      params: name === "agentCollaboration"
        ? { limit: Math.min(200, Math.max(1, Math.floor(numberParam(params.limit) ?? 50))) }
        : {},
    }),
  };
}

/** 不接受 iframe 指定命令、路径或拓展身份；新增数据出口的身份来自宿主。 */
export function managerDataRequest(name: string, params: Record<string, unknown>, declared: string[], scriptKey: string) {
  if (!Object.hasOwn(MANAGER_DATA_SOURCES, name) || !declared.includes(name)) {
    throw new Error(`清单 manager.data 没有声明 ${name}`);
  }
  const source = MANAGER_DATA_SOURCES[name];
  return {
    command: source.command,
    args: { ...source.args(params), ...(source.command === "extension_manager_data" ? { key: scriptKey } : {}) },
  };
}

function numberParam(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function stringParam(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}
