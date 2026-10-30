// dsh 插件 → Codex 钩子 / Codex++ 网关中间件的转发层。
//
// Codex++ 导入 dsh 插件时，为它生成一个配套拓展包：钩子脚本调用 runHook，网关中间件调用
// requestInstructions。插件在一个按需启动的常驻进程里加载（`node bridge.mjs daemon <配置>`，
// 经本地 socket 通信）：插件的内存状态（如按 agent 计数）跨事件保留，同一 Codex 会话拿到同一个 agent
// 对象；空闲一段时间、插件或配置变了、宿主升级了都会自动退出重启。常驻进程起不来时退回到
// 当前进程里临时加载一次（这时插件状态不跨事件）。
//
// 语义对照（与 dsh 自带的 Codex 钩子桥方向相反，见设计文档）：
//   tools/pre-execute   → PreToolUse：deny / cancel → permissionDecision deny；allow 不表态；ask 交给 Codex 自己的审批
//   tools/post-execute  → PostToolUse：block → decision block + reason；additionalContexts → additionalContext；
//                         替换结果（accept 带 content / value）Codex 钩子做不到，忽略
//   agent/pre-step      → UserPromptSubmit：reject → decision block；enter 里新增的消息 → additionalContext；
//                         改写或删掉用户原话 Codex 钩子做不到，忽略
//   agent/turn-stopping → Stop：监听器里 agent.steer(消息) → decision block + reason（Codex 会再走一步）；
//                         stop_hook_active 为真（已经多走过一轮）时不再拦，免得停不下来

