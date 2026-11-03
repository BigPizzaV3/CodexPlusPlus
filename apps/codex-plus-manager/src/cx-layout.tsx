/**
 * 页面骨架（参照 Cursor 的 Customize / 设置页）。所有页面都用这几块拼，不再各写各的卡片和表格。
 *
 * - `CxToolbar`：搜索框 + 右侧主操作；可选一排筛选胶囊（`CxFilters`）。
 * - `CxSection`：卡片外面的小标题（「已连接 2」），右侧可放次要操作。
 * - `CxCard` + `CxRow`：一张卡片里一行一项，行间细线；左边图标、标题（+ 小标签）、一行说明，右边控件。
 * - `CxAddRow`：卡片样式的「+ 新建…」一行，放在列表末尾。
 * - `CxBanner`：顶部一条提示（带图标、说明、两个按钮），用来替代大段警告。
 * - `CxStats`：一排数字（调用、成功率…），不再用彩色仪表板。
 * - `CxEmpty`：卡片里的空状态。
 *
 * 样式在 cursor-ui.css 的「页面骨架」一节。
 */
import type { ReactNode } from "react";
import { Plus, Search, X } from "lucide-react";
import { t } from "./i18n";

export function CxToolbar({ query, onQuery, placeholder, children }: {
  query?: string;
  onQuery?: (value: string) => void;
  placeholder?: string;
  /** 右侧操作（主按钮放最后）。 */
  children?: ReactNode;
}) {
  return (
    <div className="cx-toolbar">
      {onQuery ? (
        <label className="cx-search">
          <Search aria-hidden="true" />
          <input aria-label={placeholder} onChange={(event) => onQuery(event.currentTarget.value)} placeholder={placeholder} value={query ?? ""} />
          {query ? (
            <button aria-label={t("清除搜索")} className="cx-search-clear" onClick={() => onQuery("")} type="button">
              <X aria-hidden="true" />
            </button>
          ) : null}
        </label>
      ) : <span className="cx-toolbar-spacer" />}
      {children ? <div className="cx-toolbar-actions">{children}</div> : null}
    </div>
  );
}

/** 一排筛选胶囊；`value` 为当前选中项。 */
export function CxFilters<T extends string>({ items, value, onChange, label, trailing }: {
  items: Array<{ id: T; label: ReactNode; count?: number; icon?: ReactNode }>;
  value: T;
  onChange: (value: T) => void;
  label: string;
  trailing?: ReactNode;
}) {
  return (
    <div className="cx-filters">
      <nav aria-label={label} className="tp-tabs cx-pills" role="tablist">
        {items.map((item) => (
          <button aria-selected={item.id === value} data-active={item.id === value} key={item.id} onClick={() => onChange(item.id)} role="tab" type="button">
            {item.icon}
            {item.label}
            {item.count !== undefined ? <span className="cx-pill-count">{item.count}</span> : null}
          </button>
        ))}
      </nav>
      {trailing ? <div className="cx-filters-trailing">{trailing}</div> : null}
    </div>
  );
}

export function CxSection({ title, count, aside, children, id, className = "" }: {
  title?: ReactNode;
  count?: number;
  aside?: ReactNode;
  children: ReactNode;
  id?: string;
  className?: string;
}) {
  return (
    <section className={`cx-section ${className}`.trim()} id={id}>
      {title || aside ? (
        <header className="cx-section-head">
          {title ? <h2>{title}{count !== undefined ? <span className="cx-section-count">{count}</span> : null}</h2> : <span />}
          {aside ? <div className="cx-section-aside">{aside}</div> : null}
        </header>
      ) : null}
      {children}
    </section>
  );
}

export function CxCard({ children, className = "" }: { children: ReactNode; className?: string }) {
  return <div className={`cx-card ${className}`.trim()}>{children}</div>;
}

