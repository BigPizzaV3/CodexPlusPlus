// 两边供应商页共用的「显示其他 Agent 的供应商」复选框和来源标记（数据规则见 provider-owner.ts）。
import { useState } from "react";
import { CxBadge } from "./cx-layout";
import { t } from "./i18n";
import type { GatewayPoolEntry } from "./gateway-model";
import { ownerBadgeKind, readShowOtherAgents, showOtherAgentsKey, type ProviderPage } from "./provider-owner";

/** 「显示其他 Agent 的供应商」：默认勾上，改了记在这一页自己的键里。 */
export function useShowOtherAgents(page: ProviderPage): [boolean, (next: boolean) => void] {
  const storage = typeof window === "undefined" ? null : window.localStorage;
  const [shown, setShown] = useState(() => readShowOtherAgents(page, storage));
  const update = (next: boolean) => {
    setShown(next);
    try {
      storage?.setItem(showOtherAgentsKey(page), next ? "1" : "0");
    } catch {
      /* 存不了就只在本次生效 */
    }
  };
  return [shown, update];
}

/** 「供应商」标题右侧的复选框：管理器统一外观（tp-check），带文字的 label 整个可点。 */
export function ShowOtherAgentsToggle({ checked, onChange, title }: { checked: boolean; onChange: (next: boolean) => void; title: string }) {
  return (
    <label className="cx-other-agents" title={title}>
      <input checked={checked} className="tp-check" onChange={(event) => onChange(event.currentTarget.checked)} type="checkbox" />
      {t("显示其他 Agent 的供应商")}
    </label>
  );
}

/** 行上的来源标记：「来自 Codex」「来自 Claude」，Codex 页自己的直连供应商写「来自直连」。 */
export function OwnerBadge({ entry, page }: { entry: Pick<GatewayPoolEntry, "owner" | "linkedCodexRelay">; page: ProviderPage }) {
  switch (ownerBadgeKind(entry, page)) {
    case "codex":
      return <CxBadge title={t("Codex 的直连供应商，默认共享给所有 Agent；地址、Key、模型在 Codex「连接与供应商」里改，这里跟着变")}>{t("来自 Codex")}</CxBadge>;
    case "claude":
      return <CxBadge title={t("在 Claude 的「供应商」页添加或导入的，Codex 经本地服务时也能用")}>{t("来自 Claude")}</CxBadge>;
    case "direct":
      return <CxBadge title={t("引用直连分页里的同一个供应商：地址、Key、模型在那边改，这里跟着变")}>{t("来自直连")}</CxBadge>;
    case "direct-copy":
      return <CxBadge title={t("从直连供应商复制过来的，两边各改各的")}>{t("来自直连")}</CxBadge>;
    default:
      return null;
  }
}
