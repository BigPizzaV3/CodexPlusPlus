// Codex++ 兼容宿主提供的 @deepseek-ai/dsh-settings 替身。
// 插件只用它的 SettingsConflictError 识别「配置版本冲突」；真正的 settings 服务由宿主注入 ctx.settings。

export class SettingsConflictError extends Error {
  constructor(message = 'settings revision conflict', details = {}) {
    super(message)
    this.name = 'SettingsConflictError'
    this.code = 'SETTINGS_CONFLICT'
    Object.assign(this, details)
  }
}

export default { SettingsConflictError }
