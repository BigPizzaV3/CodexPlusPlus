/** 入门清单只读取现有配置：Codex 直连当前用的供应商是否就绪，以及对它发一次真实请求的测试结果。 */
export const SETUP_STORAGE_KEY = "codex-plus-plus.setup";
const SETUP_VERSION = 1;

export function hasCodexEnhancements(config: {
  settings: { enhancementsEnabled: boolean; codexAppStandaloneQuotaUnlock: boolean };
} | null): boolean {
  if (!config) return false;
  // 普通拓展注入也受增强总开关控制；独立额度解锁是启动器支持的例外。
  return config.settings.enhancementsEnabled || config.settings.codexAppStandaloneQuotaUnlock;
}

export function serializeSetupState(state: "completed" | "dismissed"): string {
  return JSON.stringify({ version: SETUP_VERSION, state });
}

/** 老用户不因新增清单再被打断；设置里的主动重开不受此规则限制。 */
export function shouldShowSetup(storedSetup: string | null, storedTour: string | null): boolean {
  if (storedTour?.trim()) return false;
  try {
    const saved: unknown = JSON.parse(storedSetup ?? "null");
    if (saved && typeof saved === "object" && "version" in saved && "state" in saved
      && typeof saved.version === "number" && saved.version >= SETUP_VERSION
      && (saved.state === "completed" || saved.state === "dismissed")) return false;
  } catch { /* 无有效记录时按首次启动处理。 */ }
  return true;
}

/** 内存中的配置版本标记；地址和代理可能含凭据，因此不把原文作为标记返回。 */
function configurationFingerprint(value: unknown): string {
  const text = JSON.stringify(value);
  let hash = 0xcbf29ce484222325n;
  for (let index = 0; index < text.length; index += 1) {
    hash = BigInt.asUintN(64, (hash ^ BigInt(text.charCodeAt(index))) * 0x100000001b3n);
  }
  return hash.toString(16);
}


/** 网关接管 Codex 时放进直连列表的托管供应商（core 的 `CODEX_GATEWAY_RELAY_ID`）。 */
const GATEWAY_RELAY_ID = "codex-plus-gateway";

/** 只取入门清单用得到的字段；App 里完整的 RelayProfile / BackendSettings 结构上兼容。 */
export type SetupRelayProfile = {
  id: string;
  name: string;
  relayMode: string;
  officialMixApiKey: boolean;
  baseUrl: string;
  apiKey: string;
  noAuth: boolean;
};
export type SetupRelaySettings = {
  relayProfilesEnabled: boolean;
  activeRelayId: string;
  relayProfiles: SetupRelayProfile[];
};
/** `relay_status` 的结果：只读 Codex 的 auth.json / config.toml，不发网络请求。 */
export type SetupRelayStatus = { authenticated: boolean; configured: boolean };

export type SetupDirectMode = "official" | "mixedApi" | "pureApi" | "aggregate" | "gateway";
export type SetupDirectIssue = "loading" | "switching_off" | "no_profile" | "not_logged_in" | "api_missing" | "not_configured";
export type SetupDirectState = {
  profile: SetupRelayProfile | null;
  mode: SetupDirectMode;
  issue: SetupDirectIssue | null;
  ready: boolean;
  /** 聚合供应商会在多个成员间轮转，单条请求测不出整体是否可用，交给供应商页逐个测。 */
  testable: boolean;
  fingerprint: string;
};

export function setupDirectMode(profile: SetupRelayProfile): SetupDirectMode {
  if (profile.id === GATEWAY_RELAY_ID) return "gateway";
  if (profile.relayMode === "aggregate") return "aggregate";
  if (profile.relayMode === "pureApi") return "pureApi";
  if (profile.relayMode === "mixedApi" || profile.officialMixApiKey) return "mixedApi";
  return "official";
}

/** Codex 直连的就绪规则：官方登录看 ChatGPT 登录，混入 API 还要 Base URL 和 Key，其余看写入 Codex 的配置是否完整。 */
export function setupDirectState(settings: SetupRelaySettings | null, status: SetupRelayStatus | null): SetupDirectState {
  const profile = settings
    ? settings.relayProfiles.find((item) => item.id === settings.activeRelayId) ?? settings.relayProfiles[0] ?? null
    : null;
  const mode = profile ? setupDirectMode(profile) : "official";
  const apiFilled = Boolean(profile?.baseUrl.trim() && (profile.apiKey.trim() || profile.noAuth));
  const issue: SetupDirectIssue | null = !settings || !status ? "loading"
    : !settings.relayProfilesEnabled ? "switching_off"
    : !profile ? "no_profile"
    : mode === "official" ? (status.authenticated ? null : "not_logged_in")
    : mode === "mixedApi" ? (!status.authenticated ? "not_logged_in" : apiFilled ? null : "api_missing")
    : status.configured || (mode === "pureApi" && profile.noAuth && apiFilled) ? null : "not_configured";
  const fingerprint = configurationFingerprint({
    enabled: settings?.relayProfilesEnabled,
    profile,
    authenticated: status?.authenticated,
    configured: status?.configured,
  });
  return { profile, mode, issue, ready: issue === null, testable: mode !== "aggregate", fingerprint };
}
