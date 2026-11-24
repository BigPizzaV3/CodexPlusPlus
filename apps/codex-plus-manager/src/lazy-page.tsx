// 不常用的页面按需加载：进到这一页才下载它的代码，管理器启动时少解析一大块。
// 启动后空闲时再把这些页面的代码预先读进来（preloadLazyPages），第一次切过去也不用等。
import { lazy, useEffect, useState, type ComponentProps, type ComponentType } from "react";
import { CxPageSkeleton } from "./cx-layout";

/** 登记过的页面代码加载函数，按登记顺序预加载。 */
const loaders: Array<() => Promise<unknown>> = [];

/**
 * 按需加载某个模块里的具名导出组件（React.lazy 只认 default 导出）。
 * 代码已经到手的（预加载过、或之前进过这一页）直接渲染真正的组件，不经 Suspense：
 * React.lazy 即使模块已经在手，第一次渲染也会先挂起一拍，内容区会空一下。
 * 用哪个在挂载时定下来，之后不换，页面不会因为代码「刚好到了」被重新挂载一次。
 */
export function lazyNamed<M extends { [P in K]: ComponentType<any> }, K extends keyof M>(
  load: () => Promise<M>,
  name: K,
): ComponentType<ComponentProps<M[K]>> {
  let loaded: M[K] | null = null;
  let pending: Promise<{ default: M[K] }> | null = null;
  const factory = () => {
    pending ??= load().then(
      (module) => {
        loaded = module[name];
        return { default: module[name] };
      },
      (error: unknown) => {
        // 这一次没读到：下次渲染或下次预加载再试，不把失败记一辈子。
        pending = null;
        throw error;
      },
    );
    return pending;
  };
  loaders.push(factory);
  const Lazy = lazy(factory);
  function LazyPage(props: ComponentProps<M[K]>) {
    const [Component] = useState<ComponentType<any>>(() => loaded ?? Lazy);
    return <Component {...props} />;
  }
  return LazyPage;
}

/** 启动后等这么久才开始预加载，先让首屏画完、首批数据读完。 */
export const PRELOAD_START_MS = 1500;
/** 两个页面之间留的空隙，免得连着解析卡住界面。 */
export const PRELOAD_GAP_MS = 120;

/**
 * 空闲时把按需加载的页面代码逐个读进来，返回取消函数。一次读一个，中间留空隙；
 * 某个读失败不影响后面的，进到那一页时还会再试。
 */
export function preloadLazyPages(): () => void {
  let cancelled = false;
  let timer = 0;
  const next = (index: number) => {
    if (cancelled || index >= loaders.length) return;
    void loaders[index]()
      .catch(() => {})
      .then(() => {
        if (!cancelled) timer = window.setTimeout(() => next(index + 1), PRELOAD_GAP_MS);
      });
  };
  timer = window.setTimeout(() => next(0), PRELOAD_START_MS);
  return () => {
    cancelled = true;
    window.clearTimeout(timer);
  };
}

/**
 * 页面代码还在加载时的占位：本地加载通常很快，前 150ms 什么都不画，免得一闪；
 * 再慢就显示和页面形状接近的灰色占位，而不是一行「读取中…」（那样整页会塌成一行，代码到了再突然撑开）。
 */
export function PageFallback() {
  const [shown, setShown] = useState(false);
  useEffect(() => {
    const timer = window.setTimeout(() => setShown(true), 150);
    return () => window.clearTimeout(timer);
  }, []);
  return shown ? <CxPageSkeleton /> : null;
}
