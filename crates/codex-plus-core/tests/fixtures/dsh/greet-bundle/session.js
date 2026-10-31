export const name = 'session-tool'
// 会话、agent 等服务 Codex++ 已有替身；子 agent 编排（subagents）仍不提供。
export const inject = ['tools', 'subagents']
export function apply() {
  throw new Error('should not be applied')
}
