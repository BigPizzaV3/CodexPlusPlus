// Service 子类插件：构造时 super(ctx, 'counter') 登记服务，[Service.init] 里做异步准备并返回撤销函数。
import { Service } from '@deepseek-ai/cordis'

export default class Counter extends Service {
  static inject = ['logger']

  constructor(ctx, config) {
    super(ctx, 'counter')
    this.value = Number(config?.start ?? 0)
    this.ready = false
  }

  async [Service.init]() {
    await new Promise((resolve) => setTimeout(resolve, 5))
    this.ready = true
    return () => {
      this.ready = false
    }
  }

  next() {
    if (!this.ready) throw new Error('counter 还没就绪')
    this.value += 1
    return this.value
  }
}
