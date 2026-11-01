// Codex++ 兼容宿主提供的 @deepseek-ai/dsh-system-prompt 替身。
// 插件通常只从它取片段名常量与渲染函数；真正的注册走宿主注入的 ctx.systemPrompt，
// 片段由 Codex++ 生成的配套拓展包经网关追加到 instructions。

// 已发布的 0.x 版本用单个 persona 片段；新版拆成前缀 / 后缀，两套名字都给。
export const PERSONA_SECTION = 'deployment:persona'
export const PERSONA_ORDER = 0
export const PERSONA_PREFIX_SECTION = 'deployment:persona-prefix'
export const PERSONA_SUFFIX_SECTION = 'deployment:persona-suffix'
export const TOOL_ORDER_REST = '<unlisted-tools>'

const join = (sections) => sections
  .map((section) => (typeof section?.text === 'string' ? section.text : ''))
  .filter((text) => text.length > 0)
  .join('\n\n')

/** 宽松版：不做变量插值（宿主拼 instructions 时才插值）。 */
export function renderPrompt(assembly) {
  return join(assembly?.sections ?? [])
}

export function renderContextSections(assembly) {
  return (assembly?.contexts ?? []).filter((context) => typeof context?.text === 'string' && context.text.length > 0)
}

export function joinContextSections(sections) {
  return join(sections ?? [])
}

export function renderContextSnapshot(assembly) {
  return joinContextSections(renderContextSections(assembly))
}

/** dsh 的系统提示词服务在这里只是占位：插件拿到的是宿主注入的 ctx.systemPrompt。 */
export class SystemPrompt {
  constructor() {
    throw new Error('SystemPrompt is provided by the host as ctx.systemPrompt in Codex++')
  }
}

export default SystemPrompt
