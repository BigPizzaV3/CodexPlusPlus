// 「从第三方导入」对话框的数据：把 cc-switch、Magpie 两个来源读回来的列表整理成同一种条目，
// 算默认勾选、默认停在哪个来源、点「导入」时各来源要导哪几条。只做计算，不碰界面和后端。
import type { RelayProtocol } from "./app-types";
import { MAGPIE_UPSTREAM_ID } from "./gateway-model.ts";

export { MAGPIE_UPSTREAM_ID };

export type ThirdPartySource = "ccswitch" | "magpie";

export const THIRD_PARTY_SOURCES: ThirdPartySource[] = ["ccswitch", "magpie"];

/** 来源在界面上的名字（产品名，不翻译）。 */
export const THIRD_PARTY_SOURCE_NAME: Record<ThirdPartySource, string> = { ccswitch: "CC Switch", magpie: "Magpie" };

export type ThirdPartyItem = {
  source: ThirdPartySource;
  /** 条目在来源里的 id（cc-switch 的 `sourceId`、Magpie 的供应商 id）。 */
  id: string;
  name: string;
  baseUrl: string;
  protocol: RelayProtocol;
  /** 已经导入过：不能再勾。 */
  imported: boolean;
  /** 在来源里是关着的（Magpie 的 `off`）；照样能导入。 */
  disabled: boolean;
  hasKey: boolean;
  /**
   * 不是复制一家供应商，而是把这个工具本身当作一个上游（Magpie 的本地网关）：
   * 请求经它转发，用它里面的账号、路由和额度。和「复制」是两种导入方式，一次只能选一种（见 MagpieImportMode）。
   */
  upstream: boolean;
};

/**
 * Magpie 的导入方式，二选一：`upstream` 把 Magpie 本身当作一个上游（只有一条，默认）；`copy` 复制它的供应商（逐条勾选）。
 * 两种一起导会有两条路通向同一批账号，所以一次只做一种；后端也会拒绝两种混在一批里。
 */
export type MagpieImportMode = "copy" | "upstream";

/**
 * 打开对话框时 Magpie 默认用哪种方式：当作上游。那一条已经导入过（没有可导的了）、
 * 或者根本没有这一条（没装 Magpie、旧版后端）时才是复制。
 */
export function defaultMagpieMode(items: ThirdPartyItem[]): MagpieImportMode {
  return items.some((item) => item.source === "magpie" && item.upstream && !item.imported) ? "upstream" : "copy";
}

export type CcsListing = { providers: Array<{ sourceId: string; name: string; baseUrl: string; apiKey: string; protocol: RelayProtocol }>; imported?: string[] };
export type MagpieListing = {
  providers: Array<{ sourceId: string; name: string; baseUrl: string; protocol: RelayProtocol; disabled: boolean; hasKey: boolean; imported: boolean }>;
  gateway?: { sourceId: string; baseUrl: string; imported: boolean } | null;
};

export const itemKey = (item: Pick<ThirdPartyItem, "source" | "id">) => `${item.source}:${item.id}`;

/** 两个来源的条目排成一份；没读到的来源就是没有条目。 */
export function thirdPartyItems(ccs: CcsListing | null, magpie: MagpieListing | null): ThirdPartyItem[] {
  const ccsImported = new Set(ccs?.imported ?? []);
  return [
    ...(ccs?.providers ?? []).map((provider): ThirdPartyItem => ({
      source: "ccswitch",
      id: provider.sourceId,
      name: provider.name,
      baseUrl: provider.baseUrl,
      protocol: provider.protocol,
      imported: ccsImported.has(provider.sourceId),
      disabled: false,
      hasKey: provider.apiKey.trim() !== "",
      upstream: false,
    })),
    // 「把 Magpie 当作一个上游」排在它的各家供应商前面。
    ...(magpie?.gateway ? [{
      source: "magpie" as const,
      id: magpie.gateway.sourceId,
      name: "Magpie",
      baseUrl: magpie.gateway.baseUrl,
      protocol: "responses" as const,
      imported: magpie.gateway.imported,
      disabled: false,
      hasKey: true,
      upstream: true,
    }] : []),
    ...(magpie?.providers ?? []).map((provider): ThirdPartyItem => ({
      source: "magpie",
      id: provider.sourceId,
      name: provider.name,
      baseUrl: provider.baseUrl,
      protocol: provider.protocol,
      imported: provider.imported,
      disabled: provider.disabled,
      hasKey: provider.hasKey,
      upstream: false,
    })),
  ];
}

