/**
 * 弹层（下拉菜单）定位与生命周期：纯逻辑，方便用 node:test 覆盖。
 *
 * 背景：Manager 里的下拉都长在滚动容器内（`.relay-detail-body`、`.mg-content`、
 * `.relay-model-row-editor` 等），父级一旦有 overflow（`overflow-x: auto` 会同时把
 * y 轴算成 auto），绝对定位的菜单就会被裁掉。所以菜单统一 portal 到 body 并用
 * fixed 定位：这里只算坐标、翻转、限高，以及维护「同一时刻只展开一个弹层」。
 */

export type PopoverRect = {
  top: number;
  left: number;
  right: number;
  bottom: number;
  width: number;
  height: number;
};

export type PopoverViewport = { width: number; height: number };

/** start = 菜单左边缘对齐触发器；end = 菜单右边缘对齐触发器。 */
export type PopoverAlign = "start" | "end";

export type PopoverPlacement = "top" | "bottom";

export type PopoverLayout = {
  left: number;
  top: number;
  width: number;
  maxHeight: number;
  placement: PopoverPlacement;
};

export type PopoverLayoutInput = {
  trigger: PopoverRect;
  viewport: PopoverViewport;
  /** 菜单内容自然高度（滚动高度），0 表示还没量到。 */
  menuHeight: number;
  align?: PopoverAlign;
  /** 菜单期望宽度；不传时跟触发器同宽（原生 select 的观感）。 */
  menuWidth?: number;
  /** 期望最小宽度，宽度不足时撑到此值（最终仍会被视口夹紧）。 */
  minWidth?: number;
  /** 期望最大高度。 */
  maxHeight?: number;
  /** 触发器与菜单的间距。 */
  gap?: number;
  /** 与视口边缘的安全距离。 */
  edge?: number;
};

export const POPOVER_DEFAULT_MAX_HEIGHT = 320;
export const POPOVER_DEFAULT_GAP = 6;
export const POPOVER_DEFAULT_EDGE = 8;

function clamp(value: number, min: number, max: number) {
  if (Number.isNaN(value)) return min;
  return Math.min(Math.max(value, min), Math.max(min, max));
}

/**
 * 计算 fixed 定位弹层的位置：
 * - 默认贴在触发器下方；下方放不下且上方更宽松时翻到上方；
 * - 两侧都放不下时选空间更大的一侧，并压缩 maxHeight（菜单内部滚动）；
 * - 水平方向按 align 对齐后再夹进视口。
 */
export function resolvePopoverLayout({
  trigger,
  viewport,
  menuHeight,
  align = "start",
  menuWidth,
  minWidth = 0,
  maxHeight = POPOVER_DEFAULT_MAX_HEIGHT,
  gap = POPOVER_DEFAULT_GAP,
  edge = POPOVER_DEFAULT_EDGE,
}: PopoverLayoutInput): PopoverLayout {
  const usableWidth = Math.max(0, viewport.width - edge * 2);
  const desiredWidth = Math.max(minWidth, menuWidth ?? trigger.width);
  const width = Math.round(clamp(desiredWidth, 0, usableWidth));

  const anchoredLeft = align === "end" ? trigger.right - width : trigger.left;
  const left = Math.round(clamp(anchoredLeft, edge, Math.max(edge, viewport.width - edge - width)));

  const heightLimit = Math.max(0, maxHeight);
  const desiredHeight = Math.min(menuHeight > 0 ? menuHeight : heightLimit, heightLimit);
  const spaceBelow = viewport.height - edge - (trigger.bottom + gap);
  const spaceAbove = trigger.top - gap - edge;
  const placement: PopoverPlacement =
    spaceBelow >= desiredHeight || spaceBelow >= spaceAbove ? "bottom" : "top";
  const available = Math.max(0, placement === "bottom" ? spaceBelow : spaceAbove);
  const finalHeight = Math.max(0, Math.min(desiredHeight, available));
  const anchoredTop = placement === "bottom" ? trigger.bottom + gap : trigger.top - gap - finalHeight;
  const top = Math.round(
    clamp(anchoredTop, edge, Math.max(edge, viewport.height - edge - finalHeight)),
  );

  return { left, top, width, maxHeight: Math.round(finalHeight), placement };
}

export type AnchoredMenuLayout = {
  /** `bottom`：贴在触发按钮下方；`top`：下方不够、已向上展开。 */
  placement: PopoverPlacement;
  top: number;
  right: number;
  maxHeight: number;
};

