// Codex++ 兼容宿主提供的 @deepseek-ai/dsh-tools 替身。
// 插件包把 dsh-tools 声明为 peer 依赖、由宿主提供；Codex++ 不带 dsh 运行时，这里实现插件真正会用到的部分：
// defineTool（参数属性表 → JSON Schema，执行前做轻量校验）与几个常量。语义对齐 dsh 的 defineTool：
// execute 返回规范值，output.render 再把值转成给模型的内容块。

import { HarnessError } from './dsh-llm.mjs'

export const RUN_CODE_NAME = 'run_code'
export const TOOL_ABORTED = 'ABORTED'
export const TOOL_ABORTED_BEFORE_DISPATCH = 'ABORTED_BEFORE_DISPATCH'

const ANNOTATIONS = ['description', 'title', 'default', 'examples']

export class ToolArgsError extends Error {
  constructor(violations) {
    super(`invalid tool arguments: ${violations.join('; ')}`)
    this.name = 'ToolArgsError'
    this.code = 'INVALID_ARGS'
    this.violations = violations
  }
}

/** 值 schema 规格 → JSON Schema 节点。 */
export function valueSchemaSpecToJsonSchema(spec, path = 'value') {
  if (spec === null || typeof spec !== 'object') throw new Error(`${path} must be a schema object`)
  const node = {}
  for (const key of ANNOTATIONS) if (spec[key] !== undefined) node[key] = spec[key]
  if (Array.isArray(spec.oneOf)) {
    node.oneOf = spec.oneOf.map((item, index) => valueSchemaSpecToJsonSchema(item, `${path}.oneOf[${index}]`))
    return node
  }
  if (spec.enum !== undefined) node.enum = [...spec.enum]
  if (spec.const !== undefined) node.const = spec.const
  switch (spec.type) {
    case 'json':
      return node
    case 'string':
    case 'number':
    case 'integer':
    case 'boolean':
    case 'null':
      node.type = spec.type
      return node
    case 'array':
      node.type = 'array'
      if (spec.items !== undefined) node.items = valueSchemaSpecToJsonSchema(spec.items, `${path}.items`)
      return node
    case 'object': {
      node.type = 'object'
      const compiled = compilePropertyMap(spec.properties ?? {}, `${path}.properties`)
      node.properties = compiled.properties
      if (compiled.required.length > 0) node.required = compiled.required
      node.additionalProperties = spec.additionalProperties === true
      return node
    }
    default:
      throw new Error(`${path}: unsupported schema type ${JSON.stringify(spec.type)}`)
  }
}

function compilePropertyMap(spec, path) {
  const properties = {}
  const required = []
  for (const [key, value] of Object.entries(spec ?? {})) {
    const { required: isRequired, ...rest } = value ?? {}
    properties[key] = valueSchemaSpecToJsonSchema(rest, `${path}.${key}`)
    if (isRequired === true) required.push(key)
  }
  return { properties, required }
}

/** 参数属性表 → 顶层 object JSON Schema。 */
export function parameterSchemaSpecToJsonSchema(spec) {
  const compiled = compilePropertyMap(spec, 'parameters')
  return {
    type: 'object',
    properties: compiled.properties,
    ...(compiled.required.length > 0 ? { required: compiled.required } : {}),
  }
}

