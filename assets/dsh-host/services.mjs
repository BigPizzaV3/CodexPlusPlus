// dsh 插件的替身服务：让依赖 sessions / agents / sessionProjections / llm / fs / shell 等服务的插件也能在 Codex++ 里加载。
//
// dsh 的 agent 循环、会话存储不在 Codex++ 手里，这里的做法是：
//   - 会话镜像（SessionMirror）：钩子常驻进程把 Codex 钩子事件翻成 dsh 的会话日志事件（turn/start、user/message、
//     tool/call、tool/result、turn/end、compaction/*），追加到每个 Codex 会话的镜像里，再按 dsh 的语义派发
//     `session/event`、`tools/result`、`agent/status`、`agent/created`；插件自己 append 的领域事件（如 todo/write）
//     同样进镜像。镜像按会话写到插件数据目录，常驻进程重启后接着用。sessionProjections 在镜像上折叠。
//   - agents：每个镜像会话一个 agent 替身；steer / inject 在钩子事件里翻成钩子输出，钩子之外经 Codex++ 往
//     Codex 会话里插话（steer）或排到下一次提交（inject）。
//   - llm / sessionQuery / userQuestions / 读写 Codex 会话：经 host-call.mjs 调 Codex++。
//   - fs / subprocess / shell / web / credentials / storageDomain：本机 Node 实现，不做沙箱（与 dsh 本地实现一致）。
// 做不到的部分（新建 dsh 会话、PTY、沙箱策略、adapter 注册……）调用时抛错并说明原因。

import { spawn } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs'
import { lstat, readFile, readdir, stat, writeFile } from 'node:fs/promises'
import { dirname, isAbsolute, join, resolve as resolvePath } from 'node:path'
import { pathToFileURL } from 'node:url'
import { hostCall } from './host-call.mjs'
import { FsError } from './shims/dsh-fs.mjs'
import { DomainError } from './shims/dsh-storage-domain.mjs'
import { SessionQueryError, SubprocessExecutableNotFoundError, UserQuestionError, WebError } from './shims/dsh-errors.mjs'

/** 宿主能提供的服务（Cordis 自带的 logger / timer 一并算上）。 */
export const SUPPORTED_SERVICES = new Set([
  'tools', 'systemPrompt', 'logger', 'timer', 'skills',
  'sessions', 'sessionQuery', 'sessionProjections', 'agents', 'llm', 'commands',
  'fs', 'subprocess', 'shell', 'web', 'credentials', 'storageDomain', 'userQuestions', 'agentDefaultModel',
  'connection',
])

/** 只在钩子常驻进程里才「活」的服务：MCP 进程里拿得到，但没有会话事件流入。 */
export const MIRROR_SERVICES = new Set(['sessions', 'sessionProjections', 'agents'])

const MAX_MIRROR_EVENTS = 2000
const DEFAULT_SESSION = 'codexpp-default-session'
const MAX_TEXT = 20_000

const freeze = (value) => {
  if (value && typeof value === 'object' && !Object.isFrozen(value)) {
    Object.freeze(value)
    for (const item of Object.values(value)) freeze(item)
  }
  return value
}

const textBlocks = (text) => [{ type: 'text', text: String(text ?? '') }]
const clip = (text) => (typeof text === 'string' && text.length > MAX_TEXT ? `${text.slice(0, MAX_TEXT)}…` : text)

function userMessage(text, source = { kind: 'user' }) {
  return freeze({ id: randomUUID(), role: 'user', content: textBlocks(text), source })
}

/** 消息 → 纯文本（bridge 把 steer / inject 的消息翻成钩子输出时用）。 */
export function messageText(message) {
  if (typeof message === 'string') return message
  const blocks = message?.content
  if (typeof blocks === 'string') return blocks
  if (!Array.isArray(blocks)) return ''
  return blocks.map((block) => (block?.type === 'text' && typeof block.text === 'string' ? block.text : '')).filter(Boolean).join('\n')
}

function assertJson(value, path = 'data') {
  const seen = new Set()
  const walk = (item, at) => {
    if (item === null || typeof item === 'string' || typeof item === 'boolean') return
    if (typeof item === 'number') {
      if (!Number.isFinite(item) || Object.is(item, -0)) throw new TypeError(`${at} 不是无损 JSON 数字`)
      return
    }
    if (typeof item !== 'object') throw new TypeError(`${at} 不是 JSON 值`)
    if (seen.has(item)) throw new TypeError(`${at} 有循环引用`)
    if (!Array.isArray(item) && Object.getPrototypeOf(item) !== Object.prototype && Object.getPrototypeOf(item) !== null) {
      throw new TypeError(`${at} 不是普通对象`)
    }
    seen.add(item)
    for (const [key, child] of Object.entries(item)) walk(child, `${at}.${key}`)
    seen.delete(item)
  }
  walk(value, path)
}

// ---------------------------------------------------------------------------
// 会话镜像
// ---------------------------------------------------------------------------

const MESSAGE_EVENTS = new Set(['system/message', 'developer/message', 'user/message', 'assistant/message', 'tool/result'])

class MirrorSession {
  constructor(mirror, id, header, events = []) {
    this.mirror = mirror
    this.id = id
    this.header = freeze({ version: 4, id, createdAt: Date.now(), isSeeded: false, ...header })
    this.events = events
    this.turn = events.reduce((turn, event) => (event.type === 'turn/start' ? Math.max(turn, Number(event.data?.turn) || 0) : turn), 0)
    this.model = undefined
  }

  get seq() {
    return this.events.length
  }

