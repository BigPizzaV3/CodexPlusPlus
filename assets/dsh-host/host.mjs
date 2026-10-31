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

import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, realpathSync, statSync, writeFileSync } from 'node:fs'
import { register } from 'node:module'
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path'
import { createInterface } from 'node:readline'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { hostCall } from './host-call.mjs'
import { parseYaml } from './yaml.mjs'
import {
  AgentRegistry, MIRROR_SERVICES, SUPPORTED_SERVICES, SessionMirror, createCredentials, createFs, createJobs, createLlm, createSchedule,
  createSessionQuery, createShell, createStorageDomain, createSubprocess, createUserQuestions, createWeb, createWebhookRuntime,
  createWorkspaceRegistry,
} from './services.mjs'

const HOST_VERSION = '3'
const HOST_DIR = dirname(fileURLToPath(import.meta.url))
const SHIMS = {
  '@deepseek-ai/dsh-tools': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-tools.mjs')).href,
  '@deepseek-ai/schemastery': pathToFileURL(join(HOST_DIR, 'shims', 'schemastery.mjs')).href,
  '@deepseek-ai/cordis': pathToFileURL(join(HOST_DIR, 'shims', 'cordis.mjs')).href,
  '@deepseek-ai/dsh-skill': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-skill.mjs')).href,
  '@deepseek-ai/dsh-system-prompt': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-system-prompt.mjs')).href,
  '@deepseek-ai/dsh-llm': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-llm.mjs')).href,
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
}
/**
 * 由会话镜像派发的事件：钩子常驻进程把 Codex 钩子翻成 dsh 会话日志，再按 dsh 的语义发这些事件。
 * 用到它们（或 sessions / agents / sessionProjections 服务）的插件，配套拓展包会挂上 MIRROR_HOOKS 全部钩子。
 */
