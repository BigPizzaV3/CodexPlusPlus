// 通用上下文（MCP / Skills / 插件）条目的纯文本 TOML 辅助函数。
// 从 App.tsx 抽出来便于独立测试：这里只做字符串层面的解析/拼装，
// 不依赖设置、后端或 React。后端 toml_edit 规范化会把子表挪到文件末尾，
// 所以解析不能只靠「子表紧跟在父表后」的位置假设（#2887）。

export type ContextKind = "mcp" | "skill" | "plugin";

export type CodexContextEntry = {
  id: string;
  kind: ContextKind;
  title: string;
  summary: string;
  tomlBody: string;
  enabled: boolean;
};

export type CodexContextEntries = {
  mcpServers: CodexContextEntry[];
  skills: CodexContextEntry[];
  plugins: CodexContextEntry[];
};

const contextKindTables: Array<{ kind: ContextKind; tableName: string }> = [
  { kind: "mcp", tableName: "mcp_servers" },
  { kind: "skill", tableName: "skills" },
  { kind: "plugin", tableName: "plugins" },
];

function ensureTrailingNewline(value: string) {
  return value.trim() ? `${value}\n` : "";
}

function tomlString(value: string): string {
  return value.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
}

function tomlKey(key: string): string {
  return /^[A-Za-z0-9_-]+$/.test(key) ? key : `"${tomlString(key)}"`;
}

export function unquoteTomlKey(key: string) {
  if (key.length >= 2 && ((key.startsWith('"') && key.endsWith('"')) || (key.startsWith("'") && key.endsWith("'")))) {
    return key.slice(1, -1);
  }
  return key;
}

export function tomlTablePathFromLine(line: string): string[] | null {
  const match = /^\s*\[([^\]]+)\]\s*$/.exec(line);
  if (!match) return null;
  return parseTomlDottedPath(match[1].trim());
}

export function parseTomlDottedPath(path: string): string[] | null {
  const parts: string[] = [];
  let current = "";
  let quote: '"' | "'" | null = null;
  let escaping = false;

  for (const char of path) {
    if (quote) {
      if (quote === '"' && escaping) {
        current += char;
        escaping = false;
      } else if (quote === '"' && char === "\\") {
        escaping = true;
      } else if (char === quote) {
        quote = null;
      } else {
        current += char;
      }
      continue;
    }

    if (char === '"' || char === "'") {
      quote = char;
      continue;
    }
    if (char === ".") {
      if (!current.trim()) return null;
      parts.push(current.trim());
      current = "";
      continue;
    }
    current += char;
  }

  if (quote || escaping || !current.trim()) return null;
  parts.push(current.trim());
  return parts;
}

export function contextEntrySummary(tomlBody: string) {
  return tomlBody
    .split(/\r?\n/)
    .map((line) => line.trim())
    .find((line) => line && !line.startsWith("#") && !/^enabled\s*=/.test(line))
    ?.slice(0, 96) ?? "";
}

