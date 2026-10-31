// dsh 插件 → Codex 钩子 / Codex++ 网关中间件 / 拓展命令的转发层。
//
// Codex++ 导入 dsh 插件时，为它生成一个配套拓展包：钩子脚本调用 runHook，网关中间件调用 requestGateway /
// requestError，页面命令经后台调用 runCommand。插件在一个按需启动的常驻进程里加载（`node bridge.mjs daemon <配置>`，
// 经本地 socket 通信）：插件的内存状态（如按 agent 计数）跨事件保留，同一 Codex 会话拿到同一个 agent
// 对象；空闲一段时间、插件或配置变了、宿主升级了都会自动退出重启。常驻进程起不来时退回到
// 当前进程里临时加载一次（这时插件状态不跨事件）。
//
// 常驻进程同时维护会话镜像（services.mjs 的 SessionMirror）：每次钩子先把 Codex 的事件翻成 dsh 会话日志
// （turn/start、user/message、tool/call、tool/result、turn/end、compaction/*）追加进镜像，插件的 session/event、
// sessionProjections、agent/status 由此驱动。
//
// 语义对照（与 dsh 自带的 Codex 钩子桥方向相反，见设计文档）：
//   tools/pre-execute   → PreToolUse：deny / cancel → permissionDecision deny；allow 不表态；ask 交给 Codex 自己的审批
//   tools/post-execute  → PostToolUse：block → decision block + reason；additionalContexts → additionalContext；
//                         替换结果（accept 带 content / value）Codex 钩子做不到，忽略；随后发 tools/result
//   agent/pre-step      → UserPromptSubmit：reject → decision block；enter 里新增的消息 → additionalContext；
//                         改写或删掉用户原话 Codex 钩子做不到，忽略
//   agent/turn-stopping → Stop：监听器里 agent.steer(消息) → decision block + reason（Codex 会再走一步）；
//                         stop_hook_active 为真（已经多走过一轮）时不再拦，免得停不下来
//   agent/created       → SessionStart：监听器里 steer / inject 的消息 → additionalContext
//   subagent/start、end → SubagentStart / SubagentStop（通知，不表态）
//   agent/request       → 网关请求中间件：改 model / reasoning.effort
//   system-prompt/assemble → 网关请求中间件：插件改写后的片段追加到 instructions
//   agent/request-error → 网关失败重试：{ kind: 'retry' } → 重发

import { AsyncLocalStorage } from 'node:async_hooks'
import { spawn } from 'node:child_process'
import { createHash } from 'node:crypto'
import { existsSync, readFileSync, realpathSync, statSync, unlinkSync } from 'node:fs'
import { createConnection, createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { EVENT_HOOKS, Host, INSTRUCTIONS_TARGET, MIRROR_HOOKS, MIRROR_TARGET, errorText } from './host.mjs'
import { messageText } from './services.mjs'

const BRIDGE_FILE = fileURLToPath(import.meta.url)
const HOST_FILE = fileURLToPath(new URL('./host.mjs', import.meta.url))
const SERVICES_FILE = fileURLToPath(new URL('./services.mjs', import.meta.url))
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
  return { ...plugin, registryPath }
}

/** 映射到 target（不给则任意映射）且没被停用的行。会话镜像行收全部 MIRROR_HOOKS。 */
function mappedRows(plugin, target) {
  const disabled = new Set(plugin.disabledRows ?? [])
  const matches = (item) => target === undefined || item === target || (item === MIRROR_TARGET && MIRROR_HOOKS.includes(target))
  return (plugin.report?.rows ?? [])
    .filter((row) => !disabled.has(row.id) && (row.mappings ?? []).some(matches))
    .map((row) => row.id)
}

/** 插件的数据目录：注册表旁边的 data/<插件 id>（会话镜像与 storageDomain 的文件在这里）。 */
function dataDirOf(plugin) {
  return plugin.registryPath ? join(dirname(plugin.registryPath), 'data', String(plugin.id).replace(/[^\w.-]/g, '_')) : undefined
}

