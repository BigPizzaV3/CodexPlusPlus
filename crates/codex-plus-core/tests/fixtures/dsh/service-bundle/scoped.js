// 组上声明了 isolate.counter：这里拿到的是组里那个从 100 开始的实例。
import { defineTool } from '@deepseek-ai/dsh-tools'

export const name = 'scoped-consumer'
export const inject = ['tools', 'counter']

export function apply(ctx) {
  ctx.tools.register(defineTool({
    name: 'count_scoped',
    description: 'next number from the isolated counter',
    parameters: {},
    output: { schema: { type: 'number' }, render: (_args, value) => [{ type: 'text', text: String(value) }] },
    async execute() { return ctx.counter.next() },
  }))
}