import { AsyncLocalStorage } from 'node:async_hooks'
import { spawn } from 'node:child_process'
import { createHash } from 'node:crypto'
import { existsSync, readFileSync, realpathSync, statSync, unlinkSync } from 'node:fs'
import { createConnection, createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { EVENT_HOOKS, Host, INSTRUCTIONS_TARGET, errorText } from './host.mjs'

const BRIDGE_FILE = fileURLToPath(import.meta.url)
const HOST_FILE = fileURLToPath(new URL('./host.mjs', import.meta.url))
const HOOK_EVENTS = Object.fromEntries(Object.entries(EVENT_HOOKS).map(([event, hook]) => [hook, event]))
/** Codex 的工具名 → dsh 的工具名（插件多按 dsh 的名字判断）。参数形状两边一致（{ command }）。 */
const TOOL_ALIASES = { Bash: 'bash', shell: 'bash', local_shell: 'bash' }
/** 常驻进程空闲多久退出。 */
const DAEMON_IDLE_MS = 20 * 60 * 1000
/** 常驻进程定期检查配置文件是否还在（插件移除、测试目录清理后自行退出）。 */
const DAEMON_CHECK_MS = 30 * 1000
const DAEMON_START_TIMEOUT_MS = 5000
const DAEMON_REPLY_TIMEOUT_MS = 60 * 1000
const MAX_FRAME_BYTES = 8 * 1024 * 1024

// ---------------------------------------------------------------------------
// 加载插件
// ---------------------------------------------------------------------------

function readConfig(config) {
  if (typeof config === 'string') return JSON.parse(readFileSync(config, 'utf8'))
  return config
}

/** 注册表里的插件记录；停用、移除或没有可映射的行时返回 undefined。 */
function readPlugin(config) {
  const { registry: registryPath, plugin: pluginId } = readConfig(config)
  const registry = JSON.parse(readFileSync(registryPath, 'utf8'))
  const plugin = (registry.plugins ?? []).find((item) => item?.id === pluginId)
  if (!plugin || plugin.enabled === false || typeof plugin.root !== 'string') return undefined
  return plugin
}

/** 映射到 target（不给则任意映射）且没被停用的行。 */
function mappedRows(plugin, target) {
  const disabled = new Set(plugin.disabledRows ?? [])
  return (plugin.report?.rows ?? [])
    .filter((row) => !disabled.has(row.id) && (row.mappings ?? []).some((item) => target === undefined || item === target))
    .map((row) => row.id)
}

async function loadHost(plugin, rows) {
  const host = new Host()
  await host.loadPackage(plugin.root, plugin.id, {
    disabledRows: plugin.disabledRows,
    configs: plugin.configs,
    onlyRows: rows,
    skipSkills: true,
  })
  await host.emitReady()
  return host
}

/** 插件、配置或宿主文件任何一处变了，常驻进程就要重启。 */
function versionOf(plugin) {
  const mtime = (path) => {
    try {
      return statSync(path).mtimeMs
    } catch {
      return 0
    }
  }
  return createHash('sha256').update(JSON.stringify({
    root: plugin.root,
    disabledRows: plugin.disabledRows ?? [],
    configs: plugin.configs ?? {},
    rows: (plugin.report?.rows ?? []).map((row) => [row.id, row.mappings ?? []]),
    importedAt: plugin.importedAt,
    // 每次重新分析都会变：改了本地源码后点「重新分析」，常驻进程随之重启。
    inspectedAt: plugin.report?.inspectedAt,
    files: [mtime(BRIDGE_FILE), mtime(HOST_FILE)],
  })).digest('hex')
}

// ---------------------------------------------------------------------------
// 事件翻译
// ---------------------------------------------------------------------------

function blocksText(blocks) {
  if (typeof blocks === 'string') return blocks
  if (!Array.isArray(blocks)) return ''
  return blocks
    .map((block) => (block?.type === 'text' && typeof block.text === 'string' ? block.text : ''))
    .filter(Boolean)
    .join('\n')
}

function messageText(message) {
  if (typeof message === 'string') return message
  return blocksText(message?.content)
}

function messagesText(messages) {
  return (messages ?? []).map(messageText).map((text) => text.trim()).filter(Boolean).join('\n\n')
}

/** 本次事件里监听器经 agent.steer / inject 交出的消息。 */
const sinks = new AsyncLocalStorage()

/** 每个 Codex 会话一个 agent 替身（常驻进程里保持同一个对象，插件按 agent 记的状态才能延续）。 */
class Agents {
  constructor() {
    this.bySession = new Map()
  }

  get(payload) {
    const id = String(payload?.session_id ?? '')
    let agent = this.bySession.get(id)
    if (!agent) {
      const target = {
        session: { id, header: { id } },
        options: {},
        steer(message) {
          sinks.getStore()?.steer.push(message)
        },
        inject(message) {
          sinks.getStore()?.inject.push(message)
        },
      }
      agent = new Proxy(target, {
        get(object, key) {
          return key in object ? object[key] : undefined
        },
      })
      this.bySession.set(id, agent)
    }
    agent.options.model = payload?.model
    return agent
  }
}

function makeExec(payload, agent, signal) {
  const callId = String(payload?.tool_use_id ?? 'codexpp-call')
  const name = String(payload?.tool_name ?? '')
  return {
    callId,
    rootCallId: callId,
    name: TOOL_ALIASES[name] ?? name,
    arguments: payload?.tool_input ?? {},
    agent,
    signal,
    token: Symbol('codexpp.hook'),
  }
}

function makeResult(response) {
  const text = typeof response === 'string' ? response : JSON.stringify(response ?? null)
  return Object.freeze({ isError: false, value: response ?? null, content: [{ type: 'text', text }] })
}

function withContext(event, output, contexts) {
  const text = contexts.filter(Boolean).join('\n\n')
  if (!text) return output
  return { ...output, hookSpecificOutput: { ...(output.hookSpecificOutput ?? {}), hookEventName: event, additionalContext: text } }
}

const TRANSLATE = {
  async PreToolUse(host, payload, agent, sink, signal) {
    const exec = makeExec(payload, agent, signal)
    const decision = await host.waterfall('tools/pre-execute', [exec], async () => ({ kind: 'allow' }))
    const contexts = [messagesText(sink.inject), messagesText(sink.steer)]
    if (decision?.kind === 'deny' || decision?.kind === 'cancel') {
      const reason = decision.kind === 'cancel' ? 'dsh 插件取消了这次工具调用' : String(decision.reason || '被 dsh 插件拒绝')
      return withContext('PreToolUse', {
        hookSpecificOutput: { hookEventName: 'PreToolUse', permissionDecision: 'deny', permissionDecisionReason: reason },
      }, contexts)
    }
    return withContext('PreToolUse', {}, contexts)
  },

  async PostToolUse(host, payload, agent, sink, signal) {
    const exec = makeExec(payload, agent, signal)
    const decision = await host.waterfall('tools/post-execute', [exec, makeResult(payload?.tool_response)], async () => ({ kind: 'accept' }))
    const contexts = [messagesText(decision?.additionalContexts), messagesText(sink.inject), messagesText(sink.steer)]
    if (decision?.kind === 'block') {
      return withContext('PostToolUse', { decision: 'block', reason: blocksText(decision.feedback) || '被 dsh 插件拦下' }, contexts)
    }
    return withContext('PostToolUse', {}, contexts)
  },

  async UserPromptSubmit(host, payload, agent, sink, signal) {
    const original = Object.freeze({
      id: 'codexpp-user-prompt',
      role: 'user',
      content: [{ type: 'text', text: String(payload?.prompt ?? '') }],
      source: { kind: 'user' },
    })
    const step = { agent, messages: [original], turn: 0, step: 0, signal, modelSelection: { model: payload?.model } }
    const decision = await host.waterfall('agent/pre-step', [step], async () => ({ kind: 'enter', messages: [original] }))
    if (decision?.kind === 'reject') {
      return { decision: 'block', reason: '这条输入被 dsh 插件拒绝' }
    }
    const added = (decision?.messages ?? []).filter((message) => message !== original && message?.id !== original.id)
    return withContext('UserPromptSubmit', {}, [messagesText(added), messagesText(sink.inject), messagesText(sink.steer)])
  },

  async Stop(host, payload, agent, sink, signal) {
    // 已经因为 Stop 钩子多走过一轮：不再拦，避免无条件 steer 的插件让 Codex 停不下来。
    if (payload?.stop_hook_active === true) return {}
    await host.serial('agent/turn-stopping', [{ agent, turn: 0, signal }])
    const text = messagesText(sink.steer)
    if (text) return { decision: 'block', reason: text }
    return {}
  },
}

async function dispatchHook(host, agents, hookEvent, payload) {
  const translate = TRANSLATE[hookEvent]
  if (!translate || !HOOK_EVENTS[hookEvent]) return null
  const controller = new AbortController()
  const sink = { steer: [], inject: [] }
  try {
    const output = await sinks.run(sink, () => translate(host, payload ?? {}, agents.get(payload), sink, controller.signal))
    return output && Object.keys(output).length > 0 ? output : null
  } catch (error) {
    throw new Error(`dsh 插件处理 ${hookEvent} 失败：${errorText(error)}`)
  } finally {
    controller.abort()
  }
}

/** 把一段文本追加到 Responses 请求体的 instructions；已经包含时不重复追加。返回新 body 或 null（不改）。 */
export function appendInstructions(body, text) {
  const addition = typeof text === 'string' ? text.trim() : ''
  if (!addition || !body || typeof body !== 'object') return null
  const current = typeof body.instructions === 'string' ? body.instructions : ''
  if (current.includes(addition)) return null
  return { ...body, instructions: current ? `${current}\n\n${addition}` : addition }
}

function instructionsFor(host, body) {
  return appendInstructions(body, host.assemblePrompt({ modelSelection: { model: body?.model } }))
}

// ---------------------------------------------------------------------------
// 常驻进程
// ---------------------------------------------------------------------------

function socketPath(configPath) {
  let key = configPath
  try {
    key = realpathSync(configPath)
  } catch {
    // 配置不在了也照样算出路径，连接会失败再退回。
  }
  const hash = createHash('sha256').update(`${process.getuid?.() ?? ''}:${key}`).digest('hex').slice(0, 16)
  return process.platform === 'win32' ? `\\\\.\\pipe\\codexpp-dsh-${hash}` : join(tmpdir(), `codexpp-dsh-${hash}.sock`)
}

/** 发一帧（JSON 一行）并等回复。连不上时抛出 code 为 ENOENT / ECONNREFUSED 的错误。 */
function exchange(path, message, timeoutMs) {
  return new Promise((resolve, reject) => {
    const socket = createConnection(path)
    let buffer = ''
    const timer = setTimeout(() => {
      socket.destroy()
      reject(new Error('dsh 插件常驻进程没有及时回复'))
    }, timeoutMs)
    socket.setEncoding('utf8')
    socket.on('connect', () => socket.write(`${JSON.stringify(message)}\n`))
    socket.on('data', (chunk) => {
      buffer += chunk
      const end = buffer.indexOf('\n')
      if (end < 0) {
        if (buffer.length > MAX_FRAME_BYTES) socket.destroy(new Error('回复过大'))
        return
      }
      clearTimeout(timer)
      socket.end()
      try {
        resolve(JSON.parse(buffer.slice(0, end)))
      } catch (error) {
        reject(error)
      }
    })
    socket.on('error', (error) => {
      clearTimeout(timer)
      reject(error)
    })
  })
}

function startDaemon(configPath) {
  const child = spawn(process.execPath, [BRIDGE_FILE, 'daemon', configPath], {
    detached: true,
    // 不继承任何管道：钩子运行器读到 stdout 结束才返回，常驻进程不能拿着它。
    stdio: 'ignore',
    windowsHide: true,
  })
  child.unref()
}

const sleep = (ms) => new Promise((done) => setTimeout(done, ms))
const unreachable = (error) => ['ENOENT', 'ECONNREFUSED', 'ECONNRESET', 'EPIPE'].includes(error?.code)

/** 经常驻进程处理一条请求；返回 { output }。常驻进程起不来时抛出 { unreachable: true } 的错误。 */
async function viaDaemon(configPath, plugin, request) {
  const path = socketPath(configPath)
  const message = { ...request, version: versionOf(plugin) }
  let started = false
  const deadline = Date.now() + DAEMON_START_TIMEOUT_MS
  for (;;) {
    let reply
    try {
      reply = await exchange(path, message, DAEMON_REPLY_TIMEOUT_MS)
    } catch (error) {
      if (!unreachable(error)) throw error
      if (!started) {
        startDaemon(configPath)
        started = true
      }
      if (Date.now() > deadline) throw Object.assign(new Error('dsh 插件常驻进程没有启动'), { unreachable: true })
      await sleep(50)
      continue
    }
    if (reply?.restart) {
      // 旧进程正在退出；等它让出 socket 再起新的。
      started = false
      if (Date.now() > deadline) throw Object.assign(new Error('dsh 插件常驻进程没有重启'), { unreachable: true })
      await sleep(50)
      continue
    }
    if (reply?.error) throw new Error(reply.error)
    return reply ?? { output: null }
  }
}

async function runDaemon(configPath) {
  const plugin = readPlugin(configPath)
  if (!plugin) return
  const version = versionOf(plugin)
  const rows = mappedRows(plugin)
  if (rows.length === 0) return
  const host = await loadHost(plugin, rows)
  const agents = new Agents()
  const path = socketPath(configPath)
  let idle
  let closing = false
  const server = createServer({ allowHalfOpen: true })
  const shutdown = async () => {
    if (closing) return
    closing = true
    clearTimeout(idle)
    server.close()
    if (process.platform !== 'win32') {
      try {
        unlinkSync(path)
      } catch {
        // 已被新进程接管或清理。
      }
    }
    await host.dispose().catch(() => {})
    process.exit(0)
  }
  const touch = () => {
    clearTimeout(idle)
    idle = setTimeout(shutdown, DAEMON_IDLE_MS)
  }
  const handle = async (message) => {
    if (message?.kind === 'ping') return { ok: true }
    if (message?.version !== version) {
      setImmediate(shutdown)
      return { restart: true }
    }
    if (message.kind === 'hook') return { output: await dispatchHook(host, agents, message.event, message.payload) }
    if (message.kind === 'instructions') return { output: instructionsFor(host, message.body) }
    return { error: `不认识的请求 ${message.kind}` }
  }
  server.on('connection', (socket) => {
    touch()
    let buffer = ''
    socket.setEncoding('utf8')
    socket.on('data', (chunk) => {
      buffer += chunk
      const end = buffer.indexOf('\n')
      if (end < 0) {
        if (buffer.length > MAX_FRAME_BYTES) socket.destroy()
        return
      }
      const line = buffer.slice(0, end)
      buffer = ''
      Promise.resolve()
        .then(() => handle(JSON.parse(line)))
        .catch((error) => ({ error: errorText(error) }))
        .then((reply) => socket.end(`${JSON.stringify(reply)}\n`))
    })
    socket.on('error', () => {})
  })
  const listen = () => new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(path, () => {
      server.off('error', reject)
      resolve()
    })
  })
  try {
    await listen()
  } catch (error) {
    if (error?.code !== 'EADDRINUSE' || process.platform === 'win32') throw error
    // 有人在听就让给它；是崩溃留下的 socket 文件就清掉重来。
    const alive = await exchange(path, { kind: 'ping', version: '' }, 1000).then(() => true, () => false)
    if (alive) {
      await host.dispose()
      return
    }
    unlinkSync(path)
    await listen()
  }
  touch()
  setInterval(() => {
    if (!existsSync(configPath)) void shutdown()
  }, DAEMON_CHECK_MS).unref()
}