export function contextEntryEnabled(tomlBody: string) {
  return !tomlBody.split(/\r?\n/).some((line) => /^\s*enabled\s*=\s*false\s*(#.*)?$/i.test(line));
}

export function setContextEntryEnabled(tomlBody: string, enabled: boolean) {
  const lines = tomlBody.trimEnd().split(/\r?\n/);
  const nextValue = `enabled = ${enabled ? "true" : "false"}`;
  let replaced = false;
  const next = lines.map((line) => {
    if (/^\s*enabled\s*=/.test(line)) {
      replaced = true;
      return nextValue;
    }
    return line;
  });
  if (!replaced) next.unshift(nextValue);
  return ensureTrailingNewline(next.join("\n").trimEnd());
}

// 表头是否属于上下文条目：`[mcp_servers.<id>]` 或它的任意子表 `[mcp_servers.<id>.a.b]`。
export function contextHeaderFromLine(line: string): { kind: ContextKind; id: string } | null {
  const path = tomlTablePathFromLine(line);
  if (!path || path.length < 2) return null;
  const option = contextKindTables.find((item) => item.tableName === path[0]);
  return option ? { kind: option.kind, id: path[1] } : null;
}

// 按 id 累积上下文条目：同一个 id 的父表键值行进 root，所有子表段按出现顺序
// 追加（header 写成相对路径）。子表不要求和父表相邻——toml_edit 规范化后
// `[mcp_servers.<id>.env]` 常被挪到文件末尾；只有子表没有父表头的条目也建。
// 输出时 root 一定排在全部子表之前，否则 root 键会落进最后一个子表（#2887）。
export function parseContextEntries(commonConfig: string, kind: ContextKind, tableName: string): CodexContextEntry[] {
  const anyHeaderPattern = /^\s*\[[^\]]+\]\s*$/;
  type EntryDraft = { root: string[]; subs: { header: string; lines: string[] }[] };
  const drafts = new Map<string, EntryDraft>();
  // -1 表示当前行属于父表本体，>=0 是 subs 里的下标。
  let currentId: string | null = null;
  let currentSub = -1;

  for (const line of commonConfig.split(/\r?\n/)) {
    const path = tomlTablePathFromLine(line);
    if (path?.[0] === tableName && path.length >= 2) {
      const id = path[1];
      let draft = drafts.get(id);
      if (!draft) {
        draft = { root: [], subs: [] };
        drafts.set(id, draft);
      }
      currentId = id;
      if (path.length === 2) {
        currentSub = -1;
      } else {
        draft.subs.push({ header: `[${path.slice(2).map(tomlKey).join(".")}]`, lines: [] });
        currentSub = draft.subs.length - 1;
      }
      continue;
    }
    if (currentId && anyHeaderPattern.test(line)) {
      currentId = null;
      currentSub = -1;
      continue;
    }
    if (currentId) {
      const draft = drafts.get(currentId);
      if (!draft) continue;
      if (currentSub < 0) draft.root.push(line);
      else draft.subs[currentSub].lines.push(line);
    }
  }

  return Array.from(drafts.entries()).map(([id, draft]) => {
    const body = [...draft.root];
    for (const sub of draft.subs) body.push(sub.header, ...sub.lines);
    const tomlBody = ensureTrailingNewline(body.join("\n").trimEnd());
    return {
      id,
      kind,
      title: id,
      summary: contextEntrySummary(tomlBody),
      tomlBody,
      enabled: contextEntryEnabled(tomlBody),
    };
  });
}

export function dedupeContextEntryList(entries: CodexContextEntry[]): CodexContextEntry[] {
  const byId = new Map<string, CodexContextEntry>();
  for (const entry of entries) {
    byId.set(entry.id, entry);
  }
  return Array.from(byId.values());
}

export function contextEntriesByKind(entries: CodexContextEntries, kind: ContextKind): CodexContextEntry[] {
  if (kind === "mcp") return dedupeContextEntryList(entries.mcpServers);
  if (kind === "skill") return dedupeContextEntryList(entries.skills);
  return dedupeContextEntryList(entries.plugins);
}

export function contextEntryToTomlSection(tableName: string, entry: CodexContextEntry): string {
  const parentHeader = `[${tableName}.${tomlKey(entry.id)}]`;
  const body = entry.tomlBody
    .trimEnd()
    .split(/\r?\n/)
    .map((line) => relativeContextSubtableToAbsolute(line, tableName, entry.id))
    .join("\n");
  return `${parentHeader}\n${body}`;
}

function relativeContextSubtableToAbsolute(line: string, tableName: string, id: string): string {
  const match = /^\s*\[([^\]]+)\]\s*$/.exec(line);
  if (!match) return line;
  const path = parseTomlDottedPath(match[1].trim());
  if (!path?.length) return line;
  // 多级子表（如 `[tools.foo]`）同样是相对路径，不能原样落到根级（#2887）。
  return `[${[tableName, id, ...path].map(tomlKey).join(".")}]`;
}

export function allContextConfigToml(entries: CodexContextEntries): string {
  const sections: string[] = [];
  for (const option of contextKindTables) {
    for (const entry of dedupeContextEntryList(contextEntriesByKind(entries, option.kind))) {
      sections.push(contextEntryToTomlSection(option.tableName, entry));
    }
  }
  return ensureTrailingNewline(sections.join("\n\n"));
}

export function selectedContextConfigToml(entries: CodexContextEntries): string {
  const sections: string[] = [];
  for (const option of contextKindTables) {
    for (const entry of dedupeContextEntryList(contextEntriesByKind(entries, option.kind))) {
      if (!entry.enabled) continue;
      sections.push(contextEntryToTomlSection(option.tableName, entry));
    }
  }
  return ensureTrailingNewline(sections.join("\n\n"));
}

export function contextEntriesFromConfig(configContents: string): CodexContextEntries {
  return {
    mcpServers: parseContextEntries(configContents, "mcp", "mcp_servers"),
    skills: parseContextEntries(configContents, "skill", "skills"),
    plugins: parseContextEntries(configContents, "plugin", "plugins"),
  };
}

export function splitContextConfigText(configContents: string): { common: string; context: string } {
  const entries = contextEntriesFromConfig(configContents);
  return {
    common: stripContextEntriesFromConfig(configContents, entries),
    context: allContextConfigToml(entries),
  };
}

// 剥掉已知上下文条目在文本里的全部痕迹：父表段和它的所有子表段（不管是否相邻）
// 都跳过；未知 id 的子表保留（#2887）。
export function stripContextEntriesFromConfig(configContents: string, entries: CodexContextEntries): string {
  const knownIds: Record<ContextKind, Set<string>> = {
    mcp: new Set(entries.mcpServers.map((entry) => entry.id)),
    skill: new Set(entries.skills.map((entry) => entry.id)),
    plugin: new Set(entries.plugins.map((entry) => entry.id)),
  };
  const lines = configContents.split(/\r?\n/);
  const kept: string[] = [];
  let skipping = false;

  for (const line of lines) {
    const contextHeader = contextHeaderFromLine(line);
    if (contextHeader) {
      skipping = knownIds[contextHeader.kind].has(contextHeader.id);
    } else if (/^\s*\[[^\]]+\]\s*$/.test(line)) {
      skipping = false;
    }
    if (!skipping) kept.push(line);
  }

  return ensureTrailingNewline(kept.join("\n").trimEnd());
}
