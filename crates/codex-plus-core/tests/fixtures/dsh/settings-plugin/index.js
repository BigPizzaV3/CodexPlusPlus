// 第三方插件常用的几个 dsh 运行时包：settings（含 SettingsConflictError）、loader.entries()、sessionPersistence.findLog、
// FileSystemSkillProvider、dsh-session/surface 的 foldSurface、dsh-subagent 的描述快照、dsh-agent 的 installModelSelection。
// settings_probe 把这些跑一遍，结果用 JSON 文本返回。
import { defineTool } from '@deepseek-ai/dsh-tools'
import { SettingsConflictError } from '@deepseek-ai/dsh-settings'
import { FileSystemSkillProvider } from '@deepseek-ai/dsh-skill-filesystem'
import { foldSurface, isSurfaceEligibleType } from '@deepseek-ai/dsh-session/surface'
import { snapshotSubagentDescriptor } from '@deepseek-ai/dsh-subagent'
import { installModelSelection } from '@deepseek-ai/dsh-agent'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

export const name = 'settings-demo'
export const inject = ['tools', 'settings', 'loader', 'sessionPersistence']

const message = { content: [{ type: 'text', text: 'x' }] }

export function apply(ctx) {
  ctx.tools.register(defineTool({
    name: 'settings_probe',
    description: 'runs the runtime package probes',
    parameters: {},
    output: { schema: { type: 'string' }, render: (_args, value) => [{ type: 'text', text: value }] },
    async execute() {
      const id = ctx.fiber.entry.options.id
      const initial = ctx.settings.get(id)
      await ctx.settings.mutate(id, [
        { op: 'set', path: ['apiKey'], value: 'sk-secret' },
        { op: 'set', path: ['nested', 'mode'], value: 'fast' },
      ])
      const afterMutate = ctx.settings.describe().find((item) => item.ns === id)
      let conflict = 'none'
      try {
        await ctx.settings.mutate(id, [{ op: 'set', path: ['mode'], value: 'x' }], 1)
      } catch (error) {
        conflict = error instanceof SettingsConflictError ? 'conflict' : String(error)
      }
      const updated = await ctx.settings.update(id, { mode: 'slow' }, afterMutate.revision)
      const redacted = ctx.settings.describe({ redactSecrets: true }).find((item) => item.ns === id)

      const own = [...ctx.loader.entries()].find((entry) => entry.fiber === ctx.fiber)

      const skills = await new FileSystemSkillProvider(ctx, { invalidate() {} }, {
        providerName: 'probe',
        includeDefaultRoots: false,
        customSkillDirs: [join(dirname(fileURLToPath(import.meta.url)), 'skills')],
      }).list({})

      const fold = foldSurface([
        { seq: 0, type: 'user/message', surfaceOp: 'append', data: message },
        { seq: 1, type: 'assistant/message', surfaceOp: 'append', data: { message } },
        { seq: 2, type: 'assistant/message', surfaceOp: { op: 'replace', start: 0, end: 1 }, sourceEventSeqs: [0, 1], data: { message } },
      ])

      const listeners = {}
      const agentCtx = { on: (event, handler) => { listeners[event] = handler; return () => { delete listeners[event] } } }
      const selection = { current: { provider: 'p', model: 'm' }, assembled: { provider: 'p', model: 'm', reasoningEffort: 'high' } }
      const dispose = installModelSelection(agentCtx, selection)
      const registered = Object.keys(listeners).sort()
      const routed = await listeners['agent/request']({}, async () => ({ reasoningEffort: 'low', keep: 1 }))
      dispose()

      return JSON.stringify({
        initial,
        afterMutate: { revision: afterMutate.revision, value: afterMutate.value },
        conflict,
        updated,
        redacted: redacted.value.apiKey,
        ownEntry: own === undefined ? null : { id: own.options.id, name: typeof own.options.name },
        findLog: await ctx.sessionPersistence.findLog('any'),
        skills: skills.candidates.map((candidate) => candidate.name),
        fold: { nodes: fold.nodes, shadowed: fold.replacements.map((item) => item.shadowedSeqs) },
        eligible: [isSurfaceEligibleType('user/message'), isSurfaceEligibleType('turn/start')],
        descriptors: [
          snapshotSubagentDescriptor({ mode: 'continuable', provider: 'p', label: 'l' }),
          snapshotSubagentDescriptor({ mode: 'one-shot', provider: 'p' }),
        ],
        modelSelection: { registered, routed, remaining: Object.keys(listeners) },
      })
    },
  }))
}
