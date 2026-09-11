import { createMcpServer } from "../shared/mcp-server.mjs";

createMcpServer({
  name: "xuan-usage",
  tools: [{
    name: "usage_query",
    description: "Query provider usage for a configured relay profile.",
    inputSchema: {
      type: "object",
      properties: {
        profileRef: { type: "string" }, startDate: { type: "string" },
        endDate: { type: "string" }, timezone: { type: "string" }
      }
    },
    bridgeMethod: "usage.query"
  }]
});
