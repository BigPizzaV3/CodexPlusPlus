// 第二批替身服务对应的 dsh 包：插件多半只 import 里面的品牌构造函数、错误类或抽象类（实例由宿主经 ctx 提供）。
// dsh-spill、dsh-user-approval、dsh-session-title、dsh-token-meter、dsh-shell-env、dsh-storage、dsh-session-telemetry、
// dsh-plan-mode、dsh-goal、dsh-subagent 的替身都从这里导出。

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

export class SessionTitleInvalidError extends Error {
  constructor(message = '会话标题不合法') {
    super(message)
    this.name = 'SessionTitleInvalidError'
  }
}

/** dsh 里 setApprovalPolicy 写会话事件；Codex++ 的审批策略经 ctx.approval.setPolicy 设置，这里只是兼容导出。 */
export function setApprovalPolicy() {}

export default SpillStore
