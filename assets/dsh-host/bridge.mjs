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
import { existsSync, mkdirSync, readFileSync, realpathSync, statSync, unlinkSync, writeFileSync } from 'node:fs'
import { createServer as createHttpServer } from 'node:http'
import { createConnection, createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { EVENT_HOOKS, Host, INSTRUCTIONS_TARGET, MIRROR_HOOKS, MIRROR_TARGET, errorText, guardProcessExit, hostExit } from './host.mjs'
import { PLAN_MODE_WRITE_TOOLS, createSchedule, messageText } from './services.mjs'

const BRIDGE_FILE = fileURLToPath(import.meta.url)
const HOST_FILE = fileURLToPath(new URL('./host.mjs', import.meta.url))
const SERVICES_FILE = fileURLToPath(new URL('./services.mjs', import.meta.url))
/** Codex 的工具名 → dsh 的工具名（插件多按 dsh 的名字判断）。参数形状两边一致（{ command }）。 */
const TOOL_ALIASES = { Bash: 'bash', shell: 'bash', local_shell: 'bash' }
/** dsh 插件自己的工具经托管 MCP 暴露，钩子里的名字带这个前缀；去掉后与插件注册的名字一致。 */
const OWN_MCP_PREFIX = 'mcp__codex_plus_dsh__'
/** 常驻进程空闲多久退出。 */
const DAEMON_IDLE_MS = 20 * 60 * 1000
/** 常驻进程定期检查配置文件是否还在（插件移除、测试目录清理后自行退出）。 */
const DAEMON_CHECK_MS = 30 * 1000
const DAEMON_START_TIMEOUT_MS = 5000
/** 探测常驻进程是否在听：单次 ping 的等待上限。 */
const DAEMON_PING_TIMEOUT_MS = 1000
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

/** 提供服务的行（Service 子类插件、ctx.set）：被映射的行可能 inject 它们，常驻进程里要一并加载。 */
function providerRows(plugin) {
  const disabled = new Set(plugin.disabledRows ?? [])
  return (plugin.report?.rows ?? []).filter((row) => !disabled.has(row.id) && (row.provides?.length || row.kind === 'service')).map((row) => row.id)
}

async function loadHost(plugin, rows, { persistent = false } = {}) {
  const dataDir = dataDirOf(plugin)
  // 临时加载（常驻进程起不来）时不落盘镜像，免得和常驻进程写同一个文件。
  const host = new Host({ mirrorDir: persistent && dataDir ? join(dataDir, 'mirror') : undefined })
  await host.loadPackage(plugin.root, plugin.id, {
    disabledRows: plugin.disabledRows,
    configs: plugin.configs,
    onlyRows: Array.isArray(rows) ? [...new Set([...rows, ...providerRows(plugin)])] : rows,
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
    name: TOOL_ALIASES[name] ?? (name.startsWith(OWN_MCP_PREFIX) ? name.slice(OWN_MCP_PREFIX.length) : name),
    arguments: payload?.tool_input ?? {},
    agent,
    signal,
    token: Symbol('codexpp.hook'),
    // 代为执行 Codex 的工具时推迟的上下文：并进这次钩子的 additionalContext。
    deferContext(message) {
      if (message) sinks.getStore()?.inject.push(message)
    },
    concludeTurn() {},
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
    const deny = (reason) => withContext('PreToolUse', {
      hookSpecificOutput: { hookEventName: 'PreToolUse', permissionDecision: 'deny', permissionDecisionReason: reason },
    }, contexts)
    if (decision?.kind === 'deny' || decision?.kind === 'cancel') {
      return deny(decision.kind === 'cancel' ? 'dsh 插件取消了这次工具调用' : String(decision.reason || '被 dsh 插件拒绝'))
    }
    // 计划模式：这个 agent 开着时拒绝 Codex 的写类工具。
    if (host.planMode?.states.get(agent) === true && PLAN_MODE_WRITE_TOOLS.has(String(payload?.tool_name ?? ''))) {
      return deny('计划模式中：先给出计划，不要修改文件')
    }
    // ctx.tools.guard：在 pre-execute 之后、执行之前，任何一个给出理由就拒绝。
    const guarded = host.guardReason(exec)
    if (guarded) return deny(guarded)
    // tools/execute：插件不调 next() 直接给结果就是代为执行——原工具以「拒绝 + 结果」结束，
    // 结果记成改写，经 Codex++ 代理的下一次请求里整条换成纯结果。插件自己的工具在 MCP 进程里跑这条链，这里跳过；
    // 调 next() 的包裹式监听器（超时、重试）包不住 Codex 的工具执行，只能放行原工具。
    if (!String(payload?.tool_name ?? '').startsWith(OWN_MCP_PREFIX) && host.hasListeners('tools/execute')) {
      const passed = Symbol('codexpp.pass')
      const result = await host.waterfall('tools/execute', [exec], async () => passed)
      if (result !== passed && result && typeof result === 'object' && Array.isArray(result.content)) {
        rememberPostDecision(host, exec.callId, { kind: 'accept', content: result.content })
        host.emitEvent('tools/result', [exec, result])
        return deny(`dsh 插件已代为执行这次调用，结果：\n${blocksText(result.content)}`)
      }
    }
    return withContext('PreToolUse', {}, contexts)
  },

  async PostToolUse(host, payload, agent, sink, signal) {
    const exec = makeExec(payload, agent, signal)
    const result = makeResult(payload?.tool_response)
    const decision = await host.waterfall('tools/post-execute', [exec, result], async () => ({ kind: 'accept' }))
    // 改写结果（accept 带 content / value，或 block）钩子表达不了：记下来，下一次模型请求在网关里换进 input。
    rememberPostDecision(host, exec.callId, decision)
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
    const messages = Array.isArray(decision?.messages) ? decision.messages : [original]
    const isOriginal = (message) => message === original || message?.id === original.id
    const added = messages.filter((message) => !isOriginal(message))
    if (!messages.some(isOriginal) && agent.session?.id) {
      // 插件删掉或改写了用户原话：钩子做不到，记下来，经 Codex++ 代理的请求里把这条用户消息换成插件给的消息
      // （官方直连的请求不经代理，原话照旧）。新增的消息随替换一起进 input，不再重复成补充上下文。
      rememberPromptRewrite(host, agent.session.id, String(payload?.prompt ?? ''), added.map(messageText).map((text) => text.trim()).filter(Boolean))
      return withContext('UserPromptSubmit', {}, sinkContexts(sink))
    }
    return withContext('UserPromptSubmit', {}, [messagesText(added), ...sinkContexts(sink)])
  },

  async Stop(host, payload, agent, sink, signal) {
    // 已经因为 Stop 钩子多走过一轮：不再拦，避免无条件 steer 的插件让 Codex 停不下来。
    // 只有 ctx.goals 的目标（有次数上限）可以接着要求继续。
    if (payload?.stop_hook_active === true) {
      const nudge = host.goals?.nudge(agent)
      if (nudge) return { decision: 'block', reason: nudge }
      agent.setStatus('idle')
      return {}
    }
    await host.serial('agent/turn-stopping', [{ agent, turn: agent.session.turn, signal }])
    await host.parallel('workspace/session-stop', [{ sessionId: agent.session.id }])
    // ctx.goals：目标还在推进就要求再走一轮。
    const nudge = host.goals?.nudge(agent)
    const text = [messagesText(sink.steer), nudge].filter(Boolean).join('\n\n')
    if (text) return { decision: 'block', reason: text }
    agent.setStatus('idle')
    return {}
  },

  // Codex 要用户审批一次工具调用：交给插件的 approval/request。allowed-once → 放行，rejected / cancelled → 拒绝，
  // 插件不表态（链尾 unavailable）就交还给 Codex 自己弹审批。
  async PermissionRequest(host, payload, agent, sink, signal) {
    const exec = makeExec(payload, agent, signal)
    const reason = typeof payload?.reason === 'string' ? payload.reason : undefined
    const outcome = await host.waterfall('approval/request', [{ agent, toolName: exec.name, callId: exec.callId, reason, signal }], async () => 'unavailable')
    if (outcome === 'allowed-once') return { hookSpecificOutput: { hookEventName: 'PermissionRequest', decision: { behavior: 'allow' } } }
    if (outcome === 'rejected' || outcome === 'cancelled') {
      return { hookSpecificOutput: { hookEventName: 'PermissionRequest', decision: { behavior: 'deny', message: outcome === 'cancelled' ? 'dsh 插件取消了这次审批' : '被 dsh 插件拒绝' } } }
    }
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
  const listened = (HOOK_LISTENS[hookEvent] && host.hasListeners(HOOK_LISTENS[hookEvent]))
    || (hookEvent === 'PreToolUse' && (host.toolGuards.length > 0 || host.hasListeners('tools/execute') || host.planMode?.states.get(agent) === true))
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

/** 网关请求中间件（提示词）：插件的片段（含 system-prompt/assemble 改写）追加到 instructions。 */
async function instructionsFor(host, body) {
  if (!body || typeof body !== 'object') return null
  const agent = sessionIdOf(body) ? host.agentRegistry.get(sessionIdOf(body)) : undefined
  const current = typeof body.instructions === 'string' ? body.instructions : ''
  const { text, replace } = await host.assembleInstructions({ agent, modelSelection: { model: body.model } }, current)
  if (replace) return text === current ? null : { ...body, instructions: text }
  return appendInstructions(body, text)
}

// ---- tools/post-execute 的改写结果 ----

const MAX_POST_DECISIONS = 2000
const OUTPUT_TYPES = new Set(['function_call_output', 'custom_tool_call_output'])

/** 只记会改结果的决定（accept 带 content / value，或 block）；按调用 id 落盘，常驻进程重启后历史请求里的改写照旧。 */
function rememberPostDecision(host, callId, decision) {
  if (!callId || !decision || typeof decision !== 'object') return
  let replacement
  if (decision.kind === 'block') replacement = blocksText(decision.feedback) || '被 dsh 插件拦下'
  else if (decision.kind === 'accept' && decision.content !== undefined) replacement = blocksText(decision.content)
  else if (decision.kind === 'accept' && decision.value !== undefined) replacement = typeof decision.value === 'string' ? decision.value : JSON.stringify(decision.value)
  if (replacement === undefined) return
  const store = postDecisions(host)
  store.map.delete(callId)
  store.map.set(callId, replacement)
  while (store.map.size > MAX_POST_DECISIONS) store.map.delete(store.map.keys().next().value)
  if (store.path) {
    try {
      mkdirSync(dirname(store.path), { recursive: true })
      writeFileSync(store.path, JSON.stringify(Object.fromEntries(store.map)))
    } catch (error) {
      console.error(`[dsh-bridge] 工具结果改写记录写入失败：${errorText(error)}`)
    }
  }
}

function postDecisions(host) {
  if (host.postDecisions) return host.postDecisions
  const path = host.mirror.dataDir ? join(host.mirror.dataDir, 'post-decisions.json') : undefined
  let map = new Map()
  if (path && existsSync(path)) {
    try {
      map = new Map(Object.entries(JSON.parse(readFileSync(path, 'utf8'))))
    } catch {
      map = new Map()
    }
  }
  host.postDecisions = { path, map, seen: new Set() }
  return host.postDecisions
}

// ---- agent/pre-step 改写的用户原话 ----

const MAX_PROMPT_REWRITES = 200

/** 按会话记下「原话 → 替换的消息」，落盘在镜像目录，常驻进程重启后历史请求里的改写照旧。 */
function promptRewrites(host) {
  if (host.promptRewrites) return host.promptRewrites
  const path = host.mirror.dataDir ? join(host.mirror.dataDir, 'prompt-rewrites.json') : undefined
  let map = new Map()
  if (path && existsSync(path)) {
    try {
      map = new Map(Object.entries(JSON.parse(readFileSync(path, 'utf8'))))
    } catch {
      map = new Map()
    }
  }
  host.promptRewrites = { path, map }
  return host.promptRewrites
}

function rememberPromptRewrite(host, sessionId, original, replacement) {
  if (!original) return
  const store = promptRewrites(host)
  const list = (store.map.get(sessionId) ?? []).filter((item) => item.original !== original)
  list.push({ original, replacement })
  store.map.set(sessionId, list.slice(-MAX_PROMPT_REWRITES))
  if (!store.path) return
  try {
    mkdirSync(dirname(store.path), { recursive: true })
    writeFileSync(store.path, JSON.stringify(Object.fromEntries(store.map)))
  } catch (error) {
    console.error(`[dsh-bridge] 用户原话改写记录写入失败：${errorText(error)}`)
  }
}

/** 一条 Responses 用户消息的文字。 */
function userMessageText(item) {
  if (typeof item?.content === 'string') return item.content
  return (Array.isArray(item?.content) ? item.content : []).map((part) => (typeof part?.text === 'string' ? part.text : '')).join('')
}

/** 本轮（最后一条用户消息之后）的工具调用与结果，给没经过钩子的结果补跑 tools/post-execute。 */
function currentTurnItems(input) {
  let start = 0
  input.forEach((item, index) => {
    if (item?.role === 'user' && (item.type === undefined || item.type === 'message')) start = index + 1
  })
  return input.slice(start)
}

function outputText(output) {
  if (typeof output === 'string') return output
  if (Array.isArray(output)) return output.map((part) => (typeof part?.text === 'string' ? part.text : '')).join('')
  return JSON.stringify(output ?? null)
}

/** 请求 tools 里一项对应的 dsh 工具名：Codex 的 shell 类工具记作 bash，插件自己的 MCP 工具去掉前缀。 */
function requestToolName(tool) {
  const raw = String(tool?.name ?? tool?.type ?? '')
  if (raw === 'exec_command') return 'bash'
  if (TOOL_ALIASES[raw]) return TOOL_ALIASES[raw]
  return raw.startsWith(OWN_MCP_PREFIX) ? raw.slice(OWN_MCP_PREFIX.length) : raw
}

/** 网关请求中间件（每一步）：agent/request 改模型、按步的 agent/pre-step、tools/post-execute 改写结果、排下的上下文。 */
async function stepFor(host, body, context = {}) {
  if (!body || typeof body !== 'object' || !Array.isArray(body.input)) return null
  const step = context?.step ?? {}
  const sessionId = sessionIdOf(body)
  const agent = host.agentRegistry.get(sessionId)
  let input = body.input
  let changed = false
  let next = body
  const controller = new AbortController()
  try {
    // 1. 本轮里还没经过钩子的工具结果（钩子没被信任、或在别的进程里）补跑一次 tools/post-execute。
    const store = postDecisions(host)
    if (host.hasListeners('tools/post-execute')) {
      const calls = new Map()
      for (const item of currentTurnItems(input)) {
        if (item?.type === 'function_call' || item?.type === 'custom_tool_call') calls.set(item.call_id, item)
        if (!OUTPUT_TYPES.has(item?.type) || !item.call_id || store.map.has(item.call_id) || store.seen.has(item.call_id)) continue
        store.seen.add(item.call_id)
        const call = calls.get(item.call_id)
        let args = call?.arguments ?? call?.input ?? {}
        try { args = typeof args === 'string' ? JSON.parse(args) : args } catch {}
        const name = String(call?.name ?? '')
        const exec = { callId: item.call_id, rootCallId: item.call_id, name: name === 'shell' || name === 'exec_command' ? 'bash' : name, arguments: args, agent, signal: controller.signal, token: Symbol('codexpp.gateway') }
        const text = outputText(item.output)
        const result = Object.freeze({ isError: false, value: text, content: [{ type: 'text', text }] })
        const decision = await host.waterfall('tools/post-execute', [exec, result], async () => ({ kind: 'accept' }))
        rememberPostDecision(host, item.call_id, decision)
      }
    }
    // 2. 把记下的改写换进 input（每次请求都带着整段历史，所以每次都换）。
    if (store.map.size) {
      const replaced = input.map((item) => {
        if (!OUTPUT_TYPES.has(item?.type) || !store.map.has(item.call_id)) return item
        const text = store.map.get(item.call_id)
        return outputText(item.output) === text ? item : { ...item, output: text }
      })
      if (replaced.some((item, index) => item !== input[index])) {
        input = replaced
        changed = true
      }
    }
    // 2b. agent/pre-step 删掉或改写的用户原话：每次请求都带着整段历史，所以每次都换。
    const rewrites = promptRewrites(host).map.get(sessionId)
    if (rewrites?.length) {
      let rewritten = false
      const replaced = input.flatMap((item) => {
        if (item?.role !== 'user' || (item.type !== undefined && item.type !== 'message')) return [item]
        const rule = rewrites.find((entry) => entry.original === userMessageText(item))
        if (!rule) return [item]
        rewritten = true
        return rule.replacement.map((text) => ({ type: 'message', role: 'user', content: [{ type: 'input_text', text }] }))
      })
      if (rewritten) {
        input = replaced
        changed = true
      }
    }
    // 3. 排下的上下文与按步的 agent/pre-step：第一步已由 UserPromptSubmit 钩子处理过 pre-step，这里只补排下的上下文。
    const pending = agent ? agent.takePending() : []
    let messages = pending
    if (!step.firstStep && host.hasListeners('agent/pre-step')) {
      const decision = await host.waterfall('agent/pre-step', [{ agent, messages: pending, turn: agent?.session.turn ?? 0, step: Number(step.index) || 1, signal: controller.signal }], async () => ({ kind: 'enter', messages: pending }))
      if (decision?.kind === 'reject') return { reject: String(decision.reason || '这一步被 dsh 插件拒绝') }
      messages = Array.isArray(decision?.messages) ? decision.messages : pending
    }
    const additions = messages.map(messageText).map((text) => text.trim()).filter(Boolean)
    if (additions.length) {
      input = [...input, ...additions.map((text) => ({ type: 'message', role: 'user', content: [{ type: 'input_text', text }] }))]
      changed = true
    }
    if (changed) next = { ...body, input }
    // 4. agent/request：改模型、推理强度。
    if (host.hasListeners('agent/request')) {
      const model = String(next.model ?? '')
      const slash = model.indexOf('/')
      const current = { provider: slash > 0 ? model.slice(0, slash) : 'codex', model, reasoningEffort: next.reasoning?.effort }
      const decided = await host.waterfall('agent/request', [{ agent, turn: agent?.session.turn ?? 0, step: Number(step.index) || 1, signal: controller.signal }], async () => current)
      if (decided?.model && decided.model !== current.model) {
        next = { ...next, model: String(decided.model) }
        changed = true
      }
      if (decided?.reasoningEffort && decided.reasoningEffort !== current.reasoningEffort) {
        next = { ...next, reasoning: { ...(next.reasoning ?? {}), effort: String(decided.reasoningEffort) } }
        changed = true
      }
    }
    // 5. ctx.tools.restrict：从请求的 tools 里去掉被限制的工具，模型就看不到它们。
    if (host.toolRestrictions.length && Array.isArray(next.tools)) {
      const kept = next.tools.filter((tool) => {
        const name = requestToolName(tool)
        return !name || host.toolVisible(name)
      })
      if (kept.length !== next.tools.length) {
        next = { ...next, tools: kept }
        changed = true
      }
    }
    // 6. 插件工具的 registerOptions.when：按这一步的 agent 与模型判断，不满足就不给模型看（同 dsh 每步解析工具集）。
    if (Array.isArray(next.tools) && [...host.tools.values()].some((entry) => entry.when)) {
      const model = String(next.model ?? '')
      const slash = model.indexOf('/')
      const selection = { provider: slash > 0 ? model.slice(0, slash) : 'codex', model, reasoningEffort: next.reasoning?.effort }
      const kept = []
      for (const tool of next.tools) {
        const entry = host.tools.get(requestToolName(tool))
        if (!entry?.when || await host.toolActive(entry, agent, selection, controller.signal)) kept.push(tool)
      }
      if (kept.length !== next.tools.length) {
        next = { ...next, tools: kept }
        changed = true
      }
    }
  } finally {
    controller.abort()
  }
  return changed ? next : null
}

/** 旧版配套拓展包只有一个网关脚本：提示词与每一步的处理一起做。 */
async function gatewayFor(host, body, context = {}) {
  const withPrompt = (await instructionsFor(host, body)) ?? body
  const stepped = await stepFor(host, withPrompt, context)
  if (stepped?.reject) return stepped
  const result = stepped ?? withPrompt
  return result === body ? null : result
}

// ---- llm/stream 与 agent/assistant-stream：网关响应流 ----

/** Responses SSE 事件 → dsh 流式块（只翻文字、推理、工具调用参数、用量与结束）。 */
function chunksOfEvent(event, state) {
  const chunks = []
  const index = Number(event?.output_index ?? 0)
  const start = (blockType) => {
    if (state.started.has(index)) return
    state.started.add(index)
    chunks.push({ type: 'block-start', index, blockType })
  }
  switch (event?.type) {
    case 'response.output_text.delta':
      start('text')
      chunks.push({ type: 'text-delta', index, text: String(event.delta ?? '') })
      break
    case 'response.reasoning_summary_text.delta':
    case 'response.reasoning_text.delta':
      start('reasoning')
      chunks.push({ type: 'reasoning-delta', index, text: String(event.delta ?? '') })
      break
    case 'response.function_call_arguments.delta':
      start('tool-call')
      chunks.push({ type: 'tool-call-delta', index, id: String(event.item_id ?? ''), argumentsDelta: String(event.delta ?? '') })
      break
    case 'response.completed':
    case 'response.incomplete': {
      const usage = event.response?.usage
      if (usage) chunks.push({ type: 'usage', usage: { inputTokens: Number(usage.input_tokens ?? 0), outputTokens: Number(usage.output_tokens ?? 0), totalTokens: Number(usage.total_tokens ?? 0) || undefined } })
      chunks.push({ type: 'finish', reason: { kind: event.type === 'response.completed' ? 'stop' : 'max-tokens' } })
      break
    }
    case 'response.failed':
      chunks.push({ type: 'finish', reason: { kind: 'error', failure: { message: String(event.response?.error?.message ?? 'failed'), code: String(event.response?.error?.code ?? 'ERROR') } } })
      break
    default:
      break
  }
  return chunks
}

/** 一条流在常驻进程里的状态：上游块排进队列，插件的 llm/stream 链从队列里读、产出的块攒进 outbox。 */
function openStreamState(host, { sessionId, model }) {
  const queue = []
  let closed = false
  let wake = null
  let idleWaiter = null
  const outbox = []
  const agent = host.agentRegistry.get(sessionId)
  const attemptId = `codexpp-${Date.now().toString(36)}`
  let revision = 0
  const frame = (value) => host.emitEvent('agent/assistant-stream', [{ agent, frame: { attemptId, revision: revision++, ...value } }])
  const upstream = (async function* () {
    for (;;) {
      if (queue.length) {
        yield queue.shift()
        continue
      }
      if (closed) return
      const resume = idleWaiter
      idleWaiter = null
      resume?.()
      await new Promise((resolve) => {
        wake = resolve
      })
    }
  })()
  const state = { started: new Set(), outbox, done: false, error: null }
  frame({ type: 'start', turn: agent?.session.turn ?? 0, step: 1 })
  const options = Object.freeze({ provider: 'codex', model: String(model ?? ''), messages: [], sessionId })
  const output = host.hasListeners('llm/stream')
    ? host.waterfallStream('llm/stream', options, () => upstream)
    : upstream
  state.pump = (async () => {
    try {
      let index = 0
      for await (const chunk of output) {
        outbox.push(chunk)
        frame({ type: 'chunk', index: index++, time: Date.now(), chunk })
      }
    } catch (error) {
      state.error = errorText(error)
    } finally {
      state.done = true
      frame({ type: 'end', index: 0, outcome: { kind: 'abandoned' } })
      const resume = idleWaiter
      idleWaiter = null
      resume?.()
    }
  })()
  /** 喂一批上游块，等插件把它们消化完（又在等下一块）或整条链结束，交回这期间产出的块。 */
  state.feed = async (chunks, end) => {
    queue.push(...chunks)
    if (end) closed = true
    const idle = new Promise((resolve) => {
      idleWaiter = resolve
    })
    const resume = wake
    wake = null
    resume?.()
    if (end) await state.pump
    else await Promise.race([idle, state.pump])
    return outbox.splice(0)
  }
  return state
}

async function streamFor(host, message) {
  host.streams ??= new Map()
  if (message.phase === 'start') {
    host.streams.set(message.rid, openStreamState(host, message))
    return { rewrite: host.hasListeners('llm/stream') }
  }
  const state = host.streams.get(message.rid)
  if (!state) return { error: '流已结束' }
  const chunks = await state.feed(message.chunks ?? [], message.phase === 'end')
  if (message.phase === 'end') host.streams.delete(message.rid)
  return { chunks, error: state.error }
}

/** 网关失败重试：agent/request-error 返回 { kind: 'retry' } 就重发。 */
async function errorFor(host, error) {
  // 网关带上了出错请求所属的会话：认出 agent，先发 agent/error（只通知），再问 agent/request-error 要不要重试。
  const agent = error?.session ? host.agentRegistry.get(error.session) : undefined
  const failure = { message: String(error?.json?.error?.message ?? error?.body ?? '').slice(0, 2000), code: String(error?.json?.error?.code ?? error?.status ?? 'ERROR'), status: error?.status }
  if (agent) host.emitEvent('agent/error', [{ agent, turn: agent.session.turn, step: 1, error: failure }])
  if (!host.hasListeners('agent/request-error')) return null
  const controller = new AbortController()
  const action = await host.waterfall('agent/request-error', [{ agent, turn: agent?.session.turn ?? 0, step: 1, provider: 'codex', failure, retryPolicy: undefined, signal: controller.signal }], async () => undefined)
  controller.abort()
  return action?.kind === 'retry' ? { retry: true, delayMs: Number(action.delayMs) || 1000 } : null
}

/** /webhook/<kind> 的请求 → dsh 的 delivery，交给插件的 webhook 规则。 */
/**
 * ctx.webServer 注册的网页路由：后台交来的 { path, method, headers, body } 包成 Node 风格的 req / res 交给处理器，
 * 收集 res 写出的状态、头与内容。
 */
async function webRouteFor(host, request = {}) {
  const path = String(request.path ?? '/').replace(/^\/ext\/[^/]+/, '') || '/'
  const route = host.webRoutes.get(path.split('?')[0]) ?? (host.webFallback ? { handler: host.webFallback } : undefined)
  if (!route) return { status: 404, body: `没有插件处理 ${path}` }
  const body = request.body === undefined || request.body === null ? '' : String(request.body)
  const req = {
    method: String(request.method ?? 'GET'),
    url: path,
    headers: Object.fromEntries(Object.entries(request.headers ?? {}).map(([key, value]) => [key.toLowerCase(), value])),
    async *[Symbol.asyncIterator]() {
      if (body) yield Buffer.from(body)
    },
  }
  const reply = { status: 200, headers: {}, chunks: [] }
  await new Promise((resolve, reject) => {
    const res = {
      statusCode: 200,
      setHeader: (name, value) => { reply.headers[String(name).toLowerCase()] = String(value) },
      getHeader: (name) => reply.headers[String(name).toLowerCase()],
      writeHead(status, headers = {}) {
        res.statusCode = status
        for (const [name, value] of Object.entries(headers)) res.setHeader(name, value)
        return res
      },
      write(chunk) {
        reply.chunks.push(Buffer.from(chunk))
        return true
      },
      end(chunk) {
        if (chunk !== undefined) res.write(chunk)
        reply.status = res.statusCode
        resolve()
      },
    }
    Promise.resolve(route.handler(req, res)).catch(reject)
  })
  return { status: reply.status, headers: reply.headers, body: Buffer.concat(reply.chunks).toString('utf8') }
}

/** 常驻进程里接协议升级的本机 socket（与控制 socket 区分开）。 */
function upgradeSocketPath(configPath) {
  const control = socketPath(configPath)
  return process.platform === 'win32' ? `${control}-upgrade` : control.replace(/\.sock$/, '-upgrade.sock')
}

/**
 * webServer.registerUpgrade 的落点：常驻进程里起一个只听本机 socket 的 Node http 服务，Codex++ 本地代理把
 * /ext/<配套拓展>/<路径> 的升级请求原样写进来，'upgrade' 事件按路径（去掉 /ext/<配套拓展> 前缀）交给插件的处理器。
 * 返回给 Rust 的是 socket 路径；路径没有处理器时返回 error。
 */
async function upgradeFor(host, configPath, request = {}) {
  const path = String(request.path ?? '/').split('?')[0] || '/'
  if (!host.webUpgrades.has(path)) return { error: `没有插件处理 ${path} 的协议升级` }
  if (!host.upgradeServer) {
    const socket = upgradeSocketPath(configPath)
    const sockets = new Set()
    const server = createHttpServer((_req, res) => {
      res.writeHead(426, { connection: 'close' })
      res.end()
    })
    server.on('upgrade', (req, socket, head) => {
      const url = String(req.url ?? '/').replace(/^\/ext\/[^/?]+/, '') || '/'
      const route = host.webUpgrades.get(new URL(url, 'http://codexpp.local').pathname)
      if (!route) {
        socket.end('HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n')
        return
      }
      req.url = url
      sockets.add(socket)
      socket.on('close', () => sockets.delete(socket))
      // http 服务的连接默认半开：客户端断开后这边也收尾，否则代理那头的对拷一直等着。
      socket.on('end', () => socket.end())
      Promise.resolve(route.handler(req, socket, head)).catch((error) => {
        console.error(`[dsh-bridge] 协议升级处理器失败：${errorText(error)}`)
        socket.destroy()
      })
    })
    if (process.platform !== 'win32') {
      try {
        unlinkSync(socket)
      } catch {}
    }
    await new Promise((resolve, reject) => {
      server.once('error', reject)
      server.listen(socket, () => {
        server.off('error', reject)
        resolve()
      })
    })
    host.upgradeServer = { server, socket, sockets }
  }
  return { socket: host.upgradeServer.socket }
}

async function webhookFor(host, request = {}) {
  const match = /\/webhook\/(.+)$/.exec(String(request.path ?? ''))
  if (!match) return webRouteFor(host, request)
  let event = request.json
  if (event === undefined) {
    try {
      event = JSON.parse(String(request.body ?? 'null'))
    } catch {
      event = String(request.body ?? '')
    }
  }
  const headers = request.headers ?? {}
  const delivery = {
    kind: decodeURIComponent(match[1]),
    source: 'codexpp-http',
    deliveryId: String(headers['x-github-delivery'] ?? headers['x-request-id'] ?? `codexpp-${Date.now()}`),
    event,
    receivedAt: Date.now(),
  }
  const started = await host.webhooks.dispatch(delivery)
  return { status: started.length ? 202 : 404, json: { kind: delivery.kind, started } }
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
    // Windows 的普通 realpath 不展开 8.3 短名；各入口必须按系统解析出的同一路径连接。
    key = process.platform === 'win32' ? realpathSync.native(configPath) : realpathSync(configPath)
  } catch {
    // 配置不在了也照样算出路径，连接会失败再退回。
  }
  const hash = createHash('sha256').update(`${process.getuid?.() ?? ''}:${key}`).digest('hex').slice(0, 16)
  return process.platform === 'win32' ? `\\\\.\\pipe\\codexpp-dsh-${hash}` : join(tmpdir(), `codexpp-dsh-${hash}.sock`)
}

// ---------------------------------------------------------------------------
// 插件间事件跨包转发
// ---------------------------------------------------------------------------

/** 跨包事件的单次往返上限（waterfall 会嵌套：别的包的监听器调 next 再回到本包）。 */
const BUS_TIMEOUT_MS = 30 * 1000
/** 注册表里「谁听哪些事件」的缓存时间。 */
const BUS_CACHE_MS = 2000

/** 配套拓展包目录名（与 Rust 的 dsh_plugins::companion_id 一致）。 */
const companionDir = (id) => `dsh-${id}`.slice(0, 64)

/** 跨进程只能传 JSON：函数、循环引用丢掉（与 dsh 里跨 Peer 的事件一样只认可序列化的数据）。 */
function toWire(value) {
  if (value === undefined) return undefined
  try {
    return JSON.parse(JSON.stringify(value, (_key, item) => (typeof item === 'bigint' ? String(item) : item)))
  } catch {
    return null
  }
}

/**
 * 常驻进程之间的事件总线：插件 ctx.emit / parallel / serial / bail / waterfall 一个自定义事件时，本包的监听器照常派发，
 * 再经本地 socket 交给别的包（各自的常驻进程，没起就按需起）里听这个事件的插件，语义与包内一致：
 * - emit：发出去不等；parallel：并发等全部；serial / bail：本包没人给出结果时按注册表顺序问别的包，取第一个非 undefined；
 * - waterfall：本包的监听器在前，链尾依次交给别的包的监听器；它们调 next() 时经 socket 回到发起方继续，最后才是调用方的 next。
 * 参数与结果按 JSON 传；每次往返有超时，别的包出错或超时按「无意见」继续。
 * MCP 进程不挂总线：所有包的工具在同一个进程里，事件本来就直接派发。
 */
function createEventBus(configPath, self) {
  const extensionsDir = dirname(dirname(configPath))
  let cache = { at: 0, peers: [] }
  const peers = () => {
    if (Date.now() - cache.at < BUS_CACHE_MS) return cache.peers
    let registry = { plugins: [] }
    try {
      registry = JSON.parse(readFileSync(self.registryPath, 'utf8'))
    } catch {}
    const found = []
    for (const plugin of registry.plugins ?? []) {
      if (!plugin || plugin.id === self.id || plugin.enabled === false) continue
      const disabled = new Set(plugin.disabledRows ?? [])
      const listens = new Set((plugin.report?.rows ?? []).filter((row) => !disabled.has(row.id) && row.enabled !== false).flatMap((row) => row.listens ?? []))
      if (!listens.size) continue
      const config = join(extensionsDir, companionDir(plugin.id), 'dsh-bridge.json')
      if (existsSync(config)) found.push({ id: plugin.id, config, listens })
    }
    cache = { at: Date.now(), peers: found }
    return found
  }
  const peersOf = (event) => peers().filter((peer) => peer.listens.has(event))
  const send = async (peer, message) => {
    const plugin = readPlugin(peer.config)
    if (!plugin) return {}
    const reply = await viaDaemon(peer.config, plugin, { kind: 'bus', origin: configPath, ...message }, BUS_TIMEOUT_MS)
    return reply.output ?? {}
  }
  const warn = (peer, event, error) => console.error(`[dsh-bridge] 事件 ${event} 交给 ${peer.id} 失败：${errorText(error)}`)
  /** waterfall 的续接：别的包的监听器调 next() 时按 token 回到这里接着跑链。 */
  const continuations = new Map()
  return {
    listened: (event) => peersOf(event).length > 0,
    emit(event, args) {
      const wire = toWire(args)
      for (const peer of peersOf(event)) void send(peer, { mode: 'emit', event, args: wire }).catch((error) => warn(peer, event, error))
    },
    async parallel(event, args) {
      const wire = toWire(args)
      await Promise.all(peersOf(event).map((peer) => send(peer, { mode: 'parallel', event, args: wire }).catch((error) => warn(peer, event, error))))
    },
    async serial(event, args, mode = 'serial') {
      const wire = toWire(args)
      for (const peer of peersOf(event)) {
        try {
          const reply = await send(peer, { mode, event, args: wire })
          if (reply.value !== undefined) return reply.value
        } catch (error) {
          warn(peer, event, error)
        }
      }
      return undefined
    },
    async waterfall(event, args, fallback) {
      const list = peersOf(event)
      const wire = toWire(args)
      const run = async (index) => {
        if (index >= list.length) return fallback()
        let continued
        const proceed = () => (continued ??= run(index + 1))
        const token = `${process.pid}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`
        continuations.set(token, proceed)
        try {
          return (await send(list[index], { mode: 'waterfall', event, args: wire, token })).value
        } catch (error) {
          warn(list[index], event, error)
          return proceed()
        } finally {
          continuations.delete(token)
        }
      }
      return run(0)
    },
    /** 别的包的监听器调了 next()：接着跑发起方这边的链。 */
    async next(token) {
      const proceed = continuations.get(String(token))
      if (!proceed) throw new Error('事件链已经结束')
      return { value: toWire(await proceed()) }
    },
  }
}

/** 收到别的包转来的事件：只派发给本包的监听器（不再往外转），结果按 JSON 交回。 */
async function busDispatch(host, message) {
  const event = String(message.event ?? '')
  const args = Array.isArray(message.args) ? message.args : []
  const next = message.mode === 'waterfall' && message.origin && message.token
    ? async () => (await exchange(socketPath(message.origin), { kind: 'bus-next', token: message.token }, BUS_TIMEOUT_MS)).output?.value
    : () => undefined
  let value
  switch (message.mode) {
    case 'emit':
      host.emitEvent(event, args)
      break
    case 'parallel':
      await host.parallel(event, args)
      break
    case 'serial':
      value = await host.serial(event, args)
      break
    case 'bail':
      value = await host.bail(event, args)
      break
    case 'waterfall':
      value = await host.waterfallSync(event, args, next)
      break
    default:
      throw new Error(`不认识的事件派发方式 ${message.mode}`)
  }
  return { value: toWire(value) }
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

const ping = (path) => exchange(path, { kind: 'ping', version: '' }, DAEMON_PING_TIMEOUT_MS)

/**
 * 本进程正在拉起的常驻进程：socket 路径 → 等它应答的 Promise。
 * 同一进程里同时发往一个还没起来的常驻进程的请求共用这一次拉起、排队等它，就绪后按排队顺序依次连上。
 * 各自轮询的话，先发的请求（比如不等结果的 emit）可能比后发的晚到，还会一次拉起好几个常驻进程互相抢 socket。
 */
const pendingStarts = new Map()

/** 拉起常驻进程（本进程里同一路径同一时刻只拉一次），等到它应答 ping 或过了 deadline。 */
function daemonStarted(configPath, path, deadline) {
  let started = pendingStarts.get(path)
  if (!started) {
    started = (async () => {
      startDaemon(configPath)
      for (;;) {
        await sleep(50)
        try {
          await ping(path)
          return
        } catch (error) {
          // 连上了但回得慢、或者到点了：交给调用方自己的请求去重试或报错。
          if (!unreachable(error) || Date.now() > deadline) return
        }
      }
    })().finally(() => pendingStarts.delete(path))
    pendingStarts.set(path, started)
  }
  return started
}

/** 经常驻进程处理一条请求；返回 { output }。常驻进程起不来时抛出 { unreachable: true } 的错误。 */
async function viaDaemon(configPath, plugin, request, timeoutMs = DAEMON_REPLY_TIMEOUT_MS) {
  const path = socketPath(configPath)
  const message = { ...request, version: versionOf(plugin) }
  const deadline = Date.now() + DAEMON_START_TIMEOUT_MS
  for (;;) {
    // 本进程正在拉起它：排在先来的请求后面，不抢先连。
    if (pendingStarts.has(path)) await pendingStarts.get(path)
    let reply
    try {
      reply = await exchange(path, message, timeoutMs)
    } catch (error) {
      if (!unreachable(error)) throw error
      if (Date.now() > deadline) throw Object.assign(new Error('dsh 插件常驻进程没有启动'), { unreachable: true })
      await daemonStarted(configPath, path, deadline)
      continue
    }
    if (reply?.restart) {
      // 旧进程正在退出；等它让出 socket 再起新的。
      if (Date.now() > deadline) throw Object.assign(new Error('dsh 插件常驻进程没有重启'), { unreachable: true })
      await sleep(50)
      continue
    }
    if (reply?.error) throw new Error(reply.error)
    return reply ?? { output: null }
  }
}

/**
 * socket 那头有没有常驻进程在听。一次连不上不算数：另一个常驻进程可能刚绑定了路径、还没开始监听
 * （机器忙时两步之间会被调度出去），这时把文件当残留清掉，它就成了谁也连不上的孤儿，插件状态分成两份。
 */
async function daemonListening(path) {
  for (let attempt = 0; attempt < 5; attempt += 1) {
    if (attempt > 0) await sleep(100)
    try {
      await ping(path)
      return true
    } catch {
      // 再问一次。
    }
  }
  return false
}

async function runDaemon(configPath) {
  const plugin = readPlugin(configPath)
  if (!plugin) return
  const version = versionOf(plugin)
  const rows = mappedRows(plugin)
  if (rows.length === 0) return
  const host = await loadHost(plugin, rows, { persistent: true })
  // 插件间自定义事件转给别的包的常驻进程。
  host.bus = createEventBus(configPath, plugin)
  const path = socketPath(configPath)
  let idle
  let closing = false
  const server = createServer({ allowHalfOpen: true })
  const shutdown = async () => {
    if (closing) return
    closing = true
    clearTimeout(idle)
    server.close()
    if (host.upgradeServer) {
      for (const socket of host.upgradeServer.sockets) socket.destroy()
      host.upgradeServer.server.close()
      if (process.platform !== 'win32') {
        try {
          unlinkSync(host.upgradeServer.socket)
        } catch {}
      }
    }
    if (process.platform !== 'win32') {
      try {
        unlinkSync(path)
      } catch {
        // 已被新进程接管或清理。
      }
    }
    host.mirror.flushAll()
    await host.dispose().catch(() => {})
    hostExit(0)
  }
  const touch = () => {
    clearTimeout(idle)
    // 还有升级后的长连接（WebSocket）开着就不算空闲。
    idle = setTimeout(() => (host.upgradeServer?.sockets.size ? touch() : shutdown()), DAEMON_IDLE_MS)
  }
  const handle = async (message) => {
    if (message?.kind === 'ping') return { ok: true }
    // 别的包的监听器调 next() 回到本进程续接事件链：发起方不知道本进程的版本号，放在版本检查前。
    if (message?.kind === 'bus-next') return { output: await host.bus.next(message.token) }
    if (message?.version !== version) {
      setImmediate(shutdown)
      return { restart: true }
    }
    if (message.kind === 'hook') return { output: await dispatchHook(host, message.event, message.payload) }
    if (message.kind === 'instructions') return { output: await instructionsFor(host, message.body) }
    if (message.kind === 'step') return { output: await stepFor(host, message.body, message.context) }
    if (message.kind === 'gateway') return { output: await gatewayFor(host, message.body, message.context) }
    if (message.kind === 'stream') return { output: await streamFor(host, message) }
    if (message.kind === 'error') return { output: await errorFor(host, message.error) }
    if (message.kind === 'command') return { output: await commandFor(host, message) }
    if (message.kind === 'fetch') return { output: await host.fetchRoute(message.request) }
    if (message.kind === 'rpc') return { output: await host.runRpc(message.request) }
    if (message.kind === 'rpc-open') return { output: await host.openRpcStream(message.request) }
    if (message.kind === 'rpc-pull') return { output: await host.pullRpcStream(message.request) }
    if (message.kind === 'rpc-push') return { output: host.pushRpcStream(message.request) }
    if (message.kind === 'rpc-close') return { output: host.closeRpcStream(message.request) }
    if (message.kind === 'webhook') return { output: await webhookFor(host, message.request) }
    if (message.kind === 'bus') return { output: await busDispatch(host, message) }
    if (message.kind === 'upgrade') return { output: await upgradeFor(host, configPath, message.request) }
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
    // 两个进程同时绑定同一路径时，macOS 上后到的可能报 EEXIST 而不是 EADDRINUSE。
    if (!['EADDRINUSE', 'EEXIST'].includes(error?.code) || process.platform === 'win32') throw error
    // 有人在听就让给它；是崩溃留下的 socket 文件就清掉重来。
    if (await daemonListening(path)) {
      await host.dispose()
      return
    }
    try {
      unlinkSync(path)
    } catch {
      // 已被别的进程清理。
    }
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
  const rows = [...new Set([INSTRUCTIONS_TARGET, 'gateway:instructions', 'gateway:request', 'gateway:step', MIRROR_TARGET].flatMap((target) => mappedRows(plugin, target)))]
  if (!rows.length) return null
  const { step } = context ?? {}
  return handle(config, plugin, mappedRows(plugin), { kind: 'gateway', body, context: { step } }, (host) => gatewayFor(host, body, { step }))
}

/** 网关请求中间件入口（提示词）：插件的片段追加到 instructions。 */
export async function requestInstructions(config, body) {
  const plugin = readPlugin(config)
  if (!plugin) return null
  const rows = [...new Set([...mappedRows(plugin, INSTRUCTIONS_TARGET), ...mappedRows(plugin, 'gateway:instructions')])]
  if (!rows.length) return null
  return handle(config, plugin, mappedRows(plugin), { kind: 'instructions', body }, (host) => instructionsFor(host, body))
}

/** 网关请求中间件入口（每一步）：agent/request、按步的 agent/pre-step、改写工具结果、排下的上下文。 */
export async function requestStep(config, body, context = {}) {
  const plugin = readPlugin(config)
  if (!plugin) return null
  const rows = [...new Set(['gateway:step', 'gateway:request', MIRROR_TARGET].flatMap((target) => mappedRows(plugin, target)))]
  if (!rows.length) return null
  const { step } = context ?? {}
  return handle(config, plugin, mappedRows(plugin), { kind: 'step', body, context: { step } }, (host) => stepFor(host, body, { step }))
}

/**
 * 网关响应流中间件入口：上游 Responses 事件翻成 dsh 流式块交给插件（llm/stream 改写、agent/assistant-stream 观察）。
 * 插件改写了文字时，文字增量换成插件产出的，收尾事件里的完整文本一并改；只观察时原样放行。
 * 只经常驻进程（流要跨多次往返保持状态）；常驻进程起不来时原样放行。
 */
export async function* responseStream(config, context, upstream) {
  const plugin = readPlugin(config)
  if (typeof config !== 'string' || !plugin || !mappedRows(plugin, 'gateway:response').length) {
    yield* upstream
    return
  }
  const rid = `${process.pid}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`
  const call = (message) => viaDaemon(config, plugin, { kind: 'stream', rid, ...message }).then((reply) => reply.output ?? {})
  let opened
  try {
    opened = await call({ phase: 'start', sessionId: context?.request?.sessionId ?? '', model: context?.request?.model ?? '' })
  } catch (error) {
    console.error(`[dsh-bridge] 响应流交给插件失败：${errorText(error)}`)
    yield* upstream
    return
  }
  const rewrite = Boolean(opened?.rewrite)
  const state = { started: new Set() }
  const meta = new Map()
  const texts = new Map()
  const held = []
  // 插件产出的块 → Responses 事件（只翻文字增量，其余块只用于观察）。
  const emit = (chunks) => {
    const events = []
    for (const chunk of chunks ?? []) {
      if (chunk?.type !== 'text-delta') continue
      const info = meta.get(Number(chunk.index)) ?? { item_id: '', content_index: 0 }
      texts.set(Number(chunk.index), `${texts.get(Number(chunk.index)) ?? ''}${chunk.text ?? ''}`)
      events.push({ type: 'response.output_text.delta', item_id: info.item_id, output_index: Number(chunk.index), content_index: info.content_index, delta: String(chunk.text ?? '') })
    }
    return events
  }
  // 收尾事件里的完整文本换成插件产出的。
  const fixText = (event) => {
    const index = Number(event?.output_index)
    const patchItem = (item, at) => {
      if (!item || item.type !== 'message' || !texts.has(at)) return item
      return { ...item, content: (item.content ?? []).map((part) => (part?.type === 'output_text' ? { ...part, text: texts.get(at) } : part)) }
    }
    switch (event?.type) {
      case 'response.output_text.done':
        return texts.has(index) ? { ...event, text: texts.get(index) } : event
      case 'response.content_part.done':
        return texts.has(index) && event.part?.type === 'output_text' ? { ...event, part: { ...event.part, text: texts.get(index) } } : event
      case 'response.output_item.done':
        return { ...event, item: patchItem(event.item, index) }
      case 'response.completed':
      case 'response.incomplete':
        return event.response ? { ...event, response: { ...event.response, output: (event.response.output ?? []).map((item, at) => patchItem(item, at)) } } : event
      default:
        return event
    }
  }
  const isFinal = (event) => /\.done$/.test(String(event?.type)) || ['response.completed', 'response.incomplete', 'response.failed'].includes(event?.type)
  let failed = false
  for await (const event of upstream) {
    if (event?.type === 'response.output_text.delta' || event?.type === 'response.content_part.added') {
      meta.set(Number(event.output_index ?? 0), { item_id: event.item_id ?? '', content_index: Number(event.content_index ?? 0) })
    }
    let chunks = []
    if (!failed) {
      try {
        const reply = await call({ phase: 'event', chunks: chunksOfEvent(event, state) })
        chunks = reply.chunks ?? []
      } catch (error) {
        console.error(`[dsh-bridge] 响应流中途交给插件失败，剩下的原样放行：${errorText(error)}`)
        failed = true
      }
    }
    if (!rewrite || failed) {
      yield* held.splice(0)
      yield event
      continue
    }
    yield* emit(chunks)
    // 文字增量由插件给；收尾事件等插件产出完再发（里面的完整文本要换成插件的）。
    if (event?.type === 'response.output_text.delta') continue
    if (isFinal(event)) held.push(event)
    else yield event
  }
  let tail = []
  try {
    tail = (await call({ phase: 'end', chunks: [] })).chunks ?? []
  } catch (error) {
    console.error(`[dsh-bridge] 响应流收尾失败：${errorText(error)}`)
  }
  if (rewrite && !failed) yield* emit(tail)
  for (const event of held) yield rewrite && !failed ? fixText(event) : event
}

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

/** 前端半边的 ctx.connection.rpc.call 入口：交给插件宿主半边经 ctx.connection.rpc 注册的处理器。 */
export async function runRpc(config, request) {
  const plugin = readPlugin(config)
  if (!plugin || !mappedRows(plugin, 'connection').length) return { ok: false, error: { code: 'UNAVAILABLE', message: '插件已停用', details: {} } }
  return handle(config, plugin, mappedRows(plugin), { kind: 'rpc', request }, (host) => host.runRpc(request))
}

/**
 * 前端半边的 ctx.connection.rpc.open（流式调用，如 typert 的 stream 方法）：流要跨多次往返保持状态，只经常驻进程。
 * action：open（{ channel, endpoint, payload } → { ok, id }）、pull（{ id, waitMs } → { items, done, error? }）、
 * push（{ id, items, end } 上行）、close（{ id }）。
 */
export async function runRpcStream(config, action, request) {
  const plugin = readPlugin(config)
  if (!plugin || !mappedRows(plugin, 'connection').length) return { ok: false, done: true, items: [], error: { code: 'UNAVAILABLE', message: '插件已停用', details: {} } }
  if (!['open', 'pull', 'push', 'close'].includes(action)) return { ok: false, error: { code: 'INVALID', message: `不认识的流操作 ${action}`, details: {} } }
  try {
    return (await viaDaemon(config, plugin, { kind: `rpc-${action}`, request: request ?? {} }, DAEMON_REPLY_TIMEOUT_MS)).output ?? null
  } catch (error) {
    return { ok: false, done: true, items: [], error: { code: error?.unreachable ? 'UNAVAILABLE' : 'TRANSPORT', message: errorText(error), details: {} } }
  }
}

/** 定时任务入口（配套拓展包的后台每分钟调一次）：到点的往会话里发起一轮。不需要加载插件。 */
export async function runTick(config) {
  const plugin = readPlugin(config)
  if (!plugin || !mappedRows(plugin, 'schedule').length) return []
  return createSchedule(dataDirOf(plugin)).tick()
}

/** webhook 入口（配套拓展包后台的 HTTP 路由）：返回 { status, json | body }。 */
export async function runWebhook(config, request) {
  const plugin = readPlugin(config)
  if (!plugin || !mappedRows(plugin, 'webhook').length) return { status: 503, body: '插件已停用' }
  return handle(config, plugin, mappedRows(plugin), { kind: 'webhook', request }, (host) => webhookFor(host, request))
}

/** 协议升级入口（配套拓展包的后台调用，Rust 拿到 socket 后把升级请求原样转过去）：返回 { socket } 或 { error }。只经常驻进程。 */
export async function runUpgrade(config, request) {
  const plugin = readPlugin(config)
  if (!plugin || !mappedRows(plugin, 'webhook').length) return { error: '插件已停用' }
  try {
    return (await viaDaemon(config, plugin, { kind: 'upgrade', request: request ?? {} })).output ?? { error: '常驻进程没有回复' }
  } catch (error) {
    return { error: errorText(error) }
  }
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
  guardProcessExit()
  runDaemon(process.argv[3]).catch((error) => {
    console.error(`[dsh-bridge] 常驻进程退出：${errorText(error)}`)
    hostExit(1)
  })
}