export const MIRROR_EVENTS = new Set(['session/event', 'session/created', 'agent/status', 'tools/result'])
export const MIRROR_HOOKS = ['SessionStart', 'UserPromptSubmit', 'PreToolUse', 'PostToolUse', 'Stop', 'PreCompact', 'PostCompact']
/** 能对上 Codex++ 网关中间件的事件：agent/request 改模型 / 推理强度，agent/request-error 决定重试。 */
export const GATEWAY_EVENTS = {
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
/** 插件注册的斜杠命令：配套拓展包在 Codex 页面里注册同名命令，经后台转给常驻进程执行。 */
export const COMMANDS_TARGET = 'commands'
/** 插件宿主半边经 ctx.connection.fetch 注册的 HTTP 路由：前端半边的 fetch('/api/…') 经配套拓展包的后台转过来。 */
export const CONNECTION_TARGET = 'connection'
/** 用到 ctx.schedule 的行：配套拓展包挂每分钟一次的定时任务，到点往会话里发消息。 */
export const SCHEDULE_TARGET = 'schedule'
/** 注册了 webhook 规则的行：配套拓展包开 /ext/<拓展>/webhook/<kind> 路由。 */
export const WEBHOOK_TARGET = 'webhook'
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
/** 配套拓展包单个文件的上限（与 Codex++ 写拓展包的限制一致，再留出胶水代码的余量）。 */
const CLIENT_MAX_BYTES = 240 * 1024

// MCP 帧走 stdout（钩子 / 网关进程里 stdout 是运行器的结果），插件的 console 输出一律改到 stderr。
const writeStdout = process.stdout.write.bind(process.stdout)
for (const level of ['log', 'info', 'debug', 'trace']) {
  console[level] = (...args) => console.error(...args)
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
function collectRows(entries, baseDir, diagnostics, out, parentDisabled = false) {
  for (const row of entries ?? []) {
    if (!row || typeof row !== 'object' || typeof row.name !== 'string') {
      diagnostics.push('patch 中有缺少 name 的插件行，已跳过')
      continue
    }
    const disabled = parentDisabled || row.disabled === true
    if (row.disabled && typeof row.disabled === 'object') diagnostics.push(`插件行 ${row.id ?? row.name} 的 disabled 是表达式，按启用处理`)
    if (row.name === GROUP_MODULE) {
      collectRows(Array.isArray(row.config) ? row.config : [], baseDir, diagnostics, out, disabled)
      continue
    }
    out.push({ id: String(row.id ?? row.name), name: row.name, config: row.config ?? undefined, disabled, baseDir })
  }
}

function hasTaggedValue(value) {
  if (!value || typeof value !== 'object') return false
  if (typeof value.tag === 'string' && 'value' in value && Object.keys(value).length === 2) return true
  return Object.values(value).some(hasTaggedValue)
}

function readBundleRows(root, pkg, diagnostics) {
  const patchField = pkg.dsh?.bundle?.patch
  const patches = Array.isArray(patchField) ? patchField : [patchField]
  const rows = []
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
    for (const entry of Array.isArray(document) ? document : []) {
      if (!entry || typeof entry !== 'object') continue
      for (const operation of Object.keys(entry)) {
        if (operation !== 'insert') diagnostics.push(`${patch} 里的 ${operation} 操作不支持，已忽略`)
      }
      collectRows(entry.insert, dirname(path), diagnostics, rows)
    }
  }
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
  if (isClass(fallback)) return { serviceClass: true, name: fallback.name }
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
    const emit = (event, args) => this.emitEvent(event, args)
    this.mirror = new SessionMirror({ dataDir: options.mirrorDir, emit })
    /** 插件经 ctx.webhookRuntime 注册的规则（配套拓展包的后台把 /webhook/<kind> 请求转过来）。 */
    this.webhooks = createWebhookRuntime()
    /** 插件包的 ctx.schedule（记录存在包的数据目录，按包共用）。 */
    this.schedules = undefined
    this.agentRegistry = new AgentRegistry({ mirror: this.mirror, emit, serial: (event, args) => this.serial(event, args) })
    this.shared = undefined
  }

  /** 所有插件共用的替身服务（按需建）。credentials / storageDomain 按插件建，见 createContext。 */
  sharedServices() {
    if (this.shared) return this.shared
    const host = this
    const mirror = this.mirror
    const sessions = {
      get: (id) => mirror.sessions.get(String(id)),
      list: () => [...mirror.sessions.values()],
      create() {
        throw new Error('Codex++ 里不能新建 dsh 会话；要新建 Codex 会话请用 ctx.sessions.codex.create')
      },
      fork() {
        throw new Error('Codex++ 里不能分叉 dsh 会话；要分叉 Codex 会话请用 ctx.sessions.codex.fork')
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
      fs: createFs(),
      subprocess: createSubprocess(),
      shell: createShell(),
      web: createWeb(),
      userQuestions: createUserQuestions(),
      jobs: createJobs((event, args) => host.emitEvent(event, args)),
      workspaceRegistry: createWorkspaceRegistry(),
      agentDefaultModel: Object.freeze({ provider: 'codex', model: undefined, reasoningEffort: undefined }),
    }
    return this.shared
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
   * 带 system-prompt/assemble 中间件的提示词：先按 dsh 的结构拼出 assembly，交给插件的 waterfall 改写，再渲染成文本。
   * 没有监听器时等同 assemblePrompt。
   */
  async assemblePromptAsync(context = {}) {
    if (!this.hasListeners('system-prompt/assemble')) return this.assemblePrompt(context)
    const base = { sections: [], contexts: [], tools: [], variables: {} }
    const fallback = async () => base
    let assembly
    try {
      assembly = await this.waterfall('system-prompt/assemble', [base, { agent: context.agent, signal: context.signal }], fallback)
    } catch (error) {
      console.error(`[dsh-host] system-prompt/assemble 失败：${errorText(error)}`)
      assembly = base
    }
    const own = this.assemblePrompt(context)
    const added = [...(assembly?.sections ?? []), ...(assembly?.contexts ?? [])]
      .map((entry) => (typeof entry?.text === 'string' ? entry.text.trim() : ''))
      .filter(Boolean)
    return [own, ...added].filter(Boolean).join('\n\n')
  }

  /**
   * 拼出系统提示词：片段按 order、name 排序，之后接动态上下文；`{{变量}}` 用插件注册的变量求值，
   * 求不出的原样保留（dsh 会直接报错，这里宁可宽松，不能挡住模型请求）。
   */
  assemblePrompt(context = {}) {
    const render = (entry) => {
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
    const byOrder = (a, b) => a.order - b.order || String(a.name).localeCompare(String(b.name))
    return [...[...this.sections].sort(byOrder), ...[...this.contexts].sort(byOrder)]
      .map(render)
      .filter(Boolean)
      .join('\n\n')
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
    const shared = this.sharedServices()
    host.schedules ??= createSchedule(host.dataDir, (event, args) => host.emitEvent(event, args))
    const pluginServices = {
      credentials: createCredentials(state.config),
      storageDomain: createStorageDomain(state.dataDir),
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
    const base = {
      name: state.pluginName,
      config: state.config,
      logger,
      timer,
      setTimeout: timer.setTimeout,
      setInterval: timer.setInterval,
      sleep: timer.sleep,
      tools: {
        register(definition, options = {}) {
          if (!definition || typeof definition.name !== 'string' || typeof definition.execute !== 'function') {
            throw new Error('tools.register 需要 defineTool() 返回的工具定义')
          }
          if (host.tools.has(definition.name)) {
            throw new Error(`tool "${definition.name}" is already registered`)
          }
          if (options && typeof options.when === 'function') note('工具的 when 条件依赖 dsh 的 agent，Codex 里视为始终可用')
          host.tools.set(definition.name, { def: definition, pluginKey: state.key })
          state.tools.push(definition.name)
          return track(() => {
            if (host.tools.get(definition.name)?.def === definition) host.tools.delete(definition.name)
          })
        },
        get(name) {
          return host.tools.get(name)?.def
        },
        has(name) {
          return host.tools.has(name)
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
          if (section?.complete) note('complete 片段在 Codex 里只追加到 instructions，不会替换 Codex 自己的系统提示词')
          return track(() => {
            host.sections = host.sections.filter((item) => item !== entry)
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
          note('systemPrompt.context 在 Codex 里并入 instructions，不会作为用户侧上下文快照逐回合刷新')
          return track(() => {
            host.contexts = host.contexts.filter((item) => item !== entry)
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
          return track(() => {
            state.skillDefinitions = state.skillDefinitions.filter((item) => item !== skill)
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
        rpc: {
          handle() {
            note('connection.rpc 在 Codex++ 里不可用，只支持 connection.fetch.register')
            return () => {}
          },
        },
      },
      commands: {
        ...shared.commands,
        register(definition) {
          mapTo(COMMANDS_TARGET)
          state.commands?.push({ name: definition?.name, description: definition?.description ?? '', hint: definition?.input?.hint ?? '' })
          return track(shared.commands.register(definition))
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
      /** 可选依赖的激活写法：服务都在就立刻以子 ctx 调用（Codex++ 里服务不会中途出现或消失）。 */
      inject(names, callback) {
        const list = Array.isArray(names) ? names : Object.keys(names ?? {})
        const missing = list.filter((name) => !SUPPORTED_SERVICES.has(name))
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
        else if (event === 'session/flush' || event === 'session/disposed') {
          // 镜像会话在常驻进程退出时落盘，不派发这两个事件。
        } else if (event !== 'ready' && event !== 'dispose') note(`事件 ${String(event)} 来自 dsh 运行时，Codex 里不会触发`)
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
      off() {},
      emit() {},
      async parallel() {},
      async serial() {
        return undefined
      },
      bail() {
        return undefined
      },
      waterfall(...args) {
        const next = args[args.length - 1]
        return typeof next === 'function' ? next() : undefined
      },
      get(name) {
        if (!SUPPORTED_SERVICES.has(name)) return undefined
        if (MIRROR_SERVICES.has(name)) mapTo(MIRROR_TARGET)
        if (name === 'schedule') mapTo(SCHEDULE_TARGET)
        return proxy[name]
      },
      async plugin(plugin, config) {
        return host.applyPlugin(plugin, config, state)
      },
    }
    const proxy = new Proxy(base, {
      get(target, key) {
        if (key in target) return target[key]
        if (typeof key === 'symbol' || key === 'then' || key === 'toJSON') return undefined
        note(`访问了 ctx.${String(key)}，Codex++ 不提供这个服务`)
        return undefined
      },
    })
    return proxy
  }

  async applyPlugin(plugin, config, parentState) {
    const normalized = typeof plugin === 'function' && !isClass(plugin)
      ? { name: plugin.name, inject: plugin.inject, Config: plugin.Config, apply: plugin }
      : plugin
    if (!normalized || typeof normalized.apply !== 'function') throw new Error('ctx.plugin 只支持带 apply 的插件')
    const inject = normalizeInject(normalized.inject)
    const missing = inject.required.filter((name) => !SUPPORTED_SERVICES.has(name))
    if (missing.length > 0) throw new Error(`子插件依赖 Codex++ 不提供的服务：${missing.join(', ')}`)
    const resolvedConfig = typeof normalized.Config === 'function' ? normalized.Config(config ?? {}) : config
    const ctx = this.createContext({ ...parentState, config: resolvedConfig })
    await normalized.apply(ctx, resolvedConfig)
    return undefined
  }

  /** 加载一行插件，返回该行的兼容性记录。 */
  async loadRow(row, root, pkg, packageKey) {
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
    if (plugin.serviceClass) {
      record.status = 'incompatible'
      record.kind = 'service'
      record.reasons.push(`${plugin.name || '该模块'} 是 Service 子类，给其他 dsh 插件提供服务，Codex 里没有消费者`)
      return record
    }
    const inject = normalizeInject(plugin.inject)
    record.inject = inject
    const missing = inject.required.filter((name) => !SUPPORTED_SERVICES.has(name))
    if (missing.length > 0) {
      record.status = 'incompatible'
      record.reasons.push(`依赖 Codex++ 不提供的服务：${missing.join(', ')}（属于 dsh 的 agent 循环 / 会话 / 模型等运行时）`)
      return record
    }
    const optionalMissing = inject.optional.filter((name) => !SUPPORTED_SERVICES.has(name))
    if (optionalMissing.length > 0) record.notes.push(`可选服务 ${optionalMissing.join(', ')} 不可用`)
    // 依赖会话 / agent / 投影的插件要会话镜像：配套拓展包会挂上把 Codex 钩子翻成 dsh 会话日志的那几个钩子。
    // 只声明 sessions 的插件多半只是 get 一下会话读 cwd，不值得为它在每次工具调用上起钩子；用到 agent / 投影才挂。
    if ([...inject.required, ...inject.optional].some((name) => name === 'agents' || name === 'sessionProjections')) record.mappings.push(MIRROR_TARGET)
    if ([...inject.required, ...inject.optional].includes('schedule')) record.mappings.push(SCHEDULE_TARGET)
    const mirrorNotes = {
      sessions: 'ctx.sessions 是 Codex 会话的镜像（由 Codex 钩子事件生成）：没有模型的流式内容与推理，Codex 自己的会话日志请用 sessionQuery',
      agents: 'ctx.agents 里的 agent 是 Codex 会话的替身：steer / inject 在钩子里翻成补充上下文或要求继续，钩子之外经 Codex++ 往会话里插话',
      sessionProjections: 'sessionProjections 在会话镜像上折叠；客户端 useProjection 视图不提供',
      llm: 'ctx.llm.stream 经 Codex++ 协议代理一次性返回，拆成流式块；不能注册 adapter',
      fs: 'ctx.fs 是本机文件系统，不做沙箱与「先读后写」策略',
      shell: 'ctx.shell 用本机 shell 执行，不做沙箱',
      schedule: 'ctx.schedule 到点经 Codex++ 往 Codex 会话里发起一轮（每分钟检查一次，需要 Codex++ 在运行）',
      webhookRuntime: 'webhook 规则经 Codex++ 本地代理的 /ext/<配套拓展>/webhook/<kind> 接收，不校验签名',
      jobs: 'ctx.jobs 只在当前进程里登记，不跨进程',
      workspaceRegistry: 'ctx.workspaceRegistry 由 Codex 会话的工作目录归出来，只读',
      subprocess: 'ctx.subprocess 不提供 PTY 终端',
      web: 'ctx.web 只有 fetch，没有联网搜索',
      userQuestions: 'ctx.userQuestions 在 Codex 页面里弹框提问（一题一问）',
    }
    for (const name of [...inject.required, ...inject.optional]) if (mirrorNotes[name]) record.notes.push(mirrorNotes[name])
    let config = row.config ?? {}
    if (hasTaggedValue(config)) record.notes.push('配置里的 !!js 表达式不会求值')
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
      pluginName: plugin.name ?? row.id,
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
    }
    try {
      const ctx = this.createContext(state)
      await withTimeout(Promise.resolve(plugin.apply(ctx, config)), APPLY_TIMEOUT_MS, `apply 超过 ${APPLY_TIMEOUT_MS / 1000} 秒未完成`)
      await Promise.all(state.pending)
    } catch (error) {
      record.status = 'error'
      record.reasons.push(`apply 失败：${errorText(error)}`)
      return record
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
    if (record.skills.length > 0 && record.tools.length === 0 && record.mappings.length === 0) {
      record.kind = 'skills'
      if (record.status === 'compatible') record.status = 'skills'
    } else if (record.tools.length === 0 && record.mappings.length === 0 && record.status !== 'partial') {
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
          ? `dsh 前端插件入口超过 ${CLIENT_MAX_BYTES / 1024} KiB，不加载`
          : 'package.json 声明了 dsh.client，但没有找到 exports["./client"] 入口')
      }
    }
    let rows
    if (pkg.dsh?.bundle?.patch) {
      report.kind = 'bundle'
      rows = readBundleRows(root, pkg, report.diagnostics)
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
      report.rows.push(await this.loadRow(row, root, pkg, packageKey))
    }
    report.bridge = this.bridgeSummary(report.rows)
    if (report.clientEntry) report.bridge.client = { entry: report.clientEntry, packageName: report.name ?? '' }
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
      commands,
    }
  }

  async dispose() {
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

/** dsh 内容块 → MCP 内容块。图片引用的是 dsh 会话附件，Codex 里拿不到。 */
function toMcpContent(blocks) {
  const content = []
  for (const block of Array.isArray(blocks) ? blocks : []) {
    if (block?.type === 'text' && typeof block.text === 'string') content.push({ type: 'text', text: block.text })
    else if (block?.type === 'image' && typeof block.data === 'string') {
      content.push({ type: 'image', data: block.data, mimeType: block.mimeType ?? 'image/png' })
    } else if (block?.type === 'image') content.push({ type: 'text', text: '[图片：引用 dsh 会话附件，Codex 中不可用]' })
    else if (block !== undefined) content.push({ type: 'text', text: JSON.stringify(block) })
  }
  return content
}

function renderValue(def, args, value) {
  try {
    return toMcpContent(def.output.render(args, value))
  } catch (error) {
    return [{ type: 'text', text: typeof value === 'string' ? value : JSON.stringify(value, undefined, 2) ?? '' }, { type: 'text', text: `（render 失败：${errorText(error)}）` }]
  }
}

async function serve(registryPath) {
  const host = new Host()
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
          capabilities: { tools: { listChanged: false } },
          serverInfo: { name: 'codex-plus-dsh', version: HOST_VERSION },
          ...(instructions ? { instructions } : {}),
        })
        return
      }
      case 'ping':
        reply(id, {})
        return
      case 'tools/list':
        reply(id, {
          tools: [...host.tools.values()].map(({ def }) => ({
            name: def.name,
            description: def.description ?? '',
            inputSchema: def.parameters && typeof def.parameters === 'object' ? def.parameters : { type: 'object', properties: {} },
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
        const args = params?.arguments ?? {}
        const controller = new AbortController()
        running.set(id, controller)
        const callId = `codexpp-${String(id)}`
        const exec = {
          callId,
          rootCallId: callId,
          name,
          arguments: args,
          signal: controller.signal,
          agent: undefined,
          token: Symbol('codexpp.tool'),
          deferContext() {},
          concludeTurn() {},
        }
        try {
          const timeout = entry.def.timeoutMs
          const value = await withTimeout(Promise.resolve(entry.def.execute(args, exec)), timeout, `工具执行超过 ${timeout} ms`)
          reply(id, { content: renderValue(entry.def, args, value), isError: false })
        } catch (error) {
          controller.abort()
          reply(id, { content: [{ type: 'text', text: errorText(error) }], isError: true })
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
    void host.dispose().finally(() => process.exit(0))
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
  inspect(target, extra).then(
    () => process.exit(0),
    (error) => {
      console.error(errorText(error))
      process.exit(1)
    },
  )
} else if (command === 'serve' && target) {
  serve(target).catch((error) => {
    console.error(errorText(error))
    process.exit(1)
  })
} else {
  console.error('用法：host.mjs inspect <包目录> [选项.json] | host.mjs serve <registry.json>')
  process.exit(2)
}
