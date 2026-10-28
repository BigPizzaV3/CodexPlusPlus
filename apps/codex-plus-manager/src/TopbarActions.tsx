/** 页面级操作按钮的挂载点：渲染进外壳顶栏右侧，而不是在页面里再画一条标题栏。 */
import { useEffect, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";

/**
 * 顶栏和页面在同一次提交里挂载，渲染阶段还查不到顶栏节点，所以挂载后再找。
 * 找不到（例如单独渲染页面的测试）时什么也不画。
 */
export function TopbarActions({ children }: { children: ReactNode }) {
  const [slot, setSlot] = useState<Element | null>(null);
  useEffect(() => {
    setSlot(document.querySelector("[data-topbar-actions]"));
  }, []);
  return slot && children ? createPortal(children, slot) : null;
}
