type Candidate = { id: string; repoKey: string; name: string };
type Result = { status: string; message: string };
export type SkillUpdateTask = Candidate & {
  group: string;
  state: "queued" | "running" | "succeeded" | "failed";
  error: string;
};
export type SkillUpdateSnapshot<R> = {
  tasks: readonly SkillUpdateTask[];
  active: boolean;
  result: R | null;
};

// 同一窗口只有一个写队列；切换页面不会丢掉任务，也不会并发覆盖安装状态。
export function createSkillUpdateQueue<R extends Result>(execute: (skill: Candidate) => Promise<R>) {
  let snapshot: SkillUpdateSnapshot<R> = { tasks: [], active: false, result: null };
  let draining = false;
  const listeners = new Set<() => void>();
  const key = (skill: Candidate) => JSON.stringify([skill.repoKey, skill.id]);
  const publish = (tasks: readonly SkillUpdateTask[], result = snapshot.result) => {
    snapshot = { tasks, result, active: tasks.some((task) => task.state === "queued" || task.state === "running") };
    listeners.forEach((listener) => listener());
  };
  const replace = (task: SkillUpdateTask, state: SkillUpdateTask["state"], error = "", result = snapshot.result) =>
    publish(snapshot.tasks.map((item) => item === task ? { ...task, state, error } : item), result);
  async function drain() {
    if (draining) return;
    draining = true;
    try {
      for (;;) {
        const task = snapshot.tasks.find((item) => item.state === "queued");
        if (!task) break;
        replace(task, "running");
        const running = snapshot.tasks.find((item) => key(item) === key(task))!;
        try {
          const result = await execute(task);
          replace(running, result.status === "ok" ? "succeeded" : "failed", result.status === "ok" ? "" : result.message, result);
        } catch (error) {
          replace(running, "failed", String(error));
        }
      }
    } finally { draining = false; }
  }
  return {
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
    getSnapshot: () => snapshot,
    enqueue(group: string, skills: readonly Candidate[]) {
      const active = new Set(snapshot.tasks.filter((task) => task.state === "queued" || task.state === "running").map(key));
      const additions = skills.filter((skill) => {
        if (active.has(key(skill))) return false;
        active.add(key(skill));
        return true;
      }).map((skill): SkillUpdateTask => ({ ...skill, group, state: "queued", error: "" }));
      if (!additions.length) return;
      const replaced = new Set(additions.map(key));
      publish([...snapshot.tasks.filter((task) => !replaced.has(key(task))), ...additions]);
      void drain();
    },
  };
}