export function CxRow({ icon, title, badges, detail, children, onClick, muted = false, selected = false, className = "", tooltip }: {
  icon?: ReactNode;
  title: ReactNode;
  badges?: ReactNode;
  detail?: ReactNode;
  /** 右侧控件：开关、按钮、下拉、箭头。 */
  children?: ReactNode;
  /** 整行可点（打开详情）；右侧控件自己的点击要 stopPropagation。 */
  onClick?: () => void;
  muted?: boolean;
  selected?: boolean;
  className?: string;
  tooltip?: string;
}) {
  const body = (
    <>
      {icon ? <span className="cx-row-icon">{icon}</span> : null}
      <span className="cx-row-copy">
        <span className="cx-row-title">
          <strong>{title}</strong>
          {badges}
        </span>
        {detail ? <span className="cx-row-detail">{detail}</span> : null}
      </span>
    </>
  );
  return (
    <div className={`cx-row ${onClick ? "cx-row-clickable" : ""} ${className}`.trim()} data-muted={muted || undefined} data-selected={selected || undefined} title={tooltip}>
      {onClick ? (
        <button className="cx-row-main" onClick={onClick} type="button">{body}</button>
      ) : (
        <div className="cx-row-main">{body}</div>
      )}
      {children ? <div className="cx-row-controls">{children}</div> : null}
    </div>
  );
}

/** 行标题旁的小标签（「User」「HTTP」「已过期」）；tone 只有中性、成功、警告、危险四种。 */
export function CxBadge({ children, tone = "neutral", title }: { children: ReactNode; tone?: "neutral" | "ok" | "warn" | "danger"; title?: string }) {
  return <span className="cx-badge" data-tone={tone} title={title}>{children}</span>;
}

/** 图标右下角的状态点（已连接 = 绿）。 */
export function CxStatusIcon({ children, tone }: { children: ReactNode; tone?: "ok" | "warn" | "danger" | "off" }) {
  return (
    <span className="cx-status-icon">
      {children}
      {tone ? <i data-tone={tone} /> : null}
    </span>
  );
}

export function CxAddRow({ title, detail, onClick, disabled }: { title: ReactNode; detail?: ReactNode; onClick: () => void; disabled?: boolean }) {
  return (
    <button className="cx-add-row" disabled={disabled} onClick={onClick} type="button">
      <span className="cx-add-row-icon"><Plus aria-hidden="true" /></span>
      <span className="cx-row-copy">
        <span className="cx-row-title"><strong>{title}</strong></span>
        {detail ? <span className="cx-row-detail">{detail}</span> : null}
      </span>
    </button>
  );
}

export function CxBanner({ icon, title, detail, children, tone = "info" }: {
  icon?: ReactNode;
  title: ReactNode;
  detail?: ReactNode;
  children?: ReactNode;
  tone?: "info" | "ok" | "warn";
}) {
  return (
    <div className="cx-banner" data-tone={tone}>
      {icon ? <span className="cx-banner-icon">{icon}</span> : null}
      <span className="cx-row-copy">
        <span className="cx-row-title"><strong>{title}</strong></span>
        {detail ? <span className="cx-row-detail">{detail}</span> : null}
      </span>
      {children ? <div className="cx-banner-actions">{children}</div> : null}
    </div>
  );
}

export function CxStats({ items }: { items: Array<{ label: ReactNode; value: ReactNode; hint?: ReactNode; tone?: "ok" | "warn" | "danger" }> }) {
  return (
    <div className="cx-stats" style={{ gridTemplateColumns: `repeat(${Math.max(items.length, 1)}, minmax(0, 1fr))` }}>
      {items.map((item, index) => (
        <div className="cx-stat" key={index}>
          <span className="cx-stat-label">{item.label}</span>
          <strong className="cx-stat-value" data-tone={item.tone}>{item.value}</strong>
          {item.hint ? <span className="cx-stat-hint">{item.hint}</span> : null}
        </div>
      ))}
    </div>
  );
}

export function CxEmpty({ title, detail, children }: { title: ReactNode; detail?: ReactNode; children?: ReactNode }) {
  return (
    <div className="cx-empty">
      <strong>{title}</strong>
      {detail ? <span>{detail}</span> : null}
      {children ? <div className="cx-empty-actions">{children}</div> : null}
    </div>
  );
}
