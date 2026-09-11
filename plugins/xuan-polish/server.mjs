import { createMcpServer } from "../shared/mcp-server.mjs";

createMcpServer({
  name: "xuan-polish",
  tools: [{
    name: "polish_text",
    description: "Rewrite a draft prompt while preserving its intent.",
    inputSchema: {
      type: "object",
      required: ["text"],
      properties: {
        text: { type: "string" }, style: { type: "string", enum: ["structured", "concise", "coding"] },
        recentTurns: { type: "array" }, projectMap: { type: "string" }
      }
    },
    bridgeMethod: "polish.generate"
  }]
});
