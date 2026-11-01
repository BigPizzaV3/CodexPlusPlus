// Codex++ 兼容宿主提供的 @deepseek-ai/cordis 替身。
// 工具插件通常只 `import type { Context }`；这里给运行时导入兜底，避免模块链接失败。
// Service 子类插件按 Cordis 语义运行：宿主 `new Class(ctx, config)`，构造函数里 `super(ctx, name)` 把实例登记成服务
// （同一个宿主里的其它插件 inject 后经 ctx[name] 拿到）；随后跑 `[Service.init]()`（没有就 `start()`），卸载时
// `[Service.dispose]()`（没有就 `stop()`）。

/** 宿主 ctx 上的登记入口（与 host.mjs 一致）。 */
const PROVIDE = Symbol.for('codexpp.dsh.provide')

export class Context {
  /** Cordis 的 Context.isolate 等符号：插件偶尔用来读作用域，这里只给出占位。 */
  static isolate = Symbol.for('cordis.isolate')
  static current = Symbol.for('cordis.current')
  /**
   * 是不是完整的 Cordis 上下文。宿主给插件的 ctx 只模拟了插件用得到的部分（没有 root.reflect、accessor 等内部结构），
   * 认成 false：调用方（如 dsh-typert-protocol）据此跳过对 Cordis 内部的操作。
   */
  static is(value) {
    return value instanceof Context
  }
}

export class Service {
  static init = Symbol.for('cordis.service.init')
  static dispose = Symbol.for('cordis.service.dispose')
  static setup = Symbol.for('cordis.service.setup')
  static invoke = Symbol.for('cordis.service.invoke')
  static extend = Symbol.for('cordis.service.extend')
  static provide = Symbol.for('cordis.service.provide')
  static immediate = Symbol.for('cordis.service.immediate')

  constructor(ctx, name, options) {
    this.ctx = ctx
    this.name = typeof name === 'string' ? name : undefined
    this.options = options
    if (this.name && ctx && typeof ctx[PROVIDE] === 'function') ctx[PROVIDE](this.name, this)
  }
}

export const symbols = { isolate: Context.isolate, current: Context.current, init: Service.init, dispose: Service.dispose }

export default { Context, Service, symbols }
