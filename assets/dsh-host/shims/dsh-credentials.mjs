// Codex++ 兼容宿主提供的 @deepseek-ai/dsh-credentials 替身：引用构造函数。取值走宿主注入的 ctx.credentials。

const ENV_NAME = /^[A-Za-z_][A-Za-z0-9_]*$/

export function credentialRef(name) {
  if (typeof name !== 'string' || !ENV_NAME.test(name)) throw new TypeError(`invalid credential reference: ${name}`)
  return Object.freeze({ kind: 'env', name })
}

export function credentialKey(owner, id) {
  if (typeof owner !== 'string' || !owner || typeof id !== 'string' || !id) throw new TypeError('credentialKey needs owner and id')
  return `${owner}:${id}`
}
