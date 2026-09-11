import { createMcpServer } from "./lib/mcp-server.mjs";

createMcpServer({
  name: "xuan-usage",
  tools: [{
    name: "usage_query",
    description: "Query provider usage for a configured relay profile.",
    inputSchema: {
      type: "object",
      additionalProperties: false,
      properties: {
        profileRef: { type: "string" },
        usagePath: { type: "string" },
        startDate: { type: "string", format: "date" },
        endDate: { type: "string", format: "date" },
        timezone: { type: "string" }
      }
    },
    bridgeMethod: "usage.query"
  }]
});
