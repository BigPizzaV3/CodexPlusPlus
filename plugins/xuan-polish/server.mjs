import { createMcpServer } from "./lib/mcp-server.mjs";

createMcpServer({
  name: "xuan-polish",
  tools: [{
    name: "polish_text",
    description: "Rewrite a draft prompt while preserving its intent.",
    inputSchema: {
      type: "object",
      additionalProperties: false,
      required: ["text"],
      properties: {
        text: { type: "string", minLength: 1, maxLength: 100000 },
        profileRef: { type: "string" },
        model: { type: "string" },
        style: { type: "string", enum: ["structured", "concise", "coding"] },
        recentTurns: {
          type: "array",
          maxItems: 4,
          items: {
            type: "object",
            additionalProperties: false,
            properties: {
              userText: { type: "string" },
              assistantText: { type: "string" }
            }
          }
        },
        projectMap: { type: "string", maxLength: 4000 }
      }
    },
    bridgeMethod: "polish.generate"
  }]
});
