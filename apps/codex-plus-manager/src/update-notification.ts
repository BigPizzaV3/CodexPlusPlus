export const UPDATE_CHECK_INTERVAL_MS = 4 * 60 * 60 * 1000;
export const UPDATE_MUTED_DAY_KEY = "codex-plus-update-muted-day";

type UpdateCandidate = {
  status: string;
  latestVersion?: string | null;
  updateAvailable?: boolean;
};

type ReminderStorage = Pick<Storage, "getItem" | "setItem">;

function localDay(now: Date): string {
  return [now.getFullYear(), String(now.getMonth() + 1).padStart(2, "0"), String(now.getDate()).padStart(2, "0")].join("-");
}

export class UpdateNotifications {
  private dismissedVersions = new Set<string>();
  private mutedDay: string | null = null;
  private storage: () => ReminderStorage;

  constructor(storage: () => ReminderStorage = () => window.localStorage) {
    this.storage = storage;
  }

  shouldPrompt(update: UpdateCandidate, manual = false, now = new Date()): boolean {
    if (update.status !== "ok" || !update.updateAvailable || !update.latestVersion) return false;
    // 用户主动检查始终可查看更新；“关闭”只影响当前会话的自动提醒。
    if (manual) return true;
    if (this.dismissedVersions.has(update.latestVersion)) return false;
    const today = localDay(now);
    if (this.mutedDay === today) return false;
    try {
      return this.storage().getItem(UPDATE_MUTED_DAY_KEY) !== today;
    } catch {
      return true;
    }
  }

  dismiss(version: string): void {
    this.dismissedVersions.add(version);
  }

  muteToday(now = new Date()): void {
    this.mutedDay = localDay(now);
    try {
      this.storage().setItem(UPDATE_MUTED_DAY_KEY, this.mutedDay);
    } catch {
      // 存储不可用时，本次会话仍遵守今日静默。
    }
  }
}
