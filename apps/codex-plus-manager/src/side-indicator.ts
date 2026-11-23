// 侧栏选中底色：一块在条目之间滑动的色块，代替各条目自己原地换底色。
import { useEffect, useLayoutEffect, useRef, type RefObject } from "react";

export type IndicatorRect = { x: number; y: number; width: number; height: number };

/** 滑动时长，与子页面列表展开 / 收起的过渡（cursor-ui.css 的 .cx-side-subnav-wrap）一致。 */
export const SIDE_INDICATOR_MS = 220;

/** 缓出：起步快、收尾慢，观感接近 --motion-ease。 */
export function easeOut(progress: number) {
  const clamped = Math.min(1, Math.max(0, progress));
  return 1 - (1 - clamped) ** 3;
}

/** 色块在 progress（0–1）时的位置：从起点滑向终点。终点每帧重新量，所以列表展开途中也跟得上。 */
export function indicatorFrame(from: IndicatorRect, to: IndicatorRect, progress: number): IndicatorRect {
  const eased = easeOut(progress);
  const mix = (start: number, end: number) => start + (end - start) * eased;
  return { x: mix(from.x, to.x), y: mix(from.y, to.y), width: mix(from.width, to.width), height: mix(from.height, to.height) };
}

/** 当前该铺底色的条目：选中的入口或子页面；展开了子页面的 Agent 行只加粗不铺底，收起的列表里的不算。 */
function findTarget(nav: HTMLElement) {
  const candidates = nav.querySelectorAll<HTMLElement>(".cx-side-item.active:not(.has-subnav), .cx-side-subitem.active");
  for (const element of candidates) {
    if (!element.closest('.cx-side-subnav-wrap[data-open="false"]')) return element;
  }
  return null;
}

/** 条目相对导航滚动内容的位置（色块是导航里的绝对定位元素，跟着内容一起滚）。 */
function measure(nav: HTMLElement, element: HTMLElement): IndicatorRect {
  const base = nav.getBoundingClientRect();
  const rect = element.getBoundingClientRect();
  return {
    x: rect.left - base.left - nav.clientLeft + nav.scrollLeft,
    y: rect.top - base.top - nav.clientTop + nav.scrollTop,
    width: rect.width,
    height: rect.height,
  };
}

/**
 * 让 nav 里的 `.cx-side-indicator` 跟着选中项走。
 * signature 变了（换页、换 Agent、收起 / 展开）就滑过去；其余重渲染和尺寸变化只对齐、不做动画。
 * 导航上的 `data-indicator` 表示色块已接管，样式据此把条目自己的选中底色去掉；脚本没跑起来时条目照旧自己铺底。
 */
export function useSideIndicator(navRef: RefObject<HTMLElement | null>, signature: string) {
  const memo = useRef<{ nav: HTMLElement | null; signature: string | null; rect: IndicatorRect | null; frame: number }>({
    nav: null,
    signature: null,
    rect: null,
    frame: 0,
  });

  useLayoutEffect(() => {
    const nav = navRef.current;
    const pill = nav?.querySelector<HTMLElement>(":scope > .cx-side-indicator");
    const state = memo.current;
    if (!nav || !pill) return;

    const place = (progress: number, from: IndicatorRect | null) => {
      const target = findTarget(nav);
      if (!target) {
        // 切 Agent 时「当前 Agent」先变、页面稍后才变，中间一小段没有选中项：先原地不动，
        // 新选中项出现后从这里滑过去；这一段过完还是没有才淡出。
        if (progress >= 1) pill.style.opacity = "0";
        return;
      }
      const to = measure(nav, target);
      const rect = from ? indicatorFrame(from, to, progress) : to;
      pill.style.transform = `translate3d(${rect.x}px, ${rect.y}px, 0)`;
      pill.style.width = `${rect.width}px`;
      pill.style.height = `${rect.height}px`;
      pill.style.borderRadius = getComputedStyle(target).borderRadius;
      pill.style.opacity = "1";
      state.rect = rect;
    };

    const sameNav = state.nav === nav;
    const changed = !sameNav || state.signature !== signature;
    state.nav = nav;
    state.signature = signature;
    nav.setAttribute("data-indicator", "");
    if (!changed) {
      // 普通重渲染：没有在滑就对齐一下（条目增减、文字变化）。
      if (!state.frame) place(1, null);
      return;
    }

    window.cancelAnimationFrame(state.frame);
    state.frame = 0;
    // 换了一个导航（主导航 ↔ 设置目录）或还没有过选中项：直接出现在目标上，不从旧位置滑过来。
    const from = sameNav ? state.rect : null;
    const reduced = typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (reduced) {
      place(1, null);
      return;
    }
    const started = performance.now();
    // 整段时长都逐帧重量终点：子页面列表这时也在展开 / 收起，条目的位置一直在变。
    const step = (now: number) => {
      const progress = Math.min(1, (now - started) / SIDE_INDICATOR_MS);
      place(progress, from);
      state.frame = progress < 1 ? window.requestAnimationFrame(step) : 0;
    };
    place(0, from);
    state.frame = window.requestAnimationFrame(step);
  });

  // 窗口缩放、显示密度变化：条目位置变了但侧栏没重渲染，这里对齐。
  useEffect(() => {
    const nav = navRef.current;
    if (!nav || typeof ResizeObserver === "undefined") return;
    const state = memo.current;
    const observer = new ResizeObserver(() => {
      const pill = nav.querySelector<HTMLElement>(":scope > .cx-side-indicator");
      const target = findTarget(nav);
      if (state.frame || !pill || !target) return;
      const rect = measure(nav, target);
      pill.style.transform = `translate3d(${rect.x}px, ${rect.y}px, 0)`;
      pill.style.width = `${rect.width}px`;
      pill.style.height = `${rect.height}px`;
      state.rect = rect;
    });
    observer.observe(nav);
    for (const group of nav.querySelectorAll(":scope > .cx-side-group")) observer.observe(group);
    return () => observer.disconnect();
  }, [navRef, signature]);

  useEffect(() => () => window.cancelAnimationFrame(memo.current.frame), []);
}
