/** 「全局管理 · MCP」：把 Codex、Claude Code、Grok 三家的 MCP 服务器按名称合成一张矩阵。
 *  三家条目都用 Codex 规范的 TOML 表体交换（Claude / Grok 的格式换算在后端 agent_mcp 里做）。 */

export type McpAgentId = "codex" | "claude" | "grok";
export const MCP_AGENTS: McpAgentId[] = ["codex", "claude", "grok"];

export type AgentMcpEntry = { id: string; tomlBody: string; enabled: boolean };
export type AgentMcpConfig = { agent: string; configPath: string; exists: boolean; entries: AgentMcpEntry[] };

export type McpRow = {
  id: string;
  /** 用于展示与编辑的表体：优先取 Codex 的，其次 Claude、Grok。 */
  body: string;
  /** 每家的条目；没有的就是 undefined。 */
  byAgent: Partial<Record<McpAgentId, AgentMcpEntry>>;
};

/** 某家是否「在用」这台服务器：有条目且没停用。 */
export function mcpOn(row: McpRow, agent: McpAgentId): boolean {
  const entry = row.byAgent[agent];
  return Boolean(entry && entry.enabled);
}

/** 已经有这台服务器条目的 Agent（不论是否停用），编辑和删除要同步到它们。 */
export function mcpPresent(row: McpRow): McpAgentId[] {
  return MCP_AGENTS.filter((agent) => row.byAgent[agent]);
}

export function mergeMcpRows(codex: AgentMcpEntry[], others: Partial<Record<"claude" | "grok", AgentMcpEntry[]>>): McpRow[] {
  const rows = new Map<string, McpRow>();
  const add = (agent: McpAgentId, entries: AgentMcpEntry[]) => {
    for (const entry of entries) {
      const row = rows.get(entry.id) ?? { id: entry.id, body: entry.tomlBody, byAgent: {} };
      row.byAgent[agent] = entry;
      rows.set(entry.id, row);
    }
  };
  add("codex", codex);
  add("claude", others.claude ?? []);
  add("grok", others.grok ?? []);
  // Codex 的条目保持原有顺序排在前面，其余按名称排。
  const codexOrder = new Map(codex.map((entry, index) => [entry.id, index]));
  return [...rows.values()].sort((left, right) => {
    const a = codexOrder.get(left.id);
    const b = codexOrder.get(right.id);
    if (a !== undefined && b !== undefined) return a - b;
    if (a !== undefined) return -1;
    if (b !== undefined) return 1;
    return left.id.localeCompare(right.id);
  });
}

/** 某一列「已启用」的数量，用于列头。 */
export function mcpCount(rows: McpRow[], agent: McpAgentId): number {
  return rows.filter((row) => mcpOn(row, agent)).length;
}
