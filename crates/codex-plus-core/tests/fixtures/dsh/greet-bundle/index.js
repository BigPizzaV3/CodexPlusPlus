import z from '@deepseek-ai/schemastery'
import { defineTool } from '@deepseek-ai/dsh-tools'

export const name = 'greet-tool'
export const inject = ['tools', 'systemPrompt']

export const Config = z.object({
  greeting: z.string().default('Hello'),
  punctuation: z.string().default('!'),
})

export function apply(ctx, config) {
  ctx.systemPrompt.section({ name: 'greet', order: 10, text: 'Use the greet tool when the user asks to greet someone.' })
  ctx.tools.register(defineTool({
    name: 'greet',
    description: 'Greet someone by name.',
    parameters: {
      name: { type: 'string', required: true, description: 'The name to greet' },
      times: { type: 'integer', description: 'How many times' },
    },
    output: {
      schema: { type: 'string' },
      render: (_args, value) => [{ type: 'text', text: value }],
    },
    async execute(args) {
      return Array.from({ length: args.times ?? 1 }, () => `${config.greeting}, ${args.name}${config.punctuation}`).join(' ')
    },
  }))
}
