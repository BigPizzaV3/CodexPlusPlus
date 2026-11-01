// Codex++ 兼容宿主提供的 @deepseek-ai/dsh-storage-domain 替身：定义领域与表的构造函数。
// 打开领域走宿主注入的 ctx.storageDomain（每个领域一个 JSON 文件，见 services.mjs）。

export class DomainError extends Error {
  constructor(message, code) {
    super(message)
    this.name = 'DomainError'
    this.code = code
  }
}

export function domainTable(schema) {
  return Object.freeze({ kind: 'table', schema })
}

export function defineDomain(spec) {
  if (!spec || typeof spec.name !== 'string' || !spec.name) throw new TypeError('defineDomain needs a name')
  return Object.freeze({ version: 1, tables: {}, ...spec })
}