/** 按 JSON Schema 子集校验，返回违规描述列表。 */
export function validateJsonSchemaValue(schema, value, path = '') {
  const at = path || '(root)'
  if (!schema || typeof schema !== 'object') return []
  if (Array.isArray(schema.oneOf)) {
    const ok = schema.oneOf.some((option) => validateJsonSchemaValue(option, value, path).length === 0)
    return ok ? [] : [`${at} does not match any allowed shape`]
  }
  if (schema.const !== undefined && !Object.is(schema.const, value)) return [`${at} must be ${JSON.stringify(schema.const)}`]
  if (Array.isArray(schema.enum) && !schema.enum.some((item) => Object.is(item, value))) {
    return [`${at} must be one of ${schema.enum.map((item) => JSON.stringify(item)).join(', ')}`]
  }
  switch (schema.type) {
    case 'string':
      return typeof value === 'string' ? [] : [`${at} must be a string`]
    case 'number':
      return typeof value === 'number' && Number.isFinite(value) ? [] : [`${at} must be a number`]
    case 'integer':
      return Number.isInteger(value) ? [] : [`${at} must be an integer`]
    case 'boolean':
      return typeof value === 'boolean' ? [] : [`${at} must be a boolean`]
    case 'null':
      return value === null ? [] : [`${at} must be null`]
    case 'array': {
      if (!Array.isArray(value)) return [`${at} must be an array`]
      if (!schema.items) return []
      return value.flatMap((item, index) => validateJsonSchemaValue(schema.items, item, `${path}[${index}]`))
    }
    case 'object': {
      if (value === null || typeof value !== 'object' || Array.isArray(value)) return [`${at} must be an object`]
      const violations = []
      for (const key of schema.required ?? []) {
        if (value[key] === undefined) violations.push(`${path ? `${path}.` : ''}${key} is required`)
      }
      for (const [key, item] of Object.entries(value)) {
        const child = schema.properties?.[key]
        if (child) violations.push(...validateJsonSchemaValue(child, item, path ? `${path}.${key}` : key))
        else if (schema.additionalProperties === false) violations.push(`${path ? `${path}.` : ''}${key} is not allowed`)
      }
      return violations
    }
    default:
      return []
  }
}

export function defineTool(options) {
  if (!options || typeof options.name !== 'string' || options.name.length === 0) throw new Error('defineTool: name is required')
  if (typeof options.execute !== 'function') throw new Error(`defineTool(${options.name}): execute is required`)
  if (!options.output || typeof options.output.render !== 'function') {
    throw new Error(`defineTool(${options.name}): output.render is required`)
  }
  if (options.timeoutMs !== undefined && (!Number.isFinite(options.timeoutMs) || options.timeoutMs <= 0)) {
    throw new Error(`defineTool(${options.name}): timeoutMs must be a positive finite number`)
  }
  const parameters = parameterSchemaSpecToJsonSchema(options.parameters ?? {})
  const outputSchema = options.output.schema === undefined
    ? {}
    : valueSchemaSpecToJsonSchema(options.output.schema, 'output')
  const validate = (args) => validateJsonSchemaValue(parameters, args ?? {}, '')
  const tool = {
    name: options.name,
    description: options.description ?? '',
    parameters,
    output: {
      schema: outputSchema,
      render: (args, value) => options.output.render(args, value),
      ...(options.output.presentationMeta ? { presentationMeta: (args, value) => options.output.presentationMeta(args, value) } : {}),
    },
    ...(options.deferLoading === true ? { deferLoading: true } : {}),
    ...(options.timeoutMs !== undefined ? { timeoutMs: options.timeoutMs } : {}),
    async execute(args, exec) {
      const violations = validate(args)
      if (violations.length > 0) throw new ToolArgsError(violations)
      return options.execute(args, exec)
    },
  }
  if (options.isConcurrencySafe) tool.isConcurrencySafe = (args) => validate(args).length === 0 && options.isConcurrencySafe(args)
  return tool
}

/** dsh 的工具注册表类型在这里只是占位：插件拿到的是宿主注入的 ctx.tools。 */
export class ToolRegistry {
  constructor() {
    throw new Error('ToolRegistry is provided by the host as ctx.tools in Codex++')
  }
}

// ---------------------------------------------------------------------------
// JSON Schema 子集的检查（同 dsh 的 json-schema.ts：结构化输出、子 agent、工作流共用）。
// 子集：单一 type / oneOf（至少两项）/ properties / required / 布尔 additionalProperties / items /
// 与类型相符的 enum、const，外加 description / title / default / examples 注解。不认的关键字一律报违规。
// ---------------------------------------------------------------------------

/** schema 超出子集时抛出；violations 列出每一处违规（不在第一处停下）。 */
export class JsonSchemaError extends HarnessError {
  constructor(violations) {
    super(`unsupported JSON schema: ${violations.join('; ')}`, 'UNSUPPORTED_SCHEMA')
    this.name = 'JsonSchemaError'
    this.violations = violations
  }
}

