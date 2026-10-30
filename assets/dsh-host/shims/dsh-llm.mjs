// Codex++ 兼容宿主提供的 @deepseek-ai/dsh-llm 替身。
// 只覆盖事件监听器构造消息要用的几个函数：监听器返回的消息由 bridge.mjs 取出文本，
// 翻译成 Codex 钩子的 additionalContext / reason。模型调用等其余能力不提供，
// 插件用到别的导出时会在分析阶段判为不兼容。

import { randomUUID } from 'node:crypto'

export function createMessage(input) {
  return Object.freeze({ ...input, id: randomUUID() })
}

export function createUserMessage(input) {
  return createMessage({ ...input, role: 'user' })
}

export function createSystemMessage(input) {
  const content = typeof input === 'string' ? [{ type: 'text', text: input }] : input?.content ?? []
  return createMessage({ ...(typeof input === 'string' ? {} : input), content, role: 'system' })
}
