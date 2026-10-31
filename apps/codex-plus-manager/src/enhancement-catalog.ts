// 增强设置项清单：管理器增强页与注入 Codex 的设置页共用的唯一来源。
//
// 清单文件在 crates/codex-plus-core/assets/enhancement-settings.json：
// - 管理器在这里直接 import，标题/说明/分组经 t() 翻译后渲染；
// - Rust 侧 include_str! 读入，注入页从只读路由 `/settings/catalog` 拿到同一份。
// 新增开关时只改那份 JSON（并在 i18n-en.ts 补英文），不要在任何一侧再写一份文案。
import catalogJson from "../../../crates/codex-plus-core/assets/enhancement-settings.json" with { type: "json" };

/** 在哪一侧显示：manager = 管理器增强页，codex = 注入 Codex 的 Codex++ 设置页。 */
export type EnhancementSurface = "manager" | "codex";
export type EnhancementPlatform = "windows" | "macos" | "linux";
/** bool = 开关；select = 下拉（需带 options）。 */
export type EnhancementControl = "bool" | "select";

export interface EnhancementSettingOption {
  value: string;
  label: string;
}

/**
 * 只在 Codex 设置页出现的行内输入框（例如重试间隔、对话宽度）。
 * 这些值存在 Codex 渲染进程本地，管理器读不到，所以不在管理器显示；
 * 取值范围由注入脚本自己的常量决定，清单只给文案。
 */
export interface EnhancementCodexInput {
  id: string;
  /** 读屏标签（aria-label）。 */
  label: string;
  /** 悬停提示（title）。 */
  hint?: string;
}

export interface EnhancementSettingGroup {
  id: string;
  title: string;
  description?: string;
  /** 管理器把这一组放在哪个标签页；缺省表示管理器另行安排位置（如总开关在工具栏）。 */
  managerTab?: "general" | "stepwise";
  /** 增强总开关关闭后仍生效的补丁组：管理器按总开关状态置顶/置底。 */
  standalone?: boolean;
}

export interface EnhancementSettingItem {
  /** BackendSettings 字段名，也是 `/settings/set` 写入的 key。 */
  key: string;
  /** 这个开关对应的内置拓展 id：开关就是那个内置拓展的启停。 */
  builtin?: string;
  /** 子选项：归在这个内置拓展下，作为它的参数显示。 */
  parent?: string;
  /** 下拉型开关作为内置拓展启停时，打开 / 关闭写入的值。 */
  builtinOn?: string;
  builtinOff?: string;
  /** 改了要重启 Codex 才生效。 */
  restart?: boolean;
  /** 能否被拓展包的 replaces 顶替，缺省 true。 */
  replaceable?: boolean;
  group: string;
  control: EnhancementControl;
  title: string;
  description: string;
  /** 增强总开关开启时改用的说明（独立模式补丁用）。 */
  descriptionWhenEnhancementsOn?: string;
  options?: EnhancementSettingOption[];
  /** 仅这些平台显示；缺省表示全平台。 */
  platforms?: EnhancementPlatform[];
  /** 是否受增强总开关控制，缺省 true。 */
  masterGated?: boolean;
  surfaces: EnhancementSurface[];
  codexInputs?: EnhancementCodexInput[];
}

/** 没有对应设置项的内置拓展（或开关在别处的，如桌宠、听写）。 */
export interface EnhancementBuiltinEntry {
  id: string;
  title: string;
  description: string;
  group: string;
  /** 启停读写的设置字段，嵌套用点号（如 agentPet.enabled）；缺省时启停记在用户脚本配置里。 */
  setting?: string;
  replaceable?: boolean;
}

export interface EnhancementSettingsCatalog {
  version: number;
  groups: EnhancementSettingGroup[];
  items: EnhancementSettingItem[];
  builtins: EnhancementBuiltinEntry[];
}

export const enhancementCatalog = catalogJson as EnhancementSettingsCatalog;

export function enhancementPlatformFromUserAgent(userAgent: string): EnhancementPlatform {
  if (/\bWindows\b/i.test(userAgent)) return "windows";
  if (/\bMacintosh\b/i.test(userAgent)) return "macos";
  return "linux";
}

export function enhancementItem(key: string, catalog = enhancementCatalog): EnhancementSettingItem {
  const item = catalog.items.find((candidate) => candidate.key === key);
  if (!item) throw new Error(`enhancement catalog is missing ${key}`);
  return item;
}

export function enhancementGroup(id: string, catalog = enhancementCatalog): EnhancementSettingGroup {
  const group = catalog.groups.find((candidate) => candidate.id === id);
  if (!group) throw new Error(`enhancement catalog is missing group ${id}`);
  return group;
}

