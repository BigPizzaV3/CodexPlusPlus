// 监听补上的事件，用一个工具（MCP 进程）和一个命令（常驻进程）读出记录。
import { defineTool } from '@deepseek-ai/dsh-tools'

export const name = 'events-demo'
export const inject = ['tools', 'commands', 'goals', 'agents', 'storageDomain', 'credentials', 'workspaceRegistry']

export function apply(ctx) {
  const seen = []
  ctx.on('agent/error', ({ agent, error }) => seen.push(`error:${agent.session.id}:${error.code}`))
  ctx.on('workspace/session-stop', ({ sessionId }) => seen.push(`stop:${sessionId}`))
  ctx.on('goal/changed', ({ change }) => seen.push(`goal:${change.operation}`))
  ctx.on('domain/changed', ({ table, key, operation }) => seen.push(`domain:${table}:${key}:${operation}`))
  ctx.on('credentials/record-updated', (key) => seen.push(`cred:${key}`))
  ctx.on('workspace/session-activity', async (request, next) => [...(await next()), { kind: 'working', sessionId: request.sessionId }])
  ctx.commands.register({ name: 'seen', description: 'events seen', handler: () => ({ kind: 'success', text: JSON.stringify(seen) }) })
  ctx.tools.register(defineTool({
    name: 'probe_events',
    description: 'Trigger service events.',
    parameters: {},
    output: { schema: { type: 'string' }, render: (_args, value) => [{ type: 'text', text: value }] },
    async execute() {
      const agent = ctx.agents.get('g1')
      ctx.goals.create(agent, { objective: 'ship' })
      ctx.goals.complete(agent)
      const domain = await ctx.storageDomain.open({ name: 'notes', tables: { items: {} } })
      await domain.table('items').put('a', 1)
      await domain.table('items').delete('a')
      await ctx.credentials.set('TOKEN_X', 'secret')
      const resolved = await ctx.credentials.resolve('TOKEN_X')
      const activity = await ctx.workspaceRegistry.activity({ sessionId: 'g1' })
      return JSON.stringify({ seen, source: resolved?.source, activity })
    },
  }))
}
