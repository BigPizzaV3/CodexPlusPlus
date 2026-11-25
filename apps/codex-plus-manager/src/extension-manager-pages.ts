/**
 * 管理器也「万物皆拓展」：拓展包的 `manager` 页面可以进侧栏（`nav`），也可以顶替管理器的内置页面（`replaces`）。
 * 被顶替的页面上方保留「查看内置页面」，拓展出错时也回得去。
 *
 * 这里只放纯函数（从拓展清单算出侧栏入口、顶替关系与顶替冲突，拓展页面能读的数据怎么映射到管理器命令，
 * 以及不带运行状态的清单怎么沿用上一份的运行状态），由 App.tsx 与 ExtensionManagerPanel 使用。允许的页面 / 数据名与 Rust 侧
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
  /** 只含清单本身的问题（空串表示清单没问题）；旧版后端没有这一项。 */
  manifestError?: string;
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

/**
 * 清单本身的问题（坏 JSON、版本不兼容……），空串表示清单能用。
 * `error` 里还混着拓展在 Codex 页面里运行时报的错：拿它判断的话，拓展在 Codex 里一抛错，
 * 它的管理器页面就跟着消失，下一次拿到不带运行状态的清单又回来。所以只看 `manifestError`；
 * 旧版后端没有这一项时退回原来看 `error` 的行为。
 */
export function scriptManifestError(script: { error?: string; manifestError?: string }): string {
  return typeof script.manifestError === "string" ? script.manifestError : script.error ?? "";
}

/**
 * 启用中、清单无误的拓展页面。用户脚本总开关关着时没有（和页面侧拓展一样）。
 * 侧栏入口、⌘K、顶替和本地拓展行的「打开面板」都以这一份为准。
 */
export function managerExtensionPages(inventory: { enabled?: boolean; scripts?: ManagerPageScript[] } | null | undefined): ManagerExtensionPage[] {
  if (!inventory || inventory.enabled === false) return [];
  return (inventory.scripts ?? [])
    .filter((script) => script.enabled && !scriptManifestError(script) && script.package?.managerPage)
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

/** 声明了顶替、但没有生效的路由：被 `by` 这个先声明的拓展页面抢先了。 */
export type ManagerPageConflict = { by: ManagerExtensionPage; routes: string[] };

/**
 * 每个拓展页面「声明了顶替、但被别的拓展抢先」的路由，按抢先者归并：拓展 key → 冲突列表。
 * 规则和 `managerPageOverrides` 是同一条（先声明的优先）；没有冲突的拓展不出现在结果里。
 */
export function managerPageOverrideConflicts(pages: ManagerExtensionPage[]): Record<string, ManagerPageConflict[]> {
  const overrides = managerPageOverrides(pages);
  const conflicts: Record<string, ManagerPageConflict[]> = {};
  for (const page of pages) {
    for (const route of new Set(page.replaces)) {
      const winner = overrides[route];
      if (!winner || winner.key === page.key) continue;
      const list = (conflicts[page.key] ??= []);
      const entry = list.find((item) => item.by.key === winner.key);
      if (entry) entry.routes.push(route);
      else list.push({ by: winner, routes: [route] });
    }
  }
  return conflicts;
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

type RuntimeScript = { key: string; enabled: boolean; status: string; error: string; manifestError?: string };
type RuntimeInventory = { enabled?: boolean; scripts?: RuntimeScript[] };

/** 后端只扫磁盘、没有问 Codex 页面时，启用且清单无误的拓展的兜底状态。 */
const FALLBACK_SCRIPT_STATUS = "not_loaded";

/**
 * 把「不带页面运行状态的新清单」和上一份清单合并，返回要显示的清单。
 *
 * 只有 `refresh_user_script_inventory` / `reload_user_scripts` 会去问 Codex 页面各拓展的运行状态；其余命令
 * （保存设置、切「下次启动跳过拓展」、保存拓展配置……）返回的清单只扫了磁盘，启用的拓展一律是兜底的 `not_loaded`、
 * `error` 为空。直接拿它替换，列表里的「已加载」会全部变成「未加载」。所以这类清单里：
 * 同一个 key、启停没变、新状态是兜底值的拓展，沿用上一份的 `status` / `error`；
 * 清单有错或已停用（`failed` / `disabled`）、新增的、启停变了的，以及上一份里是清单错误或停用状态的，都以新清单为准。
 * 没有要沿用的就原样返回 `next`（引用不变）。带运行状态的清单不要经过这里。
 */
export function keepUserScriptRuntime<T extends RuntimeInventory | null | undefined>(previous: RuntimeInventory | null | undefined, next: T): T {
  if (!next?.scripts?.length || !previous?.scripts?.length) return next;
  const known = new Map(previous.scripts.map((script) => [script.key, script]));
  let changed = false;
  const scripts = next.scripts.map((script) => {
    if (script.status !== FALLBACK_SCRIPT_STATUS || scriptManifestError(script)) return script;
    const before = known.get(script.key);
    if (!before || before.enabled !== script.enabled) return script;
    // 上一份里的「停用」「清单有错」不是运行状态：总开关刚打开、清单刚修好时不能把它们带过来。
    if (before.status === "disabled" || scriptManifestError(before)) return script;
    if (before.status === script.status && before.error === script.error) return script;
    changed = true;
    return { ...script, status: before.status, error: before.error };
  });
  return changed ? ({ ...next, scripts } as T) : next;
}
