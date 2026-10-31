export type SkillTask<T> = { key: string; sequence: number; state: "queued" | "running" | "done" | "failed"; result?: T; error?: string };

/** 页面卸载不取消任务；同一 Skill 同时只能排队一次。 */
export class SkillTaskQueue<T> {
  private listeners = new Set<() => void>();
  private pending: { key: string; sequence: number; run: () => Promise<T> }[] = [];
  private entries: SkillTask<T>[] = [];
  private sequence = 0;
  private running = false;

  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => this.listeners.delete(listener); };
  getSnapshot = () => this.entries;

  enqueue(key: string, run: () => Promise<T>): boolean {
    if (this.entries.some((entry) => entry.key === key && (entry.state === "queued" || entry.state === "running"))) return false;
    const sequence = ++this.sequence;
    this.pending.push({ key, sequence, run });
    this.update({ key, sequence, state: "queued" });
    void this.drain();
    return true;
  }

  private update(entry: SkillTask<T>) {
    this.entries = [...this.entries.filter((item) => item.key !== entry.key), entry];
    for (const listener of this.listeners) listener();
  }

  private async drain() {
    if (this.running) return;
    this.running = true;
    try {
      while (this.pending.length) {
        const task = this.pending.shift()!;
        this.update({ key: task.key, sequence: task.sequence, state: "running" });
        try {
          const result = await task.run();
          this.update({ key: task.key, sequence: task.sequence, state: "done", result });
        } catch (error) {
          this.update({ key: task.key, sequence: task.sequence, state: "failed", error: String(error) });
        }
      }
    } finally { this.running = false; }
  }
}

/** 限制并发但不等待慢仓库；完成后立即报告该仓库结果。 */
export async function checkRepositories<T>(items: T[], concurrency: number, check: (item: T) => Promise<void>): Promise<void> {
  let next = 0;
  const worker = async () => {
    while (next < items.length) {
      const item = items[next++];
      await check(item);
    }
  };
  await Promise.all(Array.from({ length: Math.min(items.length, Math.max(1, concurrency)) }, worker));
}
