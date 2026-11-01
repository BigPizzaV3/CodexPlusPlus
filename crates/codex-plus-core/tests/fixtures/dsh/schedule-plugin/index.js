// 按 dsh 写法：工具里建定时任务、注册 webhook 规则、用 jobs 跑后台任务。
import { defineTool } from '@deepseek-ai/dsh-tools'

export const name = 'schedule-demo'
export const inject = ['tools', 'schedule', 'webhookRuntime', 'jobs', 'workspaceRegistry']

export function apply(ctx) {
  ctx.tools.register(defineTool({
    name: 'remind_me',
    description: '过一会儿提醒我',
    parameters: { session: { type: 'string', required: true }, prompt: { type: 'string', required: true } },
    output: { render: (args, value) => [{ type: 'text', text: JSON.stringify(value) }] },
    async execute({ session, prompt }) {
      const record = await ctx.schedule.create(session, { prompt, title: 'demo', every_seconds: 60 })
      const job = ctx.jobs.start({ kind: 'demo', label: 'count', run: (handle) => { handle.append('done'); return { done: Promise.resolve({ status: 'completed' }) } } })
      const view = await ctx.jobs.wait(job, 1000)
      return { schedule: record.kind, listed: (await ctx.schedule.list({ sessionId: session })).length, job: view.status, workspaces: Array.isArray(ctx.workspaceRegistry.list()) }
    },
  }))
  ctx.webhookRuntime.register({
    id: 'demo-rule',
    kind: 'demo/push',
    run: (delivery) => delivery.event?.action === 'go' ? { prompt: `push ${delivery.event.ref}`, workspacePath: '/tmp', title: 'webhook demo' } : null,
  })
}
