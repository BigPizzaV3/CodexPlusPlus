// typert 方法的实现：Service 子类提供 notes 服务，按 id 查笔记的 lookup 登记到 ctx.typert.lookups。
import { Service } from '@deepseek-ai/cordis'

export default class Notes extends Service {
  static inject = ['typert']

  constructor(ctx) {
    super(ctx, 'notes')
    this.items = []
    ctx.typert.lookups.register('note', { parameter: 'note', wire: 'noteId', resolve: (id) => this.items[Number(id)] })
  }

  add(text) {
    this.items.push({ text })
    return this.items.length
  }

  list() {
    return this.items.map((item) => item.text)
  }

  read(note) {
    return `note: ${note.text}（${this.ctx.invocation?.request.method}）`
  }

  async *watch(prefix) {
    for await (const item of this.ctx.invocation.uplink()) yield `${prefix}${item}`
  }
}
