export const name = 'session-tool'
// 会话、agent、子 agent 等服务 Codex++ 已有替身；工作流引擎（workflowEngine）仍不提供。
export const inject = ['tools', 'workflowEngine']
export function apply() {
  throw new Error('should not be applied')
}
