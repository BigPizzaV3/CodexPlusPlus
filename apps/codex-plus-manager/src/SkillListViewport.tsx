import { useLayoutEffect, useRef, useState, type ReactNode } from "react";

// 固定行高只挂载可视区附近的条目，目录大小不再决定一次提交的 DOM 数量。
export function SkillListViewport<T>({ items, rowHeight, resetKey, initialScrollTop = 0, onScrollTopChange, className, header, empty, busy, renderRow }: {
  items: T[];
  rowHeight: number;
  resetKey: string;
  initialScrollTop?: number;
  onScrollTopChange?: (top: number) => void;
  className: string;
  header?: ReactNode;
  empty?: ReactNode;
  busy?: boolean;
  renderRow: (item: T) => ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const headerRef = useRef<HTMLDivElement>(null);
  const [viewport, setViewport] = useState({ top: 0, height: 640, header: 0 });
  useLayoutEffect(() => {
    const node = ref.current;
    if (!node) return;
    const measure = () => setViewport({ top: node.scrollTop, height: node.clientHeight, header: headerRef.current?.offsetHeight || 0 });
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, []);
  useLayoutEffect(() => {
    if (ref.current) ref.current.scrollTop = initialScrollTop;
    setViewport((previous) => ({ ...previous, top: ref.current?.scrollTop || 0 }));
  }, [resetKey]);
  const start = Math.min(items.length, Math.max(0, Math.floor((viewport.top - viewport.header) / rowHeight) - 4));
  const end = Math.min(items.length, Math.ceil((viewport.top + viewport.height) / rowHeight) + 4);
  return <div ref={ref} className={className} aria-busy={busy} data-total={items.length}
    onScroll={(event) => { const top = event.currentTarget.scrollTop; setViewport((previous) => ({ ...previous, top })); onScrollTopChange?.(top); }}>
    {header && <div ref={headerRef}>{header}</div>}
    {!!start && <div aria-hidden="true" style={{ height: start * rowHeight }} />}
    {items.slice(start, end).map(renderRow)}
    {end < items.length && <div aria-hidden="true" style={{ height: (items.length - end) * rowHeight }} />}
    {!items.length && empty}
  </div>;
}