  append(type, data, options = {}) {
    if (typeof type !== 'string' || !type) throw new TypeError('会话事件类型不能为空')
    assertJson(data)
    const event = { type, seq: this.events.length, time: Date.now(), data: structuredClone(data) }
    if (MESSAGE_EVENTS.has(type)) {
      event.surfaceOp = options?.surfaceOp ?? 'append'
      if (Array.isArray(options?.sourceEventSeqs)) event.sourceEventSeqs = [...options.sourceEventSeqs]
    } else if (options?.surfaceOp !== undefined) {
      throw new TypeError(`${type} 不是消息事件，不能带 surfaceOp`)
    }
    freeze(event)
    this.events.push(event)
    if (this.events.length > MAX_MIRROR_EVENTS) this.events.splice(0, this.events.length - MAX_MIRROR_EVENTS)
    this.mirror.committed(this, event)
    return event
  }

  deriveMessages() {
    return freeze(this.events.flatMap((event) => {
      if (event.type === 'user/message') return [event.data]
      if (MESSAGE_EVENTS.has(event.type) && event.data?.message) return [event.data.message]
      return []
    }))
  }

  requestHeader() {
    return { config: { provider: providerOf(this.model), model: this.model ?? '' } }
  }

  requestContext() {
    return { provider: providerOf(this.model), model: this.model ?? '' }
  }

  eventAt(seq) {
    return this.events.find((event) => event.seq === seq)
  }

  snapshotEvents(from = 0, to = this.events.length) {
    return this.events.slice(from, to)
  }

  ownEvents() {
    return [...this.events]
  }
}

function providerOf(model) {
  const text = String(model ?? '')
  const slash = text.indexOf('/')
  return slash > 0 ? text.slice(0, slash) : 'codex'
}

export class SessionMirror {
  /**
   * @param {object} options
   * @param {string | undefined} options.dataDir 镜像落盘目录（不给则只在内存里）
   * @param {(event: string, args: unknown[]) => void} options.emit 派发 ctx 事件（emit 语义，监听器出错各自隔离）
   */
  constructor({ dataDir, emit }) {
    this.dataDir = dataDir
    this.emit = emit
    this.sessions = new Map()
    this.projections = new Map()
    this.projectionCache = new WeakMap()
    this.changeListeners = new Set()
    this.saveTimers = new Map()
  }

  path(id) {
    return this.dataDir ? join(this.dataDir, 'sessions', `${String(id).replace(/[^\w.-]/g, '_')}.json`) : undefined
  }

  /** 取会话镜像；没有就从落盘恢复或新建。 */
  ensure(id, header = {}) {
    // 没带会话 id 的钩子负载（旧版 Codex、测试）归到同一个默认会话。
    const key = String(id ?? '') || DEFAULT_SESSION
    let session = this.sessions.get(key)
    if (session) {
      if (header.cwd && !session.header.cwd) session.header = freeze({ ...session.header, cwd: header.cwd })
      return session
    }
    let stored
    const path = this.path(key)
    if (path && existsSync(path)) {
      try {
        stored = JSON.parse(readFileSync(path, 'utf8'))
      } catch {
        stored = undefined
      }
    }
    const events = Array.isArray(stored?.events) ? stored.events.map((event, seq) => freeze({ ...event, seq })) : []
    session = new MirrorSession(this, key, { ...(stored?.header ?? {}), ...(header.cwd ? { cwd: header.cwd } : {}) }, events)
    this.sessions.set(key, session)
    this.emit('session/created', [session])
    return session
  }

  committed(session, event) {
    this.scheduleSave(session)
    this.emit('session/event', [session, event])
    if (this.changeListeners.size) {
      for (const key of this.projections.keys()) {
        const before = this.projectionCache.get(session)?.get(key)?.state
        const after = this.stateOf(session, key)
        if (after !== before) {
          for (const listener of this.changeListeners) {
            try {
              listener(session, key, after, event.seq)
            } catch (error) {
              console.error(`[dsh-host] sessionProjections.onChanged 失败：${error?.message ?? error}`)
            }
          }
        }
      }
    }
  }

  scheduleSave(session) {
    const path = this.path(session.id)
    if (!path || this.saveTimers.has(session.id)) return
    this.saveTimers.set(session.id, setTimeout(() => {
      this.saveTimers.delete(session.id)
      this.save(session)
    }, 200))
  }

  save(session) {
    const path = this.path(session.id)
    if (!path) return
    try {
      mkdirSync(dirname(path), { recursive: true })
      const temp = `${path}.${process.pid}.tmp`
      writeFileSync(temp, JSON.stringify({ header: session.header, events: session.events }))
      renameSync(temp, path)
    } catch (error) {
      console.error(`[dsh-host] 会话镜像写入失败：${error?.message ?? error}`)
    }
  }

  flushAll() {
    for (const [id, timer] of this.saveTimers) {
      clearTimeout(timer)
      const session = this.sessions.get(id)
      if (session) this.save(session)
    }
    this.saveTimers.clear()
  }

  stateOf(session, key) {
    const definition = this.projections.get(key)
    if (!definition || !session) return undefined
    let cache = this.projectionCache.get(session)
    if (!cache) {
      cache = new Map()
      this.projectionCache.set(session, cache)
    }
    let entry = cache.get(key)
    if (!entry || entry.definition !== definition || entry.seq > session.events.length) {
      entry = { definition, seq: 0, state: definition.init(session.header, 0) }
      cache.set(key, entry)
    }
    // 镜像有上限，旧事件被裁掉时从头重算（序号仍按镜像里的位置）。
    for (let index = entry.seq; index < session.events.length; index++) {
      entry.state = definition.apply(entry.state, session.events[index])
    }
    entry.seq = session.events.length
    return entry.state
  }

