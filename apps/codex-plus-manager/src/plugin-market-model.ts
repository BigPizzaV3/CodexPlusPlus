export type MarketSource = "public" | "full";
export type MarketFilter = "all" | "available" | "installed" | "updates" | "auth";

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
};

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
    if (filter === "available" && plugin.installed) return false;
    if (filter === "installed" && !plugin.installed) return false;
    if (filter === "updates" && !plugin.updateAvailable) return false;
    if (filter === "auth" && !plugin.requiresAuth) return false;
    const text = [plugin.name, plugin.displayName, plugin.description, plugin.author, ...plugin.tags].join(" ").toLocaleLowerCase();
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

/** 请求归属页实例。过期响应不能覆盖切换后的索引，安装期间禁止刷新与切源。 */
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

  invalidateCatalog() {
    this.catalogSequence += 1;
    this.catalogPending = false;
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

  canSelectSource(): boolean { return this.active && !this.installPending; }
}
