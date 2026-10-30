// Codex++ 的 dsh 插件兼容宿主。
//
//   node host.mjs inspect <包目录> [选项.json]  分析一个 dsh 插件包，把兼容性报告（JSON）写到 stdout
//   node host.mjs serve <registry.json>   以 stdio MCP 服务运行注册表里启用的插件，把它们的工具暴露给 Codex
//
// dsh 插件是 Cordis 插件：`inject` 声明依赖的服务，`apply(ctx, config)` 里注册工具等副作用。
// Codex App 没有 dsh 的 agent 循环与会话服务，这里只提供工具插件真正能用上的那部分 ctx：
// tools / systemPrompt（片段汇总成 MCP instructions）/ logger / timer / effect / on 等；
// 必需依赖里有别的服务时不加载，并在报告里写明缺了什么。

import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, realpathSync, statSync, writeFileSync } from 'node:fs'
import { register } from 'node:module'
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path'
import { createInterface } from 'node:readline'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { parseYaml } from './yaml.mjs'

const HOST_VERSION = '1'
const HOST_DIR = dirname(fileURLToPath(import.meta.url))
const SHIMS = {
  '@deepseek-ai/dsh-tools': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-tools.mjs')).href,
  '@deepseek-ai/schemastery': pathToFileURL(join(HOST_DIR, 'shims', 'schemastery.mjs')).href,
  '@deepseek-ai/cordis': pathToFileURL(join(HOST_DIR, 'shims', 'cordis.mjs')).href,
  '@deepseek-ai/dsh-skill': pathToFileURL(join(HOST_DIR, 'shims', 'dsh-skill.mjs')).href,
}
const MCP_CLIENT_MODULE = '@deepseek-ai/dsh-mcp-client'
const GROUP_MODULE = 'cordis:group'
/** 宿主能提供的服务；Cordis 自带的 logger / timer 一并算上。skills 只截取注册的 skill，交给 Codex Skills。 */
const SUPPORTED_SERVICES = new Set(['tools', 'systemPrompt', 'logger', 'timer', 'skills'])
const APPLY_TIMEOUT_MS = 15_000
const MAX_SKILL_SCAN_DEPTH = 4
/** 一个包（含它引用的依赖包）最多列出多少个 SKILL.md。 */
const MAX_SCANNED_SKILLS = 64
const MAX_TEXT_BYTES = 1024 * 1024

// MCP 帧走 stdout，插件的 console 输出一律改到 stderr。
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

function errorText(error) {
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

class Host {
  constructor() {
    /** name → { def, pluginKey } */
    this.tools = new Map()
    /** 系统提示词片段，汇总成 MCP instructions。 */
    this.sections = []
    this.listeners = new Map()
    this.disposers = []
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
    const logger = makeLogger(state.key)
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
          const entry = { pluginKey: state.key, name: section?.name, order: Number(section?.order) || 0, text: section?.text }
          host.sections.push(entry)
          note('系统提示词片段会汇总进 MCP instructions，只在 Codex 连接 MCP 时提供一次')
          return track(() => {
            host.sections = host.sections.filter((item) => item !== entry)
          })
        },
        context() {
          note('systemPrompt.context 依赖 dsh 的回合上下文，Codex 里不生效')
          return () => {}
        },
        variable() {
          return () => {}
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
      on(event, listener) {
        if (event !== 'ready' && event !== 'dispose') note(`事件 ${String(event)} 来自 dsh 运行时，Codex 里不会触发`)
        const list = host.listeners.get(event) ?? []
        const entry = { listener, state }
        list.push(entry)
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
        return SUPPORTED_SERVICES.has(name) ? base[name] : undefined
      },
      async plugin(plugin, config) {
        return host.applyPlugin(plugin, config, state)
      },
    }
    return new Proxy(base, {
      get(target, key) {
        if (key in target) return target[key]
        if (typeof key === 'symbol' || key === 'then' || key === 'toJSON') return undefined
        note(`访问了 ctx.${String(key)}，Codex++ 不提供这个服务`)
        return undefined
      },
    })
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
      if (runtimePackage) {
        record.status = 'incompatible'
        record.reasons.push(`依赖 dsh 运行时包 ${runtimePackage}，Codex++ 不提供（属于 dsh 的 agent 循环 / 会话 / 模型等运行时）`)
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
      disposers: this.disposers,
      pending: [],
      skillDefinitions: [],
      skillProviders: [],
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
    for (const definition of await this.collectSkills(state)) {
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
    if (record.skills.length > 0 && record.tools.length === 0) {
      record.kind = 'skills'
      if (record.status === 'compatible') record.status = 'skills'
    } else if (record.tools.length === 0 && record.status !== 'partial') {
      record.notes.push('加载成功，但没有注册任何工具')
    }
    return record
  }

  async loadPackage(root, packageKey, options = {}) {
    // inspect 时把代码注册的 skill 写到这里；serve 不写。
    this.skillsOut = typeof options.skillsOut === 'string' && options.skillsOut ? options.skillsOut : undefined
    this.dependencyDirs = new Set()
    const report = {
      hostVersion: HOST_VERSION,
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
      report.diagnostics.push('包含 dsh 前端 slot 插件（dsh.client）：它向 dsh Web 客户端的 React slot 注册组件，Codex 页面结构不同，需要改写成 Codex++ 拓展包')
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
    const configs = options.configs && typeof options.configs === 'object' ? options.configs : {}
    for (const row of rows) {
      if (disabledRows.has(row.id)) row.disabled = true
      // 管理器里填的配置浅合并到 bundle patch 的配置之上。
      const override = configs[row.id]
      if (override && typeof override === 'object' && !Array.isArray(override)) {
        row.config = { ...(row.config && typeof row.config === 'object' ? row.config : {}), ...override }
      }
      report.rows.push(await this.loadRow(row, root, pkg, packageKey))
    }
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
    const parts = []
    for (const section of [...this.sections].sort((a, b) => a.order - b.order)) {
      try {
        const text = typeof section.text === 'function' ? section.text({}) : section.text
        if (typeof text === 'string' && text.trim()) parts.push(text.trim())
      } catch (error) {
        console.error(`[dsh-host] 提示词片段 ${section.name} 生成失败：${errorText(error)}`)
      }
    }
    return parts.join('\n\n')
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

const [command, target, extra] = process.argv.slice(2)
if (command === 'inspect' && target) {
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