  // ---- Codex 钩子 → dsh 会话日志 ----

  /** 一次钩子事件写进镜像，返回对应的镜像会话。 */
  recordHook(hookEvent, payload) {
    const session = this.ensure(payload?.session_id, { cwd: typeof payload?.cwd === 'string' ? payload.cwd : undefined })
    if (!session) return undefined
    if (payload?.model) session.model = payload.model
    const step = 1
    switch (hookEvent) {
      case 'UserPromptSubmit': {
        session.turn += 1
        session.append('turn/start', { turn: session.turn })
        session.append('user/message', userMessage(clip(String(payload?.prompt ?? ''))))
        break
      }
      case 'PreToolUse':
        session.append('tool/call', {
          turn: session.turn,
          step,
          callId: String(payload?.tool_use_id ?? ''),
          name: String(payload?.tool_name ?? ''),
          arguments: clip(JSON.stringify(payload?.tool_input ?? {})),
        })
        break
      case 'PostToolUse': {
        const output = typeof payload?.tool_response === 'string' ? payload.tool_response : JSON.stringify(payload?.tool_response ?? null)
        session.append('tool/result', {
          turn: session.turn,
          step,
          message: { id: randomUUID(), role: 'tool', toolCallId: String(payload?.tool_use_id ?? ''), content: textBlocks(clip(output)), source: { kind: 'tool', callId: String(payload?.tool_use_id ?? '') } },
        })
        break
      }
      case 'Stop':
        session.append('turn/end', { turn: session.turn, reason: { kind: 'completed' } })
        break
      case 'PreCompact':
        session.append('compaction/start', { compactionId: randomUUID(), turn: session.turn || null })
        break
      case 'PostCompact':
        session.append('compaction/end', { compactionId: randomUUID(), turn: session.turn || null })
        break
      default:
        break
    }
    return session
  }
}

// ---------------------------------------------------------------------------
// agent 替身
// ---------------------------------------------------------------------------

export class AgentRegistry {
  constructor({ mirror, emit, serial }) {
    this.mirror = mirror
    this.emit = emit
    this.serial = serial
    this.agents = new Map()
    /** 当前钩子事件的 agent 与收集 steer / inject 的地方（bridge 用 AsyncLocalStorage 包住）。 */
    this.currentSink = () => undefined
  }

  get(id) {
    const key = String(id ?? '') || DEFAULT_SESSION
    let agent = this.agents.get(key)
    if (agent) return agent
    const session = this.mirror.ensure(key)
    if (!session) return undefined
    agent = this.create(session)
    this.agents.set(key, agent)
    return agent
  }

  create(session) {
    const registry = this
    let status = 'idle'
    let idleWaiters = []
    const pendingInject = []
    const sinkOf = () => {
      const sink = registry.currentSink()
      return sink?.agent === agent ? sink : undefined
    }
    const codexSteer = (message) => {
      void hostCall('sessions', 'steer', { id: session.id, input: messageText(message) }).catch((error) => {
        console.error(`[dsh-host] agent.steer 转给 Codex 失败：${error?.message ?? error}`)
      })
    }
    const inbox = {
      nextTurn: [],
      nextStep: [],
      clear() {
        inbox.nextTurn.length = 0
        inbox.nextStep.length = 0
        pendingInject.length = 0
      },
      append(target, message) {
        ;(target === 'next-step' ? inbox.nextStep : inbox.nextTurn).push(message)
        pendingInject.push(message)
      },
      prepend(target, message) {
        ;(target === 'next-step' ? inbox.nextStep : inbox.nextTurn).unshift(message)
        pendingInject.unshift(message)
      },
      replace() {
        return false
      },
      remove() {
        return false
      },
      splice() {
        return []
      },
    }
    const agent = {
      id: session.id,
      session,
      inbox,
      ctx: undefined,
      get options() {
        return freeze({ provider: providerOf(session.model), model: session.model })
      },
      get status() {
        return status
      },
      /** 插到当前这一步：钩子里翻成「要求继续 / 补上下文」，钩子之外经 Codex 往这一轮插话。 */
      steer(message) {
        const sink = sinkOf()
        if (sink) sink.steer.push(message)
        else codexSteer(message)
      },
      /** 只排上下文：钩子里并进本次输出，钩子之外等下一次钩子（通常是下一条用户输入）带上。 */
      inject(message) {
        const sink = sinkOf()
        if (sink) sink.inject.push(message)
        else pendingInject.push(message)
      },
      send(message, target, wakeup) {
        if (wakeup && status === 'idle') agent.followup(message)
        else agent.inject(message)
      },
      /** 排一个新回合：经 Codex++ 在这个会话里发起一轮。 */
      followup(message) {
        void hostCall('sessions', 'startTurn', { id: session.id, input: messageText(message) }).catch((error) => {
          console.error(`[dsh-host] agent.followup 转给 Codex 失败：${error?.message ?? error}`)
        })
      },
      cancel() {
        console.error('[dsh-host] agent.cancel 在 Codex 里不可用（拿不到正在跑的那一轮），已忽略')
      },
      whenIdle() {
        return status === 'idle' ? Promise.resolve() : new Promise((resolve) => idleWaiters.push(resolve))
      },
      async runMaintenance(task) {
        return task(new AbortController().signal)
      },
      /** 取走钩子之外排下的上下文（bridge 在下一次钩子里并进输出）。 */
      takePending() {
        const taken = pendingInject.splice(0)
        inbox.nextTurn.length = 0
        inbox.nextStep.length = 0
        return taken
      },
      setStatus(next) {
        if (next === status) return
        status = next
        registry.emit('agent/status', [{ agent, status }])
        if (status === 'idle') {
          const waiters = idleWaiters
          idleWaiters = []
          for (const resolve of waiters) resolve()
        }
      },
    }
    return agent
  }

