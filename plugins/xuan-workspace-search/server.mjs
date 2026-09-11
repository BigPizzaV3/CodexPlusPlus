import { createMcpServer } from "../shared/mcp-server.mjs";

createMcpServer({
  name: "xuan-workspace-search",
  tools: [{
    name: "workspace_search",
    description: "Search approved workspace roots and return bounded matching lines.",
    inputSchema: {
      type: "object",
      required: ["root", "query"],
      properties: {
        root: { type: "string" }, query: { type: "string" },
        include: { type: "array", items: { type: "string" } },
        exclude: { type: "array", items: { type: "string" } },
        maxResults: { type: "integer", minimum: 1, maximum: 2000 }
      }
    },
    bridgeMethod: "workspace.search.start"
  }]
});
