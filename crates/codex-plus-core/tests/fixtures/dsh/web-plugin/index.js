// 按 dsh 写法用到 webServer、mcpResources、attachments、fileReferences。
import { fileURLToPath } from 'node:url'
import { dirname } from 'node:path'
import { defineTool } from '@deepseek-ai/dsh-tools'
import { AttachmentError } from '@deepseek-ai/dsh-attachment'

export const name = 'web-demo'
export const inject = ['tools', 'webServer', 'mcpResources', 'attachments', 'fileReferences']

const here = dirname(fileURLToPath(import.meta.url))

export function apply(ctx) {
  ctx.webServer.register({ kind: 'page', path: '/hello', handler: async (req, res) => {
    let body = ''
    for await (const chunk of req) body += chunk
    res.writeHead(201, { 'content-type': 'text/plain' })
    res.end(`hi ${req.method} ${body}`)
  } })
  ctx.mcpResources.register('notes', {
    async request(request) {
      if (request.method === 'resources/list') return { resources: [{ uri: 'note://readme', name: 'readme', mimeType: 'text/markdown' }] }
      if (request.method === 'resources/read') return { contents: [{ uri: request.uri, mimeType: 'text/markdown', text: `content of ${request.uri}` }] }
      return { resourceTemplates: [] }
    },
  })
  ctx.tools.register(defineTool({
    name: 'probe_files',
    description: 'Probe attachments and file references.',
    parameters: {},
    output: { schema: { type: 'string' }, render: (_args, value) => [{ type: 'text', text: value }] },
    async execute() {
      const image = await ctx.attachments.saveImage({ data: new Uint8Array([1, 2, 3]), mediaType: 'image/png' })
      const read = await ctx.attachments.readImage(image)
      let rejected = false
      try { await ctx.attachments.saveImage({ data: new Uint8Array([1]), mediaType: 'image/tiff' }) } catch (error) { rejected = error instanceof Error && ctx.attachments.isAttachmentError(error) && typeof AttachmentError === 'function' }
      const refs = await ctx.fileReferences.list({ session: { cwd: here } }, 'readme', new AbortController().signal)
      return JSON.stringify({ bytes: read.data.length, rejected, refs })
    },
  }))
}
