// Codex++ 兼容宿主提供的 @deepseek-ai/dsh-session 替身。
// 插件多半只拿品牌类型的校验函数与常量；会话本身由宿主注入的 ctx.sessions 提供（Codex 会话的镜像，见 services.mjs）。

export const SESSION_FORMAT_VERSION = 4
export const KNOWN_SESSION_EVENT_TYPES = Object.freeze([
  'user/message', 'assistant/message', 'assistant/attempt', 'developer/message', 'system/message',
  'tool/call', 'tool/result', 'turn/start', 'turn/end', 'step/start', 'step/end',
  'compaction/start', 'compaction/summary', 'compaction/end', 'session/title', 'session/end-seed',
])

const brand = (name, check) => (value) => {
  if (!check(value)) throw new TypeError(`invalid ${name}: ${String(value)}`)
  return value
}
export const SessionId = brand('SessionId', (value) => typeof value === 'string' && value.length > 0)
export const SessionSeq = brand('SessionSeq', (value) => Number.isInteger(value) && value >= 0)
export const SessionLogOffset = brand('SessionLogOffset', (value) => Number.isInteger(value) && value >= 0)

const MESSAGE_TYPES = new Set(['system/message', 'developer/message', 'user/message', 'assistant/message', 'tool/result'])
export const isSurfaceEvent = (event) => MESSAGE_TYPES.has(event?.type)
export const isAppendSurfaceEvent = (event) => isSurfaceEvent(event) && (event.surfaceOp === undefined || event.surfaceOp === 'append')
export const isReplacementSurfaceEvent = (event) => isSurfaceEvent(event) && typeof event.surfaceOp === 'object'
export const headerEquals = (a, b) => JSON.stringify(a) === JSON.stringify(b)
export const canonicalHeader = (header) => ({ version: SESSION_FORMAT_VERSION, isSeeded: false, ...header })
export const snapshotSessionEvent = (event) => structuredClone(event)

/** 会话事件里的消息（消息类事件才有）。 */
export function deriveEventMessage(event) {
  if (event?.type === 'user/message') return event.data
  if (MESSAGE_TYPES.has(event?.type)) return event.data?.message
  return undefined
}

export class Session {
  constructor() {
    throw new Error('Session is provided by the host as ctx.sessions in Codex++')
  }
}
