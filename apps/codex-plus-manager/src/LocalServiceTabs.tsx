/** 「本地服务」页顶部的四个页签：共享供应商、路由、Agent 接入、设置。
 * 每个页签是一条路由（见 routes.ts 的 localServiceRoutes），侧栏里只有一个「本地服务」入口，和「用量」的做法一样。 */
import { t } from "./i18n";
import { localServiceRoutes, routeTitle, type Route } from "./routes";
import { onTabListKeyDown } from "./tablist-keys";

export function LocalServiceTabs({ value, onChange }: { value: Route; onChange: (next: Route) => void }) {
  return (
    <nav aria-label={t("本地服务")} className="tp-tabs usage-tabs" onKeyDown={onTabListKeyDown} role="tablist">
      {localServiceRoutes.map((id) => (
        <button aria-selected={value === id} data-active={value === id} key={id} onClick={() => onChange(id)} role="tab" tabIndex={value === id ? 0 : -1} type="button">
          {routeTitle(id)}
        </button>
      ))}
    </nav>
  );
}