/** 默认勾上所有还没导入的「复制」条目。「当作上游」那一条不靠勾选，由导入方式决定。 */
export function defaultPicked(items: ThirdPartyItem[]): Set<string> {
  return new Set(items.filter((item) => !item.imported && !item.upstream).map(itemKey));
}

/** 打开时停在哪个来源：先看哪个还有没导入的（Magpie 的「当作上游」也算），其次哪个有条目，都没有就是第一个。 */
export function initialSource(items: ThirdPartyItem[]): ThirdPartySource {
  return THIRD_PARTY_SOURCES.find((source) => items.some((item) => item.source === source && !item.imported))
    ?? THIRD_PARTY_SOURCES.find((source) => items.some((item) => item.source === source))
    ?? THIRD_PARTY_SOURCES[0];
}

/**
 * 点「导入」时各来源要导的 id：只算还没导入的，按列表里的顺序。
 * Magpie 按导入方式二选一：`copy` 导勾着的那些供应商；`upstream` 只导「当作上游」那一条，勾选不算。
 */
export function importPlan(items: ThirdPartyItem[], picked: ReadonlySet<string>, magpieMode: MagpieImportMode): Record<ThirdPartySource, string[]> {
  const fresh = items.filter((item) => !item.imported);
  const chosen = fresh.filter((item) => !item.upstream && picked.has(itemKey(item)));
  return {
    ccswitch: chosen.filter((item) => item.source === "ccswitch").map((item) => item.id),
    magpie: magpieMode === "upstream"
      ? fresh.filter((item) => item.source === "magpie" && item.upstream).map((item) => item.id)
      : chosen.filter((item) => item.source === "magpie").map((item) => item.id),
  };
}

/* —— 共享供应商的「导入」里 Magpie 那一组：同样二选一，用勾选互斥来表达（默认勾哪些见 gateway-model 的 pendingImports） —— */

type PoolPick = { source: string; id: string };
const isMagpieUpstream = (item: PoolPick) => item.source === "magpie" && item.id === MAGPIE_UPSTREAM_ID;
const poolKey = (item: PoolPick) => `${item.source}:${item.id}`;

/**
 * 勾 / 取消一条。勾上「把 Magpie 当作一个上游」会取消 Magpie 的其它条目，勾上 Magpie 的某一家会取消「当作上游」：
 * 两种方式一次只能选一种。别的来源互不影响。
 */
export function togglePoolPick<T extends PoolPick>(picked: ReadonlySet<string>, item: T, candidates: T[]): Set<string> {
  const next = new Set(picked);
  const key = poolKey(item);
  if (next.has(key)) {
    next.delete(key);
    return next;
  }
  next.add(key);
  if (item.source === "magpie") {
    const upstream = isMagpieUpstream(item);
    for (const other of candidates) {
      if (other.source === "magpie" && isMagpieUpstream(other) !== upstream) next.delete(poolKey(other));
    }
  }
  return next;
}

/** 网址只显示主机名；解析不了就去掉协议头和路径。 */
export function urlHost(value: string | undefined): string {
  const text = (value ?? "").trim();
  if (!text) return "";
  try {
    return new URL(text).host;
  } catch {
    return text.replace(/^https?:\/\//, "").replace(/\/.*$/, "");
  }
}
