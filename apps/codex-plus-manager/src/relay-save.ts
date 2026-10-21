import { localProxyEndpointMatches, type RelayModelRouteSettings } from "./model-routes.ts";

type Profile = RelayModelRouteSettings["relayProfiles"][number] & {
  configContents?: string;
  authContents?: string;
  sessionProvider?: string;
  contextWindow?: string;
  autoCompactLimit?: string;
  modelWindows?: string;
  modelAutoCompact?: string;
  modelMetadata?: string;
  modelVlm?: string;
};

type Aggregate = {
  id: string;
  modelAggregationEnabled?: boolean;
  syncDefaultMemberLogin?: boolean;
  defaultRelayId?: string;
  members: { relayId: string; weight: number }[];
  sessionProvider?: string;
};

export type RelaySaveSettings = Omit<RelayModelRouteSettings, "relayProfiles"> & {
  relayProfiles: Profile[];
  aggregateRelayProfiles?: Aggregate[];
  relayCommonConfigContents?: string;
  relayContextConfigContents?: string;
};

const displayFields = new Set([
  "name", "testModel", "hideOfficialUsageAlert", "sub2apiEnabled", "sub2apiMultiplier",
]);
const catalogFields = new Set(["modelList", "modelInsertMode"]);
const capabilityMapFields = new Set(["modelWindows", "modelAutoCompact", "modelMetadata", "modelVlm"]);
const proxyFields = new Set([
  "baseUrl", "upstreamBaseUrl", "apiKey", "customHeaders", "userAgent", "modelRoutes",
  "webSearchHistoryCompat", "standardOpenaiProtocol",
  "vlmApiKey", "vlmBaseUrl", "vlmModel",
]);

