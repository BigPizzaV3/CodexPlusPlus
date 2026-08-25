/**
 * 极简 TOML / JSON 词法着色器。
 *
 * 只为编辑器着色服务，不做语法校验：遇到不认识的内容一律回退成 plain，
 * 保证任何输入都能原样显示（拼接所有 token 必须等于原文）。
 */

export type TokenKind =
  | "plain"
  | "comment"
  | "section"
  | "key"
  | "string"
  | "number"
  | "boolean"
  | "null"
  | "date"
  | "punct"
  | "operator";

export interface CodeToken {
  kind: TokenKind;
  text: string;
}

export type CodeLanguage = "json" | "toml";

const TOML_MULTILINE_DELIMS = ['"""', "'''"];
const DATE_PATTERN = /^\d{4}-\d{2}-\d{2}([Tt ]\d{2}:\d{2}(:\d{2}(\.\d+)?)?([Zz]|[+-]\d{2}:\d{2})?)?/;
const NUMBER_PATTERN = /^[+-]?(0[xX][0-9a-fA-F_]+|0[oO][0-7_]+|0[bB][01_]+|(\d[\d_]*)(\.[\d_]+)?([eE][+-]?\d+)?|nan|inf)/;
const BARE_KEY_PATTERN = /^[A-Za-z0-9_-]+/;

/** 把文本切成「每行一组 token」。行数始终等于 text 的行数。 */
export function tokenizeCode(text: string, language: CodeLanguage): CodeToken[][] {
  return language === "json" ? tokenizeJson(text) : tokenizeToml(text);
}

class LineBuilder {
  readonly lines: CodeToken[][] = [[]];

  newline(): void {
    this.lines.push([]);
  }

  push(kind: TokenKind, text: string): void {
    if (!text) return;
    const parts = text.split("\n");
    parts.forEach((part, index) => {
      if (index > 0) this.newline();
      if (!part) return;
      const line = this.lines[this.lines.length - 1];
      const last = line[line.length - 1];
      if (last && last.kind === kind) last.text += part;
      else line.push({ kind, text: part });
    });
  }
}

function tokenizeToml(text: string): CodeToken[][] {
  const out = new LineBuilder();
  let i = 0;
  // 行首待读 key；遇到 '=' 转值模式，遇到换行回到行首
  let expectKey = true;
  let braceDepth = 0;

  while (i < text.length) {
    const ch = text[i];

    if (ch === "\n") {
      out.newline();
      i += 1;
      if (braceDepth === 0) expectKey = true;
      continue;
    }
    if (ch === " " || ch === "\t" || ch === "\r") {
      out.push("plain", ch);
      i += 1;
      continue;
    }
    if (ch === "#") {
      const end = nextIndexOr(text, "\n", i);
      out.push("comment", text.slice(i, end));
      i = end;
      continue;
    }

    // 表头 [table] / [[array]]
    if (expectKey && ch === "[") {
      const end = nextIndexOr(text, "\n", i);
      const close = text.lastIndexOf("]", end);
      if (close > i) {
        out.push("section", text.slice(i, close + 1));
        i = close + 1;
        continue;
      }
    }

    const multiline = TOML_MULTILINE_DELIMS.find((delim) => text.startsWith(delim, i));
    if (multiline && !expectKey) {
      const close = text.indexOf(multiline, i + multiline.length);
      const end = close === -1 ? text.length : close + multiline.length;
      out.push("string", text.slice(i, end));
      i = end;
      continue;
    }
    if (ch === '"' || ch === "'") {
      const len = readQuoted(text, i, ch === '"');
      out.push(expectKey ? "key" : "string", text.slice(i, i + len));
      i += len;
      continue;
    }
    if (ch === "=") {
      out.push("operator", ch);
      expectKey = false;
      i += 1;
      continue;
    }
    if (ch === "{") {
      out.push("punct", ch);
      braceDepth += 1;
      expectKey = true;
      i += 1;
      continue;
    }
    if (ch === "}") {
      out.push("punct", ch);
      braceDepth = Math.max(0, braceDepth - 1);
      expectKey = false;
      i += 1;
      continue;
    }
    if (ch === "[" || ch === "]") {
      out.push("punct", ch);
      i += 1;
      continue;
    }
    if (ch === ",") {
      out.push("operator", ch);
      if (braceDepth > 0) expectKey = true;
      i += 1;
      continue;
    }
    if (ch === ".") {
      out.push("operator", ch);
      i += 1;
      continue;
    }

    if (expectKey) {
      const bare = BARE_KEY_PATTERN.exec(text.slice(i));
      if (bare) {
        out.push("key", bare[0]);
        i += bare[0].length;
        continue;
      }
      out.push("plain", ch);
      i += 1;
      continue;
    }

    const rest = text.slice(i);
    const date = DATE_PATTERN.exec(rest);
    if (date) {
      out.push("date", date[0]);
      i += date[0].length;
      continue;
    }
    const bool = /^(true|false)\b/.exec(rest);
    if (bool) {
      out.push("boolean", bool[1]);
      i += bool[1].length;
      continue;
    }
    const num = NUMBER_PATTERN.exec(rest);
    if (num && num[0] && /[\d+-]/.test(ch)) {
      out.push("number", num[0]);
      i += num[0].length;
      continue;
    }
    const bare = BARE_KEY_PATTERN.exec(rest);
    if (bare) {
      out.push("plain", bare[0]);
      i += bare[0].length;
      continue;
    }
    out.push("plain", ch);
    i += 1;
  }
  return out.lines;
}

