// Codex++ 兼容宿主提供的 @deepseek-ai/cordis 替身。
// 工具插件通常只 `import type { Context }`；这里给运行时导入兜底，避免模块链接失败。
// Service 子类插件在 Codex 里没有消费者，宿主会识别并报告为不兼容。

export class Context {}

export class Service {
  constructor(ctx, name) {
    this.ctx = ctx
    this.name = name
  }
}

export const symbols = {}

export default { Context, Service }