const CONSTRAINT_KEYWORDS = new Set(['type', 'oneOf', 'properties', 'required', 'additionalProperties', 'items', 'enum', 'const'])
const ANNOTATION_KEYWORDS = new Set(ANNOTATIONS)
const SCHEMA_TYPES = ['object', 'array', 'string', 'number', 'integer', 'boolean', 'null']
const ONE_OF_SIBLING_KEYWORDS = ['properties', 'required', 'additionalProperties', 'items', 'enum', 'const']

export function isPlainJsonRecord(value) {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return false
  const prototype = Object.getPrototypeOf(value)
  return prototype === null || prototype === Object.prototype
}

/** 普通对象且只有自有、可枚举的字符串键（JSON 化后不会丢东西）。 */
export function isJsonSchemaRecord(value) {
  if (!isPlainJsonRecord(value)) return false
  return Reflect.ownKeys(value).every((key) => typeof key === 'string' && Object.prototype.propertyIsEnumerable.call(value, key))
}

/** 稠密、没有额外属性的普通数组。 */
export function isPlainJsonArray(value) {
  if (!Array.isArray(value) || Object.getPrototypeOf(value) !== Array.prototype) return false
  if (Reflect.ownKeys(value).length !== value.length + 1) return false
  for (let index = 0; index < value.length; index++) if (!Object.hasOwn(value, index)) return false
  return true
}

const isJsonNumber = (value) => typeof value === 'number' && Number.isFinite(value) && !Object.is(value, -0)

function scalarMatches(type, value) {
  switch (type) {
    case 'string': return typeof value === 'string'
    case 'number': return isJsonNumber(value)
    case 'integer': return isJsonNumber(value) && Number.isInteger(value)
    case 'boolean': return typeof value === 'boolean'
    default: return value === null
  }
}

function isJsonData(value, seen = new Set()) {
  if (value === null || typeof value === 'string' || typeof value === 'boolean') return true
  if (typeof value === 'number') return isJsonNumber(value)
  if (typeof value !== 'object' || seen.has(value)) return false
  seen.add(value)
  const ok = Array.isArray(value)
    ? isPlainJsonArray(value) && value.every((item) => isJsonData(item, seen))
    : isJsonSchemaRecord(value) && Object.values(value).every((item) => isJsonData(item, seen))
  seen.delete(value)
  return ok
}

