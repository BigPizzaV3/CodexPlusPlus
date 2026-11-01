// Codex++ 兼容宿主提供的 @deepseek-ai/dsh-skill 替身。
// skill 包把它声明为 peer 依赖，只用到几个常量与纯函数；真正的注册走宿主注入的 ctx.skills，
// 宿主把注册的 skill 写成 SKILL.md 目录交给 Codex Skills。

export const BUNDLED_SKILL_RANK = 600

const SKILL_NAME = /^[a-z0-9]+(?:-[a-z0-9]+)*$/

export function isSkillName(name) {
  return SKILL_NAME.test(name)
}

export function isModelInvocable(skill) {
  return skill?.invocation?.modelInvocable !== false
}

export function isUserInvocable(skill) {
  return skill?.invocation?.userInvocable !== false
}

export function escapeText(value) {
  return String(value).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}

export function renderSkillContent(skill) {
  return skill.content
}

/** dsh 的 skill 注册表在这里只是占位：插件拿到的是宿主注入的 ctx.skills。 */
export class SkillRegistry {
  constructor() {
    throw new Error('SkillRegistry is provided by the host as ctx.skills in Codex++')
  }
}

export default SkillRegistry