function tokenizeJson(text: string): CodeToken[][] {
  const out = new LineBuilder();
  let i = 0;
  while (i < text.length) {
    const ch = text[i];
    if (ch === "\n") {
      out.newline();
      i += 1;
      continue;
    }
    if (ch === '"') {
      const len = readQuoted(text, i, true);
      // 收尾引号后的第一个非空白字符是 ':' 就算键名
      let probe = i + len;
      while (probe < text.length && /\s/.test(text[probe])) probe += 1;
      out.push(text[probe] === ":" ? "key" : "string", text.slice(i, i + len));
      i += len;
      continue;
    }
    if (ch === "/" && (text[i + 1] === "/" || text[i + 1] === "*")) {
      const end = text[i + 1] === "/"
        ? nextIndexOr(text, "\n", i)
        : Math.min(text.length, indexOrEnd(text, "*/", i + 2) + 2);
      out.push("comment", text.slice(i, end));
      i = end;
      continue;
    }
    if ("{}[]".includes(ch)) {
      out.push("punct", ch);
      i += 1;
      continue;
    }
    if (ch === ":" || ch === ",") {
      out.push("operator", ch);
      i += 1;
      continue;
    }
    const word = /^(true|false|null)\b/.exec(text.slice(i));
    if (word) {
      out.push(word[1] === "null" ? "null" : "boolean", word[1]);
      i += word[1].length;
      continue;
    }
    const num = NUMBER_PATTERN.exec(text.slice(i));
    if (num && /[\d+-]/.test(ch)) {
      out.push("number", num[0]);
      i += num[0].length;
      continue;
    }
    out.push("plain", ch);
    i += 1;
  }
  return out.lines;
}

function nextIndexOr(text: string, needle: string, from: number): number {
  const found = text.indexOf(needle, from);
  return found === -1 ? text.length : found;
}

function indexOrEnd(text: string, needle: string, from: number): number {
  const found = text.indexOf(needle, from);
  return found === -1 ? text.length : found;
}

/** 读取一段带引号的字符串，返回包含引号的长度；未闭合则读到行尾。 */
function readQuoted(text: string, start: number, escaped: boolean): number {
  const quote = text[start];
  let i = start + 1;
  while (i < text.length) {
    const ch = text[i];
    if (ch === "\n") break;
    if (escaped && ch === "\\") {
      i += 2;
      continue;
    }
    i += 1;
    if (ch === quote) return i - start;
  }
  return i - start;
}
