/**
 * Codex++ MCP 服务器预设
 * 基于 cc-switch (MIT) 的 mcpPresets.ts，作者 Jason Young
 * https://github.com/farion1231/cc-switch
 *
 * cc-switch 的预设是 JSON（`{type, command, args}`），Codex 的 config.toml 里
 * 是 `[mcp_servers.<id>]` 表，所以这里直接给出 TOML 表体。字段名对齐 codex 的
 * RawMcpServerConfig：stdio 用 command/args/env，HTTP 用 url/http_headers。
 *
 * 各条目的 TOML 形态都在真实 codex（codex-cli 0.149）上验证过能解析。
 */

/** Windows 上 npx 是 npx.cmd，必须经 cmd /c 才能执行；其余平台直接调。 */
function npxCommand(packageName: string, windows: boolean): { command: string; args: string[] } {
  return windows
    ? { command: "cmd", args: ["/c", "npx", "-y", packageName] }
    : { command: "npx", args: ["-y", packageName] };
}

function tomlStringArray(values: string[]): string {
  return `[${values.map((value) => JSON.stringify(value)).join(", ")}]`;
}

function stdioTomlBody(packageName: string, windows: boolean): string {
  const { command, args } = npxCommand(packageName, windows);
  return `command = ${JSON.stringify(command)}\nargs = ${tomlStringArray(args)}\n`;
}

export interface McpPreset {
  /** 建议的条目 id，用户可以改。 */
  id: string;
  /** 展示用的包名／服务名。 */
  name: string;
  description: string;
  tags: string[];
  homepage?: string;
  /** `[mcp_servers.<id>]` 表头之下的内容。 */
  tomlBody: (options: { windows: boolean }) => string;
}

export const MCP_PRESETS: McpPreset[] = [
  {
    id: "fetch",
    name: "mcp-server-fetch",
    description: "抓取网页并转成适合模型阅读的文本。需要本机有 uvx（uv）。",
    tags: ["stdio", "web"],
    homepage: "https://github.com/modelcontextprotocol/servers/tree/main/src/fetch",
    // 官方就是用 uvx 跑，不走 npx，所以不分平台
    tomlBody: () => 'command = "uvx"\nargs = ["mcp-server-fetch"]\n',
  },
  {
    id: "time",
    name: "@modelcontextprotocol/server-time",
    description: "查询当前时间并做时区换算。",
    tags: ["stdio", "utility"],
    homepage: "https://github.com/modelcontextprotocol/servers/tree/main/src/time",
    tomlBody: ({ windows }) => stdioTomlBody("@modelcontextprotocol/server-time", windows),
  },
  {
    id: "memory",
    name: "@modelcontextprotocol/server-memory",
    description: "基于知识图谱的长期记忆，跨会话保留事实。",
    tags: ["stdio", "memory"],
    homepage: "https://github.com/modelcontextprotocol/servers/tree/main/src/memory",
    tomlBody: ({ windows }) => stdioTomlBody("@modelcontextprotocol/server-memory", windows),
  },
  {
    id: "sequential-thinking",
    name: "@modelcontextprotocol/server-sequential-thinking",
    description: "把复杂问题拆成可回溯的思考步骤。",
    tags: ["stdio", "reasoning"],
    homepage: "https://github.com/modelcontextprotocol/servers/tree/main/src/sequentialthinking",
    tomlBody: ({ windows }) =>
      stdioTomlBody("@modelcontextprotocol/server-sequential-thinking", windows),
  },
  {
    id: "context7",
    name: "@upstash/context7-mcp",
    description: "按需拉取库和框架的最新文档，减少版本过时的回答。",
    tags: ["stdio", "docs"],
    homepage: "https://context7.com",
    tomlBody: ({ windows }) => stdioTomlBody("@upstash/context7-mcp", windows),
  },
];

export function mcpPresetById(id: string): McpPreset | undefined {
  return MCP_PRESETS.find((preset) => preset.id === id);
}
