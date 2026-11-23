/**
 * 启动引导的记录：本地存储里记状态（走完 / 关掉 / 出现过），设置文件里再留一个「出现过」的记号。
 *
 * 页面本地存储会丢（例如以管理员身份启动时 WebView 数据目录不同，#2902 / #2936）。只看本地存储的话，
 * 这些用户每次启动都会被向导拦一次，所以向导第一次出现时同时在设置文件里留记号，启动时两处都看。
 *
 * 记号借用设置里的 `managerAgentFocus`：它原来记的是「使用模式」（已取消，所有功能始终显示），
 * 现在只看它是不是空的。旧版本写下的 codex / enhance / all 同样算出现过，这些用户不会再被弹一次。
 */
import { invoke } from "@tauri-apps/api/core";
import { LEGACY_FOCUS_STORAGE_KEY, SETUP_STORAGE_KEY, serializeSetupState, shouldAutoOpenSetup, shouldShowSetup, type SetupState } from "./onboarding-setup-model";
import { TOUR_STORAGE_KEY } from "./onboarding-model";
import { announceBackendSettingsChanged } from "./settings-events";

function read(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

/** 预览模式（?previewRoute=）不显示引导。 */
function previewing(): boolean {
  return typeof window === "undefined" || new URLSearchParams(window.location.search).has("previewRoute");
}

/** 侧栏要不要有进度卡（新用户，或向导出现过但还没做完）。 */
export function setupCardNeeded(): boolean {
  return !previewing() && shouldShowSetup(read(SETUP_STORAGE_KEY), read(TOUR_STORAGE_KEY));
}

/** 只看本地存储：向导是不是从没出现过。 */
export function setupAutoOpenPossible(): boolean {
  return !previewing() && shouldAutoOpenSetup(read(SETUP_STORAGE_KEY), read(TOUR_STORAGE_KEY), read(LEGACY_FOCUS_STORAGE_KEY));
}

/**
 * 启动时决定要不要自动打开向导：本地存储说没出现过时，再读设置文件里的记号。
 * 不能先按本地存储把向导弹出来、等设置读回来再关（那就是「弹一下又消失」）；设置读不到时按本地存储判断。
 */
export async function resolveSetupAutoOpen(loadMarker: () => Promise<string | null | undefined>): Promise<boolean> {
  if (!setupAutoOpenPossible()) return false;
  let marker: string | null | undefined;
  try {
    marker = await loadMarker();
  } catch {
    marker = undefined;
  }
  return !marker?.trim();
}

export function saveSetupState(state: SetupState) {
  try {
    window.localStorage.setItem(SETUP_STORAGE_KEY, serializeSetupState(state));
  } catch {
    // 存不了就只在本次窗口里生效。
  }
}

/** 向导自动打开的那一刻记下「出现过」：本地记 later（进度卡留着），设置文件里留记号。 */
export function markSetupSeen() {
  saveSetupState("later");
  if (typeof window !== "undefined" && "__TAURI_INTERNALS__" in window) {
    // 后端直接改了 settings.json，版本号随之变化：广播给主界面重读，否则它表单里的旧版本号
    // 会让紧接着的任何一次保存报 settings_revision_conflict（#2937）。
    // 不看返回状态：写入成功、回读失败时版本号也已经变了，重读一次没有副作用。
    void invoke("set_manager_agent_focus", { mode: "all" })
      .then(() => announceBackendSettingsChanged())
      .catch(() => undefined);
  }
}
