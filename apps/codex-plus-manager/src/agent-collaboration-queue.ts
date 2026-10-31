// 子 Agent 任务排队（#2772）与并发（#2800）：同一 Agent 最多同时运行 maxConcurrentTasks 个任务
// （默认 1），达到上限时新任务排队，任一任务结束后按提交顺序自动启动。
// 这里只根据快照计算提交表单和任务列表需要的排队 / 并发状态。

/** 与后端 MAX_QUEUED_PER_AGENT 一致；旧版快照未带上限时的兜底。 */
export const DEFAULT_MAX_QUEUED_PER_AGENT = 8;
/** 与后端 DEFAULT_MAX_CONCURRENT_TASKS 一致：旧配置没有并发字段时同一 Agent 一次只运行一个任务。 */
export const DEFAULT_MAX_CONCURRENT_TASKS = 1;
/** 与后端 MAX_CONCURRENT_TASKS_LIMIT 一致；旧版快照未带上限时的兜底。 */
export const DEFAULT_MAX_CONCURRENT_TASKS_LIMIT = 4;

type QueueTask = { agentId: string; status: string };

export type AgentQueueState = {
  /** 该 Agent 未结束的任务数（运行中 + 排队中）。 */
  active: number;
  /** 正在运行的任务数。 */
  running: number;
  /** 正在排队、尚未启动的任务数。 */
  queued: number;
  /** 每个 Agent 最多排队的任务数。 */
  limit: number;
  /** 该 Agent 的并发上限。 */
  concurrency: number;
  /** 新提交的任务会先排队：运行中的任务已达并发上限，或前面还有排队任务。 */
  willQueue: boolean;
  /** 排队已满，新任务会被拒绝。 */
  full: boolean;
};

const positiveInteger = (value: unknown, fallback: number) =>
  Number.isInteger(value) && (value as number) > 0 ? value as number : fallback;

/** 读取配置里的并发上限：缺省或无效时按 1 处理，超过上限时钳到上限。 */
export function normalizeConcurrency(value: unknown, max = DEFAULT_MAX_CONCURRENT_TASKS_LIMIT): number {
  return Math.min(positiveInteger(value, DEFAULT_MAX_CONCURRENT_TASKS), positiveInteger(max, DEFAULT_MAX_CONCURRENT_TASKS_LIMIT));
}

export function agentQueueState(tasks: QueueTask[], agentId: string, maxQueued?: number, maxConcurrent?: number): AgentQueueState {
  const limit = positiveInteger(maxQueued, DEFAULT_MAX_QUEUED_PER_AGENT);
  const concurrency = positiveInteger(maxConcurrent, DEFAULT_MAX_CONCURRENT_TASKS);
  const own = tasks.filter((task) => task.agentId === agentId);
  const queued = own.filter((task) => task.status === "queued").length;
  const running = own.filter((task) => task.status === "running").length;
  return {
    active: running + queued,
    running,
    queued,
    limit,
    concurrency,
    willQueue: queued > 0 || running >= concurrency,
    full: queued >= limit,
  };
}
