/** Claude 页面扩展（与 claude-local 的 extensions::ExtensionInfo、Codex++ 的 claude_page_market::MarketEntry 对应）。 */
export type PageExtension = {
  id: string;
  name: string;
  version: string;
  author: string;
  description: string;
  builtin: boolean;
  enabled: boolean;
  error?: string | null;
};

export type PageMarketEntry = {
  id: string;
  name: string;
  version: string;
  author: string;
  description: string;
  source: "official" | "community";
  repo: string;
  path: string;
  commit: string;
  url: string;
  stars?: number | null;
};

/** 比较 `1.2.3` 这样的版本号：前缀数字逐段比，`-beta` 之类的预发布版排在正式版前面。 */
export function compareVersions(a: string, b: string): number {
  const parse = (value: string) => {
    const [core, pre = ""] = value.split("+")[0].split(/-(.*)/s);
    return { parts: core.split(".").map((part) => Number.parseInt(part, 10) || 0), pre };
  };
  const left = parse(a);
  const right = parse(b);
  for (let index = 0; index < Math.max(left.parts.length, right.parts.length); index++) {
    const diff = (left.parts[index] ?? 0) - (right.parts[index] ?? 0);
    if (diff) return Math.sign(diff);
  }
  if (left.pre === right.pre) return 0;
  if (!left.pre) return 1;
  if (!right.pre) return -1;
  return left.pre < right.pre ? -1 : 1;
}

export type MarketAction = { kind: "install" } | { kind: "update"; from: string } | { kind: "installed" } | { kind: "builtin" };

/** 市场条目的按钮：没装是安装，比装的新是更新，内置的不能从市场换。 */
export function marketAction(entry: PageMarketEntry, installed: PageExtension[]): MarketAction {
  const local = installed.find((item) => item.id === entry.id);
  if (!local) return { kind: "install" };
  if (local.builtin) return { kind: "builtin" };
  if (local.error || compareVersions(entry.version, local.version) > 0) return { kind: "update", from: local.version };
  return { kind: "installed" };
}

export function countUpdates(entries: PageMarketEntry[], installed: PageExtension[]): number {
  return entries.filter((entry) => marketAction(entry, installed).kind === "update").length;
}

export function matchesQuery(query: string, ...fields: Array<string | null | undefined>): boolean {
  const needle = query.trim().toLocaleLowerCase();
  return !needle || fields.filter(Boolean).join(" ").toLocaleLowerCase().includes(needle);
}

export type PageMarketSort = "default" | "name" | "stars";

/** 默认顺序就是市场目录里的顺序（官方在前，社区按星数）；名称按当前语言排；星数只有社区扩展有，官方的排后面。 */
export function sortMarket(entries: PageMarketEntry[], sort: PageMarketSort): PageMarketEntry[] {
  if (sort === "default") return entries;
  const sorted = [...entries];
  if (sort === "name") sorted.sort((a, b) => a.name.localeCompare(b.name));
  else sorted.sort((a, b) => (b.stars ?? -1) - (a.stars ?? -1));
  return sorted;
}
