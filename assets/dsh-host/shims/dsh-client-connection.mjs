// Codex++ 兼容宿主提供的 @deepseek-ai/dsh-client-connection 替身（宿主半边）。
// 插件从这个包 import 的多是协议常量与 RPC 信封的校验器（自己在 webServer 路由里解析 /api 请求时用）；
// 连接服务本身由宿主经 ctx.connection 提供，这里不导出 HostConnectionService / Peer。
// 校验器只实现 zod 的 parse / safeParse 两个入口，信封形状与 dsh 的 rpc-schema 一致。

export const API_PATH = '/api'
export const CONNECT_PATH = '/connect'

/** RPC 关联 id 的品牌构造函数（运行时就是字符串本身）。 */
export const RpcId = (id) => id

class EnvelopeError extends Error {
  constructor(issues) {
    super(issues.join('; '))
    this.name = 'ZodError'
    this.issues = issues.map((message) => ({ message, path: [] }))
  }
}

/** 由「返回违规列表」的检查函数做出一个 zod 形状的校验器。 */
function schema(check) {
  return Object.freeze({
    safeParse(value) {
      const issues = check(value)
      return issues.length === 0 ? { success: true, data: value } : { success: false, error: new EnvelopeError(issues) }
    },
    parse(value) {
      const issues = check(value)
      if (issues.length > 0) throw new EnvelopeError(issues)
      return value
    },
  })
}

const isRecord = (value) => value !== null && typeof value === 'object' && !Array.isArray(value)

/** 插件传进来的值校验器可能是真 zod，也可能是这里的替身：都按 safeParse 判断。 */
function issuesOf(parser, value, path) {
  if (!parser || typeof parser.safeParse !== 'function') return []
  const result = parser.safeParse(value)
  return result?.success ? [] : [`${path} is invalid`]
}

const checkRpcId = (value, path = 'rpcId') => (typeof value === 'string' ? [] : [`${path} must be a string`])

function checkRpcError(value, path = 'error') {
  if (!isRecord(value)) return [`${path} must be an object`]
  const issues = []
  if (typeof value.code !== 'string') issues.push(`${path}.code must be a string`)
  if (typeof value.message !== 'string') issues.push(`${path}.message must be a string`)
  if (!isRecord(value.details)) issues.push(`${path}.details must be an object`)
  return issues
}

function checkResult(value, valueParser, path = 'result') {
  if (!isRecord(value)) return [`${path} must be an object`]
  if (value.ok === true) return issuesOf(valueParser, value.value, `${path}.value`)
  if (value.ok === false) return checkRpcError(value.error, `${path}.error`)
  return [`${path}.ok must be a boolean`]
}

function checkClientRequest(value) {
  if (!isRecord(value)) return ['request must be an object']
  const issues = []
  if (value.type !== 'client-request') issues.push('type must be "client-request"')
  issues.push(...checkRpcId(value.rpcId))
  if (typeof value.method !== 'string') issues.push('method must be a string')
  return issues
}

function checkServerResponse(value) {
  if (!isRecord(value)) return ['response must be an object']
  const issues = []
  if (value.type !== 'server-response') issues.push('type must be "server-response"')
  issues.push(...checkRpcId(value.rpcId))
  // dsh 的响应信封里 value 是 unknown().optional()：成功时任何值都接受。
  issues.push(...checkResult(value.result, undefined))
  return issues
}

export const rpcIdSchema = schema((value) => checkRpcId(value, 'value'))
export const rpcErrorSchema = schema((value) => checkRpcError(value, 'value'))
export const rpcResultSchema = (valueParser) => schema((value) => checkResult(value, valueParser, 'value'))
export const clientRequestSchema = schema(checkClientRequest)
export const serverResponseSchema = schema(checkServerResponse)
export const rpcMessageSchema = schema((value) => {
  if (isRecord(value) && value.type === 'client-request') return checkClientRequest(value)
  if (isRecord(value) && value.type === 'server-response') return checkServerResponse(value)
  return ['type must be "client-request" or "server-response"']
})

/** 传输层失败的响应值（同 dsh：code 为 gateway/internal）。 */
export function transportError(error) {
  return {
    ok: false,
    error: { code: 'gateway/internal', message: error instanceof Error ? error.message : String(error), details: {} },
  }
}
