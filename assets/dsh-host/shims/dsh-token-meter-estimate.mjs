// Codex++ 兼容宿主提供的 @deepseek-ai/dsh-token-meter/estimate 替身。
// 与 dsh 的固定密度估算一致（每 4 个字符 1 个 token，每块 4 个结构开销、每条消息 4 个角色开销），纯函数，
// 插件用它给消息 / 内容块粗估 token 数；宿主的 ctx.tokenMeter 也按字数粗估。

const CHARS_PER_TOKEN = 4
const BLOCK_OVERHEAD = 4
export const ROLE_OVERHEAD = 4

/** 内容块 JSON 结构的估价（图片块不计下放标记）。 */
export function estimateStructuralBlock(block) {
  if (block.type === 'image') {
    const { offloaded: _offloaded, ...reference } = block
    return BLOCK_OVERHEAD + Math.ceil(JSON.stringify(reference).length / CHARS_PER_TOKEN)
  }
  return BLOCK_OVERHEAD + Math.ceil(JSON.stringify(block).length / CHARS_PER_TOKEN)
}

export function estimateContent(blocks) {
  let tokens = 0
  for (const block of blocks) {
    switch (block.type) {
      case 'text':
      case 'reasoning':
        tokens += Math.ceil(block.text.length / CHARS_PER_TOKEN) + BLOCK_OVERHEAD
        break
      case 'tool-call':
        tokens += Math.ceil(block.name.length / CHARS_PER_TOKEN) + Math.ceil(block.arguments.length / CHARS_PER_TOKEN) + BLOCK_OVERHEAD
        break
      default:
        tokens += estimateStructuralBlock(block)
    }
  }
  return tokens
}

/** 系统提示词按纯文本计价，没有每块开销；空内容为 0。 */
export function estimateSystemMessage(message) {
  if (message.content.length === 0) return 0
  let characters = 0
  for (const block of message.content) {
    characters += block.type === 'text' ? block.text.length : JSON.stringify(block).length
  }
  return Math.ceil(characters / CHARS_PER_TOKEN) + ROLE_OVERHEAD
}

export function estimateMessage(message) {
  if (message.role === 'system') return estimateSystemMessage(message)
  return estimateContent(message.content) + ROLE_OVERHEAD
}

/** 请求信封里工具 schema 的估价；没有工具时为 0。 */
export function estimateToolsTokens(header) {
  if (header?.tools === undefined || header.tools.length === 0) return 0
  return Math.ceil(JSON.stringify(header.tools).length / CHARS_PER_TOKEN) + BLOCK_OVERHEAD
}
