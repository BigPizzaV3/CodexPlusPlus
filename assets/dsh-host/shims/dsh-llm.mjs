// Codex++ 兼容宿主提供的 @deepseek-ai/dsh-llm 替身。
// 覆盖插件常用的导出：构造消息的函数、错误类、把流式块拼回内容块的 BlockAssembler，以及几个判断函数。
// 模型调用本身走宿主注入的 ctx.llm（经 Codex++ 协议代理用当前供应商）；adapter 注册、图片下放等
// 只属于 dsh 运行时的能力不提供，插件用到别的导出时会在分析阶段判为不兼容。

import { randomUUID } from 'node:crypto'

export const QUOTA_EXCEEDED_CODE = 'QUOTA_EXCEEDED'
export const CONTEXT_WINDOW_EXCEEDED_CODE = 'CONTEXT_WINDOW_EXCEEDED'
export const IMAGE_OFFLOAD_REQUIRED_CODE = 'IMAGE_OFFLOAD_REQUIRED'

export class HarnessError extends Error {
  constructor(message, code = 'ERROR', options = {}) {
    super(message, options?.cause !== undefined ? { cause: options.cause } : undefined)
    this.name = 'HarnessError'
    this.code = code
  }
}

export class LlmError extends HarnessError {
  constructor(message, code = 'LLM_ERROR', details = {}) {
    super(message, code, details)
    this.name = 'LlmError'
    if (details?.status !== undefined) this.status = details.status
    if (details?.providerRetryAfterMs !== undefined) this.providerRetryAfterMs = details.providerRetryAfterMs
    if (details?.requestId !== undefined) this.requestId = details.requestId
  }
}

/** error → cause → cause … 的链。 */
export function errorChain(error) {
  const chain = []
  let current = error
  const seen = new Set()
  while (current !== undefined && current !== null && !seen.has(current)) {
    chain.push(current)
    seen.add(current)
    current = current?.cause
  }
  return chain
}

const hasCode = (error, code) => errorChain(error).some((item) => item?.code === code)
export const isQuotaExceededError = (error) => hasCode(error, QUOTA_EXCEEDED_CODE)
export const isContextWindowExceededError = (error) => hasCode(error, CONTEXT_WINDOW_EXCEEDED_CODE)

function deepFreeze(value) {
  if (value && typeof value === 'object' && !Object.isFrozen(value)) {
    Object.freeze(value)
    for (const item of Object.values(value)) deepFreeze(item)
  }
  return value
}

export function freezeMessage(message) {
  return deepFreeze(message)
}

export function createMessage(input) {
  return deepFreeze({ ...input, id: randomUUID() })
}

export function createUserMessage(input) {
  return createMessage({ ...input, role: 'user' })
}

export function createDeveloperMessage(input) {
  return createMessage({ ...input, role: 'developer' })
}

export function createSystemMessage(input) {
  const content = typeof input === 'string' ? [{ type: 'text', text: input }] : input?.content ?? []
  return createMessage({ ...(typeof input === 'string' ? {} : input), content, source: { kind: 'system-prompt' }, role: 'system' })
}

export function createAssistantMessage(input) {
  return createMessage({ ...input, source: { kind: 'model', ...(input?.source ?? {}) }, role: 'assistant' })
}

export function createToolResultMessage({ callId, content, isError } = {}) {
  return createMessage({ role: 'tool', toolCallId: callId, content: content ?? [], isError: Boolean(isError), source: { kind: 'tool', callId } })
}

export function contentHasImage(content) {
  return Array.isArray(content) && content.some((block) => block?.type === 'image')
}

/**
 * 把 ctx.llm.stream 的流式块拼回内容块：push(chunk) 逐块喂，blocks() 取完整内容块，
 * finish / usage 是收尾块里的信息。
 */
export class BlockAssembler {
  constructor() {
    this.parts = new Map()
    this.finished = []
    this.usage = undefined
    this.finish = undefined
    this.replayState = undefined
  }

  push(chunk) {
    if (!chunk || typeof chunk !== 'object') return
    switch (chunk.type) {
      case 'block-start':
        this.parts.set(chunk.index, { type: chunk.blockType, text: '', id: undefined, name: undefined, arguments: '' })
        break
      case 'text-delta':
      case 'reasoning-delta': {
        const part = this.parts.get(chunk.index) ?? { type: chunk.type === 'text-delta' ? 'text' : 'reasoning', text: '' }
        part.text += chunk.text ?? ''
        this.parts.set(chunk.index, part)
        break
      }
      case 'tool-call-delta': {
        const part = this.parts.get(chunk.index) ?? { type: 'tool-call', arguments: '' }
        if (chunk.id) part.id = chunk.id
        if (chunk.name) part.name = chunk.name
        part.arguments = `${part.arguments ?? ''}${chunk.argumentsDelta ?? ''}`
        this.parts.set(chunk.index, part)
        break
      }
      case 'block-end':
        this.parts.delete(chunk.index)
        if (chunk.block) this.finished.push({ index: chunk.index, block: chunk.block })
        break
      case 'usage':
        this.usage = chunk.usage
        break
      case 'finish':
        this.finish = chunk.reason
        this.replayState = chunk.replayState
        break
      default:
        break
    }
  }

  blocks() {
    const pending = [...this.parts.entries()].map(([index, part]) => ({ index, block: toBlock(part) }))
    return [...this.finished, ...pending].sort((a, b) => a.index - b.index).map((item) => item.block)
  }

  interruptedBlocks() {
    return [...this.parts.values()].map(toBlock)
  }

  message(source = {}) {
    return createAssistantMessage({ content: this.blocks(), source })
  }
}

function toBlock(part) {
  if (part.type === 'tool-call') return { type: 'tool-call', id: part.id ?? '', name: part.name ?? '', arguments: part.arguments ?? '' }
  return { type: part.type === 'reasoning' ? 'reasoning' : 'text', text: part.text ?? '' }
}

/** 一段流式块里助手说的文字。 */
export function joinAssistantStreamText(chunks) {
  const assembler = new BlockAssembler()
  for (const chunk of chunks ?? []) assembler.push(chunk)
  return assembler.blocks().filter((block) => block.type === 'text').map((block) => block.text).join('')
}
