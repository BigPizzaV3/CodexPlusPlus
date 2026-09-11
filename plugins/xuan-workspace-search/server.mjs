import { createMcpServer } from "./lib/mcp-server.mjs";

createMcpServer({
  name: "xuan-workspace-search",
  tools: [
    {
      name: "workspace_search",
      description: "Search an approved workspace root and return bounded matching lines.",
      inputSchema: {
        type: "object",
        additionalProperties: false,
        required: ["root", "query"],
        properties: {
          root: { type: "string" },
          query: { type: "string", minLength: 1, maxLength: 1000 },
          include: { type: "array", maxItems: 32, items: { type: "string", maxLength: 300 } },
          exclude: { type: "array", maxItems: 32, items: { type: "string", maxLength: 300 } },
          caseSensitive: { type: "boolean" },
          wholeWord: { type: "boolean" },
          regex: { type: "boolean" },
          maxResults: { type: "integer", minimum: 1, maximum: 2000 }
        }
      },
      bridgeMethod: "workspace.search.start"
    },
    {
      name: "workspace_search_preview",
      description: "Preview the lines around a workspace search match.",
      inputSchema: {
        type: "object",
        additionalProperties: false,
        required: ["root", "path", "line"],
        properties: {
          root: { type: "string" },
          path: { type: "string" },
          line: { type: "integer", minimum: 1 }
        }
      },
      bridgeMethod: "workspace.search.preview"
    }
  ]
});