  list() {
    return [...this.agents.values()]
  }

  service() {
    const registry = this
    return {
      get: (id) => registry.get(id),
      list: () => registry.list(),
      roots: () => registry.list(),
      isOwnedBy: () => false,
      currentInitiator: () => registry.currentSink()?.agent,
      requireInitiator() {
        const agent = registry.currentSink()?.agent
        if (!agent) throw new Error('当前不在某个 agent 的事件里')
        return agent
      },
      withInitiator: (agent, fn) => fn(),
      withoutInitiator: (fn) => fn(),
      async create() {
        throw new Error('Codex++ 里不能经 dsh 新建 agent；请用 ctx.sessions.codex.create 新建 Codex 会话')
      },
      async resume() {
        throw new Error('Codex++ 里不能经 dsh 恢复 agent')
      },
    }
  }
}

// ---------------------------------------------------------------------------
// 经 Codex++ 的服务
// ---------------------------------------------------------------------------

/** Codex++ 会话事件 → dsh 会话日志事件（尽量对上 dsh 的类型与 data 形状）。 */
export function mapCodexEvents(events) {
  const out = []
  let turn = 0
  for (const event of Array.isArray(events) ? events : []) {
    const data = event?.data ?? {}
    const base = { time: Date.parse(event?.time) || Number(event?.time) || 0 }
    switch (event?.type) {
      case 'turn/start':
        turn += 1
        out.push({ ...base, type: 'turn/start', data: { turn } })
        break
      case 'user/message':
        out.push({ ...base, type: 'user/message', data: { id: String(data.id || `codex-${event.seq}`), role: 'user', content: textBlocks(data.text), source: { kind: 'user' } }, surfaceOp: 'append' })
        break
      case 'assistant/message':
        out.push({ ...base, type: 'assistant/message', data: { turn, step: 1, message: { id: String(data.id || `codex-${event.seq}`), role: 'assistant', content: textBlocks(data.text), source: { kind: 'model' } }, stream: [] }, surfaceOp: 'append' })
        break
      case 'tool/call':
        out.push({ ...base, type: 'tool/call', data: { turn, step: 1, callId: String(data.callId ?? ''), name: String(data.name ?? ''), arguments: typeof data.input === 'string' ? data.input : JSON.stringify(data.input ?? {}) } })
        break
      case 'tool/result':
        out.push({ ...base, type: 'tool/result', data: { turn, step: 1, message: { id: `codex-${event.seq}`, role: 'tool', toolCallId: String(data.callId ?? ''), content: textBlocks(data.output), source: { kind: 'tool', callId: String(data.callId ?? '') } } }, surfaceOp: 'append' })
        break
      case 'turn/complete':
        out.push({ ...base, type: 'turn/end', data: { turn, reason: { kind: 'completed' } } })
        break
      case 'turn/aborted':
        out.push({ ...base, type: 'turn/end', data: { turn, reason: { kind: 'aborted' } } })
        break
      case 'session/compacted':
        out.push({ ...base, type: 'compaction/end', data: { compactionId: `codex-${event.seq}`, turn: turn || null } })
        break
      default:
        break
    }
  }
  return out.map((event, seq) => freeze({ ...event, seq }))
}

const epochMs = (value) => {
  const number = typeof value === 'number' ? value : Date.parse(value)
  if (!Number.isFinite(number)) return 0
  return number < 1e12 ? number * 1000 : number
}

function codexHeader(session) {
  return freeze({ version: 4, id: String(session?.id ?? ''), createdAt: epochMs(session?.createdAt), cwd: session?.cwd || undefined, isSeeded: false })
}

