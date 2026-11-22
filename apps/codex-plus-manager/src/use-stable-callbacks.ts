import { useRef } from "react";

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Callbacks = Record<string, (...args: any[]) => any>;

/**
 * 造一个引用不变的对象：每个方法转调 `read()` 拿到的同名最新函数。
 * 键在创建时固定，之后新增的键不会出现。
 */
export function createStableCallbacks<T extends Callbacks>(keys: readonly (keyof T)[], read: () => T): T {
  const stable = {} as T;
  for (const key of keys) {
    stable[key] = ((...args: unknown[]) => read()[key](...args)) as T[keyof T];
  }
  return stable;
}

/**
 * 一组回调的稳定版本：对象和方法引用整个生命周期不变，调用时总是执行最新一次渲染传入的函数，
 * 读到的是最新的 state。代替依赖列表很长、容易漏项的 useMemo。
 */
export function useStableCallbacks<T extends Callbacks>(callbacks: T): T {
  const latest = useRef(callbacks);
  latest.current = callbacks;
  const stable = useRef<T | null>(null);
  if (!stable.current) stable.current = createStableCallbacks(Object.keys(callbacks), () => latest.current);
  return stable.current;
}
