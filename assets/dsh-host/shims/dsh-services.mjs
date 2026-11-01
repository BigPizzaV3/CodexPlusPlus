// 第二批替身服务对应的 dsh 包：插件多半只 import 里面的品牌构造函数、错误类或抽象类（实例由宿主经 ctx 提供）。
// dsh-spill、dsh-user-approval、dsh-session-title、dsh-token-meter、dsh-shell-env、dsh-storage、dsh-session-telemetry、
// dsh-plan-mode、dsh-goal、dsh-subagent、dsh-attachment、dsh-file-reference、dsh-host-webserver、dsh-mcp-resources 的替身都从这里导出。

const brand = (value) => String(value)

export const SpillLocator = brand
export const ApprovalRequestId = brand
export const APPROVAL_POLICIES = Object.freeze(['ask', 'never'])

/** 这些服务在 Codex++ 里由宿主经 ctx 提供；插件只拿类来做类型判断，不会自己 new。 */
class HostProvided {
  constructor() {
    throw new Error(`${new.target.name} 由 Codex++ 宿主经 ctx 提供，不能在插件里直接创建`)
  }
}
export class SpillStore extends HostProvided {}
export class ApprovalService extends HostProvided {}
export class SessionTitleService extends HostProvided {}
export class TokenMeter extends HostProvided {}
export class ShellEnvRegistry extends HostProvided {}
export class Storage extends HostProvided {}
export class SessionTelemetryBackend extends HostProvided {}
export class PlanModeController extends HostProvided {}
export class GoalService extends HostProvided {}
export class SubagentRuntime extends HostProvided {}
export class AttachmentStore extends HostProvided {}
export class FileReferenceService extends HostProvided {}
export class WebServer extends HostProvided {}
export class McpResourceRuntime extends HostProvided {}

export class AttachmentError extends Error {
  constructor(message, code = 'ATTACHMENT_ERROR') {
    super(message)
    this.name = 'AttachmentError'
    this.code = code
  }
}

export class SessionTitleInvalidError extends Error {
  constructor(message = '会话标题不合法') {
    super(message)
    this.name = 'SessionTitleInvalidError'
  }
}

// dsh-session-title 的纯函数：标题清洗（去控制字符、压成一行）与按 UTF-8 字节截断，同 dsh。
export const SessionTitleProviderId = brand
const TITLE_CONTROL_PATTERNS = [
  /(?:\u001B\]|\u009D)(?:(?!\u0007|\u001B\\)[\s\S])*(?:\u0007|\u001B\\|$)/gu,
  /(?:\u001B\[|\u009B)[0-?]*[ -/]*[@-~]/gu,
  /\u001B[@-_]/gu,
  /[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F-\u009F]/gu,
  /[\u200B\u200E\u200F\u202A-\u202E\u2060-\u2064\u2066-\u206F\uFEFF]/gu,
]
const cleanTitleText = (input) => TITLE_CONTROL_PATTERNS.reduce((text, pattern) => text.replace(pattern, ''), input).replace(/\s+/gu, ' ').trim()
function assertPositiveTitleInteger(name, value) {
  if (!Number.isInteger(value) || value <= 0) throw new Error(`${name} must be a positive integer`)
}

export function truncateTitleUtf8(input, maxBytes) {
  assertPositiveTitleInteger('maxBytes', maxBytes)
  if (Buffer.byteLength(input, 'utf8') <= maxBytes) return input
  let used = 0
  let output = ''
  for (const character of input) {
    const bytes = Buffer.byteLength(character, 'utf8')
    if (used + bytes > maxBytes) break
    output += character
    used += bytes
  }
  return output
}

export function normalizeSessionTitle(input, maxBytes) {
  return truncateTitleUtf8(cleanTitleText(input), maxBytes).trimEnd()
}

export function fallbackSessionTitle(input, maxWords, maxBytes) {
  assertPositiveTitleInteger('maxWords', maxWords)
  return truncateTitleUtf8(cleanTitleText(input).split(' ').filter(Boolean).slice(0, maxWords).join(' '), maxBytes).trimEnd()
}

/** dsh 里 setApprovalPolicy 写会话事件；Codex++ 的审批策略经 ctx.approval.setPolicy 设置，这里只是兼容导出。 */
export function setApprovalPolicy() {}

export default SpillStore
