/** 右下角提示的语气与停留时长。后端的 status 字符串五花八门，这里统一归成四类。 */
export type NoticeTone = "success" | "warning" | "error" | "info";

const ERROR_STATUSES = new Set(["failed", "error", "crashed"]);
const WARNING_STATUSES = new Set(["warning", "partial", "ready_degraded", "not_implemented", "wait"]);
const INFO_STATUSES = new Set(["info", "not_checked", "starting", "running"]);

export function noticeTone(status: string | undefined): NoticeTone {
  const value = (status ?? "ok").toLowerCase();
  if (ERROR_STATUSES.has(value)) return "error";
  if (WARNING_STATUSES.has(value)) return "warning";
  if (INFO_STATUSES.has(value)) return "info";
  return "success";
}

/** 出错的提示留得更久，方便看清和复制；长文本按字数多留一会儿，上限 15 秒。 */
export function noticeDuration(tone: NoticeTone, message: string): number {
  const base = tone === "error" ? 9000 : tone === "warning" ? 6500 : 4200;
  return Math.min(15_000, base + Math.max(0, message.length - 60) * 30);
}