export function createSessionQuery(mirror) {
  const query = (method, params, signal) => hostCall('sessionQuery', method, params, { signal })
  const record = (session) => ({ header: codexHeader(session), live: mirror?.sessions.has(String(session?.id)) ?? false, persisted: true, title: session?.title ?? '' })
  const readSession = async (id) => {
    const result = await query('readSession', { id: String(id) })
    if (!result?.session) throw new SessionQueryError(`session not found: ${id}`, 'SESSION_NOT_FOUND')
    return { session: codexHeader(result.session), inheritedEventCount: 0, events: mapCodexEvents(result.events) }
  }
  return {
    async listSessions(signal) {
      return ((await query('listSessions', { limit: 200 }, signal)) ?? []).map(record)
    },
    async filterSessions(filters = [], signal) {
      const list = ((await query('listSessions', { limit: 500 }, signal)) ?? []).map(record)
      return list.filter((item) => (Array.isArray(filters) ? filters : []).every((filter) => {
        switch (filter?.kind) {
          case 'id':
            return filter.values?.includes(item.header.id)
          case 'cwd':
            return filter.values?.includes(item.header.cwd ?? null)
          case 'created-at':
            return (filter.from === undefined || item.header.createdAt >= filter.from) && (filter.to === undefined || item.header.createdAt <= filter.to)
          default:
            return true
        }
      }))
    },
    readSession,
    async readSurface(id) {
      const read = await readSession(id)
      return { ...read, lastSeq: read.events.length ? read.events.length - 1 : null }
    },
    async listEvents(id) {
      const read = await readSession(id)
      return read.events.map((event) => ({ sessionId: String(id), seq: event.seq, type: event.type, time: event.time, surface: event.surfaceOp !== undefined }))
    },
    async filterEvents(id, filters = []) {
      const types = new Set((Array.isArray(filters) ? filters : []).filter((filter) => filter?.kind === 'type').flatMap((filter) => filter.values ?? []))
      const read = await readSession(id)
      return read.events.filter((event) => !types.size || types.has(event.type))
    },
    async readTitle(id) {
      const title = await query('readTitle', { id: String(id) })
      return title ? { title: String(title), messageSeqs: [], source: { kind: 'provider', provider: 'codex' }, eventSeq: 0, updatedAt: Date.now() } : undefined
    },
    async readTitleSnapshots(ids) {
      return Promise.allSettled((ids ?? []).map((id) => this.readTitle(id)))
    },
    async readEvent({ sessionId, seq, before = 0, after = 0 } = {}) {
      const read = await readSession(sessionId)
      const index = read.events.findIndex((event) => event.seq === seq)
      if (index < 0) throw new SessionQueryError(`event not found: ${sessionId}#${seq}`, 'EVENT_NOT_FOUND')
      return { event: read.events[index], before: read.events.slice(Math.max(0, index - before), index), after: read.events.slice(index + 1, index + 1 + after) }
    },
    async traceSession(id) {
      return { sessionId: String(id), lineage: [String(id)] }
    },
    async observeSession(id) {
      const read = await readSession(id)
      return {
        source: 'prepared',
        header: read.session,
        inheritedEventCount: 0,
        events: read.events,
        cursor: read.events.length,
        retain() {
          return this
        },
        [Symbol.dispose]() {},
      }
    },
    async searchSessions({ query: text, limit, cursor } = {}) {
      const page = await query('searchSessions', { query: String(text ?? ''), limit, cursor })
      return { items: (page?.items ?? []).map(record), nextCursor: page?.nextCursor ?? null }
    },
    async searchEvents({ query: text, limit, cursor, sessionFilters } = {}) {
      const sessionId = sessionFilters?.find?.((filter) => filter?.kind === 'id')?.values?.[0]
      const page = await query('searchEvents', { query: String(text ?? ''), limit, cursor, ...(sessionId ? { sessionId } : {}) })
      return { items: (page?.items ?? []).map((item) => ({ sessionId: item.sessionId, event: mapCodexEvents([item.event])[0] ?? null })), nextCursor: page?.nextCursor ?? null }
    },
  }
}

/** ctx.llm：stream 经 Codex++ 协议代理发一次（整段返回后拆成 dsh 的流式块）。 */
export function createLlm() {
  const promptOf = (options) => (options?.messages ?? []).map((message) => {
    const role = message?.role === 'assistant' ? 'assistant' : message?.role === 'system' || message?.role === 'developer' ? 'developer' : 'user'
    return { role, content: messageText(message) }
  }).filter((item) => item.content)
  return {
    stream(options = {}) {
      return (async function* () {
        let result
        try {
          const model = options.provider && options.model && !String(options.model).includes('/') && options.provider !== 'codex'
            ? `${options.provider}/${options.model}`
            : options.model
          result = await hostCall('llm', 'complete', {
            input: promptOf(options),
            ...(options.system ? { instructions: String(options.system) } : {}),
            ...(model ? { model: String(model) } : {}),
            ...(options.reasoningEffort ? { reasoningEffort: String(options.reasoningEffort) } : {}),
            ...(options.maxTokens ? { maxOutputTokens: Number(options.maxTokens) } : {}),
            ...(options.temperature !== undefined ? { temperature: Number(options.temperature) } : {}),
          }, { signal: options.signal })
        } catch (error) {
          const aborted = options.signal?.aborted
          yield { type: 'finish', reason: { kind: aborted ? 'aborted' : 'error', failure: { message: String(error?.message ?? error), code: aborted ? 'ABORTED' : 'LLM_ERROR' } } }
          return
        }
        const text = String(result?.text ?? '')
        yield { type: 'block-start', index: 0, blockType: 'text' }
        if (text) yield { type: 'text-delta', index: 0, text }
        yield { type: 'block-end', index: 0, block: { type: 'text', text } }
        const usage = result?.usage ?? {}
        yield {
          type: 'usage',
          usage: {
            inputTokens: Number(usage.input_tokens ?? usage.prompt_tokens ?? 0),
            outputTokens: Number(usage.output_tokens ?? usage.completion_tokens ?? 0),
            totalTokens: Number(usage.total_tokens ?? 0) || undefined,
          },
        }
        yield { type: 'finish', reason: { kind: 'stop' } }
      })()
    },
    listProviders() {
      return [{ id: 'codex', name: 'Codex（经 Codex++）' }]
    },
    async listModels() {
      const listed = await hostCall('llm', 'listModels', {})
      return (listed?.models ?? []).map((model) => ({ provider: 'codex', id: String(model?.id ?? model), name: String(model?.name ?? model?.id ?? model) }))
    },
    async resolveModelInfo(provider, model) {
      return { provider, model, contextWindow: undefined }
    },
    resolveCallConfig(config) {
      return config
    },
    registerAdapter() {
      throw new Error('Codex++ 里不能注册 dsh 模型 adapter；要加模型供应商请在 Codex++ 拓展清单里声明 providers')
    },
    registerConfigurableProviders() {
      return () => {}
    },
  }
}

