// Codex++ 兼容宿主提供的 @deepseek-ai/schemastery 替身。
// 插件用它声明 Config；宿主只需要「按 schema 填默认值、拒绝缺失的必填项」，不做完整的类型校验。
// 所有修饰方法（min / max / pattern / description / role …）都接受并返回自身，未知方法同样如此。

const META = Symbol('codexpp.schema')

function clone(value) {
  if (value === undefined || value === null || typeof value !== 'object') return value
  return JSON.parse(JSON.stringify(value))
}

function makeSchema(kind, extra = {}) {
  const meta = { kind, required: false, default: undefined, ...extra }
  const schema = function (value) {
    return resolve(meta, value, '')
  }
  const target = Object.assign(schema, {
    [META]: meta,
    default(value) {
      meta.default = value
      return proxy
    },
    required(value = true) {
      meta.required = value !== false
      return proxy
    },
    toJSON() {
      return { type: meta.kind }
    },
  })
  const proxy = new Proxy(target, {
    get(object, key) {
      if (key in object) return object[key]
      if (typeof key === 'symbol' || key === 'then') return undefined
      // 其余修饰器（description、min、max、pattern、role、hidden、i18n、step…）都无副作用。
      return () => proxy
    },
  })
  return proxy
}

function resolve(meta, value, path) {
  const at = path || 'config'
  if (value === undefined || value === null) {
    if (meta.default !== undefined) value = clone(meta.default)
    else if (meta.kind === 'object') value = {}
    else if (meta.required) throw new TypeError(`${at} is required`)
    else return value
  }
  switch (meta.kind) {
    case 'object': {
      if (typeof value !== 'object' || Array.isArray(value)) throw new TypeError(`${at} must be an object`)
      const result = { ...value }
      for (const [key, child] of Object.entries(meta.dict ?? {})) {
        const childMeta = child?.[META]
        result[key] = childMeta ? resolve(childMeta, result[key], path ? `${path}.${key}` : key) : result[key]
        if (result[key] === undefined) delete result[key]
      }
      return result
    }
    case 'array':
      if (!Array.isArray(value)) throw new TypeError(`${at} must be an array`)
      return meta.inner?.[META] ? value.map((item, index) => resolve(meta.inner[META], item, `${at}[${index}]`)) : value
    case 'dict':
      if (typeof value !== 'object' || Array.isArray(value)) throw new TypeError(`${at} must be an object`)
      return value
    case 'union': {
      // 依次尝试各分支，取第一个不报错的结果。
      let lastError
      for (const option of meta.list ?? []) {
        const optionMeta = option?.[META]
        if (!optionMeta) {
          if (Object.is(option, value)) return value
          continue
        }
        try {
          return resolve(optionMeta, value, path)
        } catch (error) {
          lastError = error
        }
      }
      if (lastError) throw lastError
      return value
    }
    case 'const':
      if (!Object.is(meta.value, value)) throw new TypeError(`${at} must be ${JSON.stringify(meta.value)}`)
      return value
    case 'string':
      if (typeof value !== 'string') throw new TypeError(`${at} must be a string`)
      return value
    case 'number':
    case 'natural':
    case 'percent':
      if (typeof value !== 'number') throw new TypeError(`${at} must be a number`)
      return value
    case 'boolean':
      if (typeof value !== 'boolean') throw new TypeError(`${at} must be a boolean`)
      return value
    default:
      return value
  }
}

function Schema(options) {
  return makeSchema('any', { options })
}

Object.assign(Schema, {
  any: () => makeSchema('any'),
  never: () => makeSchema('never'),
  string: () => makeSchema('string'),
  number: () => makeSchema('number'),
  natural: () => makeSchema('natural'),
  percent: () => makeSchema('percent'),
  boolean: () => makeSchema('boolean'),
  date: () => makeSchema('any'),
  bitset: () => makeSchema('any'),
  function: () => makeSchema('any'),
  is: () => makeSchema('any'),
  const: (value) => makeSchema('const', { value }),
  array: (inner) => makeSchema('array', { inner }),
  tuple: () => makeSchema('any'),
  dict: (inner) => makeSchema('dict', { inner }),
  object: (dict) => makeSchema('object', { dict }),
  union: (list) => makeSchema('union', { list }),
  intersect: (list) => makeSchema('any', { list }),
  transform: (inner) => inner,
  computed: (inner) => inner,
  from: () => makeSchema('any'),
})

export default Schema
export { Schema }