async function loadHost(plugin, rows, { persistent = false } = {}) {
  const dataDir = dataDirOf(plugin)
  // 临时加载（常驻进程起不来）时不落盘镜像，免得和常驻进程写同一个文件。
  const host = new Host({ mirrorDir: persistent && dataDir ? join(dataDir, 'mirror') : undefined })
  await host.loadPackage(plugin.root, plugin.id, {
    disabledRows: plugin.disabledRows,
    configs: plugin.configs,
    onlyRows: rows,
    skipSkills: true,
    dataDir,
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
    files: [mtime(BRIDGE_FILE), mtime(HOST_FILE), mtime(SERVICES_FILE)],
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

function messagesText(messages) {
  return (messages ?? []).map(messageText).map((text) => text.trim()).filter(Boolean).join('\n\n')
}

/** 本次事件里监听器经 agent.steer / inject 交出的消息。 */
const sinks = new AsyncLocalStorage()

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

/** 本次事件交出的消息，加上钩子之外排下、等着带上的上下文。 */
const sinkContexts = (sink) => [messagesText(sink.pending), messagesText(sink.inject), messagesText(sink.steer)]

const TRANSLATE = {
  async SessionStart(host, payload, agent, sink, signal) {
    const source = ['resume', 'clear', 'compact'].includes(payload?.source) ? payload.source : 'startup'
    await host.serial('agent/created', [{ agent, source, signal }])
    return withContext('SessionStart', {}, sinkContexts(sink))
  },

  async PreToolUse(host, payload, agent, sink, signal) {
    const exec = makeExec(payload, agent, signal)
    const decision = await host.waterfall('tools/pre-execute', [exec], async () => ({ kind: 'allow' }))
    const contexts = sinkContexts(sink)
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
    const result = makeResult(payload?.tool_response)
    const decision = await host.waterfall('tools/post-execute', [exec, result], async () => ({ kind: 'accept' }))
    host.emitEvent('tools/result', [exec, result])
    const contexts = [messagesText(decision?.additionalContexts), ...sinkContexts(sink)]
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
    agent.setStatus('running')
    const step = { agent, messages: [original], turn: agent.session.turn, step: 1, signal, modelSelection: { model: payload?.model } }
    const decision = await host.waterfall('agent/pre-step', [step], async () => ({ kind: 'enter', messages: [original] }))
    if (decision?.kind === 'reject') {
      agent.setStatus('idle')
      return { decision: 'block', reason: '这条输入被 dsh 插件拒绝' }
    }
    const added = (decision?.messages ?? []).filter((message) => message !== original && message?.id !== original.id)
    return withContext('UserPromptSubmit', {}, [messagesText(added), ...sinkContexts(sink)])
  },

  async Stop(host, payload, agent, sink, signal) {
    // 已经因为 Stop 钩子多走过一轮：不再拦，避免无条件 steer 的插件让 Codex 停不下来。
    if (payload?.stop_hook_active === true) {
      agent.setStatus('idle')
      return {}
    }
    await host.serial('agent/turn-stopping', [{ agent, turn: agent.session.turn, signal }])
    const text = messagesText(sink.steer)
    if (text) return { decision: 'block', reason: text }
    agent.setStatus('idle')
    return {}
  },

  async SubagentStart(host, payload) {
    host.emitEvent('subagent/start', [{ runId: String(payload?.agent_id ?? payload?.tool_use_id ?? ''), provider: 'codex', id: String(payload?.agent_id ?? ''), local: true, agentType: payload?.agent_type }])
    return {}
  },

  async SubagentStop(host, payload) {
    host.emitEvent('subagent/end', [{
      runId: String(payload?.agent_id ?? payload?.tool_use_id ?? ''),
      provider: 'codex',
      id: String(payload?.agent_id ?? ''),
      local: true,
      stopReason: 'completed',
      lastAssistantMessage: payload?.last_assistant_message ? [{ type: 'text', text: String(payload.last_assistant_message) }] : undefined,
    }])
    return {}
  },

  // 只喂会话镜像（compaction/start、compaction/end），不表态。
  async PreCompact() {
    return {}
  },
  async PostCompact() {
    return {}
  },
}

/** 钩子事件对应的 dsh 监听器有没有人听：没人听又不需要镜像时直接不表态。 */
const HOOK_LISTENS = Object.fromEntries(Object.entries(EVENT_HOOKS).map(([event, hook]) => [hook, event]))

async function dispatchHook(host, hookEvent, payload) {
  const translate = TRANSLATE[hookEvent]
  if (!translate) return null
  host.mirror.recordHook(hookEvent, payload ?? {})
  const agent = host.agentRegistry.get(payload?.session_id)
  if (!agent) return null
  if (payload?.model) agent.session.model = payload.model
  const listened = HOOK_LISTENS[hookEvent] && host.hasListeners(HOOK_LISTENS[hookEvent])
  const controller = new AbortController()
  const sink = { agent, steer: [], inject: [], pending: [] }
  // 钩子之外排下的上下文：有输出能带上的钩子才取走。
  if (['SessionStart', 'UserPromptSubmit', 'PreToolUse', 'PostToolUse'].includes(hookEvent)) sink.pending = agent.takePending()
  host.agentRegistry.currentSink = () => sinks.getStore()
  try {
    if (!listened && !sink.pending.length && hookEvent !== 'UserPromptSubmit' && hookEvent !== 'Stop') {
      if (hookEvent === 'PostToolUse') host.emitEvent('tools/result', [makeExec(payload, agent, controller.signal), makeResult(payload?.tool_response)])
      if (hookEvent.startsWith('Subagent')) await translate(host, payload ?? {}, agent, sink, controller.signal)
      return null
    }
    const output = await sinks.run(sink, () => translate(host, payload ?? {}, agent, sink, controller.signal))
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

/** 请求属于哪个 Codex 会话：Codex 把会话 id 放在 prompt_cache_key。 */
const sessionIdOf = (body) => (typeof body?.prompt_cache_key === 'string' ? body.prompt_cache_key : '')

/** 网关请求中间件：插件的提示词追加到 instructions，agent/request 改模型与推理强度。返回新 body 或 null。 */
async function gatewayFor(host, body, context = {}) {
  if (!body || typeof body !== 'object') return null
  let next = body
  let changed = false
  const agent = sessionIdOf(body) ? host.agentRegistry.get(sessionIdOf(body)) : undefined
  const prompt = await host.assemblePromptAsync({ agent, modelSelection: { model: body.model } })
  const appended = appendInstructions(next, prompt)
  if (appended) {
    next = appended
    changed = true
  }
  if (host.hasListeners('agent/request')) {
    const model = String(next.model ?? '')
    const slash = model.indexOf('/')
    const current = { provider: slash > 0 ? model.slice(0, slash) : 'codex', model, reasoningEffort: next.reasoning?.effort }
    const controller = new AbortController()
    const decided = await host.waterfall('agent/request', [{ agent, turn: agent?.session.turn ?? 0, step: Number(context?.step?.index) || 1, signal: controller.signal }], async () => current)
    controller.abort()
    if (decided?.model && decided.model !== current.model) {
      next = { ...next, model: String(decided.model) }
      changed = true
    }
    if (decided?.reasoningEffort && decided.reasoningEffort !== current.reasoningEffort) {
      next = { ...next, reasoning: { ...(next.reasoning ?? {}), effort: String(decided.reasoningEffort) } }
      changed = true
    }
  }
  return changed ? next : null
}

/** 网关失败重试：agent/request-error 返回 { kind: 'retry' } 就重发。 */
async function errorFor(host, error) {
  if (!host.hasListeners('agent/request-error')) return null
  const controller = new AbortController()
  const failure = { message: String(error?.json?.error?.message ?? error?.body ?? '').slice(0, 2000), code: String(error?.json?.error?.code ?? error?.status ?? 'ERROR'), status: error?.status }
  const action = await host.waterfall('agent/request-error', [{ agent: undefined, turn: 0, step: 1, provider: 'codex', failure, retryPolicy: undefined, signal: controller.signal }], async () => undefined)
  controller.abort()
  return action?.kind === 'retry' ? { retry: true, delayMs: Number(action.delayMs) || 1000 } : null
}

async function commandFor(host, { name, rawInput, sessionId }) {
  const agent = sessionId ? host.agentRegistry.get(sessionId) : undefined
  return (await host.runCommand(name, rawInput, agent)) ?? { kind: 'error', text: `没有命令 /${name}` }
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
  const host = await loadHost(plugin, rows, { persistent: true })
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
    host.mirror.flushAll()
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
    if (message.kind === 'hook') return { output: await dispatchHook(host, message.event, message.payload) }
    if (message.kind === 'instructions' || message.kind === 'gateway') return { output: await gatewayFor(host, message.body, message.context) }
    if (message.kind === 'error') return { output: await errorFor(host, message.error) }
    if (message.kind === 'command') return { output: await commandFor(host, message) }
    if (message.kind === 'fetch') return { output: await host.fetchRoute(message.request) }
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

/** 经常驻进程处理一条请求；常驻进程起不来时在本进程临时加载 rows 再处理。 */
async function handle(config, plugin, rows, request, local) {
  if (typeof config === 'string') {
    try {
      return (await viaDaemon(config, plugin, request)).output ?? null
    } catch (error) {
      if (!error?.unreachable) throw error
      console.error(`[dsh-bridge] ${errorText(error)}，改为本进程临时加载（插件状态不跨事件）`)
    }
  }
  const host = await loadHost(plugin, rows)
  try {
    return await local(host)
  } finally {
    await host.dispose()
  }
}

/** 钩子脚本入口：返回 Codex 钩子输出，没有意见时返回 null。config 为配套拓展包里 dsh-bridge.json 的路径。 */
export async function runHook(config, hookEvent, payload) {
  if (!TRANSLATE[hookEvent]) return null
  const plugin = readPlugin(config)
  const rows = plugin ? mappedRows(plugin, hookEvent) : []
  if (!rows.length) return null
  return handle(config, plugin, mappedRows(plugin), { kind: 'hook', event: hookEvent, payload }, (host) => dispatchHook(host, hookEvent, payload))
}

/** 网关请求中间件入口：提示词追加到 instructions、agent/request 改模型。返回新 body 或 null。 */
export async function requestGateway(config, body, context = {}) {
  const plugin = readPlugin(config)
  if (!plugin) return null
  const rows = [...new Set([...mappedRows(plugin, INSTRUCTIONS_TARGET), ...mappedRows(plugin, 'gateway:instructions'), ...mappedRows(plugin, 'gateway:request')])]
  if (!rows.length) return null
  const { step } = context ?? {}
  return handle(config, plugin, mappedRows(plugin), { kind: 'gateway', body, context: { step } }, (host) => gatewayFor(host, body, { step }))
}

/** 旧版配套拓展包的入口名。 */
export const requestInstructions = (config, body) => requestGateway(config, body)

/** 网关失败重试入口：返回 { retry, delayMs } 或 null。 */
export async function requestError(config, error) {
  const plugin = readPlugin(config)
  if (!plugin || !mappedRows(plugin, 'gateway:error').length) return null
  return handle(config, plugin, mappedRows(plugin), { kind: 'error', error }, (host) => errorFor(host, error))
}

/** 前端半边的 HTTP 请求入口（配套拓展包的后台调用）：{ path, method, headers, body } → { status, headers, body }。 */
export async function runFetch(config, request) {
  const plugin = readPlugin(config)
  if (!plugin || !mappedRows(plugin, 'connection').length) return { status: 503, headers: {}, body: JSON.stringify({ ok: false, error: { message: '插件已停用' } }) }
  return handle(config, plugin, mappedRows(plugin), { kind: 'fetch', request }, (host) => host.fetchRoute(request))
}

/** 页面斜杠命令入口（配套拓展包的后台调用）：返回 dsh 的 CommandResult。 */
export async function runCommand(config, name, rawInput, sessionId) {
  const plugin = readPlugin(config)
  if (!plugin || !mappedRows(plugin, 'commands').length) return { kind: 'error', text: '插件已停用' }
  const request = { kind: 'command', name: String(name), rawInput: String(rawInput ?? ''), sessionId: sessionId ? String(sessionId) : '' }
  return handle(config, plugin, mappedRows(plugin), request, (host) => commandFor(host, request))
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
