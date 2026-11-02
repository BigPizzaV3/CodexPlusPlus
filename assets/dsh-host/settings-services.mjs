// Codex++ 兼容宿主的 settings / loader / sessionPersistence 替身。
// settings：按命名空间（默认是插件自己那一行的 id）存一份 { value, revision }，初始值取该行的配置，
//   改动经 mutate / update 原子写到插件数据目录的 settings/ 下；管理器里改配置不会同步回来。
// loader：只告诉插件宿主里有哪些行。
// sessionPersistence：Codex 的会话日志不在 Codex++ 手里，findLog 一律说没有，插件就不会去删 Codex 的目录。

import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { SettingsConflictError } from './shims/dsh-settings.mjs'

const UNSAFE_KEYS = new Set(['__proto__', 'constructor', 'prototype'])
const SECRET_KEY = /key|token|secret|password/i
const clone = (value) => (value === undefined ? undefined : JSON.parse(JSON.stringify(value)))

function applyOp(target, op) {
  const path = Array.isArray(op?.path) ? op.path.map(String) : []
  if (path.length === 0 || path.some((key) => UNSAFE_KEYS.has(key))) throw new TypeError('settings 操作的 path 不合法')
  let node = target
  for (const key of path.slice(0, -1)) {
    if (node[key] === null || typeof node[key] !== 'object') node[key] = {}
    node = node[key]
  }
  const last = path[path.length - 1]
  if (op.op === 'set') node[last] = clone(op.value)
  else if (op.op === 'unset' || op.op === 'delete') delete node[last]
  else throw new TypeError(`不认识的 settings 操作 ${op.op}`)
}

function redact(value) {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) return clone(value)
  return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, SECRET_KEY.test(key) && typeof item === 'string' && item ? '********' : clone(item)]))
}

export function createSettings({ dataDir, rowId, config, emit = () => {} }) {
  const spaces = new Map()
  const fileOf = (ns) => (dataDir ? join(dataDir, 'settings', `${String(ns).replace(/[^\w.-]/g, '_')}.json`) : undefined)
  const load = (ns) => {
    const name = String(ns)
    if (spaces.has(name)) return spaces.get(name)
    let space
    const file = fileOf(name)
    if (file && existsSync(file)) {
      try {
        space = JSON.parse(readFileSync(file, 'utf8'))
      } catch {}
    }
    if (!space && name === rowId) space = { value: clone(config ?? {}), revision: 1 }
    if (space) spaces.set(name, space)
    return space
  }
  const save = (ns, space) => {
    const file = fileOf(ns)
    if (!file) return
    mkdirSync(dirname(file), { recursive: true })
    const temp = `${file}.${process.pid}.tmp`
    writeFileSync(temp, JSON.stringify(space))
    renameSync(temp, file)
  }
  const mutate = async (ns, ops, expectedRevision) => {
    const name = String(ns)
    const space = load(name)
    if (!space) throw new Error(`settings namespace "${name}" 不存在`)
    if (expectedRevision !== undefined && expectedRevision !== space.revision) {
      throw new SettingsConflictError(`settings ${name} 的版本已经是 ${space.revision}，不是 ${expectedRevision}`, { ns: name, revision: space.revision })
    }
    const next = clone(space.value) ?? {}
    for (const op of Array.isArray(ops) ? ops : []) applyOp(next, op)
    space.value = next
    space.revision += 1
    save(name, space)
    emit('settings/changed', [{ ns: name, revision: space.revision }])
    return { value: clone(space.value), revision: space.revision }
  }
  return {
    get: (ns) => clone(load(ns)?.value),
    describe({ redactSecrets = false } = {}) {
      load(rowId)
      return [...spaces].map(([ns, space]) => ({ ns, revision: space.revision, schema: { uid: ns }, value: redactSecrets ? redact(space.value) : clone(space.value) }))
    },
    // 设置页策略（auto: false 表示插件自带设置页）；宿主不生成表单，只记下来，返回撤销函数。
    configure: () => () => {},
    mutate,
    async update(ns, patch, expectedRevision) {
      const ops = Object.entries(patch ?? {}).map(([key, value]) => ({ op: value === undefined ? 'unset' : 'set', path: [key], value }))
      return mutate(ns, ops, expectedRevision)
    },
  }
}

export function createSessionPersistence() {
  return { async findLog() { return '' } }
}

/** entries：返回当前所有行的函数，每项形如 { options: { id, name, config }, fiber, disabled }。 */
export function createLoader({ entries, baseUrl = 'http://127.0.0.1' }) {
  return {
    entries: () => [...entries()],
    ctx: { baseUrl },
    await: () => Promise.resolve(),
  }
}
