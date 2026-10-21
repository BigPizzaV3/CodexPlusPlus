export type SkillGroups = { names: string[]; assignments: Record<string, string>; order: string[] };

export function readSkillGroups(value: string | null): SkillGroups {
  try {
    const raw = JSON.parse(value || "{}");
    const strings = (input: unknown): string[] => Array.isArray(input)
      ? [...new Set(input.filter((item): item is string => typeof item === "string" && !!item.trim()))] : [];
    return {
      names: strings(raw.names),
      order: strings(raw.order),
      assignments: raw.assignments && typeof raw.assignments === "object" && !Array.isArray(raw.assignments)
        ? Object.fromEntries(Object.entries(raw.assignments).filter(([, group]) => typeof group === "string" && !!group.trim())) as Record<string, string>
        : {},
    };
  } catch { return { names: [], assignments: {}, order: [] }; }
}

export function orderedSkillGroups(names: string[], order: string[]): string[] {
  const available = new Set(names);
  return [...new Set(order.filter((name) => available.has(name))), ...[...available].filter((name) => !order.includes(name)).sort((a, b) => a.localeCompare(b))];
}

export function moveSkillGroup(order: string[], active: string, over: string): string[] {
  const from = order.indexOf(active);
  const to = order.indexOf(over);
  if (from < 0 || to < 0 || from === to) return order;
  const next = [...order];
  next.splice(from, 1);
  next.splice(to, 0, active);
  return next;
}
