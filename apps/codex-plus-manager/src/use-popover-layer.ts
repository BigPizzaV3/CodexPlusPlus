import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import {
  POPOVER_DEFAULT_MAX_HEIGHT,
  isTriggerVisible,
  registerOpenPopover,
  releaseOpenPopover,
  resolvePopoverLayout,
  type PopoverAlign,
  type PopoverLayout,
} from "./popover-layer";
import { useEscapeLayer } from "./use-escape-layer";

/**
 * 下拉弹层统一挂到 body 下的 fixed 层：
 * - 菜单不再被父级 overflow 裁掉（模型列表行、详情滚动区都会裁）；
 * - 用 document 上的 pointerdown 判定外点关闭：WKWebView 里点非可聚焦元素不会触发
 *   blur，光靠 onBlur 根本收不起来，还会多个下拉同时展开；
 * - Escape 关闭并还焦点（走 Esc 栈：只关最上层，不连带关掉外面的弹窗）；滚动/缩放跟随触发器，触发器滚出视口就收起；
 * - 打开时通过 registry 关掉其它弹层，保证同一时刻只展开一个。
 *
 * 用法：把返回的 rootRef 挂到包含触发器的容器上，menuRef + menuStyle 挂到
 * portal 出去的菜单根节点上，菜单本身用 `createPortal(..., document.body)` 渲染。
 */
export function usePopoverLayer({
  open,
  onOpenChange,
  align = "start",
  minWidth = 0,
  menuWidth,
  maxHeight = POPOVER_DEFAULT_MAX_HEIGHT,
  gap,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  align?: PopoverAlign;
  minWidth?: number;
  menuWidth?: number;
  maxHeight?: number;
  gap?: number;
}) {
  const layerId = useId();
  const rootRef = useRef<HTMLDivElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const changeRef = useRef(onOpenChange);
  changeRef.current = onOpenChange;
  const [layout, setLayout] = useState<PopoverLayout | null>(null);
  const layoutRef = useRef<PopoverLayout | null>(null);
  const close = useCallback(() => changeRef.current(false), []);

  const measure = useCallback(() => {
    const trigger = rootRef.current?.getBoundingClientRect();
    const menu = menuRef.current;
    if (!trigger || !menu) return;
    const next = resolvePopoverLayout({
      trigger,
      viewport: { width: window.innerWidth, height: window.innerHeight },
      menuHeight: menu.scrollHeight,
      align,
      minWidth,
      menuWidth,
      maxHeight,
      gap,
    });
    const previous = layoutRef.current;
    if (
      previous &&
      previous.left === next.left &&
      previous.top === next.top &&
      previous.width === next.width &&
      previous.maxHeight === next.maxHeight &&
      previous.placement === next.placement
    ) {
      return;
    }
    layoutRef.current = next;
    setLayout(next);
  }, [align, gap, maxHeight, menuWidth, minWidth]);

  // 每次渲染后校正一次（筛选结果会让菜单变高变矮）；layout effect 在绘制前跑完，
  // 所以打开瞬间不会闪一下旧坐标。
  useLayoutEffect(() => {
    if (!open) {
      if (layoutRef.current) {
        layoutRef.current = null;
        setLayout(null);
      }
      return;
    }
    measure();
  });

  useEscapeLayer(open, () => {
    close();
    rootRef.current?.querySelector<HTMLElement>("button, input")?.focus();
  });

  useEffect(() => {
    if (!open) return;
    registerOpenPopover(layerId, close);
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target as Node | null;
      if (!target) return;
      if (rootRef.current?.contains(target) || menuRef.current?.contains(target)) return;
      close();
    };
    // 键盘 Tab 走焦点离开时也要收；点非可聚焦元素不产生 focusin，由 pointerdown 兜底。
    const handleFocusIn = (event: FocusEvent) => {
      const target = event.target as Node | null;
      if (!target) return;
      if (rootRef.current?.contains(target) || menuRef.current?.contains(target)) return;
      close();
    };
    let frame = 0;
    const follow = () => {
      if (frame) return;
      frame = window.requestAnimationFrame(() => {
        frame = 0;
        const trigger = rootRef.current?.getBoundingClientRect();
        if (!trigger || !isTriggerVisible(trigger, { width: window.innerWidth, height: window.innerHeight }, 4)) {
          close();
          return;
        }
        measure();
      });
    };
    document.addEventListener("pointerdown", handlePointerDown, true);
    document.addEventListener("focusin", handleFocusIn);
    window.addEventListener("resize", follow);
    // capture 监听内层滚动容器的滚动。
    window.addEventListener("scroll", follow, { capture: true, passive: true });
    return () => {
      releaseOpenPopover(layerId);
      document.removeEventListener("pointerdown", handlePointerDown, true);
      document.removeEventListener("focusin", handleFocusIn);
      window.removeEventListener("resize", follow);
      window.removeEventListener("scroll", follow, { capture: true });
      if (frame) window.cancelAnimationFrame(frame);
    };
  }, [close, layerId, measure, open]);

  const menuStyle: CSSProperties = layout
    ? {
        position: "fixed",
        top: layout.top,
        left: layout.left,
        width: layout.width,
        maxHeight: layout.maxHeight,
      }
    : { position: "fixed", top: 0, left: 0, visibility: "hidden" };

  return { rootRef, menuRef, menuStyle };
}
