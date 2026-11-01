export type SkillAgentState = { installed: boolean; bundled: boolean; enabled: boolean; agents?: string[] };
export type SkillBatchOperation = { groupKey: string; agent: string; enabled: boolean; skillIds: string[] };
export type PendingSkillBatchOperation = SkillBatchOperation & { settledSkillIds: string[]; owner: symbol };

export class SkillBatchRegistry {
  private failures: { count: number; details: { id: string; message: string }[] } | null = null;
  private failureListeners = new Set<() => void>();
  subscribeFailures = (listener: () => void) => { this.failureListeners.add(listener); return () => { this.failureListeners.delete(listener); }; };
  getFailures = () => this.failures;
  clearFailures = () => { this.failures = null; for (const listener of this.failureListeners) listener(); };
  private updateSummary = { completed: 0, failed: 0, details: [] as string[] };
  private reportUpdateSummary?: (summary: { completed: number; failed: number; details: string[] }) => void;
  recordUpdateOutcome(id: string, outcome: { success: boolean; message: string }, report: (summary: { completed: number; failed: number; details: string[] }) => void) {
    this.reportUpdateSummary = report;
    if (outcome.success) { this.updateSummary.completed++; return; }
    this.updateSummary.failed++;
    const detail = `${id}：${outcome.message}`;
    const bounded = detail.length > 240 ? `${detail.slice(0, 239).replace(/[\uD800-\uDBFF]$/, "")}…` : detail;
    const message = outcome.message.length > 240 ? `${outcome.message.slice(0, 239).replace(/[\uD800-\uDBFF]$/, "")}…` : outcome.message;
    const previous = this.failures;
    const details = previous?.details ?? [];
    this.failures = { count: (previous?.count ?? 0) + 1, details: details.some((item) => item.id === id)
      ? details.map((item) => item.id === id ? { id, message } : item) : [...details, { id, message }] };
    for (const listener of this.failureListeners) listener();
    if (this.updateSummary.details.length < 3) {
      this.updateSummary.details.push(bounded);
    }
  }
  private updates: { id: string; repoKey: string; command: "install_skill" | "update_skill" | "uninstall_skill"; state: "queued" | "running" }[] = [];
  private updateListeners = new Set<() => void>();
  subscribeUpdates = (listener: () => void) => { this.updateListeners.add(listener); return () => { this.updateListeners.delete(listener); }; };
  getUpdates = () => this.updates;
  private publishUpdates(updates: typeof this.updates) { this.updates = updates; for (const listener of this.updateListeners) listener(); }
  reserveUninstall(id: string): boolean {
    if (this.updates.some((item) => item.id === id)) return false;
    this.publishUpdates([...this.updates, { id, repoKey: "", command: "uninstall_skill", state: "queued" }]);
    return true;
  }
  releaseUninstall(id: string) { this.publishUpdates(this.updates.filter((item) => item.id !== id || item.command !== "uninstall_skill")); }
  enqueueUpdate(id: string, command: "install_skill" | "update_skill", work: () => Promise<void>, repoKey = ""): boolean {
    if (this.updates.some((item) => item.id === id)) return false;
    this.publishUpdates([...this.updates, { id, repoKey, command, state: "queued" }]);
    void this.run(async () => {
      this.publishUpdates(this.updates.map((item) => item.id === id ? { ...item, state: "running" } : item));
      try { await work(); } finally {
        this.publishUpdates(this.updates.filter((item) => item.id !== id));
        if (!this.updates.some((item) => item.command !== "uninstall_skill") && this.reportUpdateSummary) {
          const report = this.reportUpdateSummary;
          const summary = this.updateSummary;
          this.reportUpdateSummary = undefined;
          this.updateSummary = { completed: 0, failed: 0, details: [] };
          report(summary);
        }
      }
    }).catch(() => {});
    return true;
  }
  private operations: PendingSkillBatchOperation[] = [];
  private listeners = new Set<() => void>();
  private settledListeners = new Set<(owner: symbol) => void>();
  private tail: Promise<void> = Promise.resolve();
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  subscribeSettled = (listener: (owner: symbol) => void) => { this.settledListeners.add(listener); return () => { this.settledListeners.delete(listener); }; };
  notifySettled(owner: symbol) { for (const listener of this.settledListeners) listener(owner); }
  getSnapshot = () => this.operations;
  run<T>(work: () => Promise<T>): Promise<T> {
    const result = this.tail.then(work, work);
    this.tail = result.then(() => undefined, () => undefined);
    return result;
  }
  private publish(operations: PendingSkillBatchOperation[]) {
    this.operations = operations;
    for (const listener of this.listeners) listener();
  }
  add(operation: PendingSkillBatchOperation) { this.publish([...this.operations, operation]); }
  replace(previous: PendingSkillBatchOperation, next: PendingSkillBatchOperation) {
    this.publish(this.operations.map((item) => item === previous ? next : item));
    if (next.settledSkillIds.length > previous.settledSkillIds.length) {
      this.notifySettled(next.owner);
    }
  }
  remove(operation: PendingSkillBatchOperation) { this.publish(this.operations.filter((item) => item !== operation)); }
}

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

// 只合并目标 Agent 的结果，避免覆盖其他操作产生的状态。
export function mergeSkillAgentResult<T extends SkillAgentState & { id: string }>(current: T[], result: T[], id: string, agent: string): T[] {
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
