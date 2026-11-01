// 按 dsh 插件写法用到 approval、userQuestions、spillStore、tokenMeter、storage、sessionTitle 与几种事件。
import z from '@deepseek-ai/schemastery'
import { defineTool } from '@deepseek-ai/dsh-tools'
import { SpillLocator } from '@deepseek-ai/dsh-spill'

export const name = 'service-demo'
export const inject = ['tools', 'approval', 'userQuestions', 'spillStore', 'tokenMeter', 'storage', 'storageDomain', 'sessionTitle', 'sessionTelemetry', 'sessionProjections', 'commands', 'agents']

export const Config = z.object({
  replaceCodex: z.boolean().default(false),
})

export function apply(ctx, config) {
  // Codex 的审批：bash 直接放行，apply_patch 拒绝，其余交还 Codex。
  ctx.on('approval/request', async (req, next) => {
    if (req.toolName === 'bash') return 'allowed-once'
    if (req.toolName === 'apply_patch') return 'rejected'
    return next()
  })
  // 插件自己能答的问题不弹框。
  ctx.on('user-questions/request', async (request, next) => {
    if (request.questions?.[0]?.id === 'auto') return { answers: [{ id: 'auto', selected: ['yes'] }] }
    return next()
  })
  // 提示词组装：默认追加一段；replaceCodex 时改写 Codex 自己的那一段。
  ctx.on('system-prompt/assemble', async (assembly, _context, next) => {
    const result = await next()
    if (!config.replaceCodex) return { ...result, sections: [...result.sections, { name: 'service-demo', order: 10, text: 'service-demo: be brief' }] }
    return { ...result, sections: result.sections.map((section) => (section.name === 'codex:instructions' ? { ...section, text: section.text.replace('base', 'BASE') } : section)) }
  })
  ctx.sessionProjections.register({
    key: 'service-demo:replies',
    init: () => [],
    apply: (state, event) => (event.type === 'assistant/message' ? [...state, event.data.message.content[0].text] : state),
  })
  let inserted = 0
  ctx.on('agent/inbox/inserted', () => { inserted += 1 })
  ctx.commands.register({
    name: 'replies',
    description: 'Show mirrored assistant replies',
    handler: ({ agent }) => {
      agent.inbox.append('next-turn', { id: 'm1', role: 'user', content: [{ type: 'text', text: 'queued' }] })
      const replaced = agent.inbox.replace('m1', { id: 'm1', role: 'user', content: [{ type: 'text', text: 'queued v2' }] })
      const replies = ctx.sessionProjections.stateOf(agent.session, 'service-demo:replies')
      return { kind: 'success', text: JSON.stringify({ replies, inserted, replaced, tokens: ctx.tokenMeter.measure(agent.session).totalTokens > 0 }) }
    },
  })
  ctx.tools.register(defineTool({
    name: 'probe_services',
    description: 'Probe the stand-in services.',
    parameters: {},
    output: { schema: { type: 'string' }, render: (_args, value) => [{ type: 'text', text: value }] },
    async execute() {
      const answer = await ctx.userQuestions.ask({ questions: [{ id: 'auto', question: 'continue?' }] })
      const spill = await ctx.spillStore.saveText({ owner: { sessionId: 's1' }, source: { kind: 'tool', toolName: 'probe_services', callId: 'c', label: 'probe' }, suggestedName: 'big.txt', content: 'x'.repeat(100) })
      return JSON.stringify({
        answer: answer.answers[0].selected[0],
        spillBytes: spill.bytes,
        locator: SpillLocator(spill.locator) === spill.locator,
        tokens: ctx.tokenMeter.estimateMessage({ content: [{ type: 'text', text: 'abcdefgh' }] }),
        domain: ctx.storage.domain === ctx.storageDomain,
        title: ctx.sessionTitle.get({ id: 'nope' }) ?? null,
        telemetry: ctx.sessionTelemetry.sharing,
      })
    },
  }))
}
