// 按 dsh 写法：llm/stream 把文字增量转成大写，agent/assistant-stream 统计收到的帧，命令读出统计。
export const name = 'stream-demo'
export const inject = ['commands']

export function apply(ctx) {
  const frames = { start: 0, chunk: 0, end: 0 }
  ctx.on('llm/stream', (options, next) => (async function* () {
    for await (const chunk of next()) {
      yield chunk.type === 'text-delta' ? { ...chunk, text: chunk.text.toUpperCase() } : chunk
    }
  })())
  ctx.on('agent/assistant-stream', ({ frame }) => {
    frames[frame.type] = (frames[frame.type] ?? 0) + 1
  })
  ctx.commands.register({ name: 'frames', description: '统计', handler: () => ({ kind: 'success', text: JSON.stringify(frames) }) })
}
