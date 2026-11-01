// dsh 插件替身服务向 Codex++ 发起的宿主调用：读写 Codex 会话、调模型、在 Codex 页面里问用户。
//
// 插件跑在 Codex++ 起的 Node 进程里（dsh 插件 MCP、钩子常驻进程、拓展运行器），这些进程都带着环境变量
// CODEX_PLUS_HOST_EXE（Codex++ 可执行文件）。每次调用起一个 `--codex-plus-host-call` 子进程，stdin 写
// { service, method, params }，stdout 读 { result } 或 { error }。调用频率低（插件主动发起的会话 / 模型 / 提问），
// 进程开销可以接受，换来的是所有进程用同一条通道、不必各自维护连接。

import { spawn } from 'node:child_process'

const HOST_CALL_FLAG = '--codex-plus-host-call'
const DEFAULT_TIMEOUT_MS = 130_000
/** 提问要等用户回答，最长 10 分钟。 */
const ASK_TIMEOUT_MS = 610_000

export class HostCallUnavailable extends Error {
  constructor(message) {
    super(message)
    this.name = 'HostCallUnavailable'
    this.code = 'HOST_CALL_UNAVAILABLE'
  }
}

export function hostCallAvailable() {
  return typeof process.env.CODEX_PLUS_HOST_EXE === 'string' && process.env.CODEX_PLUS_HOST_EXE.length > 0
}

/** 调 Codex++ 宿主服务；出错时抛 Error（message 为宿主给的原因）。 */
export function hostCall(service, method, params, { signal, timeoutMs } = {}) {
  const exe = process.env.CODEX_PLUS_HOST_EXE
  if (!exe) return Promise.reject(new HostCallUnavailable('当前进程不是由 Codex++ 启动的，无法调用 Codex++ 宿主服务'))
  const limit = timeoutMs ?? (service === 'ui' && method === 'ask' ? ASK_TIMEOUT_MS : DEFAULT_TIMEOUT_MS)
  return new Promise((resolve, reject) => {
    if (signal?.aborted) {
      reject(signal.reason ?? new Error('aborted'))
      return
    }
    const child = spawn(exe, [HOST_CALL_FLAG], { stdio: ['pipe', 'pipe', 'ignore'], windowsHide: true })
    let output = ''
    let settled = false
    const finish = (error, value) => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      signal?.removeEventListener?.('abort', onAbort)
      if (error) reject(error)
      else resolve(value)
    }
    const onAbort = () => {
      child.kill()
      finish(signal.reason ?? new Error('aborted'))
    }
    const timer = setTimeout(() => {
      child.kill()
      finish(new Error(`Codex++ 宿主调用 ${service}.${method} 超时`))
    }, limit)
    signal?.addEventListener?.('abort', onAbort, { once: true })
    child.stdout.setEncoding('utf8')
    child.stdout.on('data', (chunk) => {
      output += chunk
    })
    child.on('error', (error) => finish(new HostCallUnavailable(`无法启动 Codex++ 宿主调用：${error.message}`)))
    child.on('close', () => {
      const line = output.trim().split('\n').pop() ?? ''
      let reply
      try {
        reply = JSON.parse(line)
      } catch {
        finish(new Error(`Codex++ 宿主调用 ${service}.${method} 没有返回结果`))
        return
      }
      if (reply && typeof reply.error === 'string') finish(new Error(reply.error))
      else finish(undefined, reply?.result)
    })
    child.stdin.end(JSON.stringify({ service, method, params: params ?? null }))
  })
}

/**
 * 行式流的宿主调用（目前是 llm.stream）：Codex++ 每收到一段增量就写一行 `{ delta }`，最后一行是 `{ result }` 或 `{ error }`。
 * 返回异步迭代器：逐个 yield 增量，迭代结束时的返回值（`next()` 的 `{ done: true, value }`）是最终结果；出错时抛出。
 */
export async function* hostCallStream(service, method, params, { signal, timeoutMs } = {}) {
  const exe = process.env.CODEX_PLUS_HOST_EXE
  if (!exe) throw new HostCallUnavailable('当前进程不是由 Codex++ 启动的，无法调用 Codex++ 宿主服务')
  if (signal?.aborted) throw signal.reason ?? new Error('aborted')
  const child = spawn(exe, [HOST_CALL_FLAG], { stdio: ['pipe', 'pipe', 'ignore'], windowsHide: true })
  const queue = []
  let wake = null
  let closed = false
  let failure
  let result
  let received = false
  const notify = () => {
    const resume = wake
    wake = null
    resume?.()
  }
  const handleLine = (line) => {
    let reply
    try {
      reply = JSON.parse(line)
    } catch {
      return
    }
    if (reply && Object.hasOwn(reply, 'delta')) queue.push(reply.delta)
    else if (reply && typeof reply.error === 'string') failure ??= new Error(reply.error)
    else if (reply && Object.hasOwn(reply, 'result')) {
      result = reply.result
      received = true
    }
    notify()
  }
  let buffer = ''
  child.stdout.setEncoding('utf8')
  child.stdout.on('data', (chunk) => {
    buffer += chunk
    for (let end = buffer.indexOf('\n'); end >= 0; end = buffer.indexOf('\n')) {
      const line = buffer.slice(0, end).trim()
      buffer = buffer.slice(end + 1)
      if (line) handleLine(line)
    }
  })
  child.on('error', (error) => {
    failure ??= new HostCallUnavailable(`无法启动 Codex++ 宿主调用：${error.message}`)
    closed = true
    notify()
  })
  child.on('close', () => {
    if (buffer.trim()) handleLine(buffer.trim())
    closed = true
    notify()
  })
  const timer = setTimeout(() => {
    failure ??= new Error(`Codex++ 宿主调用 ${service}.${method} 超时`)
    child.kill()
  }, timeoutMs ?? DEFAULT_TIMEOUT_MS)
  const onAbort = () => {
    failure ??= signal.reason ?? new Error('aborted')
    child.kill()
  }
  signal?.addEventListener?.('abort', onAbort, { once: true })
  child.stdin.end(JSON.stringify({ service, method, params: params ?? null, stream: true }))
  try {
    for (;;) {
      if (queue.length) {
        yield queue.shift()
        continue
      }
      if (closed) break
      await new Promise((resolve) => {
        wake = resolve
      })
    }
    if (failure) throw failure
    if (!received) throw new Error(`Codex++ 宿主调用 ${service}.${method} 没有返回结果`)
    return result
  } finally {
    clearTimeout(timer)
    signal?.removeEventListener?.('abort', onAbort)
    if (!closed) child.kill()
  }
}
