// Codex「连接与供应商」页的两种连接方式：
// - 聚合：Codex 接到 Codex++ 本地服务，打开的供应商合在一起，模型都列进 Codex 的模型选择器，不用切换；
// - 直连：Codex 直接连一家，关掉 Codex++ 也能用（需协议转换的除外），换供应商点「使用」。
// 两种方式共用一份供应商：直连供应商以引用的方式就在共享供应商里，加一次两边都能用。
import { lazy, Suspense } from "react";
import { Cable, Loader2, Waypoints } from "lucide-react";
import { Avatar, Switch, type Notify, type Run } from "./gateway-ui";
import { presetHost, protocolLabel, providerEnabled, type GatewayPayload, type GatewayPoolEntry } from "./gateway-model";
import { Menu, MenuItem } from "./ManagementPages";
import { OwnerBadge } from "./ProviderOwner";
import { CxBadge, CxCard, CxEmpty, CxLoading, CxRow } from "./cx-layout";
import { t, tf } from "@/i18n";

export type CodexConnectionMode = "aggregate" | "direct";

// 默认模型下拉在 GatewayAgents 里，那是按需加载的大模块：这里也按需引入，不把它打回主包。
const AgentModelPicker = lazy(() => import("./GatewayAgents").then((module) => ({ default: module.AgentModelPicker })));

/** 「聚合 / 直连」两段切换：Codex 页和 Claude 页（Claude Code 那一行）共用。 */
export function ConnectionModeSwitch({
  label,
  mode,
  connecting,
  canAggregate = true,
  aggregateTitle,
  directTitle,
  onAggregate,
  onDirect,
}: {
  label: string;
  mode: CodexConnectionMode;
  connecting: boolean;
  canAggregate?: boolean;
  aggregateTitle: string;
  directTitle: string;
  onAggregate: () => void;
  onDirect: () => void;
}) {
  const aggregate = mode === "aggregate";
  return (
    <div aria-label={label} className="mg-segmented cx-connection-switch" role="radiogroup">
      <button
        aria-checked={aggregate}
        data-active={aggregate}
        disabled={connecting || aggregate || !canAggregate}
        onClick={onAggregate}
        role="radio"
        title={aggregateTitle}
        type="button"
      >
        {connecting ? <Loader2 className="animate-spin" /> : <Waypoints />}
        {t("聚合")}
      </button>
      <button
        aria-checked={!aggregate}
        data-active={!aggregate}
        disabled={connecting || !aggregate}
        onClick={onDirect}
        role="radio"
        title={directTitle}
        type="button"
      >
        <Cable />
        {t("直连")}
      </button>
    </div>
  );
}

/**
 * 「供应商」分区标题行右侧的连接方式控件：两段切换，聚合时紧跟 Codex 的默认模型下拉。
 * 不再单独占一张卡：供应商列表是这页的主体，连接方式只是列表的用法。
 */
export function CodexModeControls({
  mode,
  connecting,
  canSwitch,
  onAggregate,
  onDirect,
  payload,
  busy,
  run,
  notify,
}: {
  mode: CodexConnectionMode;
  connecting: boolean;
  /** 配置切换总开关关着时不能切。 */
  canSwitch: boolean;
  onAggregate: () => void;
  onDirect: () => void;
  payload: GatewayPayload | null;
  busy: string | null;
  run: Run;
  notify: Notify;
}) {
  return (
    <>
      <ConnectionModeSwitch
        aggregateTitle={canSwitch
          ? t("聚合：Codex 经 Codex++ 本地服务，下面打开的供应商合在一起，模型都列进 Codex 的模型选择器，不用切换；需要 Codex++ 在后台运行")
          : t("先在页面底部「更多设置」里打开供应商配置切换")}
        canAggregate={canSwitch}
        connecting={connecting}
        directTitle={mode === "aggregate"
          ? t("直连：断开本地服务，还原接入前的直连配置，关掉 Codex++ 也能用；会先列出要还原的文件，确认后才改")
          : t("直连：Codex 直接连一家供应商，关掉 Codex++ 也能用（需协议转换的除外）；换供应商点「使用」")}
        label={t("Codex 的连接方式")}
        mode={mode}
        onAggregate={onAggregate}
        onDirect={onDirect}
      />
      {mode === "aggregate" ? (
        <Suspense fallback={null}>
          <AgentModelPicker agentId="codex" busy={busy} notify={notify} payload={payload} run={run} />
        </Suspense>
      ) : null}
    </>
  );
}