// ---------------------------------------------------------------------------
// 配套拓展包调用的入口
// ---------------------------------------------------------------------------

/** 钩子脚本入口：返回 Codex 钩子输出，没有意见时返回 null。config 为配套拓展包里 dsh-bridge.json 的路径。 */
export async function runHook(config, hookEvent, payload) {
  if (!TRANSLATE[hookEvent]) return null
  const plugin = readPlugin(config)
  if (!plugin || mappedRows(plugin, hookEvent).length === 0) return null
  if (typeof config === 'string') {
    try {
      return (await viaDaemon(config, plugin, { kind: 'hook', event: hookEvent, payload })).output ?? null
    } catch (error) {
      if (!error?.unreachable) throw error
      console.error(`[dsh-bridge] ${errorText(error)}，改为本进程临时加载（插件状态不跨事件）`)
    }
  }
  const host = await loadHost(plugin, mappedRows(plugin, hookEvent))
  try {
    return await dispatchHook(host, new Agents(), hookEvent, payload)
  } finally {
    await host.dispose()
  }
}

/** 网关中间件入口（提示词里有动态片段时用）：求出提示词，追加到 instructions。 */
export async function requestInstructions(config, body) {
  const plugin = readPlugin(config)
  if (!plugin || mappedRows(plugin, INSTRUCTIONS_TARGET).length === 0) return null
  if (typeof config === 'string') {
    try {
      return (await viaDaemon(config, plugin, { kind: 'instructions', body })).output ?? null
    } catch (error) {
      if (!error?.unreachable) throw error
    }
  }
  const host = await loadHost(plugin, mappedRows(plugin, INSTRUCTIONS_TARGET))
  try {
    return instructionsFor(host, body)
  } finally {
    await host.dispose()
  }
}

function isMainModule() {
  if (!process.argv[1]) return false
  try {
    return realpathSync(process.argv[1]) === realpathSync(BRIDGE_FILE)
  } catch {
    return false
  }
}

if (isMainModule() && process.argv[2] === 'daemon' && process.argv[3]) {
  runDaemon(process.argv[3]).catch((error) => {
    console.error(`[dsh-bridge] 常驻进程退出：${errorText(error)}`)
    process.exit(1)
  })
}