function canonical(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.keys(value).sort().map((key) =>
      `${JSON.stringify(key)}:${canonical((value as Record<string, unknown>)[key])}`).join(",")}}`;
  }
  return JSON.stringify(value) ?? "";
}

// 这里只忽略由管理器维护的传输值；其它手写 TOML 变化仍保守要求重启。
function proxyConfigContract(config = ""): string {
  if (config.includes('"""') || config.includes("'''")) return config;
  let providerSection = false;
  return config.split(/\r?\n/).map((line) => {
    if (/^\s*\[/.test(line)) providerSection = /^\s*\[model_providers\.[\w"-]+\]\s*(?:#.*)?$/.test(line);
    return providerSection
      ? line.replace(/^(\s*(?:base_url|experimental_bearer_token)\s*=\s*)"(?:\\.|[^"\\])*"\s*(?:#.*)?$/, "$1\"<proxy-runtime>\"")
      : line;
  }).join("\n");
}

function proxyAuthContract(auth = ""): string {
  try {
    const value = JSON.parse(auth || "{}");
    if (!value || Array.isArray(value) || typeof value !== "object") return auth;
    delete value.OPENAI_API_KEY;
    return canonical(value);
  } catch {
    return auth;
  }
}

// 编辑器会把未配置的映射写成 "{}"；这与缺省值相同，不是客户端能力变化。
function capabilityMapContract(value: unknown): string | undefined {
  if (value === undefined || value === "") return undefined;
  try {
    const parsed = JSON.parse(value as string);
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      return Object.keys(parsed).length ? canonical(parsed) : undefined;
    }
  } catch {}
  return canonical(value);
}

function profileContract(profile: Profile, proxy: boolean): string {
  const contract: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(profile)) {
    if (displayFields.has(key) || catalogFields.has(key) || key === "aggregate") continue;
    if (proxy && proxyFields.has(key)) continue;
    if ((key === "customHeaders" || key === "modelRoutes")
        && (value === undefined || (Array.isArray(value) && !value.length))) continue;
    if (capabilityMapFields.has(key)) {
      const capability = capabilityMapContract(value);
      if (capability !== undefined) contract[key] = capability;
      continue;
    }
    contract[key] = proxy && key === "configContents" ? proxyConfigContract(value as string)
      : proxy && key === "authContents" ? proxyAuthContract(value as string) : value;
  }
  return canonical(contract);
}

function aggregateFor(settings: RelaySaveSettings): Aggregate | undefined {
  return settings.aggregateRelayProfiles?.find((aggregate) => aggregate.id === settings.activeRelayId);
}

function syncedLogin(settings: RelaySaveSettings): string {
  const aggregate = aggregateFor(settings);
  if (!aggregate?.syncDefaultMemberLogin) return "";
  const member = settings.relayProfiles.find((profile) => profile.id === aggregate.defaultRelayId);
  return canonical([aggregate.defaultRelayId, member?.relayMode, member?.officialMixApiKey, member?.authContents]);
}

export type RelaySaveRestartReason = "modelCapabilities" | "clientConfig" | "proxyUnavailable";
export type RelaySaveChoice = "restart" | "save" | "cancel";

function memberCapabilities(settings: RelaySaveSettings, active: Aggregate | undefined) {
  return (active?.modelAggregationEnabled ? settings.relayProfiles.filter((profile) =>
    active.members.some((member) => member.relayId === profile.id)) : [])
    .map((profile) => [profile.id, profile.contextWindow, profile.autoCompactLimit,
      ...[profile.modelWindows, profile.modelAutoCompact, profile.modelMetadata, profile.modelVlm]
        .map(capabilityMapContract)])
    .filter((entry) => entry.slice(1).some(Boolean))
    .sort((a, b) => String(a[0]).localeCompare(String(b[0])));
}

export function relaySettingsSaveCapabilityProfileNames(before: RelaySaveSettings, after: RelaySaveSettings): string[] {
  const previous = new Map(memberCapabilities(before, aggregateFor(before)).map((entry) => [String(entry[0]), canonical(entry)]));
  const next = new Map(memberCapabilities(after, aggregateFor(after)).map((entry) => [String(entry[0]), canonical(entry)]));
  return [...new Set([...previous.keys(), ...next.keys()])]
    .filter((id) => previous.get(id) !== next.get(id))
    .map((id) => after.relayProfiles.find((profile) => profile.id === id)?.name
      || before.relayProfiles.find((profile) => profile.id === id)?.name || id);
}

/** 区分建议重启的模型参数与仍需重载的连接配置，向保存确认框提供具体原因。 */
export function relaySettingsSaveRestartReason(
  before: RelaySaveSettings,
  after: RelaySaveSettings,
  liveBaseUrl: string,
  proxyAddress: string,
  helperRunning: boolean,
): RelaySaveRestartReason | null {
  if (!after.relayProfilesEnabled) return null;
  const current = before.relayProfiles.find((profile) => profile.id === before.activeRelayId);
  const proposed = after.relayProfiles.find((profile) => profile.id === after.activeRelayId);
  if (!current || !proposed) return null;
  const aggregate = aggregateFor(before);
  const nextAggregate = aggregateFor(after);
  const nativeChanged = before.activeRelayId !== after.activeRelayId
    || before.relayCommonConfigContents !== after.relayCommonConfigContents
    || before.relayContextConfigContents !== after.relayContextConfigContents
    || !!aggregate?.modelAggregationEnabled !== !!nextAggregate?.modelAggregationEnabled
    || aggregate?.sessionProvider !== nextAggregate?.sessionProvider
    || !!aggregate?.syncDefaultMemberLogin !== !!nextAggregate?.syncDefaultMemberLogin
    || syncedLogin(before) !== syncedLogin(after);
  if (nativeChanged) return "clientConfig";
  // 原生 app-server 缓存 catalog；能力变化需要提示，但允许用户保存后延迟重启。
  const capabilitiesChanged = relaySettingsSaveCapabilityProfileNames(before, after).length > 0;

  const sameProfile = profileContract(current, false) === profileContract(proposed, false);
  const aggregateChanged = canonical(aggregate && { ...aggregate, name: "" })
    !== canonical(nextAggregate && { ...nextAggregate, name: "" });
  if (sameProfile && !aggregateChanged) return capabilitiesChanged ? "modelCapabilities" : null;

  // 只有已接入同一健康代理的配置才允许传输热更；直连配置变化仍需确认重启。
  const usesSameProxy = helperRunning
    && localProxyEndpointMatches(liveBaseUrl, proxyAddress);
  if (!usesSameProxy) return "proxyUnavailable";
  // 同时修改原生连接配置时优先保留其重启要求，不能被成员能力变化掩盖。
  if (profileContract(current, true) !== profileContract(proposed, true)) return "clientConfig";
  return capabilitiesChanged ? "modelCapabilities" : null;
}

/** 兼容旧调用方：是否需要重启提示；模型能力提示允许用户选择稍后重启。 */
export function relaySettingsSaveRequiresRestart(
  ...args: Parameters<typeof relaySettingsSaveRestartReason>
): boolean {
  return relaySettingsSaveRestartReason(...args) !== null;
}