export function createUserQuestions() {
  return {
    async ask(request = {}) {
      const questions = Array.isArray(request.questions) ? request.questions : []
      if (!questions.length) throw new UserQuestionError('没有问题', 'EMPTY_QUESTIONS')
      const answers = []
      for (const question of questions) {
        const options = (question.options ?? []).map((option) => ({ value: String(option.label), label: String(option.label), description: option.description }))
        let reply
        try {
          reply = await hostCall('ui', 'ask', {
            title: String(question.header || question.question || ''),
            message: [question.header ? question.question : '', question.detail].filter(Boolean).join('\n\n'),
            ...(options.length ? { options } : { input: { placeholder: '' } }),
          }, { signal: request.signal })
        } catch (error) {
          throw new UserQuestionError(`无法在 Codex 里提问：${error?.message ?? error}`, 'NO_PROVIDER')
        }
        if (!reply) throw new UserQuestionError('用户取消了提问', 'ASK_ABORTED')
        answers.push(options.length ? { id: question.id, selected: [String(reply.value ?? reply.text ?? '')] } : { id: question.id, selected: [], custom: String(reply.text ?? reply.value ?? '') })
      }
      return { answers }
    },
    async askTimed(request, callId, timeoutMs) {
      return this.ask({ ...request, signal: AbortSignal.timeout(Math.max(1000, Number(timeoutMs) || 60_000)) })
    },
    answer() {
      return false
    },
  }
}

// ---------------------------------------------------------------------------
// 本机实现
// ---------------------------------------------------------------------------

const FS_CODES = { ENOENT: 'FS_NOT_FOUND', ENOTDIR: 'FS_NOT_DIRECTORY', EACCES: 'FS_PERMISSION_DENIED', EPERM: 'FS_PERMISSION_DENIED', EISDIR: 'FS_NOT_REGULAR_FILE' }
const fsError = (error, path) => (error instanceof FsError ? error : new FsError(`${path}: ${error?.message ?? error}`, FS_CODES[error?.code] ?? 'FS_IO_ERROR', { cause: error }))
const versionOf = (info) => `${info.mtimeMs}:${info.size}`

export function createFs() {
  const target = (path) => freeze({ targetKey: path, displayPath: path })
  const pathOf = (value) => (typeof value === 'string' ? value : value?.targetKey)
  const fs = {
    sandboxMode: undefined,
    async resolve(path, { cwd } = {}) {
      return target(isAbsolute(path) ? resolvePath(path) : resolvePath(cwd || process.cwd(), path))
    },
    processPath: (value) => pathOf(value),
    fileUrl: (value) => pathToFileURL(pathOf(value)).href,
    contains: (parent, child) => {
      const base = pathOf(parent).replace(/[\\/]+$/, '')
      const inner = pathOf(child)
      return inner === base || inner.startsWith(`${base}/`) || inner.startsWith(`${base}\\`)
    },
    async stat(value) {
      try {
        const info = await stat(pathOf(value))
        return { version: versionOf(info), type: info.isFile() ? 'file' : info.isDirectory() ? 'directory' : 'other', size: info.size }
      } catch (error) {
        if (error?.code === 'ENOENT') return undefined
        throw fsError(error, pathOf(value))
      }
    },
    async lstat(path, { cwd } = {}) {
      const full = isAbsolute(path) ? path : resolvePath(cwd || process.cwd(), path)
      try {
        const info = await lstat(full)
        return { version: versionOf(info), type: info.isSymbolicLink() ? 'symlink' : info.isFile() ? 'file' : info.isDirectory() ? 'directory' : 'other', size: info.size }
      } catch (error) {
        if (error?.code === 'ENOENT') return undefined
        throw fsError(error, full)
      }
    },
    async readText(value) {
      try {
        return await readFile(pathOf(value), 'utf8')
      } catch (error) {
        throw fsError(error, pathOf(value))
      }
    },
    async streamText(value) {
      const text = await fs.readText(value)
      return (async function* () {
        yield text
      })()
    },
    async readBytes(value, signal, maxBytes) {
      const bytes = await readFile(pathOf(value)).catch((error) => {
        throw fsError(error, pathOf(value))
      })
      if (Number.isFinite(maxBytes) && bytes.length > maxBytes) throw new FsError(`${pathOf(value)} 超过 ${maxBytes} 字节`, 'FS_TOO_LARGE')
      return new Uint8Array(bytes)
    },
    async readByteRange(value, { offset = 0, length } = {}) {
      const bytes = await fs.readBytes(value)
      return bytes.slice(offset, length === undefined ? undefined : offset + length)
    },
    async listDir(value) {
      try {
        const entries = await readdir(pathOf(value), { withFileTypes: true })
        return entries.map((entry) => ({ name: entry.name, type: entry.isFile() ? 'file' : entry.isDirectory() ? 'directory' : 'other', target: target(join(pathOf(value), entry.name)) }))
      } catch (error) {
        throw fsError(error, pathOf(value))
      }
    },
    async writeText(value, content, expected) {
      const path = pathOf(value)
      const before = await readFile(path, 'utf8').catch(() => null)
      if (expected?.kind === 'createIfAbsent' && before !== null) throw new FsError(`${path} 已存在`, 'FS_STALE_VERSION')
      if (expected?.kind === 'replaceIfVersion') {
        const current = await fs.stat(value)
        if (current?.version !== expected.version) throw new FsError(`${path} 已被改过`, 'FS_STALE_VERSION')
      }
      try {
        mkdirSync(dirname(path), { recursive: true })
        await writeFile(path, String(content))
      } catch (error) {
        throw fsError(error, path)
      }
      const info = await fs.stat(value)
      return { operation: before === null ? 'create' : 'update', version: info?.version, before, after: String(content) }
    },
    async editText(value, { oldString, newString, replaceAll } = {}, expected) {
      const path = pathOf(value)
      const before = await fs.readText(value)
      if (expected?.version) {
        const current = await fs.stat(value)
        if (current?.version !== expected.version) throw new FsError(`${path} 已被改过`, 'FS_STALE_VERSION')
      }
      const count = before.split(String(oldString)).length - 1
      if (count === 0) throw new FsError(`${path} 里找不到要替换的文字`, 'FS_EDIT_NOT_FOUND')
      if (count > 1 && !replaceAll) throw new FsError(`${path} 里要替换的文字出现了 ${count} 次`, 'FS_AMBIGUOUS_EDIT')
      const after = replaceAll ? before.split(String(oldString)).join(String(newString)) : before.replace(String(oldString), () => String(newString))
      await writeFile(path, after)
      const info = await fs.stat(value)
      return { version: info?.version, before, after }
    },
    async watch() {
      return async () => {}
    },
  }
  return fs
}

