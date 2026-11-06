/**
 * 后端在管理器表单之外单独改了 settings.json（Computer Use 开关、记下使用模式等）时广播这个事件。
 * 主界面收到后重读设置、同步版本号，否则表单里的旧版本号会让之后的保存报「已被其他操作更新」。
 */
export const BACKEND_SETTINGS_CHANGED_EVENT = "codex-plus:backend-settings-changed";

export function announceBackendSettingsChanged() {
  window.dispatchEvent(new Event(BACKEND_SETTINGS_CHANGED_EVENT));
}