/**
 * 聚合模式下的供应商列表：共享供应商一家一行，开关决定给不给 Codex 用（改 Codex 绑定的排除名单），
 * 打开的都合在一起。Codex 自己的直连供应商点「编辑」就地打开编辑页；别的到「本地服务」里管。
 */
export function CodexAggregateList({
  payload,
  entries,
  busy,
  run,
  onEditLinked,
  onManage,
  onAdd,
}: {
  payload: GatewayPayload | null;
  /** 要列的共享供应商（按「显示其他 Agent 的供应商」筛过）。 */
  entries: GatewayPoolEntry[];
  busy: string | null;
  run: Run;
  /** 引用 Codex 直连供应商的那几项：打开它的编辑页；返回 false 表示直连列表里没有它。 */
  onEditLinked: (relayId: string) => boolean;
  onManage?: () => void;
  onAdd: () => void;
}) {
  if (!payload) return <CxCard><CxLoading label={t("正在读取供应商…")} /></CxCard>;
  const exclude = payload.gateway.bindings?.codex?.poolExclude ?? [];
  if (!payload.pool.length) {
    return (
      <CxCard>
        <CxEmpty detail={t("加进来的供应商都会合在一起给 Codex 用；Claude 等其它 Agent 也能用。")} title={t("还没有供应商")}>
          <button className="cx-secondary" onClick={onAdd} type="button">{t("添加供应商")}</button>
        </CxEmpty>
      </CxCard>
    );
  }
  if (!entries.length) {
    return (
      <CxCard>
        <CxEmpty detail={t("其他 Agent 的供应商已隐藏；勾上「显示其他 Agent 的供应商」就能看到。")} title={t("这里没有 Codex 自己的供应商")} />
      </CxCard>
    );
  }
  return (
    <CxCard>
      {entries.map((entry) => {
        const enabled = providerEnabled(entry);
        const on = enabled && !exclude.includes(entry.id);
        const linked = entry.linkedCodexRelay ?? null;
        return (
          <CxRow
            badges={(
              <>
                {enabled ? null : <CxBadge>{t("已停用")}</CxBadge>}
                <OwnerBadge entry={entry} page="codex" />
              </>
            )}
            detail={[protocolLabel(entry.protocol), presetHost(entry.baseUrl), tf("{0} 个模型", [entry.models.length])].filter(Boolean).join(" · ")}
            icon={<Avatar baseUrl={entry.baseUrl} name={entry.name} size="sm" />}
            key={entry.id}
            muted={!on}
            title={entry.name}
          >
            <Switch
              disabled={!enabled}
              label={tf("给 Codex 用 {0}", [entry.name])}
              on={on}
              onToggle={() => void run(`exclude:${entry.id}`, "set_gateway_pool_member", { agents: ["codex"], id: entry.id, used: !on }, t("保存供应商开关"), true)}
              pending={busy === `exclude:${entry.id}`}
            />
            <Menu label={t("供应商操作")}>
              {linked ? <MenuItem onClick={() => { if (!onEditLinked(linked)) onManage?.(); }}>{t("编辑")}</MenuItem> : null}
              {onManage ? <MenuItem onClick={onManage}>{t("在「本地服务」里管理")}</MenuItem> : null}
            </Menu>
          </CxRow>
        );
      })}
    </CxCard>
  );
}
