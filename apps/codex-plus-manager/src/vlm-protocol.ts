/// VLM 协议归一化：老配置无 vlmProtocol -> 回落 chatCompletions；显式 responses 保留。
export function normalizeVlmProtocol(value: unknown): "responses" | "chatCompletions" {
  return value === "responses" ? "responses" : "chatCompletions";
}