/**
 * 管理器里分组卡片的 DOM id，左侧目录（PageSectionNav）靠它滚动定位。
 * 由清单分组 id 生成：改分组 id 等于改锚点，深链与目录都会跟着变，不要随意改名。
 */
export function enhancementGroupAnchorId(group: Pick<EnhancementSettingGroup, "id">): string {
  return `enhance-group-${group.id}`;
}

export function isEnhancementItemVisible(
  item: EnhancementSettingItem,
  surface: EnhancementSurface,
  platform: EnhancementPlatform,
): boolean {
  return item.surfaces.includes(surface) && (!item.platforms || item.platforms.includes(platform));
}

/** 某个分组在管理器里要渲染的项，保持清单顺序。 */
export function managerGroupItems(
  groupId: string,
  platform: EnhancementPlatform,
  catalog = enhancementCatalog,
): EnhancementSettingItem[] {
  return catalog.items.filter((item) => item.group === groupId && isEnhancementItemVisible(item, "manager", platform));
}

/** 「常用增强」标签里按清单顺序排列的普通分组（独立模式组单独安排位置）。 */
export function managerGeneralGroups(catalog = enhancementCatalog): EnhancementSettingGroup[] {
  return catalog.groups.filter((group) => group.managerTab === "general" && !group.standalone);
}

/**
 * 管理器「内置拓展」页的一张卡片：一个内置拓展。开关来自设置项（item）或清单顶层条目（entry），
 * 子选项（清单里 parent 指向它的设置项）作为参数挂在卡片下面。
 */
export interface BuiltinExtensionCard {
  id: string;
  item?: EnhancementSettingItem;
  entry?: EnhancementBuiltinEntry;
  params: EnhancementSettingItem[];
}

/** 某个分组里的内置拓展卡片，保持清单顺序：设置项里的在前，清单顶层的在后。 */
export function managerBuiltinCards(
  groupId: string,
  platform: EnhancementPlatform,
  catalog = enhancementCatalog,
): BuiltinExtensionCard[] {
  const params = (id: string) => catalog.items.filter((item) => item.parent === id && isEnhancementItemVisible(item, "manager", platform));
  return [
    ...catalog.items
      .filter((item) => item.builtin && item.group === groupId && isEnhancementItemVisible(item, "manager", platform))
      .map((item) => ({ id: item.builtin as string, item, params: params(item.builtin as string) })),
    ...(catalog.builtins ?? [])
      .filter((entry) => entry.group === groupId)
      .map((entry) => ({ id: entry.id, entry, params: params(entry.id) })),
  ];
}

/** 「内置拓展」页按清单顺序列出的分组（常规组放总开关，独立模式组单独安排位置）。 */
export function managerBuiltinGroups(catalog = enhancementCatalog): EnhancementSettingGroup[] {
  return catalog.groups.filter((group) => group.id !== "core" && !group.standalone);
}

/** 设置字段的值；嵌套字段用点号（agentPet.enabled）。 */
export function builtinSettingValue(settings: object, path: string): unknown {
  return path.split(".").reduce<unknown>((value, key) => (value && typeof value === "object" ? (value as Record<string, unknown>)[key] : undefined), settings);
}

/** 写回设置字段，嵌套对象逐层复制，不改原对象。 */
export function withBuiltinSetting<T extends object>(settings: T, path: string, value: unknown): T {
  const [head, ...rest] = path.split(".");
  const current = (settings as Record<string, unknown>)[head];
  const next = rest.length ? withBuiltinSetting((current && typeof current === "object" ? current : {}) as object, rest.join("."), value) : value;
  return { ...settings, [head]: next };
}

export function managerStandaloneGroup(catalog = enhancementCatalog): EnhancementSettingGroup {
  const group = catalog.groups.find((candidate) => candidate.managerTab === "general" && candidate.standalone);
  if (!group) throw new Error("enhancement catalog is missing the standalone group");
  return group;
}

export function isEnhancementItemMasterGated(item: EnhancementSettingItem): boolean {
  return item.masterGated !== false;
}

/** 清单里所有会经 t() 翻译的中文文案，供 i18n 校验使用。 */
export function enhancementCatalogStrings(catalog = enhancementCatalog): string[] {
  const strings = new Set<string>();
  for (const group of catalog.groups) {
    strings.add(group.title);
    if (group.description) strings.add(group.description);
  }
  for (const item of catalog.items) {
    strings.add(item.title);
    strings.add(item.description);
    if (item.descriptionWhenEnhancementsOn) strings.add(item.descriptionWhenEnhancementsOn);
    for (const option of item.options ?? []) strings.add(option.label);
  }
  for (const builtin of catalog.builtins ?? []) {
    strings.add(builtin.title);
    strings.add(builtin.description);
  }
  return [...strings];
}
