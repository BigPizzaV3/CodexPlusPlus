// 每一行注册一个以配置命名的工具，测试据此看出哪些行生效、配置是什么。
import { defineTool } from '@deepseek-ai/dsh-tools'

export const name = 'say'
export const inject = ['tools']

export function apply(ctx, config) {
  ctx.tools.register(defineTool({
    name: `say_${config.word}${config.loud === true ? '_loud' : ''}`,
    description: 'say a word',
    parameters: {},
    output: { schema: { type: 'string' }, render: (_args, value) => [{ type: 'text', text: value }] },
    async execute() { return config.word },
  }))
}
