// Codex++ 兼容宿主提供的 @deepseek-ai/dsh-llm 替身。
// 覆盖插件常用的导出：构造消息的函数、错误类、把流式块拼回内容块的 BlockAssembler，以及几个判断函数。
// 模型调用本身走宿主注入的 ctx.llm（经 Codex++ 协议代理用当前供应商）；adapter 注册、图片下放等
// 只属于 dsh 运行时的能力不提供，插件用到别的导出时会在分析阶段判为不兼容。

import { randomUUID } from 'node:crypto'

export const QUOTA_EXCEEDED_CODE = 'QUOTA_EXCEEDED'
export const ACCOUNT_QUOTA_EXCEEDED_CODE = 'ACCOUNT_QUOTA'
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

export function contentHasFile(content) {
  return Array.isArray(content) && content.some((block) => block?.type === 'file')
}

// 图片 / 文件附件给模型看的占位文字：与 dsh 逐字一致（插件常把它们拼进自己的请求或拿来比对）。
const quoted = (value) => JSON.stringify(value)
const imageIdentity = (ref) => (ref.name === undefined ? String(ref.attachmentId) : `${quoted(ref.name)} (${ref.attachmentId})`)
const attachmentDigest = (ref) => String(ref.attachmentId).slice('sha256:'.length, 'sha256:'.length + 8)
const IMAGE_EXTENSIONS = { 'image/png': '.png', 'image/jpeg': '.jpg', 'image/webp': '.webp', 'image/gif': '.gif' }

function normalizedAccessText(ref, access) {
  return ` Normalized copy (read-only; may be resized or re-encoded): ${quoted(access.readonlyPath)} (${ref.width}x${ref.height}px, ${ref.mediaType}).`
    + ' Source dimensions, format, and byte size may differ.'
    + ` Copy to a writable path ending in ${IMAGE_EXTENSIONS[ref.mediaType] ?? assertNever(ref.mediaType, 'image extension')} before editing.`
}

/** 附件提供方给出宿主路径、执行环境能映射时，返回只读路径；任一方给不出就是 undefined。 */
export function resolveImageAttachmentAccess(attachments, mapHostPath, ref) {
  const hostPath = attachments.imageHostPath(ref)
  if (hostPath === undefined) return undefined
  const readonlyPath = mapHostPath(hostPath)
  return readonlyPath === undefined ? undefined : { readonlyPath }
}

export function textOnlyImageText(ref) {
  return `[image omitted because this model accepts text only; attachment sha256:${attachmentDigest(ref)}]`
}

export function requestImageHandleText(ref, version, access) {
  const preview = `Image ${imageIdentity(ref)}; request preview ${version.width}x${version.height}px.`
  return access === undefined
    ? `${preview} It may be resized or re-encoded; source dimensions, format, and byte size may differ.`
    : preview + normalizedAccessText(ref, access)
}

export function offloadedImageText(ref, access) {
  const identity = `image omitted to fit request image limits; ${imageIdentity(ref)}.`
  if (access === undefined) {
    return `[${identity} No local normalized image path is available; ask the user to attach it again if needed.]`
  }
  return `[${identity}${normalizedAccessText(ref, access)}]`
}

