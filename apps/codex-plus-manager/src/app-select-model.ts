/** AppSelect 键盘导航：方向键 / Home / End 在可选项之间移动，跳过禁用项。 */
export type SelectNavKey = "ArrowDown" | "ArrowUp" | "Home" | "End";

export function isSelectNavKey(key: string): key is SelectNavKey {
  return key === "ArrowDown" || key === "ArrowUp" || key === "Home" || key === "End";
}

/**
 * 按键后高亮哪一项（下标）；没有可选项时返回 -1。
 * `current` 为 -1（还没高亮）时，下移落到第一项、上移落到最后一项；到头不回绕，和原生下拉一致。
 */
export function moveActiveOption(disabled: readonly boolean[], current: number, key: SelectNavKey): number {
  const enabled = disabled.map((item, index) => (item ? -1 : index)).filter((index) => index >= 0);
  if (!enabled.length) return -1;
  if (key === "Home") return enabled[0];
  if (key === "End") return enabled[enabled.length - 1];
  if (key === "ArrowDown") return enabled.find((index) => index > current) ?? (current < 0 ? enabled[0] : current);
  const before = enabled.filter((index) => index < current);
  if (current < 0) return enabled[enabled.length - 1];
  return before.length ? before[before.length - 1] : current;
}

/** 打开时先高亮当前选中的那项；它不可选（或没有）时高亮第一个可选项。 */
export function initialActiveOption(disabled: readonly boolean[], selectedIndex: number): number {
  if (selectedIndex >= 0 && !disabled[selectedIndex]) return selectedIndex;
  return disabled.findIndex((item) => !item);
}
