export type CodexRelayProtocol = "responses" | "chatCompletions";
export type CodexRelaySessionProvider = "custom" | "openai";

/** Chat Completions cannot keep an OpenAI session, so return to Custom. */
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
