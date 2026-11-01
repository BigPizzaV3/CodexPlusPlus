// ctx.set 提供服务；ctx.isolate 给出一个私有作用域，里外同名服务互不可见。
import { defineTool } from '@deepseek-ai/dsh-tools'

export const name = 'memo'
export const inject = ['tools']

export function apply(ctx) {
  const inner = ctx.isolate('memo')
  inner.set('memo', { v: 'inner' })
  ctx.set('memo', { v: 'outer' })
  ctx.tools.register(defineTool({
    name: 'memo_probe',
    description: 'read memo inside and outside the isolate',
    parameters: {},
    output: { schema: { type: 'object' }, render: (_args, value) => [{ type: 'text', text: JSON.stringify(value) }] },
    async execute() { return { outer: ctx.memo.v, inner: inner.memo.v, viaGet: ctx.get('memo').v } },
  }))
}
