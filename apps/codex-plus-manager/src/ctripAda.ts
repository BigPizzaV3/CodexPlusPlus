export const CTRIP_ADA_PROFILE_ID = "ctrip-ada";
export const CTRIP_ADA_BASE_URL = "http://ada-cli-golang.ctripcorp.com/coding-plan/openai/v1";
export const CTRIP_ADA_MODEL = "gpt-5.4-2026-03-05";
export const CODEX_DOWNLOAD_URL = "https://developers.openai.com/codex/app";

type RelayProtocol = "responses" | "chatCompletions";
type RelayMode = "official" | "mixedApi" | "pureApi";

type RelayContextSelection = {
  mcpServers: string[];
  skills: string[];
  plugins: string[];
};

export type RelayProfile = {
  id: string;
  name: string;
  model: string;
  baseUrl: string;
  upstreamBaseUrl: string;
  apiKey: string;
  protocol: RelayProtocol;
  relayMode: RelayMode;
  officialMixApiKey: boolean;
  testModel: string;
  configContents: string;
  authContents: string;
  useCommonConfig: boolean;
  contextSelection: RelayContextSelection;
  contextSelectionInitialized: boolean;
  contextWindow: string;
  autoCompactLimit: string;
  modelList: string;
  userAgent: string;
  providerId: string;
  apiKeyEnv: string;
  presetId: string;
};

export type BackendSettings = {
  codexAppPath: string;
  codexExtraArgs: string[];
  providerSyncEnabled: boolean;
  providerSyncSavedProviders: string[];
  providerSyncManualProviders: string[];
  providerSyncLastSelectedProvider: string;
  relayProfilesEnabled: boolean;
  enhancementsEnabled: boolean;
  computerUseGuardEnabled: boolean;
  codexAppPluginEntryUnlock: boolean;
  codexAppPluginMarketplaceUnlock: boolean;
  codexAppForcePluginInstall: boolean;
  codexAppModelWhitelistUnlock: boolean;
  codexAppSessionDelete: boolean;
  codexAppMarkdownExport: boolean;
  codexAppProjectMove: boolean;
  codexAppConversationTimeline: boolean;
  codexAppConversationView: boolean;
  codexAppThreadScrollRestore: boolean;
  codexAppZedRemoteOpen: boolean;
  zedRemoteOpenStrategy: string;
  zedRemoteProjectRegistryEnabled: boolean;
  zedRemoteSyncToZedSettings: boolean;
  codexAppUpstreamWorktreeCreate: boolean;
  codexAppNativeMenuPlacement: boolean;
  codexAppServiceTierControls: boolean;
  codexAppImageOverlayEnabled: boolean;
  codexAppImageOverlayPath: string;
  codexAppImageOverlayOpacity: number;
  codexGoalsEnabled: boolean;
  launchMode: "patch" | "relay";
  relayBaseUrl: string;
  relayApiKey: string;
  relayProfiles: RelayProfile[];
  relayCommonConfigContents: string;
  relayContextConfigContents: string;
  activeRelayId: string;
  relayTestModel: string;
  cliWrapperEnabled: boolean;
  cliWrapperBaseUrl: string;
  cliWrapperApiKey: string;
  cliWrapperApiKeyEnv: string;
};

const emptyContextSelection = (): RelayContextSelection => ({
  mcpServers: [],
  skills: [],
  plugins: [],
});

function tomlString(value: string): string {
  return value.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
}

export function buildCtripConfigToml(
  profile: Pick<RelayProfile, "model" | "baseUrl" | "providerId" | "apiKeyEnv">,
): string {
  const providerId = profile.providerId.trim() || "custom";
  const baseUrl = profile.baseUrl.trim();
  return [
    `model_provider = "${tomlString(providerId)}"`,
    profile.model.trim() ? `model = "${tomlString(profile.model.trim())}"` : null,
    'model_reasoning_effort = "xhigh"',
    "request_max_retries = 4",
    "stream_max_retries = 10",
    "",
    `[model_providers.${providerId}]`,
    `name = "${tomlString(providerId)}"`,
    `base_url = "${tomlString(baseUrl)}"`,
    'wire_api = "responses"',
    `env_key = "${tomlString(profile.apiKeyEnv.trim())}"`,
    "",
  ]
    .filter((line): line is string => line !== null)
    .join("\n");
}

function withGeneratedRelayFiles(profile: RelayProfile): RelayProfile {
  return {
    ...profile,
    configContents: buildCtripConfigToml(profile),
    authContents: profile.apiKeyEnv.trim() ? "{}\n" : "",
  };
}

export function createCtripAdaProfile(apiKey: string): RelayProfile {
  const base: RelayProfile = {
    id: CTRIP_ADA_PROFILE_ID,
    name: "携程 CodingPlan (ADA)",
    model: CTRIP_ADA_MODEL,
    baseUrl: CTRIP_ADA_BASE_URL,
    upstreamBaseUrl: CTRIP_ADA_BASE_URL,
    apiKey: apiKey.trim(),
    protocol: "responses",
    relayMode: "pureApi",
    officialMixApiKey: false,
    testModel: CTRIP_ADA_MODEL,
    configContents: "",
    authContents: "",
    useCommonConfig: false,
    contextSelection: emptyContextSelection(),
    contextSelectionInitialized: true,
    contextWindow: "",
    autoCompactLimit: "",
    modelList: "",
    userAgent: "",
    providerId: "ctrip",
    apiKeyEnv: "ADA_API_KEY",
    presetId: "ctrip-ada",
  };
  return withGeneratedRelayFiles(base);
}

export function upsertCtripAdaProfile(settings: BackendSettings, apiKey: string): BackendSettings {
  const profile = createCtripAdaProfile(apiKey);
  const existingIndex = settings.relayProfiles.findIndex(
    (item) => item.id === CTRIP_ADA_PROFILE_ID || item.presetId === "ctrip-ada",
  );
  const relayProfiles = [...settings.relayProfiles];
  let activeRelayId = profile.id;
  if (existingIndex >= 0) {
    activeRelayId = relayProfiles[existingIndex].id;
    relayProfiles[existingIndex] = { ...profile, id: activeRelayId };
  } else {
    relayProfiles.push(profile);
  }
  return {
    ...settings,
    relayProfiles,
    activeRelayId,
    launchMode: "patch",
    relayProfilesEnabled: true,
    relayBaseUrl: profile.baseUrl,
    relayApiKey: profile.apiKey,
  };
}

export function getAdaToken(settings: BackendSettings): string | null {
  const profile = settings.relayProfiles.find(
    (item) => item.id === CTRIP_ADA_PROFILE_ID || item.presetId === "ctrip-ada",
  );
  const token = profile?.apiKey?.trim();
  return token || null;
}