function checkSchemaNode(node, path, violations, seen) {
  if (!isJsonSchemaRecord(node)) {
    violations.push(`${path} must be a schema object`)
    return
  }
  if (seen.has(node)) {
    violations.push(`${path} is circular`)
    return
  }
  seen.add(node)
  try {
    for (const key of Object.keys(node)) {
      if (CONSTRAINT_KEYWORDS.has(key)) continue
      if (ANNOTATION_KEYWORDS.has(key)) {
        if (!isJsonData(node[key])) violations.push(`${path}.${key} annotation must be lossless JSON data`)
        continue
      }
      violations.push(`${path}.${key} is not a supported keyword (subset: type/oneOf/properties/required/additionalProperties/items/enum/const + annotations)`)
    }
    if (Object.hasOwn(node, 'description') && typeof node.description !== 'string') violations.push(`${path}.description must be a string`)
    if (Object.hasOwn(node, 'title') && typeof node.title !== 'string') violations.push(`${path}.title must be a string`)
    const hasType = Object.hasOwn(node, 'type')
    const hasOneOf = Object.hasOwn(node, 'oneOf')
    if (hasType && hasOneOf) {
      violations.push(`${path} cannot declare both type and oneOf`)
      return
    }
    if (!hasType && !hasOneOf) {
      for (const key of ONE_OF_SIBLING_KEYWORDS) if (Object.hasOwn(node, key)) violations.push(`${path}.${key} requires type or oneOf`)
      return
    }
    if (hasOneOf) {
      const oneOf = node.oneOf
      if (!isPlainJsonArray(oneOf) || oneOf.length < 2) violations.push(`${path}.oneOf must be an array of at least two schemas`)
      else oneOf.forEach((child, index) => checkSchemaNode(child, `${path}.oneOf[${index}]`, violations, seen))
      for (const key of ONE_OF_SIBLING_KEYWORDS) if (Object.hasOwn(node, key)) violations.push(`${path}.${key} is not supported beside oneOf`)
      return
    }
    const type = node.type
    if (typeof type !== 'string' || !SCHEMA_TYPES.includes(type)) {
      violations.push(Array.isArray(type) ? `${path}.type must be a single type string (type arrays are not supported)` : `${path}.type must be one of ${SCHEMA_TYPES.join('/')}`)
      return
    }
    const scalars = ['string', 'number', 'integer', 'boolean', 'null']
    const placement = { properties: ['object'], required: ['object'], additionalProperties: ['object'], items: ['array'], enum: scalars, const: scalars }
    for (const [key, types] of Object.entries(placement)) {
      if (Object.hasOwn(node, key) && !types.includes(type)) violations.push(`${path}.${key} is not supported on type "${type}"`)
    }
    if (type === 'object') {
      const properties = Object.hasOwn(node, 'properties') ? node.properties : undefined
      if (Object.hasOwn(node, 'properties')) {
        if (!isJsonSchemaRecord(properties)) violations.push(`${path}.properties must be an object of schemas`)
        else for (const [key, child] of Object.entries(properties)) checkSchemaNode(child, `${path}.properties.${key}`, violations, seen)
      }
      if (Object.hasOwn(node, 'required')) {
        const required = node.required
        if (!isPlainJsonArray(required) || required.some((entry) => typeof entry !== 'string')) violations.push(`${path}.required must be an array of strings`)
        else {
          const declared = isJsonSchemaRecord(properties) ? properties : {}
          for (const key of required) if (!Object.hasOwn(declared, key)) violations.push(`${path}.required names "${key}" which is not in properties`)
        }
      }
      if (Object.hasOwn(node, 'additionalProperties') && typeof node.additionalProperties !== 'boolean') violations.push(`${path}.additionalProperties must be a boolean`)
    } else if (type === 'array') {
      if (Object.hasOwn(node, 'items')) checkSchemaNode(node.items, `${path}.items`, violations, seen)
    } else {
      const hasEnum = Object.hasOwn(node, 'enum')
      const allowed = hasEnum ? node.enum : undefined
      const enumValid = isPlainJsonArray(allowed) && allowed.length > 0 && allowed.every((entry) => scalarMatches(type, entry))
      if (hasEnum && !enumValid) violations.push(`${path}.enum must be a non-empty array of ${type} values`)
      if (Object.hasOwn(node, 'const')) {
        if (!scalarMatches(type, node.const)) violations.push(`${path}.const must be a ${type} value`)
        else if (enumValid && !allowed.includes(node.const)) violations.push(`${path}.const must be one of ${path}.enum when both are declared`)
      }
    }
  } finally {
    seen.delete(node)
  }
}

/** schema 只用子集里的写法（只有注解的 schema 视作「任意 JSON」）。 */
export function assertSupportedJsonSchema(schema) {
  const violations = []
  checkSchemaNode(schema, 'schema', violations, new Set())
  if (violations.length > 0) throw new JsonSchemaError(violations)
}

/** 子集之外再要求根是 object（结构化输出用）。 */
export function assertObjectJsonSchema(schema) {
  const violations = []
  checkSchemaNode(schema, 'schema', violations, new Set())
  if (violations.length === 0 && (!isJsonSchemaRecord(schema) || schema.type !== 'object')) {
    violations.push('schema.type must be "object" (structured output is object-rooted)')
  }
  if (violations.length > 0) throw new JsonSchemaError(violations)
}

// PTC 模式（模型写程序调用工具）的 SDK 文本：只在 dsh 装了 ptcRuntime、提示词里有 tools:sdk 段时才会生成。
// Codex 没有 run_code 与 PTC 运行时，这一步不会走到；真被调用时说明原因，而不是给出一段 Codex 用不上的 SDK。
const NO_PTC = 'Codex++ 里没有 run_code（PTC 模式）运行时，不生成工具 SDK 文本'
export function renderToolsSdk() {
  throw new Error(NO_PTC)
}
export function renderToolsSdkPy() {
  throw new Error(NO_PTC)
}

export default { defineTool }
