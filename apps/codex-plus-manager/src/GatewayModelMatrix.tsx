/** 供应商池的「按模型看」：供应商 × 模型矩阵。 */
import { useMemo, useState, type ReactNode } from "react";
import { ChevronRight, Search } from "lucide-react";
import { t, tf } from "./i18n";
import {
  agentName,
  agentPoolProviders,
  dispatchStatus,
  formatElapsed,
  maskAccountName,
  modelDispatch,
  prettyModelName,
  speedOf,
  type GatewayPayload,
  type MemberStatus,
} from "./gateway-model";
import { Avatar } from "./gateway-ui";

/** 矩阵默认显示的行数，搜索、筛选或展开后显示全部。 */
const MATRIX_PREVIEW = 30;

type MatrixFilter = "all" | "shared" | `provider:${string}`;

/** 供应商名的短写法：`·` 前那段（「ChatGPT Pro · 邮箱」→「ChatGPT Pro」），完整名放在悬停提示里。 */
export function shortProviderName(name: string): string {
  return name.split("·")[0].trim() || name;
}

/**
 * 供应商 × 模型矩阵（供应商池的「按模型看」）：行是模型，列是池里打开的供应商（顺序就是池的顺序），
 * 格子里有点就是这家提供，颜色是状态，下面小字是平均耗时。点列头回到列表并展开那一家。
 */
export function ModelMatrix({ payload, onOpenProvider }: { payload: GatewayPayload | null; onOpenProvider: (id: string) => void }) {
  const rows = useMemo(() => modelDispatch(payload), [payload]);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<MatrixFilter>("all");
  const [expanded, setExpanded] = useState(false);
  const providers = useMemo(() => agentPoolProviders(payload), [payload]);
  const shared = rows.filter((row) => row.providers.length > 1).length;
  const needle = query.trim().toLowerCase();
  const matched = rows.filter((row) => {
    if (filter === "shared" && row.providers.length < 2) return false;
    if (filter.startsWith("provider:") && !row.providers.some((provider) => `provider:${provider.id}` === filter)) return false;
    return !needle || row.model.toLowerCase().includes(needle) || prettyModelName(row.model).toLowerCase().includes(needle);
  });
  const shown = needle || expanded || filter !== "all" ? matched : matched.slice(0, MATRIX_PREVIEW);
  const filters: Array<{ key: MatrixFilter; label: ReactNode; count: number; title?: string }> = [
    { key: "all", label: t("全部"), count: rows.length },
    { key: "shared", label: t("多家都有"), count: shared },
    ...providers.map((entry) => ({
      key: `provider:${entry.id}` as MatrixFilter,
      label: <><Avatar baseUrl={entry.baseUrl} name={entry.name} size="sm" />{maskAccountName(shortProviderName(entry.name))}</>,
      count: rows.filter((row) => row.providers.some((provider) => provider.id === entry.id)).length,
      title: maskAccountName(entry.name),
    })),
  ];
  return (
    <>
      <div className="gw-models-toolbar">
        <label className="gw-sheet-search gw-models-search">
          <Search className="h-4 w-4" />
          <input aria-label={t("搜索模型")} onChange={(event) => setQuery(event.target.value)} placeholder={t("搜索模型")} value={query} />
        </label>
      </div>
      <div className="gw-models-filters" role="group" aria-label={t("筛选")}>
        {filters.map((item) => (
          <button data-active={filter === item.key} key={item.key} onClick={() => setFilter(filter === item.key ? "all" : item.key)} title={item.title} type="button">
            {item.label}
            <em>{item.count}</em>
          </button>
        ))}
      </div>
      <div className="gw-card gw-matrix-card">
        {!payload ? <div className="gw-empty">{t("正在读取…")}</div> : null}
        {payload && rows.length === 0 ? (
          <div className="gw-empty">
            <strong>{payload.pool.length ? t("供应商都已关闭") : t("供应商池里还没有模型")}</strong>
            <span>{payload.pool.length ? t("打开供应商池里的供应商后，这里列出每个模型由哪几家提供。") : t("在「供应商池」添加并打开供应商后，这里列出每个模型由哪几家提供。")}</span>
          </div>
        ) : null}
        {payload && rows.length ? (
          <div className="gw-matrix-scroll">
            <table className="gw-matrix">
              <thead>
                <tr>
                  <th className="gw-matrix-model" scope="col">{t("模型")}</th>
                  {providers.map((entry) => (
                    <th key={entry.id} scope="col">
                      {/* 点列头回到列表展开这一家，在那里改它的模型列表、Key、价格。 */}
                      <button className="gw-matrix-provider" onClick={() => onOpenProvider(entry.id)} title={tf("在供应商池里打开「{0}」", [maskAccountName(entry.name)])} type="button">
                        <Avatar baseUrl={entry.baseUrl} name={entry.name} size="sm" />
                        <span>{maskAccountName(shortProviderName(entry.name))}</span>
                        <ChevronRight aria-hidden className="h-3.5 w-3.5" />
                      </button>
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {shown.map((row) => {
                  return (
                    <tr data-multi={row.providers.length > 1} key={row.model}>
                      <th className="gw-matrix-model" scope="row">
                        <strong title={row.model}>{prettyModelName(row.model)}</strong>
                        <code>{row.model}</code>
                      </th>
                      {providers.map((entry) => {
                        const index = row.providers.findIndex((provider) => provider.id === entry.id);
                        if (index < 0) return <td aria-label={t("不提供")} key={entry.id} />;
                        const provider = row.providers[index];
                        const status = dispatchStatus(payload, provider.id, row.model);
                        const speed = speedOf(payload, provider.id, row.model);
                        const excluded = provider.excludedBy.length ? tf("{0} 不用这家", [provider.excludedBy.map(agentName).join(t("、"))]) : "";
                        const tips = [maskAccountName(entry.name), status ? statusText(status) : "", speed !== null ? tf("平均首包 {0}", [formatElapsed(speed)]) : "", excluded].filter(Boolean).join(" · ");
                        return (
                          <td key={entry.id} title={tips}>
                            <span className="gw-matrix-cell" data-excluded={provider.excludedBy.length > 0} data-state={status?.state ?? "idle"}>
                              <Avatar baseUrl={entry.baseUrl} name={entry.name} size="sm" />
                              {speed !== null ? <small>{formatElapsed(speed)}</small> : null}
                            </span>
                          </td>
                        );
                      })}
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        ) : null}
        {payload && !needle && filter === "all" && matched.length > MATRIX_PREVIEW ? (
          <button className="gw-dispatch-more" onClick={() => setExpanded(!expanded)} type="button">
            {expanded ? t("收起") : tf("显示全部 {0} 个模型", [matched.length])}
          </button>
        ) : null}
        {payload && rows.length > 0 && matched.length === 0 ? <p className="gw-dispatch-none">{t("没有匹配的模型")}</p> : null}
      </div>
      <p className="gw-models-legend">
        {/* 格子里放的是这家的图标，图例只讲外圈和变淡是什么意思。 */}
        <span><span className="gw-matrix-cell" data-state="answered"><i /></span>{t("刚用过")}</span>
        <span><span className="gw-matrix-cell" data-state="cooling"><i /></span>{t("冷却或熔断中")}</span>
        <span><span className="gw-matrix-cell" data-excluded="true" data-state="idle"><i /></span>{t("有 Agent 不用这家")}</span>
      </p>
    </>
  );
}

/** 格子悬停提示里的状态文字。 */
export function statusText(status: MemberStatus): string {
  return tf(status.text, status.args.map((arg) => (typeof arg === "string" ? t(arg) : arg)));
}
