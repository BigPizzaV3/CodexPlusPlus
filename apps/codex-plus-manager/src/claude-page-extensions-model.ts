import type { ExtensionConfigField } from "./extension-config-schema.ts";

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
  /** manifest：页面扩展（js / css）；codexpp：Codex++ 拓展包（带页面接口、配置、后台）。老版本没有这个字段，按 manifest。 */
  format?: "manifest" | "codexpp";
  /** page / styles / backend / mods / codexOnly */
  capabilities?: string[];
  notes?: string[];
  configSchema?: ExtensionConfigField[];
  /** 生效配置，密钥项已打码。 */
  config?: Record<string, unknown> | null;
  runtime?: PageRuntime | null;
};

/** Claude 页面上次加载一个拓展包的结果。 */
export type PageRuntime = { status: string; error: string; unsupported: string[]; loadedAt: string };

/**
 * 一个拓展包在 Claude 页面里的运行情况（给列表那一行用）。页面扩展、没启用的、清单有误的不显示。
 * `kind`：pending 还没在页面里跑过，failed 没加载上，error 加载了但运行中出错，partial 加载了但用了 Claude 里还没有的接口，loaded 正常。
 */
export function pageRuntimeSummary(item: PageExtension): { kind: "pending" | "failed" | "error" | "partial" | "loaded"; detail: string } | null {
  if (item.format !== "codexpp" || !item.enabled || item.error) return null;
  const runtime = item.runtime;
  if (!runtime || (runtime.status !== "loaded" && runtime.status !== "failed")) return { kind: "pending", detail: "" };
  const firstLine = (text: string) => text.split("\n")[0].slice(0, 300);
  if (runtime.status === "failed") return { kind: "failed", detail: firstLine(runtime.error) };
  if (runtime.error) return { kind: "error", detail: firstLine(runtime.error) };
  if (runtime.unsupported.length) return { kind: "partial", detail: runtime.unsupported.join("、") };
  return { kind: "loaded", detail: "" };
}

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