function collector(mode) {
  if (!mode || typeof mode !== 'object') return undefined
  const limit = Math.max(0, Number(mode.maxBytes) || 1024 * 1024)
  let text = ''
  let lossy = false
  return {
    push(chunk) {
      if (text.length >= limit) {
        lossy = true
        return
      }
      text += chunk
      if (text.length > limit) {
        text = text.slice(0, limit)
        lossy = true
      }
    },
    readFrom(offset = 0) {
      return { text: text.slice(offset), nextOffset: text.length, lossy }
    },
  }
}

export function createSubprocess() {
  return {
    spawn({ argv, cwd, stdio = {}, env, signal } = {}) {
      if (!Array.isArray(argv) || !argv.length) throw new TypeError('spawn 需要 argv')
      const pipeOf = (mode) => (mode === 'inherit' ? 'ignore' : mode === 'ignore' ? 'ignore' : 'pipe')
      const stdinMode = stdio.stdin && typeof stdio.stdin === 'object' ? 'pipe' : pipeOf(stdio.stdin ?? 'ignore')
      const child = spawn(argv[0], argv.slice(1), { cwd, env: env ? { ...process.env, ...env } : process.env, stdio: [stdinMode, pipeOf(stdio.stdout), pipeOf(stdio.stderr)], windowsHide: true, signal })
      if (stdio.stdin && typeof stdio.stdin === 'object') child.stdin.end(String(stdio.stdin.data ?? ''))
      const out = collector(stdio.stdout)
      const err = collector(stdio.stderr)
      if (out) child.stdout?.setEncoding('utf8').on('data', (chunk) => out.push(chunk))
      if (err) child.stderr?.setEncoding('utf8').on('data', (chunk) => err.push(chunk))
      const done = new Promise((resolve) => {
        child.on('error', () => resolve({ exitCode: null, signal: 'SPAWN_FAILED' }))
        child.on('close', (exitCode, exitSignal) => resolve({ exitCode, signal: exitSignal }))
      })
      return {
        stdin: stdio.stdin === 'pipe' ? child.stdin : undefined,
        stdout: out ? undefined : child.stdout ?? undefined,
        stderr: err ? undefined : child.stderr ?? undefined,
        collected: { stdout: out, stderr: err },
        done,
        terminate: () => child.kill(),
        async waitForExit(waitSignal) {
          await Promise.race([done, new Promise((resolve) => waitSignal?.addEventListener?.('abort', resolve, { once: true }))])
          return child.exitCode !== null || child.signalCode !== null
        },
      }
    },
    async resolveExecutable(command) {
      const which = process.platform === 'win32' ? 'where' : 'which'
      const found = await new Promise((resolve) => {
        const child = spawn(which, [command], { stdio: ['ignore', 'pipe', 'ignore'], windowsHide: true })
        let output = ''
        child.stdout.setEncoding('utf8').on('data', (chunk) => {
          output += chunk
        })
        child.on('error', () => resolve(''))
        child.on('close', () => resolve(output.trim().split(/\r?\n/)[0] ?? ''))
      })
      if (!found) throw new SubprocessExecutableNotFoundError(command)
      return found
    },
    async terminalEnvironment() {
      return { platform: process.platform === 'win32' ? 'windows' : 'posix', defaultShell: process.env.SHELL }
    },
    async spawnTerminal() {
      throw new Error('Codex++ 里没有终端（PTY）服务')
    },
  }
}

export function createShell() {
  const isWindows = process.platform === 'win32'
  return {
    sandboxMode: undefined,
    resolve(options = {}) {
      return { ...options, timeoutMs: Math.min(Number(options.timeoutMs) || 120_000, 600_000), stdoutMaxBytes: Number(options.stdoutMaxBytes) || 1024 * 1024 }
    },
    async execute(spec) {
      const argv = isWindows ? ['powershell.exe', '-NoProfile', '-Command', spec.command] : [process.env.SHELL || '/bin/bash', '-lc', spec.command]
      const started = Date.now()
      let timedOut = false
      let status = 'running'
      let stdout = ''
      let stderr = ''
      const child = spawn(argv[0], argv.slice(1), { cwd: spec.workdir || process.cwd(), env: { ...process.env, ...(spec.env ?? {}) }, stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true })
      if (spec.stdin !== undefined) child.stdin.end(String(spec.stdin))
      else child.stdin.end()
      child.stdout.setEncoding('utf8').on('data', (chunk) => {
        if (stdout.length < spec.stdoutMaxBytes) stdout += chunk
      })
      child.stderr.setEncoding('utf8').on('data', (chunk) => {
        if (stderr.length < spec.stdoutMaxBytes) stderr += chunk
      })
      const timer = spec.onExpiry === 'none' ? undefined : setTimeout(() => {
        timedOut = true
        child.kill()
      }, spec.timeoutMs)
      const abort = () => child.kill()
      spec.signal?.addEventListener?.('abort', abort, { once: true })
      const execution = {
        get status() {
          return status
        },
        exitCode: null,
        signal: null,
        done: undefined,
        observed: { stdout: '', stderr: '' },
        readOutput: () => ({ delta: stdout, lossy: stdout.length >= spec.stdoutMaxBytes }),
        kill: () => child.kill(),
        async result() {
          await execution.done
          return {
            exitCode: execution.exitCode,
            signal: execution.signal,
            timedOut,
            aborted: Boolean(spec.signal?.aborted),
            timeoutMs: spec.timeoutMs,
            durationMs: Date.now() - started,
            stdout: { text: stdout, truncated: stdout.length >= spec.stdoutMaxBytes },
            stderr: { text: stderr, truncated: stderr.length >= spec.stdoutMaxBytes },
          }
        },
      }
      execution.done = new Promise((resolve) => {
        child.on('error', (error) => {
          stderr += String(error?.message ?? error)
          status = 'completed'
          resolve()
        })
        child.on('close', (code, signal) => {
          clearTimeout(timer)
          execution.exitCode = code
          execution.signal = signal
          status = signal ? 'killed' : 'completed'
          resolve()
        })
      })
      return execution
    },
  }
}

