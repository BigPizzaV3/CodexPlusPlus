export function shouldBackfillRelayProfileBeforeSwitch(
  previousActiveRelayId: string,
  nextActiveRelayId: string,
): boolean {
  const previousId = previousActiveRelayId.trim();
  return previousId.length > 0 && previousId !== nextActiveRelayId.trim();
}

export type RelayProfileFileSnapshot = {
  relayMode: "official" | "pureApi" | "mixedApi" | "aggregate";
  authContents: string;
};

/**
 * Pure API credentials belong to the stored provider, not Codex's current
 * login state. Other live-file modes still use the current auth.json so an
 * official login refresh is not replaced by an archived token set.
 */
export function relayAuthForLiveDraft(
  profile: RelayProfileFileSnapshot,
  liveAuthContents: string,
): string {
  return profile.relayMode === "pureApi" ? profile.authContents : liveAuthContents;
}
