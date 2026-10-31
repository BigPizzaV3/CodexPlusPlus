export type SkillAgentState = { installed: boolean; bundled: boolean; enabled: boolean; agents?: string[] };
export type SkillBatchOperation = { groupKey: string; agent: string; enabled: boolean; skillIds: string[] };

export function skillAgents(skill: SkillAgentState): string[] {
  return skill.agents ?? (skill.enabled ? ["codex"] : []);
}

export function skillBatchTargets<T extends SkillAgentState>(skills: T[], agent: string, enabled: boolean): T[] {
  return skills.filter((skill) => skill.installed && !skill.bundled && skillAgents(skill).includes(agent) !== enabled);
}

export function skillBatchState(skills: SkillAgentState[], agent: string) {
  const eligible = skills.filter((skill) => skill.installed && !skill.bundled);
  const selected = eligible.filter((skill) => skillAgents(skill).includes(agent)).length;
  return { total: eligible.length, checked: eligible.length > 0 && selected === eligible.length, mixed: selected > 0 && selected < eligible.length };
}

export function skillBatchHasPending(operations: SkillBatchOperation[], skills: { id: string }[], agent: string): boolean {
  const ids = new Set(skills.map((skill) => skill.id));
  return operations.some((operation) => operation.agent === agent && operation.skillIds.some((id) => ids.has(id)));
}

/** 只合并目标 Agent 的结果，避免旧快照覆盖其他 Skill 或 Agent 的操作。 */
export function mergeSkillAgentResult<T extends SkillAgentState & { id: string }>(
  current: T[], result: T[], id: string, agent: string,
): T[] {
  const updated = result.find((skill) => skill.id === id && skill.installed && !skill.bundled);
  if (!updated) return current;
  const enabled = skillAgents(updated).includes(agent);
  return current.map((skill) => {
    if (skill.id !== id || !skill.installed || skill.bundled) return skill;
    const agents = skillAgents(skill).filter((value) => value !== agent);
    if (enabled) agents.push(agent);
    return { ...skill, agents, enabled: agent === "codex" ? enabled : skill.enabled };
  });
}
