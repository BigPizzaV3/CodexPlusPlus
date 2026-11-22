export type MarketSource = "public" | "full";
export type MarketFilter = "all" | "installed" | "updates";

export type MarketPlugin = {
  id: string;
  name: string;
  displayName: string;
  description: string;
  version: string;
  author: string;
  tags: string[];
  license: string;
  path: string;
  tree: string;
  bytes: number;
  skills: number;
  requiresAuth: boolean;
  /** 带 ChatGPT 托管的连接器 / 带可在本机登录的 MCP 服务。旧索引没有这两项。 */
  hasApps?: boolean;
  hasMcp?: boolean;
  /** 包清单里的分类、一句话简介和官方目录的图标（只有 https 地址）。旧索引没有。 */
  category?: string;
  shortDescription?: string;
  iconUrl?: string;
  iconUrlDark?: string;
  installed: boolean;
  installedVersion: string;
  updateAvailable: boolean;
};

export type MarketResult = {
  status: string;
  message: string;
  source: MarketSource;
  repository: string;
  total: number;
  plugins: MarketPlugin[];
  cached?: boolean;
  /** 索引的快照日期：目录是缓存仓库的快照，不是官方实时目录。 */
  updatedAt?: string;
  /** 官方目录的分组，按官方顺序。旧索引没有。 */
  sections?: MarketSection[];
};

export type MarketSection = {
  slug: string;
  /** 语言 → 标题，至少有 `en`。 */
  title: Record<string, string>;
  /** 人工编排的合集（热门、新品推荐…）：成员同时还在各自的分类里。 */
  curated: boolean;
  plugins: string[];
};

/** 页面上的一组：标题已按语言选好。 */
export type MarketShelf = { slug: string; title: string; plugins: MarketPlugin[] };

const shelfKey = (text: string) => text.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");

/**
 * 按官方目录分组，和 Codex 插件页的排法一致（62-plugin-market-adapter 的 codexPlusPluginNativeArrange）：
 * 合集照列成员；分类里一个插件只列一次；目录没列到的按包里的分类归组，认不出的进「其他」。
 * 没有分组的旧索引返回空，页面直接平铺。
 */
export function pluginMarketShelves(plugins: MarketPlugin[], sections: MarketSection[], language: "zh" | "en"): MarketShelf[] {
  if (!sections.length) return [];
  const byId = new Map(plugins.map((plugin) => [plugin.id, plugin]));
  const shelves: MarketShelf[] = sections.map((section) => ({
    slug: section.slug,
    title: (language === "zh" ? section.title.zh : undefined) || section.title.en || section.slug,
    plugins: [],
  }));
  const rank = new Map<string, number>();
  sections.forEach((section, index) => {
    for (const key of [section.slug, section.title.en]) if (key) rank.set(shelfKey(key), index);
  });
  const placed = new Set<string>();
  sections.forEach((section, index) => {
    for (const id of section.plugins) {
      const plugin = byId.get(id);
      if (!plugin) continue;
      if (section.curated) shelves[index].plugins.push(plugin);
      else if (!placed.has(id)) {
        placed.add(id);
        shelves[index].plugins.push(plugin);
      }
    }
  });
  let other = rank.get("other");
  for (const plugin of plugins) {
    if (placed.has(plugin.id)) continue;
    let index = rank.get(shelfKey(plugin.category || "Other")) ?? other;
    if (index === undefined) {
      index = other = shelves.push({ slug: "other", title: language === "zh" ? "其他" : "Other", plugins: [] }) - 1;
    }
    shelves[index].plugins.push(plugin);
  }
  return shelves.filter((shelf) => shelf.plugins.length > 0);
}

/** 连接器怎么连：mcp 在 Codex 里登录；hosted 要 ChatGPT 登录；unknown 是旧索引，只知道带连接器。 */
export type MarketConnector = "none" | "mcp" | "hosted" | "unknown";

export function pluginMarketConnector(plugin: MarketPlugin): MarketConnector {
  if (plugin.hasMcp) return "mcp";
  if (plugin.hasApps) return "hosted";
  return plugin.requiresAuth ? "unknown" : "none";
}

export type MarketCatalogs = Partial<Record<MarketSource, MarketResult>>;

export function isPluginMarketSuccess(status: string): boolean {
  return status === "ok" || status === "success" || status === "warning";
}

export function mergePluginMarketCatalog(catalogs: MarketCatalogs, source: MarketSource, result: MarketResult): MarketCatalogs {
  if (!isPluginMarketSuccess(result.status) || result.source !== source) return catalogs;
  return { ...catalogs, [source]: result };
}

export function filterMarketPlugins(plugins: MarketPlugin[], query: string, filter: MarketFilter): MarketPlugin[] {
  const terms = query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  return plugins.filter((plugin) => {
    if (filter === "installed" && !plugin.installed) return false;
    if (filter === "updates" && !plugin.updateAvailable) return false;
    const text = [plugin.name, plugin.displayName, plugin.description, plugin.shortDescription ?? "", plugin.author, ...plugin.tags].join(" ").toLocaleLowerCase();
    return terms.every((term) => text.includes(term));
  });
}

export function pluginMarketPage(plugins: MarketPlugin[], page: number, pageSize = 50) {
  const totalPages = Math.max(1, Math.ceil(plugins.length / pageSize));
  const currentPage = Math.max(0, Math.min(Number.isFinite(page) ? Math.floor(page) : 0, totalPages - 1));
  return {
    totalPages,
    currentPage,
    visible: plugins.slice(currentPage * pageSize, (currentPage + 1) * pageSize),
  };
}

export function applyPluginInstallation(catalogs: MarketCatalogs, source: MarketSource, installed: MarketPlugin): MarketCatalogs {
  const catalog = catalogs[source];
  if (!catalog || !catalog.plugins.some((plugin) => plugin.id === installed.id)) return catalogs;
  // 同名变体和两个来源的安装实例互相独立，只改本次来源里的精确 ID。
  return {
    ...catalogs,
    [source]: {
      ...catalog,
      plugins: catalog.plugins.map((plugin) => plugin.id === installed.id ? {
        ...plugin,
        installed: installed.installed,
        installedVersion: installed.installedVersion,
        updateAvailable: installed.updateAvailable,
      } : plugin),
    },
  };
}

/** 请求归属页实例。过期响应不能覆盖新的索引，安装期间禁止刷新。 */
export class PluginMarketRequests {
  private active = false;
  private catalogSequence = 0;
  private installSequence = 0;
  private catalogPending = false;
  private installPending = false;

  activate() { this.active = true; }

  dispose() {
    this.active = false;
    this.catalogSequence += 1;
    this.installSequence += 1;
    this.catalogPending = false;
    this.installPending = false;
  }

  beginCatalog(): number | null {
    if (!this.active || this.installPending) return null;
    this.catalogPending = true;
    return ++this.catalogSequence;
  }

  isCurrentCatalog(sequence: number): boolean {
    return this.active && this.catalogPending && sequence === this.catalogSequence;
  }

  finishCatalog(sequence: number): boolean {
    if (!this.isCurrentCatalog(sequence)) return false;
    this.catalogPending = false;
    return true;
  }

  beginInstall(): number | null {
    if (!this.active || this.catalogPending || this.installPending) return null;
    this.installPending = true;
    return ++this.installSequence;
  }

  isCurrentInstall(sequence: number): boolean {
    return this.active && this.installPending && sequence === this.installSequence;
  }

  finishInstall(sequence: number): boolean {
    if (!this.isCurrentInstall(sequence)) return false;
    this.installPending = false;
    return true;
  }
}