export function fileHandleText(ref, readonlyPath) {
  const identity = `File ${quoted(ref.name)} (${ref.bytes} bytes, sha256:${attachmentDigest(ref)})`
  if (readonlyPath === undefined) {
    return `[${identity} was uploaded, but the current execution environment cannot access a readable path. Report that limitation if its contents are needed; do not claim to have read it.]`
  }
  return `[${identity}: verbatim read-only copy saved at ${quoted(readonlyPath)}. Read that path with your file tools when its contents are needed; copy it to a writable location before modifying it. When delegating file work, include this saved path in the delegation prompt; only subagents sharing this execution environment can read it.]`
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

// ---- 插件常用的纯函数与基类（与 dsh-llm 同名同义；类型只在编译期，不需要替身）----

/** 品牌化 id：dsh 里只是给字符串打类型标记，运行时原样返回。 */
export const CallId = (id) => id
export const ToolCallId = CallId
export const MessageId = (id) => id
export const ProviderRequestId = (id) => id
export const ReasoningEffortId = (id) => id

export const EMPTY_RESPONSE_CODE = 'EMPTY_RESPONSE'
export const INVALID_CREDENTIAL_CODE = 'INVALID_CREDENTIAL'
export const CONTEXT_SUMMARY_MAX_CHARS = 120

export const isHarnessError = (error) => error instanceof HarnessError

export function assertNever(value, context) {
  throw new Error(`unreachable variant${context ? ` in ${context}` : ''}: ${JSON.stringify(value) ?? String(value)}`)
}

/** 深度冻结（跳过 AbortSignal，它是请求的取消通道）。 */
export function deepFreeze(value) {
  const seen = new WeakSet()
  const pending = [value]
  while (pending.length) {
    const node = pending.pop()
    if (node === null || typeof node !== 'object' || node instanceof AbortSignal || seen.has(node)) continue
    seen.add(node)
    Object.freeze(node)
    for (const key of Object.keys(node)) pending.push(node[key])
  }
  return value
}

export const boundContextSummary = (summary) => (summary.length <= CONTEXT_SUMMARY_MAX_CHARS ? summary : `${summary.slice(0, CONTEXT_SUMMARY_MAX_CHARS - 1)}…`)

export function callConfigEquals(a, b) {
  if (a.provider !== b.provider || a.model !== b.model || a.reasoningEffort !== b.reasoningEffort || a.temperature !== b.temperature || a.maxTokens !== b.maxTokens) return false
  if (a.stop === undefined || b.stop === undefined) return a.stop === b.stop
  return a.stop.length === b.stop.length && a.stop.every((item, index) => item === b.stop[index])
}

/** Codex++ 里没有 dsh 的 agent 循环：请求都不是它组装的。 */
const AGENT_LOOP_REQUESTS = new WeakSet()
export const markAgentLoopRequest = (request) => (AGENT_LOOP_REQUESTS.add(request), request)
export const isAgentLoopRequest = (request) => AGENT_LOOP_REQUESTS.has(request)

export const APP_IDENTITY = Object.freeze({ product: 'deepseek-harness', version: '0.0.0-codexpp', url: 'https://github.com/deepseek-ai/deepseek-harness-sdk' })
export const userAgent = (identity = APP_IDENTITY) => `${identity.product}/${identity.version} (+${identity.url})`
export const attributionHeaders = (identity = APP_IDENTITY) => ({ 'user-agent': userAgent(identity) })

const LEGAL_API_KEY = /^[\x21-\x7e]+$/
export function normalizeApiKey(raw) {
  const value = String(raw ?? '').trim()
  if (!value) return { ok: false, reason: 'empty' }
  if (!LEGAL_API_KEY.test(value)) return { ok: false, reason: 'illegalCharacters' }
  return { ok: true, value }
}
export function assertUsableApiKey(raw, pkg, ref) {
  const checked = normalizeApiKey(raw)
  if (checked.ok) return checked.value
  throw new LlmError(`${pkg}: the API key resolved from ${ref} is ${checked.reason === 'empty' ? 'blank' : 'not a valid header value'}`, INVALID_CREDENTIAL_CODE)
}

const DEFAULT_RETRYABLE_CODES = Object.freeze([EMPTY_RESPONSE_CODE, 'RATE_LIMIT', 'SERVER', 'TIMEOUT', 'TRANSPORT'])
/** 供应商的重试策略：补默认值并冻结（Codex++ 不替插件执行它，只让构造供应商的代码照常跑通）。 */
export function resolveRetryPolicy(config, path = 'retryPolicy') {
  const backoff = Object.freeze({ initialDelayMs: 500, maxDelayMs: 10_000, jitterRatio: 0.1, ...(config?.backoff ?? {}) })
  if (config === undefined || config.mode === 'normal') {
    return Object.freeze({ mode: 'normal', maxRetries: config?.maxRetries ?? 2, retryableCodes: config?.retryableCodes ?? DEFAULT_RETRYABLE_CODES, ...backoff })
  }
  if (config.mode === 'always') return Object.freeze({ mode: 'always', ...backoff })
  throw new Error(`${path}.mode must be "normal" or "always"`)
}
export const RetryPolicySchema = undefined

/** 供应商适配器基类：插件继承它实现 stream，经 ctx.llm.registerAdapter(providers, adapter) 注册。 */
export class LlmAdapter {
  providerInfo(provider) {
    return { id: provider, name: provider }
  }
  providerRetryPolicy(_provider) {}
  listModels(_provider) {
    return Promise.resolve([])
  }
  resolveModel(provider, model, _signal) {
    return Promise.resolve({ provider, id: model, name: model })
  }
}
