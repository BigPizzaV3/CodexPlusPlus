import type { KeyboardEvent } from "react";

/** 方向键要移到哪个页签：左右（或上下）循环移动，Home / End 到头尾；别的键返回 null。 */
export function nextTabIndex(key: string, current: number, count: number): number | null {
  if (count <= 0) return null;
  switch (key) {
    case "ArrowRight":
    case "ArrowDown":
      return (current + 1) % count;
    case "ArrowLeft":
    case "ArrowUp":
      return (current - 1 + count) % count;
    case "Home":
      return 0;
    case "End":
      return count - 1;
    default:
      return null;
  }
}

/**
 * 挂在 role="tablist" 上：方向键 / Home / End 在页签间移动焦点并直接切过去（自动激活），
 * 和 WAI-ARIA 页签的键盘约定一致。页签本身仍是普通按钮，点击照旧。
 */
export function onTabListKeyDown(event: KeyboardEvent<HTMLElement>) {
  const tabs = [...event.currentTarget.querySelectorAll<HTMLElement>('[role="tab"]:not([disabled])')];
  const current = tabs.findIndex((tab) => tab === document.activeElement);
  if (current < 0) return;
  const next = nextTabIndex(event.key, current, tabs.length);
  if (next === null) return;
  event.preventDefault();
  tabs[next].focus();
  tabs[next].click();
}
