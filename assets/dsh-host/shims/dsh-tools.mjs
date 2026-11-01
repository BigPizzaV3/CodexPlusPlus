// Codex++ 兼容宿主提供的 @deepseek-ai/dsh-tools 替身。
// 插件包把 dsh-tools 声明为 peer 依赖、由宿主提供；Codex++ 不带 dsh 运行时，这里实现插件真正会用到的部分：
// defineTool（参数属性表 → JSON Schema，执行前做轻量校验）与几个常量。语义对齐 dsh 的 defineTool：
// execute 返回规范值，output.render 再把值转成给模型的内容块。

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

export default { defineTool }
