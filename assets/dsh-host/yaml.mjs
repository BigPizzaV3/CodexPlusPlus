// bundle patch（cordis.patch.yml）用到的 YAML 子集解析器。
// 支持：块映射、块序列（含 `- key: value` 形式）、引号/普通标量、简单的 `[..]` / `{..}` 流式集合、`|` / `>` 块标量。
// 不支持锚点、别名、多文档；遇到 `!!js` 等标签保留为 { tag, value } 交给调用方诊断。

export class YamlError extends Error {
  constructor(message, line) {
    super(line ? `${message}（第 ${line} 行）` : message)
    this.name = 'YamlError'
  }
}

function stripComment(text) {
  let quote = null
  for (let index = 0; index < text.length; index++) {
    const char = text[index]
    if (quote) {
      if (char === '\\' && quote === '"') index++
      else if (char === quote) quote = null
    } else if (char === '"' || char === "'") {
      quote = char
    } else if (char === '#' && (index === 0 || /\s/.test(text[index - 1]))) {
      return text.slice(0, index).trimEnd()
    }
  }
  return text.trimEnd()
}

function isSeqItem(text) {
  return text === '-' || text.startsWith('- ')
}

/** 找出 `key: value` 里键结束的位置；不是映射项返回 -1。 */
function entrySplit(text) {
  if (text.startsWith('"') || text.startsWith("'")) {
    const quote = text[0]
    let index = 1
    while (index < text.length) {
      if (quote === '"' && text[index] === '\\') index += 2
      else if (text[index] === quote) {
        if (quote === "'" && text[index + 1] === "'") index += 2
        else break
      } else index++
    }
    const after = text.slice(index + 1)
    return /^\s*:(\s|$)/.test(after) ? index + 1 + after.indexOf(':') : -1
  }
  if (/^[[{]/.test(text)) return -1
  const match = /:(\s|$)/.exec(text)
  return match ? match.index : -1
}

function parseKey(raw) {
  const key = raw.trim()
  if (key.startsWith('"') || key.startsWith("'")) return String(parseScalar(key, 0))
  return key
}

export function parseScalar(raw, line) {
  const text = raw.trim()
  if (text === '' || text === '~' || text === 'null' || text === 'Null' || text === 'NULL') return null
  if (text.startsWith('&') || text.startsWith('*')) throw new YamlError('不支持 YAML 锚点或别名', line)
  if (text.startsWith('!')) {
    const space = text.search(/\s/)
    const tag = space === -1 ? text : text.slice(0, space)
    return { tag, value: space === -1 ? '' : text.slice(space + 1).trim() }
  }
  if (text.startsWith('"')) {
    if (!text.endsWith('"') || text.length < 2) throw new YamlError('双引号字符串未闭合', line)
    try {
      return JSON.parse(text)
    } catch {
      return text.slice(1, -1)
    }
  }
  if (text.startsWith("'")) {
    if (!text.endsWith("'") || text.length < 2) throw new YamlError('单引号字符串未闭合', line)
    return text.slice(1, -1).replace(/''/g, "'")
  }
  if (text.startsWith('[') || text.startsWith('{')) return parseFlow(text, line)
  if (/^(true|True|TRUE)$/.test(text)) return true
  if (/^(false|False|FALSE)$/.test(text)) return false
  if (/^[-+]?\d+$/.test(text)) return Number.parseInt(text, 10)
  if (/^[-+]?(\d+\.\d*|\.\d+|\d+)([eE][-+]?\d+)?$/.test(text)) return Number.parseFloat(text)
  return text
}

function parseFlow(text, line) {
  let index = 0
  const skip = () => {
    while (index < text.length && /\s/.test(text[index])) index++
  }
  const readPlain = (stops) => {
    const start = index
    while (index < text.length && !stops.includes(text[index])) index++
    return text.slice(start, index)
  }
  const readQuoted = () => {
    const quote = text[index]
    const start = index
    index++
    while (index < text.length) {
      if (quote === '"' && text[index] === '\\') index += 2
      else if (text[index] === quote) {
        if (quote === "'" && text[index + 1] === "'") index += 2
        else {
          index++
          break
        }
      } else index++
    }
    return parseScalar(text.slice(start, index), line)
  }
  const value = (stops) => {
    skip()
    if (text[index] === '[') return seq()
    if (text[index] === '{') return map()
    if (text[index] === '"' || text[index] === "'") return readQuoted()
    return parseScalar(readPlain(stops), line)
  }
  const seq = () => {
    index++
    const items = []
    skip()
    if (text[index] === ']') {
      index++
      return items
    }
    while (index < text.length) {
      items.push(value([',', ']']))
      skip()
      if (text[index] === ',') index++
      else if (text[index] === ']') {
        index++
        return items
      } else break
    }
    throw new YamlError('流式序列未闭合', line)
  }
  const map = () => {
    index++
    const result = {}
    skip()
    if (text[index] === '}') {
      index++
      return result
    }
    while (index < text.length) {
      skip()
      const key = text[index] === '"' || text[index] === "'" ? String(readQuoted()) : readPlain([':', ',', '}']).trim()
      skip()
      if (text[index] !== ':') throw new YamlError('流式映射缺少冒号', line)
      index++
      result[key] = value([',', '}'])
      skip()
      if (text[index] === ',') index++
      else if (text[index] === '}') {
        index++
        return result
      } else break
    }
    throw new YamlError('流式映射未闭合', line)
  }
  const result = value([])
  skip()
  if (index < text.length) throw new YamlError('流式集合后有多余内容', line)
  return result
}

export function parseYaml(source) {
  const lines = []
  const raw = String(source).replace(/^﻿/, '').split(/\r?\n/)
  for (let number = 0; number < raw.length; number++) {
    const line = raw[number]
    if (/^\s*(---|\.\.\.)\s*$/.test(line)) {
      if (lines.length > 0) throw new YamlError('不支持多文档 YAML', number + 1)
      continue
    }
    if (/^\t/.test(line)) throw new YamlError('YAML 不能用 Tab 缩进', number + 1)
    const indent = line.length - line.trimStart().length
    lines.push({ indent, text: line.trim(), raw: line, number: number + 1 })
  }
  // 块标量需要原始行，其他场景先过滤空行与注释行。
  let index = 0
  const significant = () => {
    while (index < lines.length) {
      const text = stripComment(lines[index].text)
      if (text !== '') {
        lines[index].text = text
        return lines[index]
      }
      index++
    }
    return null
  }

  const blockScalar = (parentIndent, style) => {
    const collected = []
    let blockIndent = null
    while (index < lines.length) {
      const line = lines[index]
      if (line.raw.trim() === '') {
        collected.push('')
        index++
        continue
      }
      if (line.indent <= parentIndent) break
      blockIndent ??= line.indent
      collected.push(line.raw.slice(blockIndent))
      index++
    }
    while (collected.length > 0 && collected[collected.length - 1] === '') collected.pop()
    return style === '>' ? collected.join(' ').replace(/ {2,}/g, ' ') + '\n' : collected.join('\n') + '\n'
  }

  const parseNode = (indent) => {
    const line = significant()
    if (!line) return null
    return isSeqItem(line.text) ? parseSeq(line.indent) : parseMap(line.indent)
  }

  const parseValueAfterKey = (indent, rest, line) => {
    if (/^[|>][+-]?$/.test(rest)) return blockScalar(indent, rest[0])
    if (rest !== '') return parseScalar(rest, line.number)
    const next = significant()
    if (next && (next.indent > indent || (next.indent === indent && isSeqItem(next.text)))) return parseNode(next.indent)
    return null
  }

  const parseMap = (indent) => {
    const result = {}
    for (let line = significant(); line && line.indent === indent && !isSeqItem(line.text); line = significant()) {
      const split = entrySplit(line.text)
      if (split < 0) throw new YamlError(`无法解析的行：${line.text}`, line.number)
      const key = parseKey(line.text.slice(0, split))
      const rest = line.text.slice(split + 1).trim()
      index++
      if (Object.hasOwn(result, key)) throw new YamlError(`重复的键：${key}`, line.number)
      result[key] = parseValueAfterKey(indent, rest, line)
    }
    const line = significant()
    if (line && line.indent > indent) throw new YamlError('缩进不一致', line.number)
    return result
  }

  const parseSeq = (indent) => {
    const result = []
    for (let line = significant(); line && line.indent === indent && isSeqItem(line.text); line = significant()) {
      const rest = line.text === '-' ? '' : line.text.slice(2).trimStart()
      const itemIndent = indent + (line.text.length - rest.length)
      if (rest === '') {
        index++
        const next = significant()
        result.push(next && next.indent > indent ? parseNode(next.indent) : null)
      } else if (isSeqItem(rest) || entrySplit(rest) >= 0) {
        // `- key: value`：把这一行改写成从 itemIndent 开始的虚拟行，再按映射/序列解析。
        lines[index] = { ...line, indent: itemIndent, text: rest }
        result.push(isSeqItem(rest) ? parseSeq(itemIndent) : parseMap(itemIndent))
      } else {
        index++
        result.push(parseScalar(rest, line.number))
      }
    }
    return result
  }

  const first = significant()
  if (!first) return null
  if (first.indent !== 0) throw new YamlError('文档顶层不能缩进', first.number)
  const document = parseNode(0)
  const rest = significant()
  if (rest) throw new YamlError(`无法解析的行：${rest.text}`, rest.number)
  return document
}
