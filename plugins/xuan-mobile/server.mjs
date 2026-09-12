import { createMcpServer } from "./lib/mcp-server.mjs";

createMcpServer({
  name: "xuan-mobile",
  version: "0.1.0",
  tools: [
    {
      name: "xuan_mobile_status",
      description: "Read the local phone pairing and synchronization status.",
      inputSchema: { type: "object", additionalProperties: false, properties: {} },
      bridgeMethod: "mobile.status"
    },
    {
      name: "xuan_mobile_pair",
      description: "Generate a new phone pairing QR code.",
      inputSchema: { type: "object", additionalProperties: false, properties: {} },
      bridgeMethod: "mobile.pair"
    },
    {
      name: "xuan_mobile_confirm",
      description: "Confirm or reject a pending phone pairing request on this computer.",
      inputSchema: {
        type: "object",
        additionalProperties: false,
        required: ["requestId", "confirmed"],
        properties: {
          requestId: { type: "string", minLength: 1 },
          confirmed: { type: "boolean" }
        }
      },
      bridgeMethod: "mobile.confirm"
    },
    {
      name: "xuan_mobile_tasks",
      description: "List the local Codex tasks available for phone synchronization.",
      inputSchema: { type: "object", additionalProperties: false, properties: {} },
      bridgeMethod: "mobile.tasks"
    }
  ]
});
