/** 统一共享供应商配置弹窗的标签页定义（纯逻辑，供宿主组件与测试共用）。 */
import type { GatewayPoolEntry } from "./gateway-model";

export type PoolProviderTabKind = "edit" | "models" | "windows" | "reasoning" | "price" | "proxy" | "limits";
export type PoolProviderTabScope = "full" | "gateway";

export type PoolProviderTabItem = {
  kind: PoolProviderTabKind;
  /** i18n 原文键，渲染时经 `t()` 翻译。 */
  label: string;
};

/** 根据供应商类型与范围，列出统一配置弹窗里要显示的标签页。 */
export function poolProviderTabs(
  entry: Pick<GatewayPoolEntry, "account" | "linkedCodexRelay" | "models">,
  scope: PoolProviderTabScope = entry.linkedCodexRelay ? "gateway" : "full",
): PoolProviderTabItem[] {
  if (scope === "gateway") {
    return [
      { kind: "price", label: "价格" },
      { kind: "proxy", label: "代理" },
      { kind: "limits", label: "限速" },
    ];
  }
  const tabs: PoolProviderTabItem[] = [];
  const canEditGeneral = !entry.account || entry.account === "chatgpt";
  if (canEditGeneral) tabs.push({ kind: "edit", label: "常规" });
  tabs.push({ kind: "models", label: "模型" });
  if (entry.models.length > 0) {
    tabs.push({ kind: "windows", label: "窗口" });
    tabs.push({ kind: "reasoning", label: "思考" });
  }
  if (entry.account) {
    // 订阅账号按月计费，代理和限速比按 token 计价更常用，价格放最后。
    tabs.push({ kind: "proxy", label: "代理" });
    tabs.push({ kind: "limits", label: "限速" });
    tabs.push({ kind: "price", label: "价格" });
  } else {
    tabs.push({ kind: "price", label: "价格" });
    tabs.push({ kind: "proxy", label: "代理" });
    tabs.push({ kind: "limits", label: "限速" });
  }
  return tabs;
}

/** 打开统一配置弹窗时的默认标签页。 */
export function defaultPoolDialogKind(
  entry: Pick<GatewayPoolEntry, "account" | "linkedCodexRelay" | "models">,
  scope?: PoolProviderTabScope,
): PoolProviderTabKind {
  return poolProviderTabs(entry, scope)[0]?.kind ?? "models";
}
