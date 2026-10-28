export const UPDATE_CHECK_INTERVAL_MS = 4 * 60 * 60 * 1000;
export const UPDATE_MUTED_DAY_KEY = "codex-plus-update-muted-day";

type UpdateCandidate = {
  status: string;
  latestVersion?: string | null;
  updateAvailable?: boolean;
};

type ReminderStorage = Pick<Storage, "getItem" | "setItem">;

/** 存在 settings.json 里的更新提醒偏好（#1272）。 */
export type UpdatePreferences = {
  /** 启动时及运行期间自动检查更新；关掉后只有手动检查会联网。 */
  autoCheck: boolean;
  /** 「跳过此版本」记下的版本；该版本及更低版本不再自动提示。空串表示没有跳过。 */
  skippedVersion: string;
};

/** 从后端设置里取出更新提醒偏好；旧设置缺键时按默认（自动检查、不跳过）。 */
export function updatePreferencesFrom(settings: { updateAutoCheck?: boolean; updateSkippedVersion?: string } | null | undefined): UpdatePreferences {
  return {
    autoCheck: settings?.updateAutoCheck !== false,
    skippedVersion: (settings?.updateSkippedVersion ?? "").trim(),
  };
}

function localDay(now: Date): string {
  return [now.getFullYear(), String(now.getMonth() + 1).padStart(2, "0"), String(now.getDate()).padStart(2, "0")].join("-");
}

/** 与后端 `parse_version_tag` 同规则：去掉前缀 v，取开头的数字段；解析不了返回 null。 */
function versionParts(value: string): number[] | null {
  const digits = value.trim().replace(/^[vV]+/, "").match(/^[0-9.]+/)?.[0];
  if (!digits) return null;
  const parts = digits.split(".").map((part) => Number.parseInt(part, 10));
  return parts.some((part) => !Number.isFinite(part)) ? null : parts;
}

/** candidate 不高于 skipped 时返回 true；版本号解析不了就只按字面相同判断。 */
export function isVersionSkipped(candidate: string, skipped: string): boolean {
  if (!skipped.trim()) return false;
  const left = versionParts(candidate);
  const right = versionParts(skipped);
  if (!left || !right) return candidate.trim() === skipped.trim();
  for (let index = 0; index < Math.max(left.length, right.length); index++) {
    const a = left[index] ?? 0;
    const b = right[index] ?? 0;
    if (a !== b) return a < b;
  }
  return true;
}

export class UpdateNotifications {
  private dismissedVersions = new Set<string>();
  private mutedDay: string | null = null;
  private preferences: UpdatePreferences | null = null;
  private storage: () => ReminderStorage;

  constructor(storage: () => ReminderStorage = () => window.localStorage) {
    this.storage = storage;
  }

  /** 后端设置是否已经读到过；没读到前自动检查要先读一次，免得启动时漏掉「跳过」或「关闭自动检查」。 */
  get preferencesLoaded(): boolean {
    return this.preferences !== null;
  }

  setPreferences(preferences: UpdatePreferences): void {
    this.preferences = { autoCheck: preferences.autoCheck, skippedVersion: preferences.skippedVersion.trim() };
  }

  /** 是否允许后台自动检查；读不到设置时按默认开启。 */
  autoCheckEnabled(): boolean {
    return this.preferences?.autoCheck !== false;
  }

  shouldPrompt(update: UpdateCandidate, manual = false, now = new Date()): boolean {
    if (update.status !== "ok" || !update.updateAvailable || !update.latestVersion) return false;
    // 用户主动检查始终可查看更新；“关闭”“跳过此版本”“今日不再提示”只影响自动提醒。
    if (manual) return true;
    if (this.dismissedVersions.has(update.latestVersion)) return false;
    if (this.preferences && isVersionSkipped(update.latestVersion, this.preferences.skippedVersion)) return false;
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

  /** 「跳过此版本」：本次会话立即生效；持久化由调用方写入后端设置。 */
  skipVersion(version: string): void {
    this.preferences = { autoCheck: this.autoCheckEnabled(), skippedVersion: version.trim() };
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
