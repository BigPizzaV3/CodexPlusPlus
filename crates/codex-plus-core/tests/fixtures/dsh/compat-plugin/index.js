// 按 dsh 生态里常见的写法：继承 LlmAdapter 注册一个供应商、用 cordis timer 的 interval / throttle、
// 从 dsh-llm 引品牌 id 与纯函数；读 ctx.fiber.entry.options.id、Context.is、commands.registerFileReceiptResolver。compat_probe 把这些跑一遍，结果用 JSON 文本返回。
import { defineTool } from '@deepseek-ai/dsh-tools'
import { Context } from '@deepseek-ai/cordis'
import { LlmAdapter, CallId, ReasoningEffortId, assertNever, attributionHeaders, resolveRetryPolicy, deepFreeze, isAgentLoopRequest } from '@deepseek-ai/dsh-llm'

export const name = 'compat'
export const inject = ['tools', 'llm', 'timer', 'commands']

class EchoAdapter extends LlmAdapter {
  providerInfo(provider) {
    return { id: provider, name: 'Echo' }
  }
  async listModels() {
    return [{ provider: 'echo', id: 'echo-1', name: 'Echo 1' }]
  }
  async *stream(options) {
    const text = options.messages.map((message) => message.content.map((block) => block.text).join('')).join('|')
    yield { type: 'text-delta', text: `echo:${text}` }
    yield { type: 'finish', reason: { kind: 'stop' } }
  }
}

export function apply(ctx) {
  let ticks = 0
  let throttled = 0
  ctx.timer.interval(() => { ticks += 1 }, 10)
  const throttle = ctx.timer.throttle(() => { throttled += 1 }, 1000)
  const registration = ctx.llm.registerAdapter(['echo'], new EchoAdapter())
  ctx.tools.register(defineTool({
    name: 'compat_probe',
    description: 'runs the compatibility probes',
    parameters: {},
    output: { schema: { type: 'string' }, render: (_args, value) => [{ type: 'text', text: value }] },
    async execute() {
      throttle(); throttle(); throttle()
      await new Promise((done) => setTimeout(done, 60))
      let text = ''
      for await (const chunk of ctx.llm.stream({ provider: 'echo', model: 'echo-1', messages: [{ role: 'user', content: [{ type: 'text', text: 'hi' }] }] })) {
        if (chunk.type === 'text-delta') text += chunk.text
      }
      let never = ''
      try { assertNever('x', 'probe') } catch (error) { never = error.message }
      return JSON.stringify({
        ticked: ticks > 0,
        throttled,
        text,
        models: (await ctx.llm.listModels('echo')).map((model) => model.id),
        providers: ctx.llm.listProviders().map((provider) => provider.name),
        ids: [CallId('c1'), ReasoningEffortId('high')],
        never,
        userAgent: attributionHeaders()['user-agent'].startsWith('deepseek-harness/'),
        maxRetries: resolveRetryPolicy().maxRetries,
        frozen: Object.isFrozen(deepFreeze({ a: { b: 1 } }).a),
        loop: isAgentLoopRequest({}),
        replace: typeof registration.replace,
        fiberId: ctx.fiber?.entry?.options?.id,
        isContext: Context.is(ctx),
        receipts: typeof ctx.commands?.registerFileReceiptResolver,
      })
    },
  }))
}
