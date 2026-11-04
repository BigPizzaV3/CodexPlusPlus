import { providerEnabled, type GatewayPoolEntry } from "./gateway-model.ts";

/** 分组仅用于展示，不改供应商在原始池里的调度顺序。 */
export function providerPoolSections(pool: GatewayPoolEntry[], query: string) {
  const needle = query.trim().toLowerCase();
  const visible = pool.filter((entry) => !needle || entry.name.toLowerCase().includes(needle)
    || entry.baseUrl.toLowerCase().includes(needle) || entry.models.some((model) => model.toLowerCase().includes(needle)));
  return {
    active: visible.filter(providerEnabled),
    inactive: visible.filter((entry) => !providerEnabled(entry)),
  };
}

/** 组内排序回填原来的槽位，另一组即使折叠也不会被挪到队尾。 */
export function reorderProviderSlots(order: string[], groupOrder: string[]): string[] {
  const members = new Set(groupOrder);
  const slots = order.filter((id) => members.has(id));
  if (members.size !== groupOrder.length || slots.length !== groupOrder.length) return order;
  let index = 0;
  return order.map((id) => members.has(id) ? groupOrder[index++] : id);
}
