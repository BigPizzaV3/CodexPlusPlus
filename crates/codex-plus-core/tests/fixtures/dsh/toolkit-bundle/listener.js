// 监听同一个包里另一个插件发的事件。
export const name = 'toolkit-listener'

export function apply(ctx, config) {
  let seen = 0
  ctx.on('toolkit/counted', () => { seen += 1 })
  ctx.on('toolkit/ask', async (count) => `${config.prefix} ${count}`)
  ctx.on('toolkit/shout', (text, next) => `${next()}!`)
  ctx.on('toolkit/seen', () => seen)
}
