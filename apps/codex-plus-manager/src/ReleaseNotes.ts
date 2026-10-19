import { createElement as h, type MouseEvent, type ReactNode } from "react";

type OpenUrl = (url: string) => void;

function webUrl(value: string): string | null {
  try {
    const url = new URL(value);
    return ["https:", "http:"].includes(url.protocol) ? url.href : null;
  } catch {
    return null;
  }
}

// Release 文本只生成 React 节点，不执行远端 HTML，也不加载远端图片。
function inline(text: string, onOpenUrl: OpenUrl, depth = 0): ReactNode[] {
  if (depth > 12) return [text];
  const pattern = /\\([\\`*_[\]{}()#+.!>~-])|(`+)([^`]*?)\2|\[([^\]]+)\]\(([^\s)]+)(?:\s+"[^"]*")?\)|\*\*(.+?)\*\*|__(.+?)__|~~(.+?)~~|\*([^*\n]+)\*|_([^_\n]+)_|<(https?:\/\/[^\s>]+)>/g;
  const nodes: ReactNode[] = [];
  let cursor = 0;
  for (const match of text.matchAll(pattern)) {
    const index = match.index!;
    if (index > cursor) nodes.push(text.slice(cursor, index));
    const key = index;
    if (match[1]) nodes.push(match[1]);
    else if (match[2]) nodes.push(h("code", { key }, match[3]));
    else if (match[4] || match[11]) {
      const label = match[4] || match[11];
      const url = webUrl(match[5] || match[11]);
      nodes.push(url ? h("a", {
        key, href: url, rel: "noreferrer noopener", onClick: (event: MouseEvent<HTMLAnchorElement>) => {
          event.preventDefault();
          onOpenUrl(url);
        },
      }, inline(label, onOpenUrl, depth + 1)) : label);
    } else {
      const content = match[6] ?? match[7] ?? match[8] ?? match[9] ?? match[10];
      nodes.push(h(match[6] || match[7] ? "strong" : match[8] ? "del" : "em", { key }, inline(content, onOpenUrl, depth + 1)));
    }
    cursor = index + match[0].length;
  }
  if (cursor < text.length) nodes.push(text.slice(cursor));
  return nodes;
}

const listItem = /^(\s*)([-+*]|\d+[.)])\s+(.*)$/;
const fence = /^\s{0,3}(`{3,}|~{3,})(.*)$/;
const heading = /^\s{0,3}(#{1,6})\s+(.+?)\s*#*\s*$/;
const rule = /^\s{0,3}(?:\*\s*){3,}$|^\s{0,3}(?:-\s*){3,}$|^\s{0,3}(?:_\s*){3,}$/;
const tableDivider = /^\s*\|?\s*:?-{3,}:?\s*(?:\|\s*:?-{3,}:?\s*)+\|?\s*$/;

function cells(line: string): string[] {
  return line.trim().replace(/^\|/, "").replace(/\|$/, "").split(/(?<!\\)\|/).map((cell) => cell.trim());
}

function blocks(lines: string[], onOpenUrl: OpenUrl, depth = 0): ReactNode[] {
  if (depth > 12) return [h("p", { key: 0 }, lines.join("\n"))];
  const nodes: ReactNode[] = [];
  let index = 0;
  const startsBlock = (i: number) => heading.test(lines[i]) || fence.test(lines[i]) || rule.test(lines[i])
    || listItem.test(lines[i]) || /^\s*>/.test(lines[i]) || (i + 1 < lines.length && tableDivider.test(lines[i + 1]));
  while (index < lines.length) {
    const line = lines[index];
    const key = index;
    if (!line.trim()) { index++; continue; }
    const code = fence.exec(line);
    const title = heading.exec(line);
    if (code) {
      index++;
      const content: string[] = [];
      const closing = new RegExp(`^\\s{0,3}${code[1][0]}{${code[1].length},}\\s*$`);
      while (index < lines.length && !closing.test(lines[index])) content.push(lines[index++]);
      if (index < lines.length) index++;
      nodes.push(h("pre", { key }, h("code", null, content.join("\n"))));
    } else if (title) {
      nodes.push(h(`h${title[1].length}`, { key }, inline(title[2], onOpenUrl)));
      index++;
    } else if (index + 1 < lines.length && /^\s{0,3}(?:=+|-+)\s*$/.test(lines[index + 1]) && line.trim()) {
      nodes.push(h(lines[index + 1].trim()[0] === "=" ? "h1" : "h2", { key }, inline(line, onOpenUrl)));
      index += 2;
    } else if (rule.test(line)) {
      nodes.push(h("hr", { key }));
      index++;
    } else if (/^\s*>/.test(line)) {
      const quote: string[] = [];
      while (index < lines.length && /^\s*>/.test(lines[index])) quote.push(lines[index++].replace(/^\s*>\s?/, ""));
      nodes.push(h("blockquote", { key }, blocks(quote, onOpenUrl, depth + 1)));
    } else if (listItem.test(line)) {
      const first = listItem.exec(line)!;
      const indent = first[1].length;
      const ordered = /^\d/.test(first[2]);
      const items: ReactNode[] = [];
      while (index < lines.length) {
        const item = listItem.exec(lines[index]);
        if (!item || item[1].length !== indent || /^\d/.test(item[2]) !== ordered) break;
        const itemKey = index++;
        const content = [item[3]];
        const contentIndent = item[0].length - item[3].length;
        while (index < lines.length) {
          const next = lines[index];
          if (!next.trim()) {
            if (index + 1 < lines.length && /^\s*/.exec(lines[index + 1])![0].length > indent) { content.push(""); index++; continue; }
            break;
          }
          if (/^\s*/.exec(next)![0].length <= indent) break;
          content.push(next.slice(Math.min(contentIndent, /^\s*/.exec(next)![0].length)));
          index++;
        }
        const task = /^\[([ xX])\]\s+(.*)$/.exec(content[0]);
        if (task) content[0] = task[2];
        items.push(h("li", { key: itemKey }, task ? h("input", { type: "checkbox", checked: task[1] !== " ", disabled: true, readOnly: true }) : null, blocks(content, onOpenUrl, depth + 1)));
        if (!lines[index]?.trim() && listItem.test(lines[index + 1] ?? "")) index++;
      }
      nodes.push(h(ordered ? "ol" : "ul", { key, start: ordered ? Number.parseInt(first[2], 10) : undefined }, items));
    } else if (index + 1 < lines.length && line.includes("|") && tableDivider.test(lines[index + 1])) {
      const headers = cells(line);
      index += 2;
      const rows: ReactNode[] = [];
      while (index < lines.length && lines[index].trim() && lines[index].includes("|")) {
        const row = cells(lines[index]);
        rows.push(h("tr", { key: index++ }, headers.map((_, i) => h("td", { key: i }, inline(row[i] ?? "", onOpenUrl)))));
      }
      nodes.push(h("div", { key, className: "release-notes-table" }, h("table", null,
        h("thead", null, h("tr", null, headers.map((cell, i) => h("th", { key: i }, inline(cell, onOpenUrl))))), h("tbody", null, rows))));
    } else {
      const paragraph = [line];
      index++;
      while (index < lines.length && lines[index].trim() && !startsBlock(index)) paragraph.push(lines[index++]);
      nodes.push(h("p", { key }, inline(paragraph.join("\n"), onOpenUrl)));
    }
  }
  return nodes;
}

export function ReleaseNotes({ markdown, onOpenUrl }: { markdown: string; onOpenUrl: OpenUrl }) {
  return h("div", { className: "release-notes" }, blocks(markdown.replace(/\r\n?/g, "\n").split("\n"), onOpenUrl));
}
