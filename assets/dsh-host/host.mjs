// Codex++ 的 dsh 插件兼容宿主。
//
//   node host.mjs inspect <包目录> [选项.json]  分析一个 dsh 插件包，把兼容性报告（JSON）写到 stdout
//   node host.mjs serve <registry.json>   以 stdio MCP 服务运行注册表里启用的插件，把它们的工具暴露给 Codex
//   bridge.mjs 引入本模块，在 Codex 钩子 / 网关中间件进程里加载插件、派发映射过去的事件
//
// dsh 插件是 Cordis 插件：`inject` 声明依赖的服务，`apply(ctx, config)` 里注册工具等副作用。
// Codex App 没有 dsh 的 agent 循环与会话服务，这里提供插件能用上的那部分 ctx：
// tools / systemPrompt（片段经网关追加到 instructions，也汇总进 MCP instructions）/ logger / timer / effect / on 等，
// 以及 services.mjs 里的替身服务（会话镜像、agent 替身、llm、commands、fs、shell……）；
// 能对上 Codex 钩子 / 网关的事件（见 EVENT_HOOKS、GATEWAY_EVENTS）由 Codex++ 生成的配套拓展包转发；
// 必需依赖里有别的服务时不加载，并在报告里写明缺了什么。

import { AsyncLocalStorage } from 'node:async_hooks'
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, realpathSync, statSync, writeFileSync } from 'node:fs'
import { register } from 'node:module'
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path'
import { createInterface } from 'node:readline'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { runInNewContext } from 'node:vm'
import { hostCall } from './host-call.mjs'
import { parseYaml } from './yaml.mjs'
import { createLoader, createSessionPersistence, createSettings } from './settings-services.mjs'
import {
  AgentRegistry, MIRROR_SERVICES, messageText, SUPPORTED_SERVICES, SessionMirror, createCredentials, createFs, createJobs, createLlm, createSchedule,
  createApproval, createAttachments, createFileReferences, createGoals, createPlanMode, createSubagents, PLAN_MODE_WRITE_TOOLS, createSessionTelemetry, createSessionTitle, createShellEnv, createSpillStore, createStorage, createTokenMeter, createSessionQuery, createShell, createStorageDomain, createSubprocess, createUserQuestions, createWeb, createWebhookRuntime,
  createWorkspaceRegistry,
} from './services.mjs'

const HOST_VERSION = '6'
const HOST_DIR = dirname(fileURLToPath(import.meta.url))
/** 替身 Service 构造时经这个符号把自己登记到 ctx（与 shims/cordis.mjs 一致）。 */
const PROVIDE = Symbol.for('codexpp.dsh.provide')
/** Cordis Service 的生命周期符号（与 shims/cordis.mjs 的 Service.init / Service.dispose 一致）。 */
const SERVICE_INIT = Symbol.for('cordis.service.init')
const SERVICE_DISPOSE = Symbol.for('cordis.service.dispose')
/** ctx 上的这些名字是宿主自己的能力（工具、提示词……），插件提供的同名服务不能顶替它们。 */
const CORE_CONTEXT_KEYS = new Set(['tools', 'systemPrompt', 'skills', 'commands', 'connection', 'logger', 'timer'])
const SHIMS = {
  '@deepseek-ai/dsh-tools': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-tools.mjs')).href,
  '@deepseek-ai/schemastery': pathToFileURL(join(HOST_DIR, 'shims', 'schemastery.mjs')).href,
  '@deepseek-ai/cordis': pathToFileURL(join(HOST_DIR, 'shims', 'cordis.mjs')).href,
  '@deepseek-ai/dsh-skill': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-skill.mjs')).href,
  '@deepseek-ai/dsh-system-prompt': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-system-prompt.mjs')).href,
  '@deepseek-ai/dsh-llm': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-llm.mjs')).href,
  // 子路径导出：dsh-llm/message 是根导出里消息构造函数的子集，同一个替身即可。
  '@deepseek-ai/dsh-llm/message': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-llm.mjs')).href,
  '@deepseek-ai/dsh-session': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-session.mjs')).href,
  '@deepseek-ai/dsh-fs': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-fs.mjs')).href,
  '@deepseek-ai/dsh-credentials': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-credentials.mjs')).href,
  '@deepseek-ai/dsh-storage-domain': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-storage-domain.mjs')).href,
  '@deepseek-ai/dsh-web': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-errors.mjs')).href,
  '@deepseek-ai/dsh-user-questions': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-errors.mjs')).href,
  '@deepseek-ai/dsh-session-query': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-errors.mjs')).href,
  '@deepseek-ai/dsh-subprocess': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-errors.mjs')).href,
  '@deepseek-ai/dsh-shell': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-errors.mjs')).href,
  '@deepseek-ai/dsh-commands': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-errors.mjs')).href,
  '@deepseek-ai/dsh-spill': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-services.mjs')).href,
  '@deepseek-ai/dsh-user-approval': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-services.mjs')).href,
  '@deepseek-ai/dsh-session-title': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-services.mjs')).href,
  '@deepseek-ai/dsh-token-meter': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-services.mjs')).href,
  '@deepseek-ai/dsh-token-meter/estimate': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-token-meter-estimate.mjs')).href,
  '@deepseek-ai/dsh-shell-env': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-services.mjs')).href,
  '@deepseek-ai/dsh-storage': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-services.mjs')).href,
  '@deepseek-ai/dsh-session-telemetry': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-services.mjs')).href,
  '@deepseek-ai/dsh-plan-mode': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-services.mjs')).href,
  '@deepseek-ai/dsh-goal': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-services.mjs')).href,
  '@deepseek-ai/dsh-subagent': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-subagent.mjs')).href,
  '@deepseek-ai/dsh-attachment': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-services.mjs')).href,
  '@deepseek-ai/dsh-file-reference': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-services.mjs')).href,
  '@deepseek-ai/dsh-host-webserver': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-services.mjs')).href,
  '@deepseek-ai/dsh-mcp-resources': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-services.mjs')).href,
  '@deepseek-ai/dsh-client-connection': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-client-connection.mjs')).href,
  '@deepseek-ai/dsh-settings': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-settings.mjs')).href,
  '@deepseek-ai/dsh-skill-filesystem': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-skill-filesystem.mjs')).href,
  '@deepseek-ai/dsh-session/surface': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-session-surface.mjs')).href,
  '@deepseek-ai/dsh-agent': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-agent.mjs')).href,
}
/**
 * 能映射到 Codex 钩子的 dsh 事件。监听器由配套拓展包的钩子脚本经 bridge.mjs 调用，
 * dsh 的决定翻译成 Codex 钩子输出（语义对照见设计文档）。
 */
export const EVENT_HOOKS = {
  'tools/pre-execute': 'PreToolUse',
  'tools/post-execute': 'PostToolUse',
  'agent/pre-step': 'UserPromptSubmit',
  'agent/turn-stopping': 'Stop',
  'agent/created': 'SessionStart',
  'subagent/start': 'SubagentStart',
  'subagent/end': 'SubagentStop',
  'approval/request': 'PermissionRequest',
  'workspace/session-stop': 'Stop',
}
/**
 * 由会话镜像派发的事件：钩子常驻进程把 Codex 钩子翻成 dsh 会话日志，再按 dsh 的语义发这些事件。
 * 用到它们（或 sessions / agents / sessionProjections 服务）的插件，配套拓展包会挂上 MIRROR_HOOKS 全部钩子。
 */
export const MIRROR_EVENTS = new Set(['session/event', 'session/created', 'agent/status', 'tools/result'])
export const MIRROR_HOOKS = ['SessionStart', 'UserPromptSubmit', 'PreToolUse', 'PostToolUse', 'Stop', 'PreCompact', 'PostCompact']
/** 能对上 Codex++ 网关中间件的事件：agent/request 改模型 / 推理强度，agent/request-error 决定重试。 */
export const GATEWAY_EVENTS = {
  'agent/error': 'error',
  'agent/request': 'step',
  'system-prompt/assemble': 'instructions',
  'agent/request-error': 'error',
  'llm/stream': 'response',
  'agent/assistant-stream': 'response',
}
/**
 * 除了钩子之外还要在网关请求侧跑的事件：agent/pre-step 在工具调用之后的每一步也触发，
 * tools/post-execute 改写的工具结果在下一次请求里换进 input。
 */
export const STEP_EVENTS = new Set(['agent/pre-step', 'tools/post-execute'])
/** system-prompt/assemble 里代表 Codex 自己系统提示词的片段名：插件删改它就整段替换 instructions。 */
export const CODEX_INSTRUCTIONS_SECTION = 'codex:instructions'
/** 会话镜像与 agent 替身自己发的生命周期事件。 */
const AGENT_LIFECYCLE_EVENTS = new Set(['session/flush', 'session/disposed', 'agent/disposed'])
/** tools/execute（waterfall）：插件不调 next() 直接给结果就是代为执行。 */
export const TOOL_EXECUTE_EVENT = 'tools/execute'
/** 宿主自己会发的注册变化事件：监听它们不提示「不会触发」。 */
const HOST_EMITTED_EVENTS = new Set([
  'tools/change', 'skills/change', 'commands/change', 'system-prompt/change', 'schedule/changed', 'domain/changed',
  'credentials/reference-updated', 'credentials/record-updated', 'goal/changed', 'workspace/session-activity',
  // 宿主的 fs 服务在写 / 改 / 读时发（只管插件自己经 ctx.fs 的读写，管不到 Codex 的工具）。
  'fs/write-intent', 'fs/edit-intent', 'fs/observed',
  'subagent/start', 'subagent/end',
])
/** Cordis 的事件方法可以带 thisArg：`ctx.emit(thisArg, name, ...args)`。返回 [事件名, 其余参数]。 */
function eventArgs(args) {
  const list = [...args]
  if (typeof list[0] !== 'string' && typeof list[1] === 'string') list.shift()
  return [String(list.shift()), list]
}
/** 插件注册的斜杠命令：配套拓展包在 Codex 页面里注册同名命令，经后台转给常驻进程执行。 */
export const COMMANDS_TARGET = 'commands'
/** 插件宿主半边经 ctx.connection.fetch 注册的 HTTP 路由：前端半边的 fetch('/api/…') 经配套拓展包的后台转过来。 */
export const CONNECTION_TARGET = 'connection'
/** 用到 ctx.schedule 的行：配套拓展包挂每分钟一次的定时任务，到点往会话里发消息。 */
export const SCHEDULE_TARGET = 'schedule'
/** 注册了 webhook 规则的行：配套拓展包开 /ext/<拓展>/webhook/<kind> 路由。 */
export const WEBHOOK_TARGET = 'webhook'
/** 监听插件之间自定义事件的行：别的包的插件 emit 时，经常驻进程之间的本地 socket 转过来（见 bridge.mjs）。 */
export const EVENTS_TARGET = 'events'
/** 需要会话镜像（钩子常驻进程把 Codex 钩子翻成 dsh 会话日志）的行，映射到这个目标。 */
export const MIRROR_TARGET = 'mirror'
/** 系统提示词片段的映射目标：经网关中间件追加到每次模型请求的 instructions。 */
export const INSTRUCTIONS_TARGET = 'instructions'
const MCP_CLIENT_MODULE = '@deepseek-ai/dsh-mcp-client'
const GROUP_MODULE = 'cordis:group'
const APPLY_TIMEOUT_MS = 15_000
const MAX_SKILL_SCAN_DEPTH = 4
/** 一个包（含它引用的依赖包）最多列出多少个 SKILL.md。 */
const MAX_SCANNED_SKILLS = 64
const MAX_TEXT_BYTES = 1024 * 1024
/** dsh 前端插件入口的上限（与 dsh_plugins.rs 的 DSH_CLIENT_MAX_BYTES 一致）：整个夹进配套拓展包的页面脚本。 */
const CLIENT_MAX_BYTES = 4 * 1024 * 1024

// MCP 帧走 stdout（钩子 / 网关进程里 stdout 是运行器的结果），插件的 console 输出一律改到 stderr。
const writeStdout = process.stdout.write.bind(process.stdout)
for (const level of ['log', 'info', 'debug', 'trace']) {
  console[level] = (...args) => console.error(...args)
}

// 插件代码不能把宿主进程退掉：MCP 进程、常驻进程里同时跑着别的插件，分析时退掉也就没有报告。
// 宿主自己的进程（inspect / serve / 常驻进程）里把 process.exit 换成抛错（模块顶层调用时这一行加载失败、报出原因），
// 宿主自己退出用 hostExit。被钩子运行器等当库引入时不换：那些进程跑完要自己调 process.exit。
export const hostExit = process.exit.bind(process)
export function guardProcessExit() {
  process.exit = (code) => {
    throw new Error(`插件调用了 process.exit(${code ?? ''})：Codex++ 不允许插件退出宿主进程`)
  }
}

register(pathToFileURL(join(HOST_DIR, 'loader.mjs')).href, import.meta.url, { data: { shims: SHIMS } })

// ---------------------------------------------------------------------------
// 包分析
// ---------------------------------------------------------------------------

function readJson(path) {
  return JSON.parse(readFileSync(path, 'utf8'))
}

export function errorText(error) {
  if (error instanceof Error) return error.message || String(error)
  return String(error)
}

function readLocaleMeta(root) {
  const meta = {}
  for (const language of ['zh', 'en']) {
    const path = join(root, 'locale', `${language}.json`)
    if (!existsSync(path)) continue
    try {
      const value = readJson(path)?.meta ?? {}
      meta.title ??= typeof value.title === 'string' ? value.title : undefined
      meta.description ??= typeof value.description === 'string' ? value.description : undefined
    } catch {
      // locale 损坏只影响展示文案。
    }
  }
  return meta
}

/** 按 Node 的 exports 条件挑出 ESM 入口（import → node → default）。 */
function pickExport(entry) {
  if (typeof entry === 'string') return entry
  if (Array.isArray(entry)) {
    for (const item of entry) {
      const picked = pickExport(item)
      if (picked) return picked
    }
    return undefined
  }
  if (entry && typeof entry === 'object') {
    for (const condition of ['import', 'node', 'default', 'module', 'require']) {
      if (entry[condition] !== undefined) {
        const picked = pickExport(entry[condition])
        if (picked) return picked
      }
    }
  }
  return undefined
}

function packageEntry(directory, pkg, subpath = '.') {
  const exportsField = pkg.exports
  let target
  if (exportsField !== undefined) {
    const isSubpathMap = exportsField && typeof exportsField === 'object' && !Array.isArray(exportsField)
      && Object.keys(exportsField).some((key) => key.startsWith('.'))
    if (isSubpathMap) target = pickExport(exportsField[subpath])
    else if (subpath === '.') target = pickExport(exportsField)
  }
  if (!target && subpath === '.') target = pkg.module ?? pkg.main ?? 'index.js'
  if (!target && subpath !== '.') target = subpath
  if (!target) return undefined
  const path = resolve(directory, target)
  if (!isInside(directory, path)) return undefined
  return path
}

function isInside(root, path) {
  const rel = relative(root, path)
  return rel === '' || (!rel.startsWith('..') && !isAbsolute(rel))
}

function splitPackageName(specifier) {
  const parts = specifier.split('/')
  const count = specifier.startsWith('@') ? 2 : 1
  return { name: parts.slice(0, count).join('/'), subpath: parts.length > count ? `./${parts.slice(count).join('/')}` : '.' }
}

/** 从 fromDir 向上找 node_modules/<name>。 */
function findInstalledPackage(fromDir, name) {
  let directory = fromDir
  for (;;) {
    const candidate = join(directory, 'node_modules', ...name.split('/'))
    if (existsSync(join(candidate, 'package.json'))) return candidate
    const parent = dirname(directory)
    if (parent === directory) return undefined
    directory = parent
  }
}

function resolveRowModule(row, root, pkg) {
  const name = row.name
  if (name.startsWith('.') || isAbsolute(name)) {
    const path = resolve(row.baseDir, name)
    return existsSync(path) ? { path } : { error: `找不到模块文件 ${name}` }
  }
  const { name: packageName, subpath } = splitPackageName(name)
  if (packageName === pkg.name) {
    const path = packageEntry(root, pkg, subpath)
    return path && existsSync(path) ? { path } : { error: `包入口 ${name} 不存在，可能还没有构建产物（lib/）` }
  }
  const directory = findInstalledPackage(root, packageName)
  if (!directory) {
    if (packageName.startsWith('@deepseek-ai/dsh-')) {
      return { error: `引用的是 dsh 内置模块 ${packageName}，Codex++ 不提供（多为 Codex 已自带的能力或依赖 dsh 的 agent 循环）`, builtin: true }
    }
    return { error: `依赖 ${packageName} 没有安装` }
  }
  let installed
  try {
    installed = readJson(join(directory, 'package.json'))
  } catch (error) {
    return { error: `无法读取 ${packageName}/package.json：${errorText(error)}` }
  }
  const path = packageEntry(directory, installed, subpath)
  return path && existsSync(path) ? { path, packageDir: directory } : { error: `${name} 没有可加载的入口`, packageDir: directory }
}

