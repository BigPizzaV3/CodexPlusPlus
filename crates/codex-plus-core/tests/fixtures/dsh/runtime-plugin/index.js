// 按 dsh 写法用到 planMode、goals、subagents、fs 与 fs 的意图事件。
import { defineTool } from '@deepseek-ai/dsh-tools'
import { PlanModeController } from '@deepseek-ai/dsh-plan-mode'

export const name = 'runtime-demo'
export const inject = ['tools', 'commands', 'planMode', 'goals', 'subagents', 'fs']

export function apply(ctx) {
  ctx.commands.register({ name: 'plan', description: 'toggle plan mode', handler: ({ agent, rawInput }) => ({ kind: 'success', text: ctx.planMode.set(agent, rawInput.trim() !== 'off') }) })
  ctx.commands.register({ name: 'goal', description: 'set a goal', handler: ({ agent, rawInput }) => ({ kind: 'success', text: ctx.goals.create(agent, { objective: rawInput.trim() }).status }) })
  ctx.commands.register({ name: 'done', description: 'complete the goal', handler: ({ agent }) => ({ kind: 'success', text: ctx.goals.complete(agent).status }) })
  let observed = 0
  ctx.on('fs/observed', () => { observed += 1 })
  // 插件自己的写：不存在才写（第一个给出意图的监听器说了算）。
  ctx.on('fs/write-intent', async (target, actor, next) => (target.targetKey.endsWith('.lock') ? { kind: 'createIfAbsent' } : next()))
  ctx.tools.register(defineTool({
    name: 'probe_runtime',
    description: 'Probe runtime stand-ins.',
    parameters: { dir: { type: 'string', required: true } },
    output: { schema: { type: 'string' }, render: (_args, value) => [{ type: 'text', text: value }] },
    async execute({ dir }) {
      const lock = await ctx.fs.resolve(`${dir}/a.lock`)
      await ctx.fs.writeText(lock, 'one')
      let second = 'written'
      try { await ctx.fs.writeText(lock, 'two') } catch (error) { second = error.code }
      await ctx.fs.stat(lock)
      await ctx.fs.stat(await ctx.fs.resolve(`${dir}/missing`))
      return JSON.stringify({
        second,
        observed,
        providers: ctx.subagents.list(),
        planClass: typeof PlanModeController,
      })
    },
  }))
}
