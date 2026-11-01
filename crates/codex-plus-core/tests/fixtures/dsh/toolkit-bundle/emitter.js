// 注册工具；工具执行时经 ctx.serial 问同一个包里的另一个插件要答复。
import { defineTool } from '@deepseek-ai/dsh-tools'

export const name = 'toolkit-emitter'
export const inject = ['tools']

export function apply(ctx) {
  let count = 0
  ctx.tools.register(defineTool({
    name: 'count_up',
    description: 'Count up and ask the listener plugin for a reply.',
    parameters: {},
    output: { schema: { type: 'string' }, render: (_args, value) => [{ type: 'text', text: value }] },
    async execute() {
      count += 1
      ctx.emit('toolkit/counted', count)
      const reply = await ctx.serial('toolkit/ask', count)
      const shouted = ctx.waterfall('toolkit/shout', String(reply), () => '')
      return `${reply}${shouted} (seen ${ctx.bail('toolkit/seen')})`
    },
  }))
  // 守卫：pre-execute 之后、执行之前拒绝。
  ctx.tools.guard((exec) => (exec.name === 'bash' && /shutdown/.test(String(exec.arguments?.command ?? '')) ? 'toolkit: shutdown is guarded' : undefined))
  // 可见性限制：模型看不到 web_search。
  ctx.tools.restrict({ deny: ['web_search'] })
  // 代为执行：toolkit-now 这条命令由插件直接给结果，原工具不跑。
  ctx.on('tools/execute', async (exec, next) => {
    if (exec.name === 'bash' && String(exec.arguments?.command ?? '') === 'toolkit-now') {
      return { isError: false, value: 'TOOLKIT-TIME', content: [{ type: 'text', text: 'TOOLKIT-TIME' }] }
    }
    return next()
  })
}
