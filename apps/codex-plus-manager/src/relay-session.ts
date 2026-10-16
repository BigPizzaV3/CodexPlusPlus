export type CodexRelayProtocol = "responses" | "chatCompletions" | "anthropicMessages";
export type CodexRelaySessionProvider = "custom" | "openai";

/** Chat Completions 与 Anthropic Messages 都不能保持 OpenAI 会话身份，回退 Custom。 */
export function sessionProviderForProtocol(
  sessionProvider: CodexRelaySessionProvider,
  protocol: CodexRelayProtocol,
): CodexRelaySessionProvider {
  if (protocol !== "responses" && sessionProvider === "openai") return "custom";
  return sessionProvider;
}

/** Mirrors the supplier save gate: OpenAI session identity requires Responses. */
export function openAiSessionBlocksModelSave(
  sessionProvider: CodexRelaySessionProvider,
  protocol: CodexRelayProtocol,
): boolean {
  return sessionProvider === "openai" && protocol !== "responses";
}
