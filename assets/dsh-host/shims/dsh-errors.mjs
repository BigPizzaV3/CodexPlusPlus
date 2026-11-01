// 几个 dsh 包里只被插件拿来 instanceof / 抛出的错误类，与少量常量（dsh-web、dsh-user-questions、
// dsh-session-query、dsh-subprocess、dsh-shell、dsh-commands 的替身都从这里导出）。

const define = (name, fallbackCode) => class extends Error {
  constructor(message, code = fallbackCode, options) {
    super(message, options)
    this.name = name
    this.code = code
  }
}

export const WebError = define('WebError', 'WEB_ERROR')
export const UserQuestionError = define('UserQuestionError', 'NO_PROVIDER')
export const SessionQueryError = define('SessionQueryError', 'SESSION_QUERY_ERROR')
export class SubprocessExecutableNotFoundError extends Error {
  constructor(command) {
    super(`executable not found: ${command}`)
    this.name = 'SubprocessExecutableNotFoundError'
    this.code = 'EXECUTABLE_NOT_FOUND'
    this.command = command
  }
}

export const DSH_ENV_PREFIX = 'DSH_'
export const TIMED_WAIT_PARAMETER = 'wait'

const SENSITIVE_ENV = /(TOKEN|SECRET|PASSWORD|API_KEY|APIKEY|CREDENTIAL|PRIVATE_KEY)/i

/** 去掉敏感变量的父进程环境。 */
export function scrubbedParentEnv() {
  const env = {}
  for (const [key, value] of Object.entries(process.env)) {
    if (value !== undefined && !SENSITIVE_ENV.test(key) && !key.startsWith('CODEX_PLUS_')) env[key] = value
  }
  return env
}

/** shell 退出状态：{ exitCode, signal } → 一句话。 */
export function parseExitStatus(status) {
  if (status?.signal) return { kind: 'signal', signal: status.signal }
  return { kind: 'exit', code: status?.exitCode ?? null }
}

/** `/name 参数` → { name, rawInput }。 */
export function parseCommand(line) {
  const match = /^\/([a-z][a-z0-9_-]*)(?:\s+([\s\S]*))?$/.exec(String(line ?? '').trim())
  return match ? { name: match[1], rawInput: (match[2] ?? '').trim() } : undefined
}
