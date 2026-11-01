// gated 只在会话 allowed 里可用（when 按 agent 判断）；deferring 把一段上下文推迟到结果之后；
// /observe 读当前会话的 observeSession 切面（钩子常驻进程里会话在镜像中，是 live 的）。
import { defineTool } from '@deepseek-ai/dsh-tools'

export const name = 'tool-extras'
export const inject = ['tools', 'commands', 'sessionQuery', 'agents']

const text = (value) => [{ type: 'text', text: String(value) }]

export function apply(ctx) {
  ctx.tools.register(defineTool({
    name: 'gated',
    description: 'only in session allowed',
    parameters: {},
    output: { schema: { type: 'string' }, render: (_args, value) => text(value) },
    async execute() { return 'gated ran' },
  }), { when: ({ agent }) => agent?.session?.id === 'allowed' })
  ctx.tools.register(defineTool({
    name: 'deferring',
    description: 'defers a context message',
    parameters: {},
    output: { schema: { type: 'string' }, render: (_args, value) => text(value) },
    async execute(_args, exec) {
      exec.deferContext({ role: 'user', content: [{ type: 'text', text: 'remember the deferred note' }], source: { kind: 'plugin' } })
      return 'done'
    },
  }))
  ctx.commands.register({
    name: 'observe',
    description: 'observe the current session',
    async handler({ agent }) {
      const observation = await ctx.sessionQuery.observeSession(agent.session.id)
      return { kind: 'success', text: JSON.stringify({ source: observation.source, types: observation.events.map((event) => event.type), cursor: observation.cursor }) }
    },
  })
}
