// 发起方：/ping 命令把五种语义的事件都发一遍，别的包（dsh-bus-b）的监听器经常驻进程之间的 socket 收到。
// /job-start 在常驻进程里起一个一直跑的后台任务；工具 jobs_probe 在 MCP 进程里看得到它、能停掉它。
import { defineTool } from '@deepseek-ai/dsh-tools'

export const name = 'bus-a'
export const inject = ['tools', 'commands', 'jobs']

export function apply(ctx) {
  // 本包自己也听 demo/flow：waterfall 里本包的监听器在前。
  ctx.on('demo/flow', async (input, next) => {
    const rest = await next()
    return { trail: ['a', ...rest.trail] }
  })
  ctx.commands.register({
    name: 'ping',
    description: 'send events to other packages',
    async handler() {
      ctx.emit('demo/note', 'from-a')
      const answer = await ctx.serial('demo/ask', 'q')
      const flow = await ctx.waterfall('demo/flow', { start: true }, () => ({ trail: ['end'] }))
      await ctx.parallel('demo/sync', 'x')
      const bailed = await ctx.bail('demo/bail', 2)
      const seen = await ctx.serial('demo/seen')
      return { kind: 'success', text: JSON.stringify({ answer, flow, bailed, seen }) }
    },
  })
  ctx.commands.register({
    name: 'job-start',
    description: 'start a long job in this process',
    async handler() {
      const id = ctx.jobs.start({
        kind: 'demo',
        label: 'long-job',
        run(job) {
          let stop
          const done = new Promise((resolve) => { stop = resolve })
          const timer = setInterval(() => job.append('tick\n'), 30)
          return { done: done.then(() => { clearInterval(timer); return { status: 'completed' } }), cancel: () => stop() }
        },
      })
      return { kind: 'success', text: id }
    },
  })
  ctx.tools.register(defineTool({
    name: 'jobs_probe',
    description: 'list, read and kill jobs started in other processes',
    parameters: {},
    output: { schema: { type: 'object' }, render: (_args, value) => [{ type: 'text', text: JSON.stringify(value) }] },
    async execute() {
      const job = ctx.jobs.list().find((item) => item.label === 'long-job')
      if (!job) return { found: false }
      const before = job.status
      const output = ctx.jobs.read(job.id).chunks.map((chunk) => chunk.text).join('').includes('tick')
      const kill = ctx.jobs.kill(job.id, undefined, 'probe')
      const after = await ctx.jobs.wait(job.id, 5000)
      return { found: true, before, output, kill, after: after.status }
    },
  }))
}
