/** 固定槽位按供应商 id 持久化；不按列表排名染色、不循环复用颜色。 */
const STORAGE_KEY = "codex-plus-flow-provider-colors-v1";
const slots = new Map<string, number>();
let loaded = false;

export function assignProviderSlot(registry: Map<string, number>, id: string): number {
  if (!id) return -1;
  const existing = registry.get(id);
  if (existing !== undefined) return existing;
  const used = new Set(registry.values());
  const slot = [0, 1, 2].find((value) => !used.has(value)) ?? 3;
  registry.set(id, slot);
  return slot;
}

export function flowProviderSlot(id: string): number {
  if (!loaded) {
    loaded = true;
    try {
      const saved: unknown = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "[]");
      if (Array.isArray(saved)) for (const item of saved) {
        if (Array.isArray(item) && typeof item[0] === "string" && Number.isInteger(item[1]) && item[1] >= 0 && item[1] <= 3
          && (item[1] === 3 || ![...slots.values()].includes(item[1]))) slots.set(item[0], item[1]);
      }
    } catch { /* 存储不可用时保持本次会话内稳定。 */ }
  }
  const known = slots.has(id);
  const slot = assignProviderSlot(slots, id);
  if (id && !known) {
    try { localStorage.setItem(STORAGE_KEY, JSON.stringify([...slots])); } catch { /* 不影响请求。 */ }
  }
  return slot;
}