export const ANCHORED_MENU_DEFAULT_GAP = 8;
export const ANCHORED_MENU_DEFAULT_EDGE = 12;
/**
 * 菜单至少要留出的可用高度。低于这个值就不再「贴着触发按钮」了：
 * 宁可压住按钮，也不能把高度算成 0 让整个菜单消失（#2648 review）。
 */
export const ANCHORED_MENU_MIN_HEIGHT = 240;

/**
 * 贴在触发按钮下方、右边缘对齐的 fixed 菜单的坐标与限高。
 *
 * 这类菜单（如「从第三方导入」）内容长度不可预期（cc-switch 里有多少供应商就有多少行），
 * 所以关键是**把触发按钮自身的 top 偏移从视口高度里扣掉**：只写
 * `max-height: calc(100vh - 24px)` 时，菜单越靠下、底部溢出越多，最后一排按钮
 * （导入 / 刷新列表）会掉到窗口外面——issue #1906 的「点了没反应」就是这么来的。
 *
 * 但只扣 top 还不够：触发按钮贴着窗口底部时会扣成 0 高，菜单直接消失。所以这里
 * 和 `resolvePopoverLayout` 同口径——下方连最小可用高度都放不下、且上方更宽裕时
 * 向上展开；两侧都不宽裕时保留最小可用高度，并整体夹进视口。
 */
export function resolveAnchoredMenuLayout(
  trigger: PopoverRect,
  viewport: PopoverViewport,
  gap = ANCHORED_MENU_DEFAULT_GAP,
  edge = ANCHORED_MENU_DEFAULT_EDGE,
): AnchoredMenuLayout {
  const right = Math.round(Math.max(edge, viewport.width - trigger.right));
  const spaceBelow = viewport.height - edge - (trigger.bottom + gap);
  const spaceAbove = trigger.top - edge - gap;
  const placement: PopoverPlacement =
    spaceBelow >= ANCHORED_MENU_MIN_HEIGHT || spaceBelow >= spaceAbove ? "bottom" : "top";
  const available = Math.max(0, placement === "bottom" ? spaceBelow : spaceAbove);
  // 上限永远不超过视口可用高度，下限保底到 ANCHORED_MENU_MIN_HEIGHT。
  const maxHeight = Math.round(
    Math.min(
      Math.max(available, ANCHORED_MENU_MIN_HEIGHT),
      Math.max(0, viewport.height - edge * 2),
    ),
  );
  const preferredTop =
    placement === "bottom" ? trigger.bottom + gap : trigger.top - gap - maxHeight;
  const top = Math.round(
    Math.min(
      Math.max(preferredTop, edge),
      Math.max(edge, viewport.height - edge - maxHeight),
    ),
  );
  return { placement, top, right, maxHeight };
}

/** 触发器（几乎）滚出视口时弹层就该收起来，避免菜单挂在看不到的地方。 */
export function isTriggerVisible(
  trigger: PopoverRect,
  viewport: PopoverViewport,
  edge = 0,
): boolean {
  if (trigger.width <= 0 || trigger.height <= 0) return false;
  return (
    trigger.bottom > edge &&
    trigger.top < viewport.height - edge &&
    trigger.right > edge &&
    trigger.left < viewport.width - edge
  );
}

/** 已打开的弹层：id -> 强制关闭回调。 */
const openPopovers = new Map<string, () => void>();

/**
 * 打开弹层前先关掉其它弹层：blur 在 WKWebView 里点非可聚焦元素不触发，
 * 光靠 blur 会出现「多个下拉同时展开」。这里做一次兜底的单开约束。
 */
export function registerOpenPopover(id: string, close: () => void): void {
  // 先摘掉旧登记再回调：被关掉的弹层自己也会 release，避免互相删到新记录。
  const others = [...openPopovers.entries()].filter(([otherId]) => otherId !== id);
  openPopovers.clear();
  for (const [, closeOther] of others) closeOther();
  openPopovers.set(id, close);
}

export function releaseOpenPopover(id: string): void {
  openPopovers.delete(id);
}

/** 测试用：当前登记在案的弹层数量。 */
export function openPopoverCount(): number {
  return openPopovers.size;
}

/** 测试用：清空登记表。 */
export function resetOpenPopovers(): void {
  openPopovers.clear();
}