/** 展开 patch 里的 insert 行；cordis:group 的子行放在 config 里。 */
/** include 行：把另一个 YAML / JSON 条目文件（包内）的内容当作一组子行，带上它自己的 patches。 */
const INCLUDE_MODULES = new Set(['cordis:include', '@deepseek-ai/cordis-plugin-include'])

/** Cordis 条目 YAML 的表达式标签。 */
const JS_TAGS = new Set(['!!js', '!js', 'tag:yaml.org,2002:js'])
/** 单个 `!!js` 表达式的求值上限（插件代码本身就在宿主进程无沙箱运行，这里只防死循环卡住分析）。 */
const EXPRESSION_TIMEOUT_MS = 1000

/** 标签值的原文：YAML 里常写成 `!!js "expr"`，去掉一层引号。 */
function taggedText(value) {
  const raw = String(value?.value ?? '').trim()
  if (raw.startsWith('"') && raw.endsWith('"') && raw.length >= 2) {
    try {
      return String(JSON.parse(raw))
    } catch {}
  }
  return raw.replace(/^(["'])([\s\S]*)\1$/, '$2').trim()
}

/**
 * 求一个带标签的 YAML 值，返回 `{ value }` 或 `{ error }`。
 * `!!js` 照 Cordis Loader 的写法在 Node `vm` 里求值完整表达式：能用 `ctx`（本包的服务，如 `ctx.webStartup.port ?? 3080`、
 * `ctx.dshHomePath('x')`）、`env` / `process.env` 与 JS 内置对象；插件代码本来就在宿主进程无沙箱运行，
 * 求值 patch 表达式不额外扩大权限，只加超时。YAML 的 `!!str` / `!!int` / `!!float` / `!!bool` / `!!null` 按类型转换。
 */
function evalTagged(value, scope = {}) {
  if (!value || typeof value !== 'object' || typeof value.tag !== 'string') return { value }
  const text = taggedText(value)
  switch (value.tag) {
    case '!!str':
      return { value: text }
    case '!!int':
    case '!!float': {
      const number = Number(text)
      return Number.isFinite(number) ? { value: value.tag === '!!int' ? Math.trunc(number) : number } : { error: `${text} 不是数字` }
    }
    case '!!bool':
      return /^(true|false)$/i.test(text) ? { value: text.toLowerCase() === 'true' } : { error: `${text} 不是布尔值` }
    case '!!null':
      return { value: null }
    default:
      break
  }
  if (!JS_TAGS.has(value.tag)) return { error: `不认识的 YAML 标签 ${value.tag}` }
  if (!text) return { error: '表达式是空的' }
  try {
    // 换行放在括号前：表达式末尾的 `// 注释` 不会吃掉右括号。
    const result = runInNewContext(`(${text}\n)`, expressionScope(scope), { timeout: EXPRESSION_TIMEOUT_MS, filename: 'cordis-patch-expression.js' })
    return { value: result }
  } catch (error) {
    return { error: errorText(error) }
  }
}

/** `!!js` 表达式能看到的名字：ctx（本包的服务视图）、env、精简的 process。 */
function expressionScope({ ctx } = {}) {
  return {
    ctx: ctx ?? {},
    env: process.env,
    process: Object.freeze({ env: process.env, platform: process.platform, arch: process.arch, version: process.version, versions: process.versions, cwd: () => process.cwd() }),
  }
}

/** 求 patch 行 / 配置里的标签值；求不出时记一条诊断并返回 fallback。 */
function resolveTagged(value, scope, report, fallback) {
  const { value: result, error } = evalTagged(value, scope)
  if (error === undefined) return result
  report(`表达式 ${value.tag} ${taggedText(value)} 求值失败：${error}`)
  return fallback
}

function collectRows(entries, baseDir, diagnostics, out, parentDisabled = false, depth = 0, scope = {}, parentIsolate = {}) {
  for (const row of entries ?? []) {
    if (!row || typeof row !== 'object' || typeof row.name !== 'string') {
      diagnostics.push('patch 中有缺少 name 的插件行，已跳过')
      continue
    }
    let disabledValue = row.disabled
    if (disabledValue && typeof disabledValue === 'object') {
      // Cordis Loader 把求值结果按真假处理（其余真值也算停用）。
      disabledValue = resolveTagged(disabledValue, scope, (text) => diagnostics.push(`插件行 ${row.id ?? row.name} 的 disabled ${text}，按启用处理`), false)
    }
    const disabled = parentDisabled || Boolean(disabledValue)
    // Cordis 的 isolate：`{ 服务名: true | '标签' }`。true 给这一行（组则整组）一个私有作用域，字符串是同名标签共享的作用域。
    const isolate = { ...parentIsolate, ...isolateOf(row, diagnostics) }
    if (row.name === GROUP_MODULE || row.group === true) {
      collectRows(Array.isArray(row.config) ? row.config : [], baseDir, diagnostics, out, disabled, depth, scope, isolate)
      continue
    }
    if (INCLUDE_MODULES.has(row.name)) {
      collectRows(readIncludedRows(row, baseDir, diagnostics, depth), baseDir, diagnostics, out, disabled, depth + 1, scope, isolate)
      continue
    }
    out.push({ id: String(row.id ?? row.name), name: row.name, config: row.config ?? undefined, disabled, baseDir, isolate })
  }
}

/** 行上的 isolate 声明 → `{ 服务名: 作用域键 }`。true 的作用域以这一行的 id 区分，字符串按标签共享。 */
function isolateOf(row, diagnostics) {
  const spec = row.isolate
  if (!spec || typeof spec !== 'object' || Array.isArray(spec)) return {}
  const out = {}
  for (const [name, value] of Object.entries(spec)) {
    if (value === true) out[name] = `entry:${String(row.id ?? row.name)}`
    else if (typeof value === 'string' && value) out[name] = `label:${value}`
    else diagnostics.push(`插件行 ${row.id ?? row.name} 的 isolate.${name} 只能是 true 或标签字符串，已忽略`)
  }
  return out
}

/** 读 include 行指向的条目文件：只认包内路径，最多嵌套 4 层；文件里的条目再应用 include 自己的 patches。 */
function readIncludedRows(row, baseDir, diagnostics, depth) {
  const path = typeof row.config?.path === 'string' ? resolve(baseDir, row.config.path) : undefined
  if (!path || depth >= 4) {
    diagnostics.push(`include 行 ${row.id ?? row.name} 缺少 path 或嵌套太深，已跳过`)
    return []
  }
  if (!isInside(baseDir, path) || !existsSync(path)) {
    diagnostics.push(`include 行 ${row.id ?? row.name} 指向的 ${row.config.path} 不在包内或不存在`)
    return Array.isArray(row.config?.initial) ? row.config.initial : []
  }
  let entries
  try {
    const text = readFileSync(path, 'utf8')
    entries = path.endsWith('.json') ? JSON.parse(text) : parseYaml(text)
  } catch (error) {
    diagnostics.push(`无法解析 include 文件 ${row.config.path}：${errorText(error)}`)
    return []
  }
  return applyEntryPatches(Array.isArray(entries) ? entries : [], row.config?.patches, diagnostics)
}

/**
 * dsh 的 patch 语义（同 dsh-app-boot 的 applyEntryPatches）：`{ insert }` 追加行（带 id 时插进那个组），
 * 其余带 id 的项按 id 覆盖已有行的字段（写了 name 时要对得上）。不改输入。
 */
function applyEntryPatches(data, patches, diagnostics) {
  const entries = structuredClone(Array.isArray(data) ? data : [])
  if (!Array.isArray(patches) || !patches.length) return entries
  const byId = new Map()
  const index = (list) => {
    for (const entry of list) {
      if (!entry || typeof entry !== 'object') continue
      if (entry.id) byId.set(String(entry.id), entry)
      if ((entry.group || entry.name === GROUP_MODULE) && Array.isArray(entry.config)) index(entry.config)
    }
  }
  index(entries)
  for (const patch of patches) {
    if (!patch || typeof patch !== 'object') continue
    const { id, insert, name, ...overrides } = patch
    if (insert) {
      const rows = Array.isArray(insert) ? structuredClone(insert) : []
      if (id) {
        const target = byId.get(String(id))
        if (!target || !(target.group || target.name === GROUP_MODULE)) {
          diagnostics.push(`patch insert：没有找到组 ${id}，已跳过`)
          continue
        }
        if (!Array.isArray(target.config)) target.config = []
        target.config.push(...rows)
      } else {
        entries.push(...rows)
      }
      index(rows)
      continue
    }
    if (!id) {
      diagnostics.push('patch：非 insert 的项需要 id，已跳过')
      continue
    }
    const target = byId.get(String(id))
    if (!target) {
      diagnostics.push(`patch：没有找到行 ${id}，已跳过`)
      continue
    }
    if (name && name !== target.name) {
      diagnostics.push(`patch：行 ${id} 的 name 对不上（${target.name} ≠ ${name}），已跳过`)
      continue
    }
    for (const [key, value] of Object.entries(overrides)) target[key] = value
  }
  return entries
}

const isTaggedNode = (value) => Boolean(value) && typeof value === 'object' && typeof value.tag === 'string' && 'value' in value && Object.keys(value).length === 2

/** 配置里的标签值逐个求值（同 Cordis 在 apply 前解开配置里的表达式）；求不出的保留原样并经 report 说明。 */
function resolveTaggedConfig(value, scope = {}, report = () => {}) {
  if (!value || typeof value !== 'object') return value
  if (isTaggedNode(value)) return resolveTagged(value, scope, report, value)
  if (Array.isArray(value)) return value.map((item) => resolveTaggedConfig(item, scope, report))
  return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, resolveTaggedConfig(item, scope, report)]))
}

function readBundleRows(root, pkg, diagnostics, scope = {}) {
  const patchField = pkg.dsh?.bundle?.patch
  const patches = Array.isArray(patchField) ? patchField : [patchField]
  const rows = []
  let entries = []
  let baseDir = root
  for (const patch of patches) {
    if (typeof patch !== 'string') continue
    const path = resolve(root, patch)
    if (!isInside(root, path) || !existsSync(path)) {
      diagnostics.push(`bundle patch ${patch} 不存在`)
      continue
    }
    let document
    try {
      document = parseYaml(readFileSync(path, 'utf8'))
    } catch (error) {
      diagnostics.push(`无法解析 ${patch}：${errorText(error)}`)
      continue
    }
    // 多个 patch 文件按顺序作用在同一棵条目树上，最后再展开成行。
    entries = applyEntryPatches(entries, Array.isArray(document) ? document : [], diagnostics)
    baseDir = dirname(path)
  }
  collectRows(entries, baseDir, diagnostics, rows, false, 0, scope)
  return rows
}

function normalizeInject(inject) {
  if (!inject) return { required: [], optional: [] }
  if (Array.isArray(inject)) return { required: inject.map(String), optional: [] }
  if (typeof inject === 'object') {
    if (Array.isArray(inject.required) || Array.isArray(inject.optional)) {
      return { required: (inject.required ?? []).map(String), optional: (inject.optional ?? []).map(String) }
    }
    const required = []
    const optional = []
    for (const [key, value] of Object.entries(inject)) {
      ;(value && typeof value === 'object' && value.required === false ? optional : required).push(key)
    }
    return { required, optional }
  }
  return { required: [], optional: [] }
}

