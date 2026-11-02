// Codex++ 兼容宿主提供的 @deepseek-ai/dsh-skill-filesystem 替身：按目录扫描 SKILL.md。
// dsh 的文件系统 skill 提供方还会监听目录变化并默认扫描一批默认根目录；这里只扫 customSkillDirs，
// 不监听（插件改完目录自己调 control.invalidate()，宿主下次 list 时重新扫）。

import { readdir, readFile } from 'node:fs/promises'
import { join } from 'node:path'

function frontmatter(text) {
  const match = /^---\r?\n([\s\S]*?)\r?\n---/.exec(text)
  const fields = {}
  if (!match) return fields
  for (const line of match[1].split(/\r?\n/)) {
    const pair = /^([A-Za-z0-9_-]+):\s*(.*)$/.exec(line)
    if (pair) fields[pair[1]] = pair[2].replace(/^["']|["']$/g, '')
  }
  return fields
}

async function scan(dir, depth, found) {
  if (depth > 3) return
  let entries
  try {
    entries = await readdir(dir, { withFileTypes: true })
  } catch {
    return
  }
  for (const entry of entries) {
    if (entry.name.startsWith('.') || entry.name === 'node_modules') continue
    const full = join(dir, entry.name)
    if (entry.isDirectory()) {
      await scan(full, depth + 1, found)
    } else if (entry.name.toLowerCase() === 'skill.md') {
      let text = ''
      try {
        text = await readFile(full, 'utf8')
      } catch {
        continue
      }
      const fields = frontmatter(text)
      found.push({ name: fields.name || dir.split(/[\\/]/).pop(), description: fields.description ?? '', path: full })
    }
  }
}

export class FileSystemSkillProvider {
  constructor(ctx, control, options = {}) {
    this.ctx = ctx
    this.control = control
    this.options = options
    this.name = options.providerName ?? 'filesystem'
  }

  async list() {
    const found = []
    for (const dir of this.options.customSkillDirs ?? []) await scan(dir, 0, found)
    return { candidates: found }
  }

  async get(name) {
    return (await this.list()).candidates.find((candidate) => candidate.name === name)
  }

  dispose() {}
}

export default FileSystemSkillProvider
