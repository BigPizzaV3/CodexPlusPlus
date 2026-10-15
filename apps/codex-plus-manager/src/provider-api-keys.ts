import type { RelayProfile } from "./App";

export type RelayApiKey = { id: string; name: string; apiKey: string };

const DEFAULT_KEY_ID = "default";

function uniqueKeyId(base: string, used: Set<string>): string {
  let id = base;
  let suffix = 2;
  while (used.has(id)) {
    id = `${base}-${suffix}`;
    suffix += 1;
  }
  return id;
}

export function normalizeRelayApiKeys(profile: Pick<RelayProfile, "apiKey" | "apiKeys" | "activeApiKeyId">): {
  apiKeys: RelayApiKey[];
  activeApiKeyId: string;
} {
  const used = new Set<string>();
  const apiKeys = (Array.isArray(profile.apiKeys) ? profile.apiKeys : []).map((entry, index) => {
    const requestedId = String(entry?.id || "").trim() || `key-${index + 1}`;
    const id = uniqueKeyId(requestedId, used);
    used.add(id);
    return {
      id,
      name: String(entry?.name || "").trim() || `Key ${index + 1}`,
      apiKey: String(entry?.apiKey || ""),
    };
  });
  if (!apiKeys.length) {
    apiKeys.push({ id: DEFAULT_KEY_ID, name: "默认", apiKey: profile.apiKey || "" });
  }
  const requestedActiveId = String(profile.activeApiKeyId || "").trim();
  const activeApiKeyId = apiKeys.some((entry) => entry.id === requestedActiveId)
    ? requestedActiveId
    : apiKeys[0]?.id || "";
  return { apiKeys, activeApiKeyId };
}

export function selectedRelayApiKey(profile: Pick<RelayProfile, "apiKey" | "apiKeys" | "activeApiKeyId">): RelayApiKey | null {
  const normalized = normalizeRelayApiKeys(profile);
  return normalized.apiKeys.find((entry) => entry.id === normalized.activeApiKeyId) || null;
}

export function relayProfileWithNormalizedApiKeys(profile: RelayProfile): RelayProfile {
  const normalized = normalizeRelayApiKeys(profile);
  const selected = normalized.apiKeys.find((entry) => entry.id === normalized.activeApiKeyId);
  return {
    ...profile,
    ...normalized,
    apiKey: selected?.apiKey ?? profile.apiKey,
  };
}

export function relayProfileWithUpdatedApiKey(profile: RelayProfile, apiKey: string): RelayProfile {
  if (profile.noAuth || profile.relayMode === "aggregate"
    || (profile.relayMode === "official" && !profile.officialMixApiKey)) return profile;
  const normalized = normalizeRelayApiKeys(profile);
  return {
    ...profile,
    ...normalized,
    apiKey,
    apiKeys: normalized.apiKeys.map((entry) => entry.id === normalized.activeApiKeyId
      ? { ...entry, apiKey } : entry),
  };
}

export function createRelayApiKey(apiKeys: RelayApiKey[]): RelayApiKey {
  const used = new Set(apiKeys.map((entry) => entry.id));
  return {
    id: uniqueKeyId(`key-${Date.now().toString(36)}`, used),
    name: `Key ${apiKeys.length + 1}`,
    apiKey: "",
  };
}
