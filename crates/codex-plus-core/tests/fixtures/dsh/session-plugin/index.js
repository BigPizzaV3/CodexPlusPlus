// 依赖会话、agent、投影、命令服务的插件，按 dsh 插件的写法编写（参照 dsh-tool-todo / dsh-command-compact）。
import { createUserMessage } from '@deepseek-ai/dsh-llm'
import { SessionId } from '@deepseek-ai/dsh-session'

export const name = 'session-demo'
export const inject = ['sessions', 'agents', 'sessionProjections', 'commands', 'tools']

const note = (text) => createUserMessage({ content: [{ type: 'text', text }], source: { kind: 'session-demo' } })

export function apply(ctx) {
  // 每个会话里调用了几次工具：在会话镜像上折叠。
  ctx.sessionProjections.register({
    key: 'session-demo:calls',
    stateVersion: 1,
    init: () => ({ calls: 0, turns: 0 }),
    apply: (state, event) => event.type === 'tool/call' ? { ...state, calls: state.calls + 1 }
      : event.type === 'turn/start' ? { ...state, turns: state.turns + 1 } : state,
  })

  const seen = []
  ctx.on('session/event', (session, event) => {
    seen.push(`${SessionId(session.id)}:${event.type}`)
  })

  // 第三次工具调用时排一条上下文（钩子里翻成 additionalContext）。
  ctx.on('tools/result', (exec) => {
    const agent = ctx.agents.get(exec.agent?.session.id)
    const state = ctx.sessionProjections.stateOf(agent?.session, 'session-demo:calls')
    if (state?.calls === 3) agent.inject(note(`session-demo: ${state.calls} tool calls so far`))
  })

  // 会话开头补上下文。
  ctx.on('agent/created', ({ agent, source }) => {
    agent.inject(note(`session-demo: hello ${agent.id} (${source})`))
  })

  // 换模型。
  ctx.on('agent/request', async (params, next) => {
    const config = await next()
    return { ...config, reasoningEffort: 'high' }
  })

  ctx.commands.register({
    name: 'calls',
    description: '本会话调用了几次工具',
    input: { hint: '' },
    handler: ({ agent }) => {
      const state = ctx.sessionProjections.stateOf(agent?.session, 'session-demo:calls')
      return { kind: 'success', text: `calls=${state?.calls ?? 0} turns=${state?.turns ?? 0} events=${seen.length}` }
    },
  })
}
