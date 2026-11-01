// Codex++ 兼容宿主提供的 @deepseek-ai/dsh-credentials 替身：引用 / 键的构造与判断函数，语义同 dsh。
// 引用（CredentialRef）是环境变量名，键（CredentialKey）是 `<插件名>/<id>`，运行时都是字符串本身。
// 取值走宿主注入的 ctx.credentials。

const REF_PATTERN = /^[A-Za-z_][A-Za-z0-9_]*$/
const KEY_SEGMENT_PATTERN = /^[a-z][a-z0-9-]*$/

/** 原样字符串能不能当引用名（不合语法的读成「未设置」，而不是抛错）。 */
export function isCredentialRefName(value) {
  return typeof value === 'string' && REF_PATTERN.test(value)
}

export function credentialRef(value) {
  if (!isCredentialRefName(value)) throw new TypeError(`credential ref "${value}" must match ${String(REF_PATTERN)}`)
  return value
}

export function isCredentialKeySegment(value) {
  return typeof value === 'string' && KEY_SEGMENT_PATTERN.test(value)
}

export function credentialKey(scope, id) {
  for (const segment of [scope, id]) {
    if (!isCredentialKeySegment(segment)) throw new TypeError(`credential key segment "${segment}" must match ${String(KEY_SEGMENT_PATTERN)}`)
  }
  return `${scope}/${id}`
}

export function parseCredentialKey(value) {
  const segments = String(value).split('/')
  const [scope, id] = segments
  if (segments.length !== 2 || scope === undefined || id === undefined) throw new TypeError(`credential key "${value}" must be "<scope>/<id>"`)
  return credentialKey(scope, id)
}

export const credentialKeyScope = (key) => key.slice(0, key.indexOf('/'))
export const credentialKeyId = (key) => key.slice(key.indexOf('/') + 1)