function isClass(value) {
  return typeof value === 'function' && /^class[\s{]/.test(Function.prototype.toString.call(value))
}

function pluginFromModule(module) {
  if (typeof module.apply === 'function') {
    return { name: module.name, inject: module.inject, Config: module.Config, apply: module.apply }
  }
  const fallback = module.default
  if (isClass(fallback)) return { serviceClass: fallback, name: fallback.name, inject: fallback.inject, Config: fallback.Config }
  if (typeof fallback === 'function') {
    return { name: fallback.name, inject: fallback.inject, Config: fallback.Config, apply: fallback }
  }
  if (fallback && typeof fallback.apply === 'function') {
    return { name: fallback.name, inject: fallback.inject, Config: fallback.Config, apply: fallback.apply.bind(fallback) }
  }
  return null
}

function toMcpServer(row) {
  const config = row.config ?? {}
  const serverName = typeof config.serverName === 'string' ? config.serverName : row.id
  if (config.transport === 'streamable-http' || (config.url && !config.command)) {
    if (typeof config.url !== 'string') return { error: 'streamable-http 缺少 url' }
    return { serverName, server: { url: config.url, ...(isStringMap(config.headers) ? { headers: config.headers } : {}) } }
  }
  if (typeof config.command !== 'string') return { error: 'stdio MCP 缺少 command' }
  const server = { command: config.command }
  if (Array.isArray(config.args)) server.args = config.args.map(String)
  if (isStringMap(config.env) && Object.keys(config.env).length > 0) server.env = config.env
  if (typeof config.cwd === 'string' && config.cwd) server.cwd = config.cwd
  return { serverName, server }
}

function isStringMap(value) {
  return value && typeof value === 'object' && !Array.isArray(value) && Object.values(value).every((item) => typeof item === 'string')
}

function parseFrontmatter(text) {
  const match = /^---\r?\n([\s\S]*?)\r?\n---(?:\r?\n|$)/.exec(text)
  if (!match) return {}
  try {
    const value = parseYaml(match[1])
    return value && typeof value === 'object' ? value : {}
  } catch {
    return {}
  }
}

/**
 * 包里所有 SKILL.md（不进 node_modules / 隐藏目录、不跟软链），每个所在目录是一个 skill。
 * `budget.remaining` 是多次扫描共用的数量上限。
 */
function scanSkills(root, budget = { remaining: MAX_SCANNED_SKILLS }) {
  const skills = []
  const walk = (directory, depth) => {
    if (budget.remaining <= 0) return
    let entries
    try {
      entries = readdirSync(directory, { withFileTypes: true })
    } catch {
      return
    }
    if (entries.some((entry) => entry.isFile() && entry.name === 'SKILL.md')) {
      const path = join(directory, 'SKILL.md')
      let text = ''
      try {
        if (statSync(path).size <= MAX_TEXT_BYTES) text = readFileSync(path, 'utf8')
      } catch {
        // 读不了就只用目录名。
      }
      const meta = parseFrontmatter(text)
      skills.push({
        name: typeof meta.name === 'string' && meta.name ? meta.name : directory.split(sep).pop(),
        description: typeof meta.description === 'string' ? meta.description : '',
        directory,
      })
      budget.remaining--
      return
    }
    if (depth >= MAX_SKILL_SCAN_DEPTH) return
    for (const entry of entries) {
      if (!entry.isDirectory() || entry.name === 'node_modules' || entry.name.startsWith('.')) continue
      walk(join(directory, entry.name), depth + 1)
    }
  }
  walk(root, 0)
  return skills
}

/** skill 名 → 目录名：Codex Skills 的 id 只接受小写字母、数字与 -_。 */
function skillDirectoryName(name) {
  const id = String(name).toLowerCase().replace(/[^a-z0-9_-]+/g, '-').replace(/^-+|-+$/g, '')
  return id || 'skill'
}

/** 把插件经 ctx.skills 注册的 skill 写成 SKILL.md 目录；资源目录里的其他文件一并带上。 */
function writeRuntimeSkill(outDir, definition) {
  const directory = join(outDir, skillDirectoryName(definition.name))
  mkdirSync(directory, { recursive: true })
  const base = definition.resourceBase?.kind === 'directory' ? definition.resourceBase.path : undefined
  if (base && existsSync(base) && statSync(base).isDirectory()) {
    cpSync(base, directory, {
      recursive: true,
      force: true,
      filter: (source) => {
        const name = source.split(sep).pop()
        return !(source !== base && (name === 'node_modules' || name.startsWith('.') || (dirname(source) === base && name === 'SKILL.md')))
      },
    })
  }
  // JSON 字符串是合法的 YAML 双引号标量，描述里的冒号、换行都不会破坏 frontmatter。
  const frontmatter = [`name: ${JSON.stringify(String(definition.name))}`, `description: ${JSON.stringify(String(definition.description ?? ''))}`]
  writeFileSync(join(directory, 'SKILL.md'), `---\n${frontmatter.join('\n')}\n---\n\n${String(definition.content ?? '').trim()}\n`)
  return directory
}

// ---------------------------------------------------------------------------
// 插件运行时
// ---------------------------------------------------------------------------

export class Host {
  /** @param {{ mirrorDir?: string }} options mirrorDir：会话镜像落盘目录（钩子常驻进程给；MCP / 分析进程不给，只在内存里）。 */
  constructor(options = {}) {
    /** name → { def, pluginKey } */
    this.tools = new Map()
    /** ctx.tools.guard 注册的守卫：在 tools/pre-execute 之后、工具执行之前，返回字符串即拒绝。 */
    this.toolGuards = []
    /** ctx.tools.restrict 注册的可见性限制（allow / deny 工具名）。 */
    this.toolRestrictions = []
    /** 系统提示词片段，汇总成 MCP instructions，并由配套拓展包经网关追加到 instructions。 */
    this.sections = []
    /** dsh 的动态上下文（systemPrompt.context）：Codex 里没有用户侧快照，并到片段后面。 */
    this.contexts = []
    /** name → provider：片段里 `{{name}}` 的取值。 */
    this.variables = new Map()
    this.listeners = new Map()
    this.disposers = []
    /** name → { def, state }：插件经 ctx.commands 注册的斜杠命令。 */
    this.commands = new Map()
    /** path → { methods, fetch }：插件经 ctx.connection.fetch 注册的路由。 */
    this.routes = new Map()
    /** channel → handler、拦截器：插件经 ctx.connection.rpc 注册，前端半边的 rpc.call 转过来。 */
    this.rpcHandlers = new Map()
    this.rpcInterceptors = []
    /** path → route、兜底处理：插件经 ctx.webServer 注册的网页路由。 */
    this.webRoutes = new Map()
    this.webFallback = undefined
    /** path → route：插件经 ctx.webServer.registerUpgrade 注册的协议升级（WebSocket）处理器。 */
    this.webUpgrades = new Map()
    /** server → provider：插件经 ctx.mcpResources 注册的 MCP 资源。 */
    this.mcpResources = new Map()
    const emit = (event, args) => this.emitEvent(event, args)
    this.mirror = new SessionMirror({ dataDir: options.mirrorDir, emit })
    /** 插件经 ctx.webhookRuntime 注册的规则（配套拓展包的后台把 /webhook/<kind> 请求转过来）。 */
    this.webhooks = createWebhookRuntime()
    /** 插件包的 ctx.schedule（记录存在包的数据目录，按包共用）。 */
    this.schedules = undefined
    this.agentRegistry = new AgentRegistry({ mirror: this.mirror, emit, serial: (event, args) => this.serial(event, args) })
    this.shared = undefined
    /**
     * 插件提供的服务（Service 子类插件、ctx.set / ctx.provide）：作用域键 → 服务名 → { instance }。
     * 作用域键是 'global'，或行上 isolate 声明 / ctx.isolate 给出的私有键（Cordis 的 isolate 语义）。
     */
    this.realms = new Map()
    this.isolateSeq = 0
    /**
     * typert 类型化 RPC（dsh 的 ctx.typert）：endpoint（`<namespace>/<method>`）→ 调用描述；
     * 包导出的 `./typert`（生成的 TYPERT 清单）在加载时自动登记，插件也可以 ctx.typert.register。
     * 前端半边的 ctx.remote.<namespace>.<method>() 经 connection.rpc 的 `/api` 通道调到这里。
     */
    this.typertInvocations = new Map()
    this.typertSeen = new Set()
    this.typertLookups = new Map()
    this.typertContexts = new Map()
    this.typertResources = new Map()
    this.typertListeners = new Set()
    this.typertPackages = new Set()
    /** 正在进行的 typert 调用（dsh 里 Service 方法经 this.ctx.invocation 读到它）。 */
    this.invocations = new AsyncLocalStorage()
    /** connection.rpc.open 打开的流：id → 状态（只在常驻进程里，前端经后台分批拉取）。 */
    this.rpcStreams = new Map()
  }

  /** 服务名在一组 isolate 声明下落在哪个作用域。 */
  realmKey(name, isolate) {
    return isolate?.[name] ?? 'global'
  }

  /** 插件提供的服务实例（按 isolate 找作用域）；没有返回 undefined。 */
  findService(name, isolate) {
    return this.realms.get(this.realmKey(name, isolate))?.get(name)?.instance
  }

  /** 服务是否可用：宿主替身，或同一个宿主里有插件提供了它。 */
  serviceAvailable(name, isolate) {
    return SUPPORTED_SERVICES.has(name) || this.findService(name, isolate) !== undefined
  }

  /** 登记插件提供的服务，返回撤销函数（撤销时恢复被它顶替的旧实现）。发 internal/service 同 Cordis。 */
  provideService(name, instance, isolate) {
    const key = this.realmKey(name, isolate)
    if (!this.realms.has(key)) this.realms.set(key, new Map())
    const realm = this.realms.get(key)
    const previous = realm.get(name)
    const entry = { instance }
    realm.set(name, entry)
    queueMicrotask(() => this.emitEvent('internal/service', [name]))
    return () => {
      if (realm.get(name) !== entry) return
      if (previous) realm.set(name, previous)
      else realm.delete(name)
      queueMicrotask(() => this.emitEvent('internal/service', [name]))
    }
  }

  /**
   * 按 Cordis 语义实例化 Service 子类：`new Class(ctx, config)`，构造函数里 `super(ctx, name)` 经替身 Service 登记服务；
   * 随后跑 `[Service.init]()`（可返回 / yield 撤销函数），没有就跑 `start()`；卸载时调 `[Service.dispose]()` 或 `stop()`。
   */
  async instantiateService(ServiceClass, ctx, config, track) {
    const instance = new ServiceClass(ctx, config)
    const init = instance?.[SERVICE_INIT]
    if (typeof init === 'function') {
      const result = init.call(instance)
      if (result && typeof result[Symbol.asyncIterator] === 'function') {
        for await (const dispose of result) if (typeof dispose === 'function') track(dispose)
      } else if (result && typeof result[Symbol.iterator] === 'function' && typeof result.next === 'function') {
        for (const dispose of result) if (typeof dispose === 'function') track(dispose)
      } else {
        const dispose = await result
        if (typeof dispose === 'function') track(dispose)
      }
    } else if (typeof instance?.start === 'function') {
      await instance.start()
    }
    track(() => (typeof instance?.[SERVICE_DISPOSE] === 'function' ? instance[SERVICE_DISPOSE]() : instance?.stop?.()))
    return instance
  }

  /** loader.entries() 的数据：每个加载过的行一项；返回这一行的 fiber（插件比较 entry.fiber === ctx.fiber）。同一行派生的子 ctx 复用它。 */
  registerLoaderEntry(state) {
    this.loaderEntries ??= new Map()
    const existing = this.loaderEntries.get(state.key)
    if (existing) return existing.fiber
    const options = { id: state.rowId, name: state.moduleName ?? state.pluginName, config: state.config }
    const fiber = { entry: { options } }
    this.loaderEntries.set(state.key, { options, fiber, disabled: false })
    return fiber
  }

  /** 宿主级 loader：和每个插件 ctx.loader 读同一份已登记的行。 */
  hostLoader() {
    return (this.hostLoaderInstance ??= createLoader({ entries: () => (this.loaderEntries ? this.loaderEntries.values() : []) }))
  }

  /** 所有插件共用的替身服务（按需建）。credentials / storageDomain 按插件建，见 createContext。 */
  sharedServices() {
    if (this.shared) return this.shared
    const host = this
    const mirror = this.mirror
    const sessions = {
      get: (id) => mirror.sessions.get(String(id)),
      list: () => [...mirror.sessions.values()],
      /** 新建会话：经 Codex++ 新建一个真实的 Codex 会话，返回它的镜像（dsh 的会话日志格式不在 Codex 里）。 */
      async create(options = {}) {
        const cwd = typeof options?.cwd === 'string' ? options.cwd : options?.meta?.cwd
        const result = await hostCall('sessions', 'create', { options: cwd ? { cwd } : {} })
        const id = result?.thread?.id
        if (!id) throw new Error('Codex 没有返回新会话的 id')
        return mirror.ensure(String(id), { cwd })
      },
      /** 分叉会话：分叉对应的 Codex 会话，返回新会话的镜像（只有之后的事件进镜像）。 */
      async fork(source, options = {}) {
        const id = typeof source === 'string' ? source : source?.id ?? source?.sessionId
        if (!id) throw new Error('sessions.fork 需要会话或会话 id')
        const thread = await hostCall('sessions', 'fork', { id: String(id), options: options?.codex ?? {} })
        if (!thread?.id) throw new Error('Codex 没有返回分叉出的会话 id')
        return mirror.ensure(String(thread.id), { cwd: thread.cwd })
      },
      async flush(session) {
        mirror.flushAll()
        await host.parallel('session/flush', [session])
        return true
      },
      registerMessageProjection: () => () => Promise.resolve(),
      /** Codex++ 扩展：直接读写 Codex 会话（经 Codex 页面里的 app-server 连接）。 */
      codex: Object.freeze(Object.fromEntries(
        ['list', 'create', 'fork', 'rename', 'append', 'startTurn', 'steer', 'interrupt', 'compact'].map((method) => [method, (...args) => {
          const [first, second, third] = args
          const params = method === 'list' ? (first ?? {})
            : method === 'create' ? { input: first, options: second }
              : method === 'rename' ? { id: first, name: second }
                : method === 'append' ? { id: first, items: second }
                  : method === 'startTurn' ? { id: first, input: second, options: third }
                    : method === 'steer' ? { id: first, input: second, turnId: third }
                      : method === 'interrupt' ? { id: first, turnId: second }
                        : { id: first, options: second }
          return hostCall('sessions', method, params)
        }]),
      )),
    }
    const sessionProjections = {
      register(definition) {
        const key = String(definition?.key ?? '')
        if (!key || typeof definition?.init !== 'function' || typeof definition?.apply !== 'function') {
          throw new Error('sessionProjections.register 需要 key、init、apply')
        }
        mirror.projections.set(key, definition)
        return () => {
          if (mirror.projections.get(key) === definition) mirror.projections.delete(key)
        }
      },
      stateOf: (session, key) => mirror.stateOf(session, key),
      snapshot(session, keys) {
        const wanted = Array.isArray(keys) ? keys : [...mirror.projections.keys()]
        return { asOfSeq: (session?.seq ?? 0) - 1, values: Object.fromEntries(wanted.map((key) => [key, mirror.stateOf(session, key)])) }
      },
      onChanged(listener) {
        mirror.changeListeners.add(listener)
        return () => mirror.changeListeners.delete(listener)
      },
      cachedSnapshot: () => undefined,
      checkpoint: () => undefined,
      restore: () => undefined,
      hydrate: async () => undefined,
      viewCheckpoint: () => undefined,
    }
    const commands = {
      register(definition) {
        const name = String(definition?.name ?? '')
        if (!/^[a-z][a-z0-9_-]*$/.test(name)) throw new TypeError(`命令名不合法：${name}`)
        if (typeof definition?.handler !== 'function') throw new TypeError(`命令 /${name} 缺少 handler`)
        if (host.commands.has(name)) throw new Error(`命令 /${name} 已注册`)
        const entry = { def: definition }
        host.commands.set(name, entry)
        return () => {
          if (host.commands.get(name) === entry) host.commands.delete(name)
        }
      },
      list: () => [...host.commands.values()].map(({ def }) => ({ definitionId: def.definitionId, name: def.name, description: def.description, input: def.input })),
      find: (agent, name) => host.commands.get(String(name))?.def,
      async execute(agent, line, attachments = [], signal) {
        const match = /^\/([a-z][a-z0-9_-]*)(?:\s+([\s\S]*))?$/.exec(String(line ?? '').trim())
        if (!match) return undefined
        const result = await host.runCommand(match[1], match[2] ?? '', agent, signal)
        return result ? { commandId: `codexpp-${Date.now()}`, result } : undefined
      },
    }
    this.shared = {
      sessions,
      sessionQuery: createSessionQuery(mirror),
      sessionProjections,
      agents: this.agentRegistry.service(),
      llm: createLlm(),
      commands,
      fs: createFs({ waterfall: (event, args, fallback) => host.waterfall(event, args, fallback), emit: (event, args) => host.emitEvent(event, args) }),
      subprocess: createSubprocess(),
      shell: createShell(),
      web: createWeb(),
      userQuestions: createUserQuestions((event, args, fallback) => host.waterfall(event, args, fallback)),
      approval: createApproval((event, args, fallback) => host.waterfall(event, args, fallback)),
      // 任务快照放在所有插件共用的 data/.jobs（插件 id 不能以 . 开头，不会撞上），别的进程也看得到、停得了。
      jobs: createJobs((event, args) => host.emitEvent(event, args), { dir: host.dataDir ? join(dirname(host.dataDir), '.jobs') : undefined }),
      workspaceRegistry: Object.assign(createWorkspaceRegistry(), {
        /** dsh 的 workspace/session-activity：问插件这个会话正在做什么（waterfall），没人答时为空。 */
        activity: (request = {}) => host.waterfall('workspace/session-activity', [request], async () => []),
      }),
      agentDefaultModel: Object.freeze({ provider: 'codex', model: undefined, reasoningEffort: undefined }),
      shellEnv: createShellEnv(),
      fileReferences: createFileReferences(),
      planMode: (host.planMode ??= createPlanMode()),
      goals: (host.goals ??= createGoals({ emit: (event, args) => host.emitEvent(event, args) })),
      subagents: createSubagents({ emit: (event, args) => host.emitEvent(event, args) }),
      sessionTitle: createSessionTitle(),
      tokenMeter: createTokenMeter(),
      sessionTelemetry: createSessionTelemetry(),
    }
    return this.shared
  }

  // ---- typert 类型化 RPC ----

  /** 登记一份 typert 清单（生成的 TYPERT：`{ package, face, schemas, invocations }`，或 TypertRemoteContribution 的 descriptors）。 */
  registerTypert(manifest, owner = {}) {
    const list = Array.isArray(manifest?.invocations) ? manifest.invocations : Array.isArray(manifest?.descriptors) ? manifest.descriptors : []
    const added = []
    for (const descriptor of list) {
      if (!descriptor || typeof descriptor.namespace !== 'string' || typeof descriptor.method !== 'string') continue
      const endpoint = `${descriptor.namespace}/${descriptor.method}`
      if (this.typertInvocations.has(endpoint)) {
        for (const item of added) this.typertInvocations.delete(item)
        throw new Error(`typert endpoint ${endpoint} 已经登记过`)
      }
      this.typertInvocations.set(endpoint, { descriptor, isolate: owner.isolate })
      this.typertSeen.add(endpoint)
      added.push(endpoint)
    }
    // 生成的 schema 工厂：按 `<包>#<名字>` 登记，ctx.typert.get(key) 首次读取时建出解析器。
    const schemaKeys = []
    for (const schema of Array.isArray(manifest?.schemas) ? manifest.schemas : []) {
      if (!schema || typeof schema.name !== 'string' || typeof schema.create !== 'function') continue
      const key = `${String(manifest.package ?? '')}#${schema.name}`
      this.typertSchemas ??= new Map()
      this.typertSchemas.set(key, { create: schema.create, value: undefined })
      schemaKeys.push(key)
    }
    if (added.length) this.notifyTypert({ kind: 'local', added })
    return async () => {
      for (const endpoint of added) this.typertInvocations.delete(endpoint)
      for (const key of schemaKeys) this.typertSchemas?.delete(key)
      if (added.length) this.notifyTypert({ kind: 'local', removed: added })
    }
  }

  notifyTypert(change) {
    for (const listener of [...this.typertListeners]) {
      try {
        listener(change)
      } catch (error) {
        console.error(`[dsh-host] typert 监听器失败：${errorText(error)}`)
      }
    }
  }

  /** 包导出了 `./typert`（dsh 的 typert-loader 约定）：导入它的 TYPERT 清单并登记，一个包只登记一次。 */
  async registerTypertExport(directory, owner, track) {
    if (!directory || this.typertPackages.has(directory)) return false
    this.typertPackages.add(directory)
    let pkg
    try {
      pkg = readJson(join(directory, 'package.json'))
    } catch {
      return false
    }
    const exportsField = pkg.exports
    if (!exportsField || typeof exportsField !== 'object' || Array.isArray(exportsField) || exportsField['./typert'] === undefined) return false
    const path = packageEntry(directory, pkg, './typert')
    if (!path || !existsSync(path)) throw new Error(`${pkg.name} 导出了 ./typert，但找不到 ${path ?? '入口'}`)
    const module = await import(pathToFileURL(path).href)
    const manifest = module.TYPERT
    if (!manifest || typeof manifest !== 'object') throw new Error(`${pkg.name} 导出了 ./typert，但模块里没有 TYPERT 清单`)
    if (manifest.face !== undefined && manifest.face !== 'host') throw new Error(`${pkg.name} 的 TYPERT.face 不是 host`)
    track(this.registerTypert(manifest, owner))
    return Array.isArray(manifest.invocations) && manifest.invocations.length > 0
  }

  /** 按 codec 校验一个边界值：生成的严格 codec 有 create() 给出 zod 解析器；没有 schema 就放行（同 dsh 的 src-json 模式只要求 JSON）。 */
  static parseWith(codec, value) {
    const schema = typeof codec?.create === 'function' ? codec.create() : codec?.schema ?? (typeof codec?.parse === 'function' ? codec : undefined)
    return schema && typeof schema.parse === 'function' ? schema.parse(value) : value
  }

  /** 调用请求里的参数：dsh 的 `{ args: { 线上字段: 值 } }`，或 Codex++ 前端没有描述时的 `{ positional: [...] }`。 */
  async typertArguments(endpoint, descriptor, payload) {
    const parameters = Array.isArray(descriptor.parameters) ? descriptor.parameters : []
    const named = payload && typeof payload === 'object' && payload.args && typeof payload.args === 'object' ? payload.args : undefined
    const positional = Array.isArray(payload?.positional) ? payload.positional : undefined
    if (named) {
      const unknown = Object.keys(named).filter((key) => !parameters.some((parameter) => parameter.wire === key))
      if (unknown.length) throw Object.assign(new Error(`typert ${endpoint}：多余的参数 ${unknown.join(', ')}`), { code: 'gateway/arguments-invalid' })
    }
    const values = []
    for (const [index, parameter] of parameters.entries()) {
      let value = named ? named[parameter.wire] : positional?.[index]
      if (value === undefined && !parameter.acceptsUndefined && !(named && Object.hasOwn(named, parameter.wire))) {
        throw Object.assign(new Error(`typert ${endpoint}：缺少参数 ${parameter.wire}`), { code: 'gateway/arguments-invalid' })
      }
      try {
        value = Host.parseWith(parameter.codec, value)
      } catch (error) {
        throw Object.assign(new Error(`typert ${endpoint}：参数 ${parameter.wire} 不合法：${errorText(error)}`), { code: 'gateway/arguments-invalid' })
      }
      if (parameter.source === 'lookup') {
        const provider = this.typertLookups.get(String(parameter.lookup))
        const resolve = typeof provider === 'function' ? provider : provider?.resolve ?? provider?.lookup
        if (typeof resolve !== 'function') throw Object.assign(new Error(`typert ${endpoint}：没有 ${parameter.lookup} 的查找提供方`), { code: 'gateway/lookup-unavailable' })
        const resolved = await resolve(value)
        if (resolved === undefined) throw Object.assign(new Error(`typert ${endpoint}：找不到 ${parameter.lookup} ${JSON.stringify(value)}`), { code: 'gateway/lookup-not-found' })
        value = resolved
      }
      values.push(value)
    }
    return values
  }

  /** 找调用的接收者：direct 是 descriptor.service 这个服务；context 先按线上身份解析出作用域 ctx 再取服务。 */
  async typertReceiver(endpoint, entry, payload) {
    const { descriptor } = entry
    if (descriptor.invocation?.kind === 'context') {
      const adapter = this.typertContexts.get(String(descriptor.invocation.context))
      const resolve = typeof adapter === 'function' ? adapter : adapter?.resolve
      if (typeof resolve !== 'function') throw Object.assign(new Error(`typert ${endpoint}：没有 ${descriptor.invocation.context} 的作用域提供方`), { code: 'gateway/context-unavailable' })
      const scoped = await resolve(payload?.args?.[descriptor.invocation.wire] ?? payload?.context)
      if (!scoped) throw Object.assign(new Error(`typert ${endpoint}：作用域不存在`), { code: 'gateway/context-not-found' })
      return scoped[descriptor.service] ?? scoped
    }
    const receiver = this.findService(String(descriptor.service), entry.isolate)
    if (!receiver) throw Object.assign(new Error(`typert ${endpoint}：服务 ${descriptor.service} 没有提供`), { code: 'gateway/service-unavailable' })
    return receiver
  }

  /** 执行一个 typert 调用；stream 方法返回异步迭代器。 */
  async invokeTypert(endpoint, payload, signal, { stream = false, uplink } = {}) {
    const entry = this.typertInvocations.get(endpoint)
    if (!entry) throw Object.assign(new Error(`没有 typert endpoint ${endpoint}`), { code: 'gateway/unknown-endpoint' })
    const { descriptor } = entry
    if (Boolean(descriptor.mode === 'stream') !== stream) {
      throw Object.assign(new Error(`typert ${endpoint} 是${descriptor.mode === 'stream' ? '流式方法，要用 rpc.open' : '一次性方法，要用 rpc.call'}`), { code: 'gateway/binding-invalid' })
    }
    const receiver = await this.typertReceiver(endpoint, entry, payload)
    const name = descriptor.implementation ?? descriptor.method
    if (typeof receiver?.[name] !== 'function') throw Object.assign(new Error(`typert ${endpoint}：${descriptor.service} 没有方法 ${name}`), { code: 'gateway/binding-invalid' })
    const args = await this.typertArguments(endpoint, descriptor, payload)
    if (descriptor.cancellation) args.push(signal)
    let used = false
    const invocation = Object.freeze({
      request: Object.freeze({ namespace: descriptor.namespace, method: descriptor.method, args: payload?.args ?? {} }),
      service: descriptor.service,
      peer: Object.freeze({ id: 'codex-plus', principal: { kind: 'owner' }, signal, interactions: 'answer' }),
      signal,
      uplink() {
        if (used) throw new Error('invocation.uplink() 只能取一次')
        used = true
        if (!uplink) return (async function* () {})()
        return descriptor.uplink?.codec
          ? (async function* () { for await (const item of uplink) yield Host.parseWith(descriptor.uplink.codec, item) })()
          : uplink
      },
    })
    const result = await this.invocations.run(invocation, () => receiver[name](...args))
    if (!stream) return Host.parseWith(descriptor.result, result)
    if (!result || typeof result[Symbol.asyncIterator] !== 'function' && typeof result[Symbol.iterator] !== 'function') {
      throw Object.assign(new Error(`typert ${endpoint} 是流式方法，但没有返回可迭代对象`), { code: 'gateway/binding-invalid' })
    }
    // 异步生成器的函数体在迭代时才跑：每次 next() 都放回这次调用的上下文里，方法里读得到 this.ctx.invocation。
    const iterator = typeof result[Symbol.asyncIterator] === 'function' ? result[Symbol.asyncIterator]() : result[Symbol.iterator]()
    const within = (call) => this.invocations.run(invocation, call)
    return {
      [Symbol.asyncIterator]() {
        return this
      },
      async next() {
        const step = await within(() => iterator.next())
        return step.done ? step : { done: false, value: Host.parseWith(descriptor.result, step.value) }
      },
      async return(value) {
        return (await within(() => iterator.return?.(value))) ?? { done: true, value }
      },
    }
  }

  /** 前端 ctx.remote 用的描述清单（只给出调用需要的字段）。 */
  typertDescriptors() {
    return [...this.typertInvocations.values()].map(({ descriptor }) => ({
      namespace: descriptor.namespace,
      method: descriptor.method,
      ...(descriptor.mode === 'stream' ? { mode: 'stream' } : {}),
      ...(descriptor.uplink ? { uplink: true } : {}),
      parameters: (descriptor.parameters ?? []).map((parameter) => ({ name: parameter.name, wire: parameter.wire, ...(parameter.acceptsUndefined ? { optional: true } : {}) })),
      ...(descriptor.description ? { description: String(descriptor.description) } : {}),
    }))
  }

  // ---- connection.rpc.open：流式调用（前端经后台分批拉取） ----

  /** 打开一个流：目前只有 typert 的 stream 方法（dsh 里由 Gateway 的 wireStream 承载）。返回 { ok, id } 或失败。 */
  async openRpcStream({ channel, endpoint, payload } = {}) {
    const key = String(endpoint ?? '')
    if (String(channel ?? '') !== '/api' || !this.typertInvocations.has(key)) {
      return { ok: false, error: { code: 'NOT_FOUND', message: `没有流式 endpoint ${channel} ${key}`, details: {} } }
    }
    const controller = new AbortController()
    const uplinkQueue = []
    let uplinkWake = null
    let uplinkEnded = false
    const uplink = (async function* () {
      for (;;) {
        if (uplinkQueue.length) {
          yield uplinkQueue.shift()
          continue
        }
        if (uplinkEnded || controller.signal.aborted) return
        await new Promise((resolve) => {
          uplinkWake = resolve
        })
      }
    })()
    const id = `rpc-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`
    const state = { items: [], done: false, error: undefined, wake: null, controller, touched: Date.now() }
    state.push = (items = [], end = false) => {
      uplinkQueue.push(...items)
      if (end) uplinkEnded = true
      const resume = uplinkWake
      uplinkWake = null
      resume?.()
    }
    const notify = () => {
      const resume = state.wake
      state.wake = null
      resume?.()
    }
    let iterator
    try {
      iterator = await this.invokeTypert(key, payload, controller.signal, { stream: true, uplink })
    } catch (error) {
      return { ok: false, error: { code: String(error?.code ?? 'HANDLER_FAILED'), message: errorText(error), details: {} } }
    }
    this.rpcStreams.set(id, state)
    void (async () => {
      try {
        for await (const item of iterator) {
          if (controller.signal.aborted) break
          state.items.push(item)
          notify()
        }
      } catch (error) {
        state.error = { code: String(error?.code ?? 'STREAM_FAILED'), message: errorText(error), details: {} }
      } finally {
        state.done = true
        state.push([], true)
        notify()
      }
    })()
    return { ok: true, id }
  }

  /** 取一批流里的值：没有就最多等 waitMs；拿完且结束时给 done（并清掉这个流）。 */
  async pullRpcStream({ id, waitMs = 1000, max = 256 } = {}) {
    const state = this.rpcStreams.get(String(id))
    if (!state) return { items: [], done: true, error: { code: 'NOT_FOUND', message: '流已结束或不存在', details: {} } }
    state.touched = Date.now()
    if (!state.items.length && !state.done) {
      await new Promise((resolve) => {
        const timer = setTimeout(resolve, Math.max(0, Math.min(Number(waitMs) || 0, 25_000)))
        state.wake = () => {
          clearTimeout(timer)
          resolve()
        }
      })
    }
    const items = state.items.splice(0, Math.max(1, Number(max) || 256))
    const done = state.done && !state.items.length
    if (done) this.rpcStreams.delete(String(id))
    return { items, done, ...(done && state.error ? { error: state.error } : {}) }
  }

  /** 前端往流里送上行的值（dsh 的 RemoteStream 上行），end 表示上行结束。 */
  pushRpcStream({ id, items = [], end = false } = {}) {
    const state = this.rpcStreams.get(String(id))
    if (!state) return { ok: false }
    state.push(Array.isArray(items) ? items : [], Boolean(end))
    return { ok: true }
  }

  /** 前端放弃这个流：取消调用并清掉。 */
  closeRpcStream({ id } = {}) {
    const state = this.rpcStreams.get(String(id))
    if (!state) return { ok: true }
    state.controller.abort()
    this.rpcStreams.delete(String(id))
    state.push([], true)
    return { ok: true }
  }

  /**
   * `!!js` 表达式里的 `ctx`：按名字取本包已提供的服务（含 Service 子类插件提供的、按行的 isolate 作用域），
   * 再取宿主替身服务；`ctx.dshHomePath(...)` 指到插件的数据目录（dsh 里是 Harness home）。
   */
  expressionContext(row) {
    const host = this
    return new Proxy({}, {
      get(_target, key) {
        if (typeof key !== 'string') return undefined
        if (key === 'dshHomePath') return (...parts) => join(host.dataDir ?? join(HOST_DIR, '..', 'data'), ...parts.map(String))
        const provided = host.findService(key, row?.isolate)
        if (provided !== undefined) return provided
        // loader 在 !!js 表达式里读宿主里已登记的行（better-sidebar 的 disabled 表达式用它）；其余按插件建的服务在表达式里不可用。
        if (key === 'loader') return host.hostLoader()
        return SUPPORTED_SERVICES.has(key) ? host.sharedServices()[key] : undefined
      },
      has: (_target, key) => typeof key === 'string' && (key === 'dshHomePath' || SUPPORTED_SERVICES.has(key) || host.findService(key, row?.isolate) !== undefined),
    })
  }

  /** 执行插件注册的斜杠命令；没有这个命令返回 undefined。handler 抛错收敛成 error 结果（同 dsh）。 */
  async runCommand(name, rawInput, agent, signal) {
    const entry = this.commands.get(String(name))
    if (!entry) return undefined
    try {
      const result = await entry.def.handler({
        commandId: `codexpp-${Date.now()}`,
        agent,
        rawInput: String(rawInput ?? ''),
        attachments: [],
        signal: signal ?? new AbortController().signal,
      })
      return result && typeof result === 'object' ? result : { kind: 'success' }
    } catch (error) {
      return { kind: 'error', text: errorText(error) }
    }
  }

  /**
   * 调插件注册的路由：{ path, method, headers, body } → { status, headers, body }。
   * 前端半边（dsh.client）在 Codex 页面里发的 fetch('/api/…') 经配套拓展包的后台转到这里。
   */
  async fetchRoute({ path, method = 'GET', headers = {}, body = null } = {}) {
    const url = new URL(String(path ?? '/'), 'http://codexpp.local')
    const route = this.routes.get(url.pathname)
    if (!route) return { status: 404, headers: {}, body: JSON.stringify({ ok: false, error: { message: `没有路由 ${url.pathname}` } }) }
    const verb = String(method).toUpperCase()
    if (route.methods.length && !route.methods.includes(verb)) return { status: 405, headers: {}, body: '' }
    const request = new Request(url, { method: verb, headers, ...(body !== null && verb !== 'GET' && verb !== 'HEAD' ? { body: String(body) } : {}) })
    let response
    try {
      response = await route.fetch(request)
    } catch (error) {
      return { status: 500, headers: {}, body: JSON.stringify({ ok: false, error: { message: errorText(error) } }) }
    }
    if (!(response instanceof Response)) response = new Response(typeof response === 'string' ? response : JSON.stringify(response ?? null))
    return { status: response.status, headers: Object.fromEntries(response.headers.entries()), body: await response.text() }
  }

  /**
   * 调插件注册的 RPC：{ channel, endpoint, payload } → dsh 的 ConnectionRpcResult。拦截器（按 endpoint 匹配）优先；
   * 附件的字节转成 base64（经后台 JSON 往返）。
   */
  async runRpc({ channel, endpoint, payload } = {}) {
    const key = String(channel ?? '')
    const target = String(endpoint ?? '')
    // typert：`/api` 通道上登记过的 endpoint 由宿主按调用描述派发（同 dsh 的 Typert Gateway 拦截 /api）。
    if (key === '/api' && target === '$codexpp/typert-descriptors') return { ok: true, value: this.typertDescriptors() }
    if (key === '/api' && this.typertInvocations.has(target)) {
      const controller = new AbortController()
      try {
        return { ok: true, value: (await this.invokeTypert(target, payload, controller.signal)) ?? null }
      } catch (error) {
        return { ok: false, error: { code: String(error?.code ?? 'HANDLER_FAILED'), message: errorText(error), details: {} } }
      } finally {
        controller.abort()
      }
    }
    const interceptor = this.rpcInterceptors.find((item) => item.channel === key && safeMatch(item.matches, target))
    const handler = interceptor?.handler ?? this.rpcHandlers.get(key)
    if (!handler) return { ok: false, error: { code: 'NOT_FOUND', message: `没有插件处理 ${key} ${target}`, details: {} } }
    const controller = new AbortController()
    try {
      const result = await handler(target, payload, controller.signal, { kind: 'operator', id: 'codex-plus' })
      if (!result || typeof result !== 'object' || typeof result.ok !== 'boolean') return { ok: true, value: result ?? null }
      if (!result.ok) return { ok: false, error: { code: String(result.error?.code ?? 'ERROR'), message: String(result.error?.message ?? ''), details: result.error?.details ?? {} } }
      const attachments = (result.attachments ?? []).map((item) => ({ path: item.path, base64: Buffer.from(item.bytes ?? []).toString('base64') }))
      return { ok: true, value: result.value ?? null, ...(attachments.length ? { attachments } : {}) }
    } catch (error) {
      return { ok: false, error: { code: 'HANDLER_FAILED', message: errorText(error), details: {} } }
    } finally {
      controller.abort()
    }
  }

  /** emit 语义：同步依次调用，不等待；监听器出错只记日志。 */
  emitEvent(event, args) {
    for (const { listener } of [...(this.listeners.get(event) ?? [])]) {
      try {
        const result = listener(...args)
        if (result && typeof result.then === 'function') result.catch((error) => console.error(`[dsh-host] ${event} 监听器失败：${errorText(error)}`))
      } catch (error) {
        console.error(`[dsh-host] ${event} 监听器失败：${errorText(error)}`)
      }
    }
  }

  /** parallel 语义：并发等待全部监听器。 */
  async parallel(event, args) {
    await Promise.allSettled([...(this.listeners.get(event) ?? [])].map(({ listener }) => listener(...args)))
  }

  hasListeners(event) {
    return (this.listeners.get(event)?.length ?? 0) > 0
  }

  /**
   * 带 system-prompt/assemble 中间件的提示词。assembly 里按 dsh 的结构放进插件自己的片段，外加一段
   * `codex:instructions`（Codex 自己的系统提示词，经代理时才有）。插件的 waterfall 可以增删改片段：
   * - 没动 codex 那一段：返回 `{ text }`，追加到 Codex 的 instructions 后面；
   * - 删了或改了 codex 那一段：返回 `{ text, replace: true }`，整段 instructions 换成渲染结果。
   * 没有监听器时等同 assemblePrompt（只追加）。
   */
  async assembleInstructions(context = {}, codexInstructions = '') {
    if (!this.hasListeners('system-prompt/assemble')) return { text: this.assemblePrompt(context) }
    const own = (entries) => entries.map((entry) => ({ name: entry.name, order: entry.order, text: this.renderEntry(entry, context) })).filter((entry) => entry.text)
    const codex = codexInstructions ? { name: CODEX_INSTRUCTIONS_SECTION, order: -1_000_000, text: codexInstructions } : undefined
    const base = { sections: [...(codex ? [codex] : []), ...own(this.sections)], contexts: own(this.contexts), tools: [], variables: {} }
    let assembly
    try {
      assembly = await this.waterfall('system-prompt/assemble', [base, { agent: context.agent, signal: context.signal }], async () => base)
    } catch (error) {
      console.error(`[dsh-host] system-prompt/assemble 失败：${errorText(error)}`)
      assembly = base
    }
    const byOrder = (a, b) => (Number(a?.order) || 0) - (Number(b?.order) || 0)
    const sections = [...(Array.isArray(assembly?.sections) ? assembly.sections : [])].sort(byOrder)
    const contexts = [...(Array.isArray(assembly?.contexts) ? assembly.contexts : [])].sort(byOrder)
    const textOf = (entry) => (typeof entry?.text === 'string' ? entry.text.trim() : '')
    const kept = sections.find((entry) => entry?.name === CODEX_INSTRUCTIONS_SECTION)
    const replace = Boolean(codex) && (!kept || textOf(kept) !== codexInstructions.trim())
    const rest = [...sections, ...contexts].filter((entry) => replace || entry?.name !== CODEX_INSTRUCTIONS_SECTION)
    return { text: rest.map(textOf).filter(Boolean).join('\n\n'), replace }
  }

  /**
   * 拼出系统提示词：片段按 order、name 排序，之后接动态上下文；`{{变量}}` 用插件注册的变量求值，
   * 求不出的原样保留（dsh 会直接报错，这里宁可宽松，不能挡住模型请求）。
   */
  assemblePrompt(context = {}) {
    const byOrder = (a, b) => a.order - b.order || String(a.name).localeCompare(String(b.name))
    return [...[...this.sections].sort(byOrder), ...[...this.contexts].sort(byOrder)]
      .map((entry) => this.renderEntry(entry, context))
      .filter(Boolean)
      .join('\n\n')
  }

  /** 渲染一个片段：函数片段按上下文求值，`{{变量}}` 用插件注册的变量替换，求不出的原样保留。 */
  renderEntry(entry, context = {}) {
    try {
      const text = typeof entry.text === 'function' ? entry.text(context) : entry.text
      if (typeof text !== 'string' || !text.trim()) return ''
      if (entry.interpolate === false) return text.trim()
      return text.replace(/\{\{([A-Za-z_][\w.-]*)\}\}/g, (whole, name) => {
        const provider = this.variables.get(name)
        if (typeof provider !== 'function') return whole
        try {
          const value = provider(context)
          return typeof value === 'string' ? value : whole
        } catch {
          return whole
        }
      }).trim()
    } catch (error) {
      console.error(`[dsh-host] 提示词 ${entry.name} 生成失败：${errorText(error)}`)
      return ''
    }
  }

  /** 提示词是否全是静态文本（可以在分析时求好，直接写进配套拓展包，不必每次请求加载插件）。 */
  promptIsStatic() {
    return this.variables.size === 0
      && [...this.sections, ...this.contexts].every((entry) => typeof entry.text === 'string')
  }

  /** 按 Cordis waterfall 语义派发：每个监听器拿到 next()，最后一个的 next() 返回 fallback()。 */
  async waterfall(event, args, fallback) {
    const list = [...(this.listeners.get(event) ?? [])]
    const run = async (index) => {
      if (index >= list.length) return fallback()
      return list[index].listener(...args, () => run(index + 1))
    }
    return run(0)
  }

  /**
   * 流式 waterfall（llm/stream）：监听器 `(options, next) => AsyncIterable`，next() 同步返回下游的可迭代对象，
   * 链尾是 fallback()。某个监听器抛错时跳过它，用下游的流。
   */
  waterfallStream(event, options, fallback) {
    const list = [...(this.listeners.get(event) ?? [])]
    const run = (index) => {
      if (index >= list.length) return fallback()
      let downstream
      const next = () => {
        downstream ??= run(index + 1)
        return downstream
      }
      try {
        const result = list[index].listener(options, next)
        if (result && typeof result[Symbol.asyncIterator] === 'function') return result
        if (result && typeof result[Symbol.iterator] === 'function') return (async function* () { yield* result })()
        console.error(`[dsh-host] ${event} 监听器没有返回可迭代对象，已跳过`)
      } catch (error) {
        console.error(`[dsh-host] ${event} 监听器失败，已跳过：${errorText(error)}`)
      }
      return next()
    }
    return run(0)
  }

  /** 按 Cordis serial 语义派发：依次等待每个监听器，返回第一个非 undefined 的结果。 */
  async serial(event, args) {
    for (const { listener } of [...(this.listeners.get(event) ?? [])]) {
      const result = await listener(...args)
      if (result !== undefined) return result
    }
    return undefined
  }

  /** 工具名在 ctx.tools.restrict 之下是否可见。 */
  toolVisible(name) {
    for (const filter of this.toolRestrictions) {
      if (Array.isArray(filter.allow) && !filter.allow.includes(name)) return false
      if (Array.isArray(filter.deny) && filter.deny.includes(name)) return false
    }
    return true
  }

  /** 第一个给出拒绝理由的守卫；守卫抛错按「无意见」处理。 */
  guardReason(exec) {
    for (const guard of this.toolGuards) {
      try {
        const reason = guard(exec)
        if (typeof reason === 'string' && reason) return reason
      } catch (error) {
        console.error(`[dsh-host] 工具守卫失败：${errorText(error)}`)
      }
    }
    return undefined
  }

  /**
   * 执行本包注册的工具（MCP tools/call 与 ctx.tools.execute 共用）：守卫 → tools/execute waterfall → 工具本体，
   * 返回 dsh 的 ToolExecutionResult。tools/pre-execute 与 tools/post-execute 由 Codex 的钩子负责（工具名去掉 MCP 前缀后与这里一致）。
   */
  async executeTool(input = {}) {
    const name = String(input.name ?? '')
    const callId = String(input.callId ?? `codexpp-${Date.now()}`)
    const args = input.arguments ?? {}
    const controller = new AbortController()
    const signal = input.signal ?? controller.signal
    const deferred = []
    const exec = {
      callId,
      rootCallId: String(input.rootCallId ?? callId),
      name,
      arguments: args,
      signal,
      agent: input.agent,
      token: Symbol('codexpp.tool'),
      // dsh 把推迟的上下文接在这次工具结果之后：这里并进结果内容，模型紧跟着结果就能看到。
      deferContext(message) {
        if (message) deferred.push(message)
      },
      // 结束本轮要靠 dsh 自己的 agent 循环；Codex 的循环不在宿主手里，空操作。
      concludeTurn() {},
    }
    const failure = (message) => Object.freeze({ isError: true, error: { kind: 'error', message }, content: [{ type: 'text', text: message }] })
    const entry = this.tools.get(name)
    if (!entry || !this.toolVisible(name)) return failure(`未知工具：${name}`)
    if (entry.when && !(await this.toolActive(entry, input.agent, { provider: 'codex', model: input.model }, signal))) return failure(`工具 ${name} 现在不可用（when 条件不满足）`)
    const denied = this.guardReason(exec)
    if (denied) return failure(denied)
    const body = async () => {
      try {
        const timeout = entry.def.timeoutMs
        const value = await withTimeout(Promise.resolve(entry.def.execute(exec.arguments, exec)), timeout, `工具执行超过 ${timeout} ms`)
        // 图片块引用的附件存在这个插件的数据目录里：读出字节，作为真图片交给 Codex。
        const content = await renderValue(entry.def, exec.arguments, value, this.pluginAttachments?.get(entry.pluginKey))
        return Object.freeze({ isError: false, value: value ?? null, content })
      } catch (error) {
        controller.abort()
        return failure(errorText(error))
      }
    }
    let result
    try {
      result = this.hasListeners(TOOL_EXECUTE_EVENT) ? await this.waterfall(TOOL_EXECUTE_EVENT, [exec], body) : await body()
    } catch (error) {
      result = failure(errorText(error))
    }
    if (!result || typeof result !== 'object' || !Array.isArray(result.content)) result = failure('tools/execute 监听器没有返回工具结果')
    if (deferred.length) {
      const extra = deferred.map(messageText).map((text) => text.trim()).filter(Boolean).map((text) => ({ type: 'text', text }))
      if (extra.length) result = Object.freeze({ ...result, content: [...result.content, ...extra] })
    }
    this.emitEvent('tools/result', [exec, result])
    return result
  }

  /** 工具的 registerOptions.when 是否满足；判断出错视为可用（不挡住模型请求与工具调用）。 */
  async toolActive(entry, agent, modelSelection, signal) {
    if (!entry?.when) return true
    try {
      return Boolean(await entry.when({ agent, modelSelection: Object.freeze({ ...modelSelection }), signal: signal ?? new AbortController().signal }))
    } catch (error) {
      console.error(`[dsh-host] 工具 ${entry.def?.name} 的 when 判断失败，按可用处理：${errorText(error)}`)
      return true
    }
  }

  /** 按 Cordis bail 语义派发：同步依次调用，返回第一个非 undefined 的结果。 */
  bail(event, args) {
    for (const { listener } of [...(this.listeners.get(event) ?? [])]) {
      const result = listener(...args)
      if (result !== undefined) return result
    }
    return undefined
  }

  /** Cordis waterfall 的原样组合：不额外包 Promise，返回最外层监听器的返回值（插件间的同步 waterfall 也成立）。 */
  waterfallSync(event, args, fallback) {
    const list = [...(this.listeners.get(event) ?? [])]
    const run = (index) => (index >= list.length ? fallback() : list[index].listener(...args, () => run(index + 1)))
    return run(0)
  }

  /** 收集本行经 ctx.skills 注册的 skill：直接注册的定义，加上各 provider 列出并读取到的定义。 */
  async collectSkills(state) {
    const definitions = [...state.skillDefinitions]
    for (const provider of state.skillProviders) {
      try {
        const listed = await provider.list({})
        const candidates = Array.isArray(listed) ? listed : (listed?.candidates ?? [])
        for (const candidate of candidates) {
          const definition = await provider.get(candidate, {})
          if (definition && typeof definition.content === 'string') definitions.push({ ...candidate, ...definition })
        }
      } catch (error) {
        state.notes.push(`skill 提供方 ${provider?.name ?? ''} 读取失败：${errorText(error)}`)
      }
    }
    return definitions.filter((definition) => typeof definition?.name === 'string' && definition.name)
  }

  createContext(state) {
    const host = this
    const track = (dispose) => {
      state.disposers.push(dispose)
      return dispose
    }
    const note = (text) => {
      if (!state.notes.includes(text)) state.notes.push(text)
    }
    const mapTo = (target) => {
      if (state.mappings && !state.mappings.includes(target)) state.mappings.push(target)
    }
    const logger = makeLogger(state.key)
    // 注册变化事件（tools/change 等）：微任务里发，注册方的同步代码先跑完。
    const changed = (event) => queueMicrotask(() => host.emitEvent(event, []))
    const shared = this.sharedServices()
    host.schedules ??= createSchedule(host.dataDir, (event, args) => host.emitEvent(event, args))
    const storageDomain = createStorageDomain(state.dataDir, (event, args) => host.emitEvent(event, args))
    const pluginServices = {
      credentials: createCredentials(state.config, { dataDir: state.dataDir, emit: (event, args) => host.emitEvent(event, args) }),
      storageDomain,
      storage: createStorage(storageDomain),
      // settings 落在插件自己的数据目录（初始值取该行配置）；loader 报告宿主里加载过的行；sessionPersistence 不暴露 Codex 的会话日志。
      settings: createSettings({ dataDir: state.dataDir, rowId: state.rowId, config: state.config, emit: (event, args) => host.emitEvent(event, args) }),
      loader: createLoader({ entries: () => (host.loaderEntries ? host.loaderEntries.values() : []) }),
      sessionPersistence: createSessionPersistence(),
      spillStore: createSpillStore(state.dataDir),
      attachments: (host.pluginAttachments ??= new Map()).set(state.key, createAttachments(state.dataDir)).get(state.key),
      // 网页路由（dsh 的 webServer）：挂到配套拓展包后台的 /ext/<配套拓展>/<路径>，经 Codex++ 本地代理接收。
      webServer: {
        register(route) {
          if (!route || typeof route.handler !== 'function') return () => {}
          mapTo(WEBHOOK_TARGET)
          const path = `/${String(route.path ?? '').replace(/^\/+/, '')}`
          host.webRoutes.set(path, route)
          return track(() => {
            if (host.webRoutes.get(path) === route) host.webRoutes.delete(path)
          })
        },
        registerFallback(handler) {
          if (typeof handler !== 'function') return () => {}
          mapTo(WEBHOOK_TARGET)
          host.webFallback = handler
          return track(() => {
            if (host.webFallback === handler) host.webFallback = undefined
          })
        },
        // WebSocket 等协议升级：Codex++ 本地代理把 /ext/<配套拓展>/<路径> 的升级请求原样转到常驻进程里的 Node http 服务，
        // 处理器拿到真的 IncomingMessage 与 socket（同 dsh 的 node:http upgrade 事件）。
        registerUpgrade(route) {
          if (!route || typeof route.handler !== 'function') return () => {}
          const path = `/${String(route.path ?? '').replace(/^\/+/, '')}`
          if (host.webUpgrades.has(path)) throw new Error(`webserver: duplicate upgrade route "${path}"`)
          mapTo(WEBHOOK_TARGET)
          host.webUpgrades.set(path, route)
          return track(() => {
            if (host.webUpgrades.get(path) === route) host.webUpgrades.delete(path)
          })
        },
        tapIndex: () => () => {},
        applyIndexTaps: (html) => html,
        collectIndexInjections: () => [],
        renderIndex: (html) => html,
      },
      // MCP 资源（dsh 的 mcpResources）：经 Codex++ 托管的 dsh 插件 MCP 的 resources/* 暴露给 Codex。
      mcpResources: {
        register(server, provider) {
          if (!provider || typeof provider.request !== 'function') return () => {}
          const key = String(server)
          host.mcpResources.set(key, provider)
          return track(() => {
            if (host.mcpResources.get(key) === provider) host.mcpResources.delete(key)
          })
        },
      },
      // dsh 的 ctx.typert：登记类型化 RPC 方法（生成的 TYPERT 清单或 descriptors），前端 ctx.remote 经 connection.rpc 调过来。
      typert: (() => {
        const register = (manifest) => {
          const registered = host.registerTypert(manifest, { isolate: state.isolate })
          if (Array.isArray(manifest?.invocations) ? manifest.invocations.length : manifest?.descriptors?.length) mapTo(CONNECTION_TARGET)
          // 记下已登记的 包/面（cost-meter 等用 ctx.typert.getPackage 判断清单是不是已经登记过）。
          const faceKey = manifest?.package && manifest?.face ? `${manifest.package}/${manifest.face}` : undefined
          if (faceKey) (host.typertFaces ??= new Set()).add(faceKey)
          const dispose = async () => {
            if (faceKey) host.typertFaces?.delete(faceKey)
            return registered()
          }
          track(dispose)
          return dispose
        }
        const subscribe = (listener) => {
          host.typertListeners.add(listener)
          const dispose = async () => {
            host.typertListeners.delete(listener)
          }
          track(dispose)
          return dispose
        }
        const descriptors = {
          register,
          get: (endpoint) => host.typertInvocations.get(String(endpoint))?.descriptor,
          hasSeen: (endpoint) => host.typertSeen.has(String(endpoint)),
          list: () => [...host.typertInvocations.values()].map((entry) => entry.descriptor),
          subscribe,
        }
        const keyed = (map) => ({
          register(key, value) {
            map.set(String(key), value)
            const dispose = async () => {
              if (map.get(String(key)) === value) map.delete(String(key))
            }
            track(dispose)
            return dispose
          },
          get: (key) => map.get(String(key)),
          entries: () => [...map.entries()].map(([kind, value]) => ({ kind, ...value })),
          subscribe,
        })
        const lookups = keyed(host.typertLookups)
        const contexts = keyed(host.typertContexts)
        const schema = (key) => {
          const entry = host.typertSchemas?.get(String(key))
          if (!entry) return undefined
          entry.value ??= entry.create()
          return entry.value
        }
        return {
          register,
          // dsh 的 typert.getPackage(包, 面)：这个包的这一面清单登记过就返回 { package, face }，没有返回 undefined。
          getPackage: (name, face) => (host.typertFaces?.has(`${name}/${face}`) ? { package: String(name), face: String(face) } : undefined),
          local: descriptors,
          remotes: descriptors,
          lookups: { ...lookups, registerHost: lookups.register, configureHost: lookups.register },
          contexts: { ...contexts, registerHost: contexts.register, registerClient: () => async () => {} },
          resources: keyed(host.typertResources),
          get: schema,
          resolve(key) {
            const found = schema(key)
            if (!found) throw new Error(`typert schema ${key} 不存在`)
            return found
          },
          list: () => [...(host.typertSchemas?.keys() ?? [])],
        }
      })(),
      schedule: host.schedules,
      webhookRuntime: {
        ...host.webhooks,
        register(rule) {
          mapTo(WEBHOOK_TARGET)
          return track(host.webhooks.register(rule))
        },
      },
    }
    const timer = {
      setTimeout(callback, ms, ...args) {
        const handle = setTimeout(callback, ms, ...args)
        return track(() => clearTimeout(handle))
      },
      setInterval(callback, ms, ...args) {
        const handle = setInterval(callback, ms, ...args)
        return track(() => clearInterval(handle))
      },
      sleep: (ms) => new Promise((done) => setTimeout(done, ms)),
    }
    // @cordisjs/plugin-timer 的写法（dsh 插件常用）：timeout / interval 同上；throttle / debounce 返回包装过的函数，
    // 带 dispose()，插件卸载时一并撤销还没触发的调用。
    timer.timeout = timer.setTimeout
    timer.interval = timer.setInterval
    const deferred = (callback, delay, leading) => {
      let handle
      let last = 0
      let pendingArgs
      const flush = () => {
        handle = undefined
        last = Date.now()
        const args = pendingArgs
        pendingArgs = undefined
        if (args) callback(...args)
      }
      const wrapped = (...args) => {
        if (leading) {
          const wait = delay - (Date.now() - last)
          if (wait <= 0 && !handle) {
            last = Date.now()
            callback(...args)
            return
          }
          pendingArgs = args
          handle ??= setTimeout(flush, Math.max(wait, 0))
          return
        }
        pendingArgs = args
        if (handle) clearTimeout(handle)
        handle = setTimeout(flush, delay)
      }
      wrapped.dispose = track(() => {
        if (handle) clearTimeout(handle)
        handle = undefined
        pendingArgs = undefined
      })
      return wrapped
    }
    timer.throttle = (callback, delay) => deferred(callback, delay, true)
    timer.debounce = (callback, delay) => deferred(callback, delay, false)
    const base = {
      name: state.pluginName,
      config: state.config,
      // Cordis 的 fiber：插件常读 ctx.fiber.entry.options.id 拿自己这一行的 id（当命名空间、存储键）。
      fiber: host.registerLoaderEntry(state),
      logger,
      timer,
      setTimeout: timer.setTimeout,
      setInterval: timer.setInterval,
      sleep: timer.sleep,
      throttle: timer.throttle,
      debounce: timer.debounce,
      tools: {
        register(definition, options = {}) {
          if (!definition || typeof definition.name !== 'string' || typeof definition.execute !== 'function') {
            throw new Error('tools.register 需要 defineTool() 返回的工具定义')
          }
          if (host.tools.has(definition.name)) {
            throw new Error(`tool "${definition.name}" is already registered`)
          }
          // registerOptions.when：经 Codex++ 代理的每一步请求里按 agent 判断（不满足就从请求的 tools 去掉），MCP 调用时也判断一次。
          const when = options && typeof options.when === 'function' ? options.when : undefined
          if (when) {
            mapTo('gateway:step')
            note('工具的 when 条件在经 Codex++ 代理的请求里按 agent 判断（不满足就不给模型看）；MCP 进程里没有 agent，按无 agent 判断一次，判断出错视为可用')
          }
          host.tools.set(definition.name, { def: definition, pluginKey: state.key, when })
          state.tools.push(definition.name)
          changed('tools/change')
          return track(() => {
            if (host.tools.get(definition.name)?.def === definition) host.tools.delete(definition.name)
            changed('tools/change')
          })
        },
        get(name) {
          return host.tools.get(name)?.def
        },
        has(name) {
          return host.tools.has(name)
        },
        /** 本包注册的工具（Codex 的原生工具不在这里）：只给 name / description / parameters。 */
        schemas() {
          return [...host.tools.values()]
            .filter(({ def }) => host.toolVisible(def.name))
            .map(({ def }) => ({ name: def.name, description: def.description ?? '', parameters: toolParameters(def) }))
        },
        /** 执行本包注册的工具：守卫 → tools/execute → 工具本体，之后发 tools/result。 */
        execute(input) {
          return host.executeTool(input)
        },
        executionMode: () => ({ kind: 'parallel' }),
        guard(guard) {
          if (typeof guard !== 'function') return () => {}
          host.toolGuards.push(guard)
          mapTo('PreToolUse')
          return track(() => {
            host.toolGuards = host.toolGuards.filter((item) => item !== guard)
          })
        },
        restrict(filter) {
          if (!filter || typeof filter !== 'object') return () => {}
          host.toolRestrictions.push(filter)
          mapTo('gateway:step')
          changed('tools/change')
          return track(() => {
            host.toolRestrictions = host.toolRestrictions.filter((item) => item !== filter)
            changed('tools/change')
          })
        },
        presentAs() {
          note('tools.presentAs 是 dsh 的工具展示方式，Codex 里没有对应，忽略')
          return () => {}
        },
      },
      systemPrompt: {
        section(section) {
          const entry = {
            pluginKey: state.key,
            name: section?.name,
            order: Number(section?.order) || 0,
            text: section?.text,
            interpolate: section?.interpolate,
          }
          host.sections.push(entry)
          mapTo(INSTRUCTIONS_TARGET)
          changed('system-prompt/change')
          if (section?.complete) note('complete 片段在 Codex 里只追加到 instructions，不会替换 Codex 自己的系统提示词')
          return track(() => {
            host.sections = host.sections.filter((item) => item !== entry)
            changed('system-prompt/change')
          })
        },
        context(context) {
          const entry = {
            pluginKey: state.key,
            name: context?.name,
            order: Number(context?.order) || 0,
            text: context?.text,
            interpolate: context?.interpolate,
          }
          host.contexts.push(entry)
          mapTo(INSTRUCTIONS_TARGET)
          changed('system-prompt/change')
          note('systemPrompt.context 在 Codex 里并入 instructions，不会作为用户侧上下文快照逐回合刷新')
          return track(() => {
            host.contexts = host.contexts.filter((item) => item !== entry)
            changed('system-prompt/change')
          })
        },
        variable(name, provider) {
          if (typeof name === 'string' && typeof provider === 'function') host.variables.set(name, provider)
          return track(() => {
            if (host.variables.get(name) === provider) host.variables.delete(name)
          })
        },
        tools() {
          return () => {}
        },
        suppressRuntimeContext() {
          return () => {}
        },
        getSectionOrder: () => 0,
        getContextOrder: () => 0,
      },
      skills: {
        register(skill) {
          state.skillDefinitions.push(skill)
          changed('skills/change')
          return track(() => {
            state.skillDefinitions = state.skillDefinitions.filter((item) => item !== skill)
            changed('skills/change')
          })
        },
        registerProvider(create) {
          const controller = new AbortController()
          const provider = create({ signal: controller.signal, invalidate() {} })
          state.skillProviders.push(provider)
          return track(() => {
            controller.abort()
            state.skillProviders = state.skillProviders.filter((item) => item !== provider)
          })
        },
        async list() {
          return []
        },
        async get() {
          return undefined
        },
      },
      ...shared,
      ...pluginServices,
      connection: {
        fetch: {
          register(route) {
            const path = String(route?.path ?? '')
            if (!path.startsWith('/') || typeof route?.fetch !== 'function') throw new TypeError('connection.fetch.register 需要以 / 开头的 path 与 fetch(request)')
            if (host.routes.has(path)) throw new Error(`路由 ${path} 已注册`)
            mapTo(CONNECTION_TARGET)
            const entry = { methods: (route.methods ?? []).map((item) => String(item).toUpperCase()), fetch: route.fetch }
            host.routes.set(path, entry)
            return track(() => {
              if (host.routes.get(path) === entry) host.routes.delete(path)
            })
          },
        },
        // 前端半边的 ctx.connection.rpc.call(channel, endpoint, payload) 经配套拓展包的后台转到这里。
        rpc: {
          handle(channel, handler) {
            if (typeof handler !== 'function') return async () => {}
            mapTo(CONNECTION_TARGET)
            const key = String(channel)
            host.rpcHandlers.set(key, handler)
            const dispose = track(() => {
              if (host.rpcHandlers.get(key) === handler) host.rpcHandlers.delete(key)
            })
            return async () => dispose()
          },
          intercept(channel, matches, handler) {
            if (typeof handler !== 'function' || typeof matches !== 'function') return async () => {}
            mapTo(CONNECTION_TARGET)
            const entry = { channel: String(channel), matches, handler }
            host.rpcInterceptors.push(entry)
            const dispose = track(() => {
              host.rpcInterceptors = host.rpcInterceptors.filter((item) => item !== entry)
            })
            return async () => dispose()
          },
        },
      },
      commands: {
        ...shared.commands,
        register(definition) {
          mapTo(COMMANDS_TARGET)
          state.commands?.push({ name: definition?.name, description: definition?.description ?? '', hint: definition?.input?.hint ?? '' })
          changed('commands/change')
          const dispose = shared.commands.register(definition)
          return track(() => {
            dispose?.()
            changed('commands/change')
          })
        },
        // dsh 网页客户端把上传的文件作为「回执」交给命令；Codex 的斜杠命令不带附件，登记了也不会被调用。
        registerFileReceiptResolver() {
          note('commands.registerFileReceiptResolver 已登记但不会被调用：Codex 的斜杠命令不带上传文件')
          return track(() => {})
        },
      },
      sessionProjections: {
        ...shared.sessionProjections,
        register(definition) {
          mapTo(MIRROR_TARGET)
          return track(shared.sessionProjections.register(definition))
        },
        onChanged(listener) {
          mapTo(MIRROR_TARGET)
          return track(shared.sessionProjections.onChanged(listener))
        },
      },
      /** 可选依赖的激活写法：服务都在就立刻以子 ctx 调用（宿主替身不会中途出现或消失；插件提供的服务先于消费方加载）。 */
      inject(names, callback) {
        const list = Array.isArray(names) ? names : Object.keys(names ?? {})
        const missing = list.filter((name) => !host.serviceAvailable(name, state.isolate))
        if (missing.length > 0) {
          note(`ctx.inject 里的 ${missing.join(', ')} 在 Codex++ 里没有，这一段不会生效`)
          return () => {}
        }
        if (list.some((name) => MIRROR_SERVICES.has(name))) mapTo(MIRROR_TARGET)
        if (list.includes('schedule')) mapTo(SCHEDULE_TARGET)
        const result = callback(proxy)
        if (typeof result === 'function') track(result)
        else if (result && typeof result.then === 'function') state.pending.push(result.then((value) => typeof value === 'function' && track(value)))
        return () => {}
      },
      effect(callback) {
        let dispose
        const result = callback()
        if (typeof result === 'function') dispose = result
        else if (result && typeof result.then === 'function') {
          state.pending.push(result.then((value) => {
            if (typeof value === 'function') track(value)
          }))
        }
        return track(() => dispose?.())
      },
      on(event, listener, options) {
        if (STEP_EVENTS.has(event)) mapTo('gateway:step')
        if (EVENT_HOOKS[event]) mapTo(EVENT_HOOKS[event])
        else if (GATEWAY_EVENTS[event]) mapTo(`gateway:${GATEWAY_EVENTS[event]}`)
        else if (MIRROR_EVENTS.has(event)) mapTo(MIRROR_TARGET)
        else if (event === TOOL_EXECUTE_EVENT) {
          // 代为执行 Codex 的工具：在 PreToolUse 钩子里跑，结果经网关换进下一次请求。
          mapTo('PreToolUse')
          mapTo('gateway:step')
        } else if (AGENT_LIFECYCLE_EVENTS.has(event) || event.startsWith('agent/inbox/')) {
          // 由常驻进程里的会话镜像与 agent 替身派发（退出时落盘、cancel、排消息……）。
          mapTo(MIRROR_TARGET)
        } else if (!HOST_EMITTED_EVENTS.has(event) && event !== 'ready' && event !== 'dispose' && !String(event).startsWith('internal/')) {
          // 插件之间的自定义事件：同一个包里直接派发；别的包的插件 emit 时经常驻进程之间的本地 socket 转过来。
          if (state.listens && !state.listens.includes(event)) state.listens.push(String(event))
          mapTo(EVENTS_TARGET)
        }
        const list = host.listeners.get(event) ?? []
        const entry = { listener, state }
        // Cordis 的 prepend：排到最前（waterfall 里最外层，先调 next() 就能看到其余监听器的结果）。
        if (options === true || options?.prepend) list.unshift(entry)
        else list.push(entry)
        host.listeners.set(event, list)
        return track(() => {
          const current = host.listeners.get(event) ?? []
          host.listeners.set(event, current.filter((item) => item !== entry))
        })
      },
      once(event, listener) {
        const dispose = base.on(event, (...args) => {
          dispose()
          return listener(...args)
        })
        return dispose
      },
      // 插件之间的事件：同一个宿主进程（同一个包）里的监听器共享一张表，按 Cordis 的五种语义派发；
      // 常驻进程里还经 host.bus 转给别的包（各自的常驻进程）里听这个事件的插件，语义见 bridge.mjs 的 createEventBus。
      // 第一个参数不是字符串时按 Cordis 的 thisArg 写法跳过。
      off(event, listener) {
        const current = host.listeners.get(event) ?? []
        host.listeners.set(event, current.filter((item) => item.listener !== listener))
      },
      emit(...args) {
        const [event, rest] = eventArgs(args)
        host.emitEvent(event, rest)
        host.bus?.emit(event, rest)
      },
      async parallel(...args) {
        const [event, rest] = eventArgs(args)
        await Promise.all([host.parallel(event, rest), host.bus?.parallel(event, rest)])
      },
      async serial(...args) {
        const [event, rest] = eventArgs(args)
        const local = await host.serial(event, rest)
        if (local !== undefined || !host.bus?.listened(event)) return local
        return host.bus.serial(event, rest)
      },
      /** 同步的 bail：本包没人给出结果、别的包有人听时，返回别的包的结果的 Promise（跨进程做不到同步）。 */
      bail(...args) {
        const [event, rest] = eventArgs(args)
        const local = host.bail(event, rest)
        if (local !== undefined || !host.bus?.listened(event)) return local
        return host.bus.serial(event, rest, 'bail')
      },
      /** waterfall：本包的监听器在前，链尾依次交给别的包的监听器（每个都能调 next 回到链上），最后才是调用方的 next。 */
      waterfall(...args) {
        const [event, rest] = eventArgs(args)
        const next = typeof rest[rest.length - 1] === 'function' ? rest.pop() : () => undefined
        const tail = host.bus?.listened(event) ? () => host.bus.waterfall(event, rest, next) : next
        return host.waterfallSync(event, rest, tail)
      },
      get(name) {
        const provided = host.findService(String(name), state.isolate)
        if (provided !== undefined) return provided
        if (!SUPPORTED_SERVICES.has(name)) return undefined
        if (MIRROR_SERVICES.has(name)) mapTo(MIRROR_TARGET)
        if (name === 'schedule') mapTo(SCHEDULE_TARGET)
        return proxy[name]
      },
      /** Cordis 的 ctx.set(name, value)：把一个值作为服务提供给同一个宿主里的其它插件（随本插件卸载撤销）。 */
      set(name, value) {
        if (typeof name !== 'string' || !name) throw new TypeError('ctx.set 需要服务名')
        state.provides?.push(name)
        return track(host.provideService(name, value, state.isolate))
      },
      /** Cordis 的 ctx.provide(name, value?)：声明服务；给了值就同 ctx.set。 */
      provide(name, value) {
        if (value === undefined) return () => {}
        return base.set(name, value)
      },
      /**
       * Cordis 的 ctx.isolate(name, label?)：返回一个子 ctx，其中 name 这个服务落在私有作用域（给了 label 就是同名标签共享的作用域），
       * 在子 ctx 里提供或 inject 的 name 与外面互不可见。
       */
      isolate(name, label) {
        const key = typeof label === 'string' && label ? `label:${label}` : `scope:${++host.isolateSeq}`
        return host.createContext({ ...state, isolate: { ...(state.isolate ?? {}), [String(name)]: key } })
      },
      /** 替身 Service 的构造函数经这里登记服务（`super(ctx, name)`）。 */
      [PROVIDE](name, instance) {
        state.provides?.push(name)
        track(host.provideService(name, instance, state.isolate))
      },
      async plugin(plugin, config) {
        return host.applyPlugin(plugin, config, state)
      },
    }
    const proxy = new Proxy(base, {
      get(target, key) {
        // 插件提供的服务可以顶替宿主替身（同 Cordis：后提供的生效），但不顶替 ctx 自己的方法与核心能力。
        if (typeof key === 'string' && !CORE_CONTEXT_KEYS.has(key) && (!(key in target) || SUPPORTED_SERVICES.has(key))) {
          const provided = host.findService(key, state.isolate)
          if (provided !== undefined) return provided
        }
        if (key in target) return target[key]
        // dsh 的 this.ctx.invocation：正在进行的 typert 调用（不在调用里时为 undefined）。
        if (key === 'invocation') return host.invocations.getStore()
        if (typeof key === 'symbol' || key === 'then' || key === 'toJSON') return undefined
        note(`访问了 ctx.${String(key)}，Codex++ 不提供这个服务`)
        return undefined
      },
    })
    return proxy
  }

  async applyPlugin(plugin, config, parentState) {
    const serviceClass = isClass(plugin) ? plugin : undefined
    const normalized = serviceClass
      ? { name: serviceClass.name, inject: serviceClass.inject, Config: serviceClass.Config }
      : typeof plugin === 'function'
        ? { name: plugin.name, inject: plugin.inject, Config: plugin.Config, apply: plugin }
        : plugin
    if (!serviceClass && (!normalized || typeof normalized.apply !== 'function')) throw new Error('ctx.plugin 需要带 apply 的插件或 Service 子类')
    const inject = normalizeInject(normalized.inject)
    const missing = inject.required.filter((name) => !this.serviceAvailable(name, parentState.isolate))
    if (missing.length > 0) throw new Error(`子插件依赖 Codex++ 不提供的服务：${missing.join(', ')}`)
    const resolvedConfig = typeof normalized.Config === 'function' ? normalized.Config(config ?? {}) : config
    const state = { ...parentState, config: resolvedConfig }
    const ctx = this.createContext(state)
    if (serviceClass) {
      await this.instantiateService(serviceClass, ctx, resolvedConfig, (dispose) => state.disposers.push(dispose))
      return undefined
    }
    await normalized.apply(ctx, resolvedConfig)
    return undefined
  }

  /**
   * 加载一行插件，返回该行的兼容性记录。`options.defer` 为真时，必需的服务还没人提供就返回 `{ deferred }`，
   * 由 loadPackage 在别的行加载后重试。
   */
  async loadRow(row, root, pkg, packageKey, options = {}) {
    const record = {
      id: row.id,
      module: row.name,
      enabled: !row.disabled,
      status: 'compatible',
      kind: 'plugin',
      reasons: [],
      notes: [],
      tools: [],
      mappings: [],
      commands: [],
    }
    if (row.name === MCP_CLIENT_MODULE) {
      const converted = toMcpServer(row)
      record.kind = 'mcp'
      if (converted.error) {
        record.status = 'error'
        record.reasons.push(converted.error)
      } else {
        record.mcp = converted
        record.notes.push('MCP 配置可导入 Codex；reconnect、toolCallTimeoutMs 等 dsh 专属字段会被丢弃')
      }
      return record
    }
    if (row.disabled) {
      record.status = 'disabled'
      record.reasons.push('bundle 默认停用这一行')
      return record
    }
    const resolved = resolveRowModule(row, root, pkg)
    // 行引用的依赖包里可能放着现成的 SKILL.md，即使它的代码加载不了也要扫到。
    if (resolved.packageDir) this.dependencyDirs?.add(resolved.packageDir)
    if (resolved.error) {
      record.status = resolved.builtin ? 'incompatible' : 'error'
      record.reasons.push(resolved.error)
      return record
    }
    let module
    try {
      module = await import(pathToFileURL(resolved.path).href)
    } catch (error) {
      // 引用了宿主才有的 dsh 运行时包（peer 依赖）：这是不兼容，不是包坏了。
      const runtimePackage = error?.code === 'ERR_MODULE_NOT_FOUND'
        ? /Cannot find package '(@deepseek-ai\/(?:dsh-[^'/]+|cordis[^'/]*))/.exec(errorText(error))?.[1]
        : undefined
      // 替身只提供运行时包的一小部分导出，用到别的就同样算不兼容。
      const shimGap = /The requested module '(@deepseek-ai\/[^']+)' does not provide an export named '([^']+)'/.exec(errorText(error))
      if (runtimePackage) {
        record.status = 'incompatible'
        record.reasons.push(`依赖 dsh 运行时包 ${runtimePackage}，Codex++ 不提供（属于 dsh 的 agent 循环 / 会话 / 模型等运行时）`)
      } else if (shimGap && SHIMS[shimGap[1]]) {
        record.status = 'incompatible'
        record.reasons.push(`用到 dsh 运行时包 ${shimGap[1]} 的 ${shimGap[2]}，Codex++ 只提供它的少量替身导出`)
      } else {
        record.status = 'error'
        record.reasons.push(`加载失败：${errorText(error)}`)
      }
      return record
    }
    const plugin = pluginFromModule(module)
    if (!plugin) {
      record.status = 'error'
      record.reasons.push('模块没有导出 apply，不是 dsh 插件')
      return record
    }
    if (plugin.serviceClass) record.kind = 'service'
    const inject = normalizeInject(plugin.inject)
    record.inject = inject
    const missing = inject.required.filter((name) => !this.serviceAvailable(name, row.isolate))
    if (missing.length > 0) {
      // 依赖可能由同一个包里后面的行（Service 子类插件）提供：先跳过，等别的行加载完再试（同 Cordis 等服务就绪再 apply）。
      if (options.defer) return { deferred: missing }
      record.status = 'incompatible'
      record.reasons.push(`依赖 Codex++ 不提供、包里也没有插件提供的服务：${missing.join(', ')}（属于 dsh 的 agent 循环 / 会话 / 模型等运行时）`)
      return record
    }
    const optionalMissing = inject.optional.filter((name) => !this.serviceAvailable(name, row.isolate))
    if (optionalMissing.length > 0) record.notes.push(`可选服务 ${optionalMissing.join(', ')} 不可用`)
    // 依赖会话 / agent / 投影的插件要会话镜像：配套拓展包会挂上把 Codex 钩子翻成 dsh 会话日志的那几个钩子。
    // 只声明 sessions 的插件多半只是 get 一下会话读 cwd，不值得为它在每次工具调用上起钩子；用到 agent / 投影才挂。
    if ([...inject.required, ...inject.optional].some((name) => name === 'agents' || name === 'sessionProjections')) record.mappings.push(MIRROR_TARGET)
    if ([...inject.required, ...inject.optional].includes('schedule')) record.mappings.push(SCHEDULE_TARGET)
    // 计划模式靠 PreToolUse 拒绝写类工具，目标靠 Stop 要求继续；两者都要按会话认出 agent（镜像）。
    if ([...inject.required, ...inject.optional].includes('planMode')) record.mappings.push('PreToolUse', MIRROR_TARGET)
    if ([...inject.required, ...inject.optional].includes('goals')) record.mappings.push('Stop', MIRROR_TARGET)
    const mirrorNotes = {
      sessions: 'ctx.sessions 是 Codex 会话的镜像（由 Codex 钩子事件生成）：没有模型的流式内容与推理，Codex 自己的会话日志请用 sessionQuery',
      agents: 'ctx.agents 里的 agent 是 Codex 会话的替身：steer / inject 在钩子里翻成补充上下文或要求继续，钩子之外经 Codex++ 往会话里插话',
      sessionProjections: 'sessionProjections 在会话镜像上折叠；客户端 useProjection 视图不提供',
      llm: 'ctx.llm.stream 经 Codex++ 协议代理边收边吐，工具调用与用量在回复结束时给出；registerAdapter 注册的供应商只给宿主进程里的 ctx.llm 用，Codex 自己的请求不经过它',
      fs: 'ctx.fs 是本机文件系统，不做沙箱与「先读后写」策略',
      shell: 'ctx.shell 用本机 shell 执行，不做沙箱',
      schedule: 'ctx.schedule 到点经 Codex++ 往 Codex 会话里发起一轮（每分钟检查一次，需要 Codex++ 在运行）',
      webhookRuntime: 'webhook 规则经 Codex++ 本地代理的 /ext/<配套拓展>/webhook/<kind> 接收，不校验签名',
      jobs: 'ctx.jobs 的任务在启动它的进程里跑，别的进程（MCP / 常驻进程）能看到、等待、停止它；任务变化事件只在本进程里发',
      workspaceRegistry: 'ctx.workspaceRegistry 由 Codex 会话的工作目录归出来，只读',
      subprocess: 'ctx.subprocess 不提供 PTY 终端',
      web: 'ctx.web 只有 fetch，没有联网搜索',
      userQuestions: 'ctx.userQuestions 在 Codex 页面里弹框提问（一题一问）',
      planMode: 'ctx.planMode 开着时，Codex 的写类工具（apply_patch 等）在 PreToolUse 里被拒绝；需要信任 Codex++ 拓展钩子',
      goals: 'ctx.goals 的目标在 Codex 每轮要停下时要求继续推进，连续 8 次后自动暂停',
      subagents: 'ctx.subagents 新建一个 Codex 会话跑子任务并等它结束，不是 Codex 原生的子 agent 树',
    }
    for (const name of [...inject.required, ...inject.optional]) if (mirrorNotes[name]) record.notes.push(mirrorNotes[name])
    // 配置里的 !!js：在 apply 前按 Cordis 的写法求值（能用本包已提供的服务）；求不出的保留原样并说明。
    let config = resolveTaggedConfig(row.config ?? {}, { ctx: this.expressionContext(row) }, (text) => record.notes.push(`配置里的${text}`))
    // 配置 schema 给管理器生成表单：先记下来，配置不合法（比如缺必填项）时用户才有地方填。
    if (plugin.Config && typeof plugin.Config.toJSON === 'function') {
      try {
        record.configSchema = JSON.parse(JSON.stringify(plugin.Config.toJSON()))
      } catch {}
    }
    if (typeof plugin.Config === 'function') {
      try {
        config = plugin.Config(config)
      } catch (error) {
        record.status = 'error'
        record.reasons.push(`配置不合法：${errorText(error)}`)
        return record
      }
    }
    const state = {
      key: `${packageKey}/${row.id}`,
      rowId: row.id,
      pluginName: plugin.name ?? row.id,
      // 这一行引用的模块名（通常是包名）：loader.entries() 的 options.name 用它，插件按包名认自己那一行。
      moduleName: row.name,
      config,
      tools: record.tools,
      notes: record.notes,
      mappings: record.mappings,
      disposers: this.disposers,
      pending: [],
      skillDefinitions: [],
      skillProviders: [],
      commands: record.commands,
      // 插件自己的数据（storageDomain 的 JSON 文件）放这里；分析进程不给，只在内存里。
      dataDir: this.dataDir ? join(this.dataDir, row.id.replace(/[^\w.-]/g, '_')) : undefined,
      // 行上的 isolate 声明（Cordis 作用域服务）与本行提供的服务名。
      isolate: row.isolate ?? {},
      provides: [],
      // 本行监听的插件间自定义事件（别的包 emit 时转过来）。
      listens: [],
    }
    try {
      const ctx = this.createContext(state)
      const running = plugin.serviceClass
        ? this.instantiateService(plugin.serviceClass, ctx, config, (dispose) => state.disposers.push(dispose))
        : Promise.resolve(plugin.apply(ctx, config))
      await withTimeout(running, APPLY_TIMEOUT_MS, `${plugin.serviceClass ? '服务启动' : 'apply'} 超过 ${APPLY_TIMEOUT_MS / 1000} 秒未完成`)
      await Promise.all(state.pending)
    } catch (error) {
      record.status = 'error'
      record.reasons.push(`${plugin.serviceClass ? '服务启动' : 'apply'} 失败：${errorText(error)}`)
      return record
    }
    if (state.provides.length) record.provides = [...new Set(state.provides)]
    if (state.listens.length) record.listens = [...new Set(state.listens)]
    // 包导出了 ./typert（生成的类型化 RPC 清单）：同 dsh 的 typert-loader 自动登记，前端 ctx.remote 就能调。
    try {
      if (await this.registerTypertExport(resolved.packageDir ?? root, { isolate: state.isolate }, (dispose) => state.disposers.push(dispose))) {
        if (!record.mappings.includes(CONNECTION_TARGET)) record.mappings.push(CONNECTION_TARGET)
      }
    } catch (error) {
      record.notes.push(`typert 清单登记失败：${errorText(error)}`)
    }
    if (plugin.serviceClass && !record.provides?.length) {
      record.notes.push(`${plugin.name || '该模块'} 是 Service 子类，但构造时没有用 super(ctx, 名字) 登记服务名，别的插件拿不到它`)
    }
    record.tools = record.tools.map((name) => {
      const def = this.tools.get(name)?.def
      return { name, description: def?.description ?? '' }
    })
    record.skills = []
    for (const definition of this.skipSkills ? [] : await this.collectSkills(state)) {
      const skill = { name: definition.name, description: String(definition.description ?? ''), resourceDir: definition.resourceBase?.path }
      if (this.skillsOut) {
        try {
          skill.directory = writeRuntimeSkill(this.skillsOut, definition)
        } catch (error) {
          record.notes.push(`skill ${definition.name} 写出失败：${errorText(error)}`)
          continue
        }
      }
      record.skills.push(skill)
    }
    if (record.notes.length > 0) record.status = 'partial'
    if (record.provides?.length) {
      // 说明它提供了什么，不算部分兼容。
      const scoped = record.provides.filter((name) => state.isolate[name])
      record.notes.push(`提供服务 ${record.provides.join(', ')}：同一个包里的插件可以 inject${scoped.length ? `（${scoped.join(', ')} 按 isolate 只在声明的作用域里可见）` : ''}；别的包拿不到（服务对象不能跨进程）`)
    }
    if (record.kind === 'service') {
      // Service 子类插件只提供服务，没有工具是正常的。
    } else if (record.skills.length > 0 && record.tools.length === 0 && record.mappings.length === 0) {
      record.kind = 'skills'
      if (record.status === 'compatible') record.status = 'skills'
    } else if (record.tools.length === 0 && record.mappings.length === 0 && record.status !== 'partial' && !record.provides?.length) {
      record.notes.push('加载成功，但没有注册任何工具')
    }
    return record
  }

  async loadPackage(root, packageKey, options = {}) {
    this.dataDir = typeof options.dataDir === 'string' && options.dataDir ? options.dataDir : undefined
    // inspect 时把代码注册的 skill 写到这里；serve 不写。
    this.skillsOut = typeof options.skillsOut === 'string' && options.skillsOut ? options.skillsOut : undefined
    // 钩子 / 网关进程只要事件监听器与提示词，不碰 skill。
    this.skipSkills = Boolean(options.skipSkills)
    this.dependencyDirs = new Set()
    const report = {
      hostVersion: HOST_VERSION,
      inspectedAt: Date.now(),
      root,
      name: undefined,
      version: undefined,
      description: undefined,
      title: undefined,
      kind: 'plugin',
      rows: [],
      skills: [],
      client: false,
      diagnostics: [],
      bridge: { hooks: [], instructions: false, staticPrompt: null },
    }
    let pkg
    try {
      pkg = readJson(join(root, 'package.json'))
    } catch (error) {
      report.diagnostics.push(`无法读取 package.json：${errorText(error)}`)
      report.kind = 'invalid'
      return report
    }
    const meta = readLocaleMeta(root)
    report.name = typeof pkg.name === 'string' ? pkg.name : undefined
    report.version = typeof pkg.version === 'string' ? pkg.version : undefined
    report.description = meta.description ?? (typeof pkg.description === 'string' ? pkg.description : undefined)
    report.title = meta.title
    report.engines = pkg.engines?.dsh
    report.client = Boolean(pkg.dsh?.client)
    if (report.client) {
      // 前端半边：exports["./client"] 指向的懒 CJS 工厂，由配套拓展包夹上胶水在 Codex 页面里运行（96-dsh-client.js）。
      const entry = packageEntry(root, pkg, './client')
      if (entry && existsSync(entry) && statSync(entry).size <= CLIENT_MAX_BYTES) {
        report.clientEntry = entry
        report.diagnostics.push(`包含 dsh 前端插件（dsh.client）：在 Codex 页面里用 Codex 的 React 运行，dsh 插槽映射到 Codex++ 的界面插槽；对不上的插槽、dsh 专属的前端服务（store、remote、conversation 等）不可用`)
      } else {
        report.diagnostics.push(entry && existsSync(entry)
          ? `dsh 前端插件入口超过 ${CLIENT_MAX_BYTES / 1024 / 1024} MiB，不加载`
          : 'package.json 声明了 dsh.client，但没有找到 exports["./client"] 入口')
      }
    }
    let rows
    if (pkg.dsh?.bundle?.patch) {
      report.kind = 'bundle'
      rows = readBundleRows(root, pkg, report.diagnostics, { ctx: this.expressionContext() })
    } else {
      const entry = packageEntry(root, pkg)
      rows = entry && existsSync(entry) ? [{ id: pkg.name ?? 'plugin', name: pkg.name ?? entry, config: undefined, disabled: false, baseDir: root }] : []
      if (rows.length > 0 && !pkg.name) rows[0].name = entry
    }
    const disabledRows = new Set(options.disabledRows ?? [])
    // 钩子 / 网关进程只加载用得上的行，其余行不跑 apply。
    const onlyRows = Array.isArray(options.onlyRows) ? new Set(options.onlyRows) : undefined
    const configs = options.configs && typeof options.configs === 'object' ? options.configs : {}
    for (const row of rows) {
      if (disabledRows.has(row.id) || (onlyRows && !onlyRows.has(row.id))) row.disabled = true
      // 管理器里填的配置浅合并到 bundle patch 的配置之上。
      const override = configs[row.id]
      if (override && typeof override === 'object' && !Array.isArray(override)) {
        row.config = { ...(row.config && typeof row.config === 'object' ? row.config : {}), ...override }
      }
    }
    // 依赖插件提供的服务（Service 子类行）的行先跳过，别的行加载完再试，直到没有进展（Cordis 也是服务就绪才 apply）。
    const records = new Array(rows.length)
    let queue = rows.map((row, index) => ({ row, index }))
    for (let progress = true; queue.length && progress;) {
      progress = false
      const waiting = []
      for (const item of queue) {
        const record = await this.loadRow(item.row, root, pkg, packageKey, { defer: true })
        if (record.deferred) {
          waiting.push(item)
          continue
        }
        records[item.index] = record
        progress = true
      }
      queue = waiting
    }
    for (const item of queue) records[item.index] = await this.loadRow(item.row, root, pkg, packageKey)
    report.rows.push(...records)
    report.bridge = this.bridgeSummary(report.rows)
    if (report.clientEntry) {
      // 前端半边拿包自己那一行的配置（含管理器里的覆盖）；bundle 没有同名行时用第一行。
      const own = rows.find((row) => row.name === report.name) ?? rows[0]
      const config = own?.config && typeof own.config === 'object' && !Array.isArray(own.config) ? own.config : {}
      report.bridge.client = { entry: report.clientEntry, packageName: report.name ?? '', config }
    }
    if (this.skipSkills) return report
    // 代码注册的 skill（可能来自依赖包）优先；同名或同一资源目录的 SKILL.md 文件不再重复列出。
    const runtimeSkills = report.rows.flatMap((row) => (row.skills ?? []).filter((skill) => skill.directory))
    const covered = new Set(runtimeSkills.flatMap((skill) => [skill.name, skill.resourceDir].filter(Boolean)))
    const budget = { remaining: MAX_SCANNED_SKILLS }
    const scanned = scanSkills(root, budget)
    let realRoot
    try {
      realRoot = realpathSync(root)
    } catch {
      realRoot = root
    }
    for (const directory of this.dependencyDirs) {
      // 依赖包必须真的在本包目录里；软链到包外（如 pnpm store、别的 checkout）不扫，避免读到包外的文件。
      let real
      try {
        real = realpathSync(directory)
      } catch {
        continue
      }
      if (!isInside(realRoot, real)) {
        report.diagnostics.push(`依赖包 ${relative(root, directory)} 指向包外，未扫描其中的 SKILL.md`)
        continue
      }
      scanned.push(...scanSkills(real, budget))
    }
    if (budget.remaining <= 0) report.diagnostics.push(`SKILL.md 超过 ${MAX_SCANNED_SKILLS} 个，只列出前 ${MAX_SCANNED_SKILLS} 个`)
    const seen = new Set()
    report.skills = [
      ...runtimeSkills.map(({ name, description, directory }) => ({ name, description, directory })),
      ...scanned.filter((skill) => {
        if (covered.has(skill.name) || covered.has(skill.directory) || seen.has(skill.name)) return false
        seen.add(skill.name)
        return true
      }),
    ]
    if (report.rows.length === 0 && report.skills.length === 0) {
      report.diagnostics.push('没有找到可加载的插件入口、bundle patch 或 SKILL.md')
    }
    return report
  }

  instructions() {
    return this.assemblePrompt({})
  }

  /** 包级汇总：哪些 Codex 钩子要转发、提示词是否追加到 instructions、能否在分析时求成静态文本。 */
  bridgeSummary(rows) {
    const active = rows.filter((row) => row.enabled && (row.status === 'compatible' || row.status === 'partial'))
    const targets = new Set(active.flatMap((row) => row.mappings ?? []))
    const hookEvents = new Set(Object.values(EVENT_HOOKS))
    const hooks = new Set([...targets].filter((target) => hookEvents.has(target)))
    const mirror = targets.has(MIRROR_TARGET)
    if (mirror) for (const hook of MIRROR_HOOKS) hooks.add(hook)
    const instructions = targets.has(INSTRUCTIONS_TARGET) || targets.has('gateway:instructions')
    const staticPrompt = instructions && !targets.has('gateway:instructions') && this.promptIsStatic() ? this.assemblePrompt({}) || null : null
    const commands = active.flatMap((row) => row.commands ?? []).filter((command) => typeof command?.name === 'string' && command.name)
    return {
      hooks: [...hooks].sort(),
      instructions,
      staticPrompt,
      mirror,
      // step：每一步的请求都交给插件（agent/request、按步的 agent/pre-step、改写工具结果、钩子之外排下的上下文）。
      gateway: {
        step: targets.has('gateway:step') || targets.has('gateway:request') || mirror,
        error: targets.has('gateway:error'),
        response: targets.has('gateway:response'),
      },
      connection: targets.has(CONNECTION_TARGET),
      schedule: targets.has(SCHEDULE_TARGET),
      webhook: targets.has(WEBHOOK_TARGET),
      // 监听插件间自定义事件：要有配套拓展包（标记文件与常驻进程），别的包才能把事件转过来。
      events: targets.has(EVENTS_TARGET),
      commands,
    }
  }

  async dispose() {
    // 常驻进程退出：会话先落盘（session/flush），再按 dsh 的顺序发 agent/disposed、session/disposed。
    for (const session of this.mirror.sessions.values()) {
      try {
        await this.parallel('session/flush', [session])
      } catch {}
    }
    this.mirror.flushAll()
    for (const agent of this.agentRegistry.list()) this.emitEvent('agent/disposed', [{ agent }])
    for (const session of this.mirror.sessions.values()) this.emitEvent('session/disposed', [session])
    for (const { listener } of this.listeners.get('dispose') ?? []) {
      try {
        await listener()
      } catch {
        // 退出路径上忽略插件错误。
      }
    }
    for (const dispose of this.disposers.splice(0).reverse()) {
      try {
        await dispose()
      } catch {
        // 同上。
      }
    }
  }

  async emitReady() {
    for (const { listener, state } of this.listeners.get('ready') ?? []) {
      try {
        await listener()
      } catch (error) {
        state.notes.push(`ready 回调失败：${errorText(error)}`)
      }
    }
  }
}

function makeLogger(scope) {
  const write = (level) => (...args) => console.error(`[dsh:${scope}] ${level}`, ...args)
  const logger = (name) => makeLogger(`${scope}:${name}`)
  for (const level of ['debug', 'info', 'warn', 'error', 'success']) logger[level] = write(level)
  return logger
}

function withTimeout(promise, ms, message) {
  if (!Number.isFinite(ms) || ms <= 0) return promise
  let handle
  return Promise.race([
    promise,
    new Promise((_, reject) => {
      handle = setTimeout(() => reject(new Error(message)), ms)
    }),
  ]).finally(() => clearTimeout(handle))
}

// ---------------------------------------------------------------------------
// MCP 服务
// ---------------------------------------------------------------------------

/**
 * dsh 内容块 → MCP 内容块。dsh 的图片块是 `{ type: 'image', attachment: { attachmentId, mediaType } }`，
 * 字节在附件服务里：给了 `attachments`（产出这个结果的插件的附件服务）就读出来作为真图片，读不到再退成文字说明。
 */
async function toMcpContent(blocks, attachments) {
  const content = []
  for (const block of Array.isArray(blocks) ? blocks : []) {
    if (block?.type === 'text' && typeof block.text === 'string') content.push({ type: 'text', text: block.text })
    else if (block?.type === 'image' && typeof block.data === 'string') {
      content.push({ type: 'image', data: block.data, mimeType: block.mimeType ?? 'image/png' })
    } else if (block?.type === 'image') {
      const ref = block.attachment
      let image
      if (ref?.attachmentId && attachments) {
        try {
          const request = await attachments.readImageRequest(ref)
          image = { type: 'image', data: request.data, mimeType: ref.mediaType ?? 'image/png' }
        } catch (error) {
          image = { type: 'text', text: `[图片：附件 ${ref.attachmentId} 读取失败：${errorText(error)}]` }
        }
      }
      content.push(image ?? { type: 'text', text: '[图片：附件不在这个插件的数据目录里，Codex 中不可用]' })
    } else if (block !== undefined) content.push({ type: 'text', text: JSON.stringify(block) })
  }
  return content
}

function safeMatch(matches, endpoint) {
  try {
    return Boolean(matches(endpoint))
  } catch {
    return false
  }
}

/** 工具的参数 JSON Schema（defineTool 已经把 dsh 的参数表转好）。 */
function toolParameters(def) {
  return def.parameters && typeof def.parameters === 'object' ? def.parameters : { type: 'object', properties: {} }
}

async function renderValue(def, args, value, attachments) {
  try {
    return await toMcpContent(await def.output.render(args, value), attachments)
  } catch (error) {
    return [{ type: 'text', text: typeof value === 'string' ? value : JSON.stringify(value, undefined, 2) ?? '' }, { type: 'text', text: `（render 失败：${errorText(error)}）` }]
  }
}

async function serve(registryPath) {
  const host = new Host()
  // Codex 每次重连 MCP 都会 spawn 一套新宿主，却不一定关掉旧连接的管道：只靠 stdin 的
  // close 事件退出会留下孤儿——它一直停在 createInterface 上等数据，什么也不做，白占内存
  // （实测一天内累积到 45 个进程、2.2 GB，把整台机器的调度都拖重）。
  // 这里盯住父进程 id：被 init 收养（ppid 变 1）或父进程换了，就当作连接已断，主动收摊。
  const parentPid = process.ppid
  const parentWatch = setInterval(() => {
    if (process.ppid === parentPid && process.ppid > 1) return
    clearInterval(parentWatch)
    void host.dispose().catch(() => {}).finally(() => hostExit(0))
  }, 5000)
  // 定时器不撑住事件循环：stdio 还连着的时候进程本来就活着。
  parentWatch.unref?.()
  let registry = { plugins: [] }
  try {
    registry = readJson(registryPath)
  } catch (error) {
    console.error(`[dsh-host] 无法读取注册表 ${registryPath}：${errorText(error)}`)
  }
  for (const plugin of Array.isArray(registry.plugins) ? registry.plugins : []) {
    if (!plugin || plugin.enabled === false || typeof plugin.root !== 'string') continue
    try {
      const report = await host.loadPackage(plugin.root, String(plugin.id ?? plugin.root), {
        disabledRows: plugin.disabledRows,
        configs: plugin.configs,
        dataDir: join(dirname(registryPath), 'data', String(plugin.id ?? 'plugin').replace(/[^\w.-]/g, '_')),
      })
      for (const row of report.rows) {
        if (row.status === 'error') console.error(`[dsh-host] ${plugin.id}/${row.id}：${row.reasons.join('；')}`)
      }
    } catch (error) {
      console.error(`[dsh-host] 加载 ${plugin.id} 失败：${errorText(error)}`)
    }
  }
  await host.emitReady()

  const running = new Map()
  const send = (message) => writeStdout(`${JSON.stringify(message)}\n`)
  const reply = (id, result) => send({ jsonrpc: '2.0', id, result })
  const fail = (id, code, message) => send({ jsonrpc: '2.0', id, error: { code, message } })

  const handle = async (message) => {
    const { id, method, params } = message
    switch (method) {
      case 'initialize': {
        const instructions = host.instructions()
        reply(id, {
          protocolVersion: typeof params?.protocolVersion === 'string' ? params.protocolVersion : '2025-06-18',
          capabilities: { tools: { listChanged: false }, ...(host.mcpResources.size ? { resources: { listChanged: false } } : {}) },
          serverInfo: { name: 'codex-plus-dsh', version: HOST_VERSION },
          ...(instructions ? { instructions } : {}),
        })
        return
      }
      case 'ping':
        reply(id, {})
        return
      case 'resources/list':
      case 'resources/templates/list': {
        // 各插件登记的资源合在一起；URI 前面加上 server，读的时候据此找回提供方。
        const listKey = method === 'resources/list' ? 'resources' : 'resourceTemplates'
        const uriKey = method === 'resources/list' ? 'uri' : 'uriTemplate'
        const items = []
        for (const [server, provider] of host.mcpResources) {
          try {
            const result = await provider.request({ method }, { name: 'resources', callId: `codexpp-${String(id)}` })
            for (const item of result?.[listKey] ?? []) items.push({ ...item, [uriKey]: `dsh-${server}+${item[uriKey]}` })
          } catch (error) {
            console.error(`[dsh-host] ${server} 的 ${method} 失败：${errorText(error)}`)
          }
        }
        reply(id, { [listKey]: items })
        return
      }
      case 'resources/read': {
        const match = /^dsh-([^+]+)\+(.+)$/.exec(String(params?.uri ?? ''))
        const provider = match && host.mcpResources.get(match[1])
        if (!provider) {
          fail(id, -32602, `未知资源：${params?.uri}`)
          return
        }
        try {
          const result = await provider.request({ method: 'resources/read', uri: match[2] }, { name: 'resources', callId: `codexpp-${String(id)}` })
          reply(id, { contents: (result?.contents ?? []).map((item) => ({ ...item, uri: params.uri })) })
        } catch (error) {
          fail(id, -32603, errorText(error))
        }
        return
      }
      case 'tools/list':
        reply(id, {
          tools: [...host.tools.values()].filter(({ def }) => host.toolVisible(def.name)).map(({ def }) => ({
            name: def.name,
            description: def.description ?? '',
            inputSchema: toolParameters(def),
          })),
        })
        return
      case 'tools/call': {
        const name = params?.name
        const entry = host.tools.get(name)
        if (!entry) {
          fail(id, -32602, `未知工具：${name}`)
          return
        }
        const controller = new AbortController()
        running.set(id, controller)
        try {
          const result = await host.executeTool({ callId: `codexpp-${String(id)}`, name, arguments: params?.arguments ?? {}, signal: controller.signal })
          reply(id, { content: await toMcpContent(result.content), isError: result.isError === true })
        } finally {
          running.delete(id)
        }
        return
      }
      case 'notifications/cancelled':
        running.get(params?.requestId)?.abort()
        return
      default:
        if (id !== undefined && id !== null && !method?.startsWith('notifications/')) fail(id, -32601, `不支持的方法：${method}`)
    }
  }

  const lines = createInterface({ input: process.stdin, crlfDelay: Infinity })
  lines.on('line', (line) => {
    if (!line.trim()) return
    let message
    try {
      message = JSON.parse(line)
    } catch {
      send({ jsonrpc: '2.0', id: null, error: { code: -32700, message: 'JSON 解析失败' } })
      return
    }
    handle(message).catch((error) => {
      if (message?.id !== undefined) fail(message.id, -32603, errorText(error))
    })
  })
  lines.on('close', () => {
    void host.dispose().finally(() => hostExit(0))
  })
}

async function inspect(root, optionsPath) {
  const host = new Host()
  const options = optionsPath ? readJson(optionsPath) : {}
  const report = await host.loadPackage(resolve(root), 'inspect', options)
  await host.dispose()
  writeStdout(`${JSON.stringify(report)}\n`)
}

function isMainModule() {
  if (!process.argv[1]) return false
  try {
    return realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url))
  } catch {
    return false
  }
}

const [command, target, extra] = isMainModule() ? process.argv.slice(2) : ['library']
if (command === 'library') {
  // 被 bridge.mjs 引入，不跑命令行入口。
} else if (command === 'inspect' && target) {
  guardProcessExit()
  inspect(target, extra).then(
    () => hostExit(0),
    (error) => {
      console.error(errorText(error))
      hostExit(1)
    },
  )
} else if (command === 'serve' && target) {
  guardProcessExit()
  serve(target).catch((error) => {
    console.error(errorText(error))
    hostExit(1)
  })
} else {
  console.error('用法：host.mjs inspect <包目录> [选项.json] | host.mjs serve <registry.json>')
  hostExit(2)
}
