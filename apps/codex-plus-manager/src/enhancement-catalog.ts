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

export interface EnhancementSettingsCatalog {
  version: number;
  groups: EnhancementSettingGroup[];
  items: EnhancementSettingItem[];
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
  return [...strings];
}