export function createWeb() {
  return {
    async fetch({ url } = {}, signal) {
      const response = await fetch(String(url), { signal, redirect: 'follow' }).catch((error) => {
        throw new WebError(`抓取 ${url} 失败：${error?.message ?? error}`, 'WEB_FETCH_FAILED')
      })
      const type = response.headers.get('content-type') ?? ''
      const content = await response.text()
      const limit = 512 * 1024
      return { url: response.url, statusCode: response.status, body: { kind: type.includes('html') ? 'html' : 'text', content: content.slice(0, limit) }, truncated: content.length > limit }
    },
    async search() {
      throw new WebError('Codex++ 里没有联网搜索服务', 'WEB_PROVIDER_UNAVAILABLE')
    },
    registerFetchProvider() {
      return () => {}
    },
    registerSearchProvider() {
      return () => {}
    },
  }
}

/** ctx.credentials：引用的是环境变量，取值先看插件配置里同名的键，再看进程环境。 */
export function createCredentials(config) {
  const lookup = (ref) => {
    const name = typeof ref === 'string' ? ref : ref?.name
    if (!name) return undefined
    const fromConfig = config && typeof config === 'object' ? config[name] : undefined
    if (typeof fromConfig === 'string' && fromConfig) return { value: fromConfig, source: 'config' }
    if (typeof process.env[name] === 'string' && process.env[name]) return { value: process.env[name], source: 'env' }
    return undefined
  }
  return {
    async resolve(ref) {
      return lookup(ref)
    },
    async describe(ref) {
      const found = lookup(ref)
      return { configured: Boolean(found), source: found?.source, writable: false }
    },
    async set() {
      throw new Error('Codex++ 里不能写 dsh 凭据；请在 dsh 插件的配置里填写，或设置环境变量')
    },
    async unset() {
      throw new Error('Codex++ 里不能写 dsh 凭据')
    },
    async readRecord() {
      return undefined
    },
    async listRecords() {
      return []
    },
  }
}

/** ctx.storageDomain：每个领域一个 JSON 文件（插件数据目录下），表常驻内存。 */
export function createStorageDomain(dataDir) {
  const open = new Map()
  return {
    async open(spec) {
      const name = String(spec?.name ?? '')
      if (!name) throw new DomainError('领域缺少 name', 'missing-key')
      if (open.has(name)) throw new DomainError(`领域 ${name} 已经打开`, 'already-open')
      const path = dataDir ? join(dataDir, 'storage', `${name.replace(/[^\w.-]/g, '_')}.json`) : undefined
      let stored = {}
      if (path && existsSync(path)) {
        try {
          stored = JSON.parse(readFileSync(path, 'utf8'))
        } catch {
          stored = {}
        }
      }
      let closed = false
      let globalValue = stored.global !== undefined ? stored.global : spec.global?.initial
      const tables = new Map(Object.keys(spec.tables ?? {}).map((table) => [table, new Map(Object.entries(stored.tables?.[table] ?? {}))]))
      const save = async () => {
        if (!path) return
        mkdirSync(dirname(path), { recursive: true })
        const temp = `${path}.${process.pid}.tmp`
        writeFileSync(temp, JSON.stringify({ version: spec.version ?? 1, global: globalValue, tables: Object.fromEntries([...tables].map(([key, map]) => [key, Object.fromEntries(map)])) }))
        renameSync(temp, path)
      }
      const live = () => {
        if (closed) throw new DomainError(`领域 ${name} 已关闭`, 'closed')
      }
      const keyOf = (key) => (typeof key === 'string' ? key : JSON.stringify(key))
      open.set(name, true)
      return {
        global: {
          get: () => globalValue,
          async set(value) {
            live()
            globalValue = value
            await save()
          },
        },
        table(tableName) {
          live()
          const map = tables.get(tableName)
          if (!map) throw new DomainError(`领域 ${name} 没有表 ${tableName}`, 'missing-key')
          return {
            get: (key) => map.get(keyOf(key)),
            entries: () => [...map.entries()],
            keys: () => [...map.keys()],
            get size() {
              return map.size
            },
            async put(key, value) {
              live()
              map.set(keyOf(key), value)
              await save()
            },
            async delete(key) {
              live()
              const removed = map.delete(keyOf(key))
              if (removed) await save()
              return removed
            },
            async update(key, update) {
              live()
              const next = update(map.get(keyOf(key)))
              map.set(keyOf(key), next)
              await save()
              return next
            },
          }
        },
        async close() {
          closed = true
          open.delete(name)
        },
      }
    },
  }
}
