// 依赖同一个包里 Service 子类提供的 counter：工具里用，钩子事件里也用（常驻进程要一并加载提供方）。
import { defineTool } from '@deepseek-ai/dsh-tools'

export const name = 'counter-consumer'
export const inject = ['tools', 'counter']

export function apply(ctx) {
  ctx.tools.register(defineTool({
    name: 'count_next',
    description: 'next number from the shared counter',
    parameters: {},
    output: { schema: { type: 'number' }, render: (_args, value) => [{ type: 'text', text: String(value) }] },
    async execute() { return ctx.counter.next() },
  }))
  ctx.on('tools/pre-execute', (exec, next) => {
    if (exec.name !== 'counter_probe') return next()
    return { kind: 'deny', reason: `counter=${ctx.counter.next()}` }
  })
}
