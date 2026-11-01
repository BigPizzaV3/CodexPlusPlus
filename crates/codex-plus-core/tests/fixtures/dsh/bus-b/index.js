// 只监听插件间的自定义事件：别的包 emit 时经常驻进程转过来，状态留在本包的常驻进程里。
export const name = 'bus-b'

export function apply(ctx) {
  const seen = []
  ctx.on('demo/note', (text) => { seen.push(`note:${text}`) })
  ctx.on('demo/ask', (question) => `b answers ${question}`)
  ctx.on('demo/flow', async (input, next) => {
    const rest = await next()
    return { trail: ['b', ...rest.trail], sawStart: input.start }
  })
  ctx.on('demo/sync', async (value) => {
    await new Promise((resolve) => setTimeout(resolve, 20))
    seen.push(`sync:${value}`)
  })
  ctx.on('demo/bail', (value) => value * 10)
  ctx.on('demo/seen', () => seen)
}
