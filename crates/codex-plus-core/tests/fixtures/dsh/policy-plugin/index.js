// 一个用到系统提示词与 dsh 事件的策略插件，按 dsh 插件的写法编写。
import z from '@deepseek-ai/schemastery'
import { createUserMessage } from '@deepseek-ai/dsh-llm'

export const name = 'policy-demo'
export const inject = ['systemPrompt']

export const Config = z.object({
  rules: z.string().default('Never run destructive shell commands without asking.'),
  dynamicPrompt: z.boolean().default(false),
  requireSummary: z.boolean().default(true),
})

const note = text => createUserMessage({ content: [{ type: 'text', text }], source: { kind: 'policy-demo' } })
const textOf = message => message.content.map(block => block.text ?? '').join('')

export function apply(ctx, config) {
  if (config.dynamicPrompt) {
    ctx.systemPrompt.variable('policyModel', context => context.modelSelection?.model ?? 'unknown')
    ctx.systemPrompt.section({ name: 'policy-demo:rules', order: 500, text: () => `${config.rules} (model: {{policyModel}})` })
  } else {
    ctx.systemPrompt.section({ name: 'policy-demo:rules', order: 500, text: config.rules })
  }

  ctx.on('tools/pre-execute', async (exec, next) => {
    if (exec.name === 'bash' && /rm\s+-rf/.test(String(exec.arguments?.command ?? ''))) {
      return { kind: 'deny', reason: 'policy-demo: rm -rf is blocked' }
    }
    return next()
  })

  // 按 agent 记住上一条命令：同一会话里连续重复时提醒（插件状态要跨事件保留才行）。
  const lastCommand = new WeakMap()
  ctx.on('tools/post-execute', async (exec, result, next) => {
    const decision = await next()
    if (exec.name !== 'bash') return decision
    if (textOf(result).includes('SECRET')) {
      return { kind: 'block', feedback: [{ type: 'text', text: 'policy-demo: output contained a secret and was withheld' }] }
    }
    const command = String(exec.arguments?.command ?? '')
    const last = lastCommand.get(exec.agent)
    const count = last?.command === command ? last.count + 1 : 1
    lastCommand.set(exec.agent, { command, count })
    if (count < 2) return decision
    return { ...decision, additionalContexts: [...(decision.additionalContexts ?? []), note(`policy-demo: "${command}" repeated ${count} times`)] }
  })

  ctx.on('agent/pre-step', async ({ messages }, next) => {
    if (messages.some(message => textOf(message).includes('forbidden-word'))) return { kind: 'reject' }
    const decision = await next()
    if (decision.kind !== 'enter') return decision
    return { ...decision, messages: [...decision.messages, note('policy-demo: remember the house rules')] }
  }, { prepend: true })

  // 改写用户原话：带 secret-token 的输入换成打码后的一条（钩子做不到，经 Codex++ 代理的请求里换掉）。
  ctx.on('agent/pre-step', async (_step, next) => {
    const decision = await next()
    if (decision.kind !== 'enter') return decision
    return { ...decision, messages: decision.messages.map(message => (textOf(message).includes('secret-token')
      ? createUserMessage({ content: [{ type: 'text', text: textOf(message).replace(/secret-token \S+/, 'secret-token [redacted]') }], source: { kind: 'user' } })
      : message)) }
  })

  ctx.on('agent/turn-stopping', ({ agent }) => {
    if (config.requireSummary) agent.steer(note('policy-demo: summarize the changes before stopping'))
  })
}
