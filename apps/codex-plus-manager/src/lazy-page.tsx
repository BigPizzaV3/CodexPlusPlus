// 不常用的页面按需加载：进到这一页才下载它的代码，管理器启动时少解析一大块。
import { lazy, useEffect, useState, type ComponentType, type LazyExoticComponent } from "react";
import { t } from "@/i18n";

/** 按需加载某个模块里的具名导出组件（React.lazy 只认 default 导出）。 */
export function lazyNamed<M extends { [P in K]: ComponentType<any> }, K extends keyof M>(
  load: () => Promise<M>,
  name: K,
): LazyExoticComponent<M[K]> {
  return lazy(() => load().then((module) => ({ default: module[name] })));
}

/** 页面代码还在加载时的占位：本地加载通常很快，前 150ms 什么都不画，免得一闪；再慢才显示「读取中…」。 */
export function PageFallback() {
  const [shown, setShown] = useState(false);
  useEffect(() => {
    const timer = window.setTimeout(() => setShown(true), 150);
    return () => window.clearTimeout(timer);
  }, []);
  return shown ? <div className="empty" role="status">{t("读取中…")}</div> : null;
}
