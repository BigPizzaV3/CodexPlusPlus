import { useEffect, useRef } from "react";

export type PollingOptions = {
  /** 关掉时不轮询（例如页面不在前台、开关没开）。 */
  enabled?: boolean;
  /** 挂上时先跑一次；默认是。 */
  immediate?: boolean;
};

/**
 * 按间隔静默轮询：上一轮跑完才排下一轮，窗口不可见（最小化、藏到托盘）时跳过，
 * 重新可见时立刻补一轮。任务抛错只吞掉，保留上一轮的数据。
 */
export function usePolling(task: () => unknown, intervalMs: number, { enabled = true, immediate = true }: PollingOptions = {}) {
  const taskRef = useRef(task);
  taskRef.current = task;
  useEffect(() => {
    if (!enabled) return;
    let cancelled = false;
    let running = false;
    let timer: number | undefined;
    const run = async () => {
      if (running || cancelled || document.visibilityState !== "visible") return;
      running = true;
      try {
        await taskRef.current();
      } catch {
        // 静默轮询：失败时等下一轮。
      } finally {
        running = false;
      }
    };
    const schedule = () => {
      timer = window.setTimeout(async () => {
        await run();
        if (!cancelled) schedule();
      }, intervalMs);
    };
    const onVisibility = () => {
      if (document.visibilityState === "visible") void run();
    };
    if (immediate) void run();
    schedule();
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [enabled, intervalMs, immediate]);
}

/** 轮询回来的数据和上一份一样时沿用旧引用，免得每轮都触发重渲染：`setX((prev) => keepIfSame(prev, next))`。 */
export function keepIfSame<T>(previous: T, next: T): T {
  if (previous === next) return previous;
  try {
    return JSON.stringify(previous) === JSON.stringify(next) ? previous : next;
  } catch {
    return next;
  }
}
