/**
 * 供应商（RelayProfile）编辑的纯函数：由字段生成 / 回读 config.toml 与 auth.json、新建 / 复制 / 删除 / 排序、
 * 以及切换与保存前的校验。从 App.tsx 原样搬出，管理器和注入到 Codex 页面的「供应商」页共用这一份，
 * 不能依赖 React 或 Tauri（注入页由 scripts/assemble-provider-editing.mjs 打包，i18n 换成中文直出）。
 */

import type {
  AggregateMode,
  AggregateRelayProfile,
  BackendSettings,
  RelayAggregateConfig,
  RelayAggregateStrategy,
  RelayContextSelection,
  RelayMode,
  RelayProfile,
  RelayProtocol,
  RelaySessionProvider,
} from "./App";
import { t, tf } from "@/i18n";
import { aggregateMemberCandidates, isAggregateProfile, normalizeAggregateDefaultRelayId, normalizeAggregateMembers } from "./aggregate-members";
import { clampAggregateRoutePriority, normalizeAggregateRoutes, validateAggregateRoutes } from "./aggregate-routes";
import { modelSlugFromRowName, suffixWindowString } from "./model-metadata";
import {
  findRelayModelRouteIssue,
  independentProxyEnabled,
  normalizeRelayModelRoutes,
  PROTOCOL_PROXY_BASE_URL,
  pureApiImageGenerationEnabled,
  requestCompatibilityEnabled,
  stripImageGenerationToolEnabled,
  stripResponsesImageDetailEnabled,
} from "./model-routes";
import { relayProfileWithNormalizedApiKeys, relayProfileWithUpdatedApiKey } from "./provider-api-keys";
import { resolveProviderName } from "./provider-name";
import { sessionProviderForProtocol } from "./relay-session";

/**
 * 依赖宿主的少数几项：默认设置与 MCP / Skill / 插件的全选。管理器在 App.tsx 里注册真实实现；
 * 注入页由服务端在 /providers/list 里给出默认值。未注册时给安全的空值，函数仍可用。
 */
export type RelayProfileEditingHost = {
  defaultRelayBaseUrl: () => string;
  defaultRelayProfiles: () => RelayProfile[];
  contextSelectionForAllEntries: (settings: BackendSettings) => RelayContextSelection;
};

export const relayProfileEditingHost: RelayProfileEditingHost = {
  defaultRelayBaseUrl: () => "",
  defaultRelayProfiles: () => [],
  contextSelectionForAllEntries: () => ({ mcpServers: [], skills: [], plugins: [] }),
};

export function configureRelayProfileEditing(host: Partial<RelayProfileEditingHost>): void {
  Object.assign(relayProfileEditingHost, host);
}


export function withGeneratedRelayFiles(profile: RelayProfile): RelayProfile {
  if (isAggregateRelayProfile(profile)) {
    return { ...profile, configContents: "", authContents: "", aggregate: normalizeAggregateConfig(profile.aggregate, []) };
  }
  if (profile.relayMode === "official") {
    return {
      ...profile,
      configContents: profile.officialMixApiKey
        ? buildRelayConfigToml(profile, { includeBearerToken: true, requiresOpenAiAuth: true })
        : "",
      authContents: profile.authContents || "",
    };
  }
  return {
    ...profile,
    configContents: buildRelayConfigToml(profile, { includeBearerToken: !profile.noAuth, requiresOpenAiAuth: false }),
    authContents: buildRelayAuthJson(profile),
  };
}

export function buildRelayConfigToml(
  profile: Pick<RelayProfile, "id" | "model" | "baseUrl" | "upstreamBaseUrl" | "apiKey" | "protocol" | "sessionProvider" | "relayMode" | "officialMixApiKey" | "webSearchHistoryCompat" | "localCompactionPromptAtEnd" | "strictResponsesReasoning" | "stripImageGenerationTool" | "stripResponsesImageDetail" | "pureApiImageGeneration" | "modelMetadata">,
  options: { includeBearerToken: boolean; requiresOpenAiAuth?: boolean },
): string {
  const searchHistoryCompat = profile.webSearchHistoryCompat === true
    && profile.protocol === "responses"
    && profile.relayMode !== "aggregate"
    && (profile.relayMode !== "official" || profile.officialMixApiKey);
  const localCompaction = profile.localCompactionPromptAtEnd === true
    && (profile.protocol === "responses" || profile.protocol === "chatCompletions")
    && profile.relayMode !== "aggregate"
    && (profile.relayMode !== "official" || profile.officialMixApiKey);
  const baseUrl = profile.protocol !== "responses" || independentProxyEnabled(profile) || searchHistoryCompat || localCompaction || requestCompatibilityEnabled(profile) || stripImageGenerationToolEnabled(profile) || stripResponsesImageDetailEnabled(profile) || pureApiImageGenerationEnabled(profile)
    ? PROTOCOL_PROXY_BASE_URL
    : profile.baseUrl.trim();
  const apiKey = profile.apiKey.trim();
  const sessionProvider = normalizeRelaySessionProvider(profile.sessionProvider);
  const rootLines = [
    profile.model.trim() ? `model = "${tomlString(profile.model.trim())}"` : null,
    `model_provider = "${sessionProvider}"`,
    sessionProvider === "openai" ? `openai_base_url = "${PROTOCOL_PROXY_BASE_URL}"` : null,
    "",
  ].filter((line): line is string => line !== null);
  const requiresOpenAiAuthLine = options.requiresOpenAiAuth === undefined
    ? null
    : `requires_openai_auth = ${options.requiresOpenAiAuth ? "true" : "false"}`;
  return [
    ...rootLines,
    "[model_providers.custom]",
    'name = "custom"',
    'wire_api = "responses"',
    requiresOpenAiAuthLine,
    `base_url = "${tomlString(baseUrl)}"`,
    options.includeBearerToken && apiKey ? `experimental_bearer_token = "${tomlString(apiKey)}"` : null,
    "",
  ].filter((line): line is string => line !== null).join("\n");
}

export function buildRelayAuthJson(profile: Pick<RelayProfile, "apiKey">): string {
  return `${JSON.stringify({ OPENAI_API_KEY: profile.apiKey.trim() }, null, 2)}\n`;
}

export function buildOfficialRelayAuthJson(contents: string): string {
  const trimmed = contents.trim();
  if (!trimmed) return "";
  try {
    const parsed = JSON.parse(trimmed) as Record<string, unknown>;
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return contents;
    const knownOAuth = String(parsed.auth_mode || "").toLowerCase() === "chatgpt" && parsed.tokens && typeof parsed.tokens === "object";
    if (!knownOAuth && Object.keys(parsed).some((key) => key !== "OPENAI_API_KEY")) return contents;
    delete parsed.OPENAI_API_KEY;
    return `${JSON.stringify(parsed, null, 2)}\n`;
  } catch {
    return contents;
  }
}

export function deriveRelayProfileFromFiles(profile: RelayProfile, editedFile?: "config" | "auth"): RelayProfile {
  if (isAggregateRelayProfile(profile)) {
    return normalizeAggregateRelayProfile(profile, null);
  }
  if (editedFile === "auth") {
    try {
      JSON.parse(profile.authContents || "{}");
    } catch {
      return profile;
    }
  }
  let configContents = profile.configContents || "";
  let authContents = profile.relayMode === "official" ? buildOfficialRelayAuthJson(profile.authContents || "") : profile.authContents || "";
  const configBaseUrl = codexBaseUrlFromConfig(configContents);
  const chatUpstreamBaseUrl = rootTomlStringValue(configContents, CHAT_UPSTREAM_BASE_URL_KEY);
  const isProxyConfig = configBaseUrl === PROTOCOL_PROXY_BASE_URL;
  const upstreamBaseUrl = profile.upstreamBaseUrl || chatUpstreamBaseUrl || (configBaseUrl && !isProxyConfig ? configBaseUrl : profile.baseUrl || "");
  const configApiKey = codexExperimentalBearerTokenFromConfig(configContents);
  const apiKey = editedFile === "config" ? configApiKey
    : editedFile === "auth" && profile.relayMode === "pureApi" ? codexApiKeyFromAuth(authContents)
    : profile.relayMode === "official"
    ? configApiKey || profile.apiKey || ""
    : codexApiKeyFromAuth(authContents) || configApiKey || "";
  if (profile.relayMode === "pureApi") {
    // 旧 false/缺省模板也需要显式 provider 认证；保留用户已有的 true/false。
    configContents = profile.noAuth
      ? removeCodexExperimentalBearerToken(configContents)
      : apiKey && configContents.trim()
        ? setCodexExperimentalBearerToken(configContents, apiKey, { requiresOpenAiAuth: false })
        : configContents;
    if (editedFile && !profile.noAuth) {
      if (editedFile === "config") authContents = setAuthOpenAiApiKey(authContents, apiKey);
      if (!apiKey) configContents = removeCodexExperimentalBearerToken(configContents);
    }
  }
  const configModel = codexModelFromConfig(configContents);
  // 如果用户输入了带后缀的模型名，优先保留在界面的「配置模型」字段中；
  // config.toml 里实际写的是剥离后缀的 slug（由 applyRelayProfilePatchToFiles 处理）。
  const model = /\[.+\]$/.test(profile.model.trim()) ? profile.model.trim() : configModel;
  const derived = {
    ...profile,
    model,
    sessionProvider: relaySessionProviderFromConfig(configContents),
    baseUrl: upstreamBaseUrl,
    upstreamBaseUrl,
    apiKey,
    contextWindow: codexTopLevelIntFromConfig(configContents, "model_context_window"),
    autoCompactLimit: codexTopLevelIntFromConfig(configContents, "model_auto_compact_token_limit"),
    configContents,
    authContents,
  };
  return editedFile ? relayProfileWithUpdatedApiKey(derived, apiKey) : derived;
}

export function applyRelayProfilePatchToFiles(
  profile: RelayProfile,
  patch: Partial<RelayProfile>,
  options: { allowGenerateFiles?: boolean } = {},
): RelayProfile {
  const protocol = patch.protocol ?? profile.protocol;
  const sessionProvider = "sessionProvider" in patch
    ? normalizeRelaySessionProvider(patch.sessionProvider)
    : relaySessionProvider(profile);
  const compatibleSession = sessionProviderForProtocol(sessionProvider, protocol);
  const normalizedPatch = compatibleSession === sessionProvider
    ? patch
    : { ...patch, sessionProvider: compatibleSession };
  let next: RelayProfile = { ...profile, ...normalizedPatch };
  if (isAggregateRelayProfile(next)) {
    return normalizeAggregateRelayProfile(next, null);
  }
  const shouldHaveFiles =
    next.relayMode !== "official" || next.officialMixApiKey || next.configContents.trim() || next.authContents.trim();
  const needsAuthFile = next.relayMode === "pureApi";
  if (options.allowGenerateFiles && shouldHaveFiles && (!next.configContents.trim() || (needsAuthFile && !next.authContents.trim()))) {
    next = withGeneratedRelayFiles(next);
  }

  if ("sessionProvider" in patch) {
    const sessionProvider = normalizeRelaySessionProvider(patch.sessionProvider);
    next.sessionProvider = sessionProvider;
    next.configContents = setRootTomlStringKey(next.configContents, "model_provider", sessionProvider);
    next.configContents = setManagedOpenAiBaseUrl(next.configContents, sessionProvider === "openai");
  }

  if ("model" in patch) {
    // 模型后缀（如 [1M]）仅供 CodexPlusPlus 内部使用，写入 config.toml 前需剥离，
    // 否则 codex 会按带后缀的字符串去匹配 catalog slug，导致窗口回退到默认值。
    const { slug } = parseModelSuffix(patch.model || "");
    next.configContents = setRootTomlStringKey(next.configContents, "model", slug);
  }
  if ("apiKey" in patch) {
    next = relayProfileWithUpdatedApiKey(next, patch.apiKey || "");
    if (next.relayMode === "pureApi") {
      next.authContents = setAuthOpenAiApiKey(next.authContents, patch.apiKey || "");
      next.configContents = setCodexExperimentalBearerToken(next.configContents, patch.apiKey || "", { requiresOpenAiAuth: false });
    } else {
      next.configContents = setCodexExperimentalBearerToken(next.configContents, patch.apiKey || "");
    }
  }
  if ("baseUrl" in patch) {
    next.upstreamBaseUrl = patch.baseUrl || "";
  }
  if ("upstreamBaseUrl" in patch) {
    next.baseUrl = patch.upstreamBaseUrl || "";
  }
  if ("baseUrl" in patch || "upstreamBaseUrl" in patch || "protocol" in patch || "modelRoutes" in patch || "webSearchHistoryCompat" in patch || "localCompactionPromptAtEnd" in patch || "strictResponsesReasoning" in patch || "stripImageGenerationTool" in patch || "stripResponsesImageDetail" in patch || "pureApiImageGeneration" in patch || "modelMetadata" in patch) {
    const searchHistoryCompat = next.webSearchHistoryCompat === true
      && next.protocol === "responses"
      && next.relayMode !== "aggregate"
      && (next.relayMode !== "official" || next.officialMixApiKey);
    const localCompaction = next.localCompactionPromptAtEnd === true
      && (next.protocol === "responses" || next.protocol === "chatCompletions")
      && next.relayMode !== "aggregate"
      && (next.relayMode !== "official" || next.officialMixApiKey);
    const baseUrlForConfig = next.protocol !== "responses" || independentProxyEnabled(next) || normalizeRelayModelRoutes(next.modelRoutes).length > 0 || searchHistoryCompat || localCompaction || requestCompatibilityEnabled(next) || stripImageGenerationToolEnabled(next) || stripResponsesImageDetailEnabled(next) || pureApiImageGenerationEnabled(next)
      ? PROTOCOL_PROXY_BASE_URL
      : next.upstreamBaseUrl || next.baseUrl;
    next.configContents = setCodexProviderStringKey(next.configContents, "base_url", baseUrlForConfig, {
      requiresOpenAiAuth: next.relayMode !== "pureApi",
    });
    next.configContents = removeRootTomlKey(next.configContents, CHAT_UPSTREAM_BASE_URL_KEY);
  }
  if ("contextWindow" in patch) {
    next.configContents = setRootTomlIntKey(next.configContents, "model_context_window", patch.contextWindow || "");
  }
  if ("autoCompactLimit" in patch) {
    next.configContents = setRootTomlIntKey(
      next.configContents,
      "model_auto_compact_token_limit",
      patch.autoCompactLimit || "",
    );
  }
  if ("relayMode" in patch || "officialMixApiKey" in patch) {
    if (next.relayMode === "official" && !next.officialMixApiKey) {
      next.configContents = "";
      next.authContents = buildOfficialRelayAuthJson(next.authContents);
    } else if (options.allowGenerateFiles && (!next.configContents.trim() || (next.relayMode === "pureApi" && !next.authContents.trim()))) {
      next = withGeneratedRelayFiles(next);
    }
  }
  return deriveRelayProfileFromFiles(next);
}

export function codexModelFromConfig(contents: string): string {
  const lines = contents.split(/\r?\n/);
  const structural = tomlStructureMask(lines);
  for (const [index, line] of lines.entries()) {
    if (!structural[index]) continue;
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    if (trimmed.startsWith("[")) break;
    const match = /^model\s*=\s*(["'])(.*)\1\s*$/.exec(trimmed);
    if (match) return match[2].replace(/\\(["'\\])/g, "$1");
  }
  return "";
}

/// 解析模型后缀语法，如 deepseek-v4-flash[1M] -> { slug: "deepseek-v4-flash", window: 1000000 }
/// 非法或没有后缀时返回原串作为 slug。剥离与换算统一走 model-metadata.ts 的
/// suffixWindowString/modelSlugFromRowName，避免两处实现对「什么算合法后缀」
/// 的判断分叉。
export function parseModelSuffix(raw: string): { slug: string; window?: number } {
  const window = suffixWindowString(raw);
  if (window === null) return { slug: raw.trim() };
  return { slug: modelSlugFromRowName(raw), window: Number(window) };
}

export function codexBaseUrlFromConfig(contents: string): string {
  return codexProviderStringFromConfig(contents, "base_url");
}

export function codexExperimentalBearerTokenFromConfig(contents: string): string {
  return codexProviderStringFromConfig(contents, "experimental_bearer_token");
}

export function codexProviderStringFromConfig(contents: string, key: string): string {
  const provider = rootTomlStringValue(contents, "model_provider");
  const targetSection = codexProviderSectionName(provider || "custom");
  const fallbackSection = provider === "openai" && rootTomlStringValue(contents, "openai_base_url") === PROTOCOL_PROXY_BASE_URL
    ? codexProviderSectionName("custom") : "";
  const lines = contents.split(/\r?\n/);
  const structural = tomlStructureMask(lines);
  let currentSection = "";
  let rootValue = "";
  let fallbackValue = "";

  for (const [index, line] of lines.entries()) {
    if (!structural[index]) continue;
    const section = tomlSectionName(line);
    if (section !== null) {
      currentSection = section;
      continue;
    }
    const value = tomlStringAssignmentValue(line, key);
    if (value === null) continue;
    if (currentSection === targetSection) return value;
    if (fallbackSection && currentSection === fallbackSection) fallbackValue = value;
    if (!provider && currentSection === "") rootValue = value;
  }

  // 不借用 inactive provider 的 Key；仅托管 OpenAI 会话允许回落 custom transport。
  return fallbackValue || rootValue;
}

export function codexApiKeyFromAuth(contents: string): string {
  try {
    const parsed = JSON.parse(contents || "{}") as { OPENAI_API_KEY?: unknown };
    return typeof parsed.OPENAI_API_KEY === "string" ? parsed.OPENAI_API_KEY : "";
  } catch {
    return "";
  }
}

export function codexTopLevelIntFromConfig(contents: string, key: string): string {
  const topLevel = splitTomlRootAndTables(contents).root;
  const pattern = new RegExp(`^\\s*${key}\\s*=\\s*(\\d+)\\s*(?:#.*)?$`);
  const lines = topLevel.split(/\r?\n/);
  const structural = tomlStructureMask(lines);
  for (const [index, line] of lines.entries()) {
    if (!structural[index]) continue;
    const match = pattern.exec(line);
    if (match) return match[1];
  }
  return "";
}

export function rootTomlStringValue(contents: string, key: string): string {
  const topLevel = splitTomlRootAndTables(contents).root;
  const lines = topLevel.split(/\r?\n/);
  const structural = tomlStructureMask(lines);
  for (const [index, line] of lines.entries()) {
    if (!structural[index]) continue;
    const value = tomlStringAssignmentValue(line, key);
    if (value !== null) return value;
  }
  return "";
}

export function tomlStructureMask(lines: string[]): boolean[] {
  // 提示词里的伪表头/赋值只是字符串正文，不能参与 provider 鉴权读写。
  let multiline: string | null = null;
  return lines.map((line) => {
    const structural = multiline === null;
    for (let index = 0; index < line.length; index += 1) {
      if (multiline !== null) {
        if (multiline === '"' && line[index] === "\\") {
          index += 1;
        } else if (line[index] === multiline) {
          let end = index;
          while (line[end] === multiline) end += 1;
          if (end - index >= 3) multiline = null;
          index = end - 1;
        }
      } else if (line[index] === "#") {
        break;
      } else if (line[index] === '"' || line[index] === "'") {
        if (line[index + 1] === line[index] && line[index + 2] === line[index]) {
          multiline = line[index];
          index += 2;
        } else {
          const quoted = tomlQuotedValue(line, index);
          if (!quoted) break;
          index = quoted.end - 1;
        }
      }
    }
    return structural;
  });
}

export function tomlSectionName(line: string): string | null {
  const text = line.trim();
  if (!text.startsWith("[")) return null;
  const array = text.startsWith("[[");
  const start = array ? 2 : 1;
  for (let index = start; index < text.length; index += 1) {
    if (text[index] === '"' || text[index] === "'") {
      const quoted = tomlQuotedValue(text, index);
      if (!quoted) return null;
      index = quoted.end - 1;
    } else if (text[index] === "]") {
      const end = index + (array ? 2 : 1);
      if (array && text[index + 1] !== "]") return null;
      const rest = text.slice(end).trim();
      if (rest && !rest.startsWith("#")) return null;
      const path = tomlDottedKeyPath(text.slice(start, index));
      // 数组表也是前一个表的边界，但不能与同名普通表混为一谈。
      return path ? (array ? "[]" : "") + tomlKeyPathName(path) : null;
    }
  }
  return null;
}

export function tomlQuotedValue(text: string, start: number): { value: string; end: number } | null {
  const quote = text[start];
  if (quote !== '"' && quote !== "'") return null;
  let value = "";
  for (let index = start + 1; index < text.length; index += 1) {
    const character = text[index];
    if (character === quote) return { value, end: index + 1 };
    if ((character.charCodeAt(0) < 32 && character !== "\t") || character.charCodeAt(0) === 127) return null;
    if (quote === '"' && character === "\\") {
      const escape = text[++index];
      const escapes: Record<string, string> = { '"': '"', "\\": "\\", b: "\b", t: "\t", n: "\n", f: "\f", r: "\r" };
      if (Object.prototype.hasOwnProperty.call(escapes, escape)) value += escapes[escape];
      else if (escape === "u" || escape === "U") {
        const length = escape === "u" ? 4 : 8;
        const digits = text.slice(index + 1, index + 1 + length);
        if (digits.length !== length || !/^[0-9a-f]+$/i.test(digits)) return null;
        const codepoint = Number.parseInt(digits, 16);
        if (codepoint > 0x10ffff || (codepoint >= 0xd800 && codepoint <= 0xdfff)) return null;
        value += String.fromCodePoint(codepoint);
        index += length;
      } else return null;
    } else value += character;
  }
  return null;
}

export function tomlDottedKeyPath(text: string): string[] | null {
  const path: string[] = [];
  let index = 0;
  while (index < text.length) {
    while (text[index] === " " || text[index] === "\t") index += 1;
    if (text[index] === '"' || text[index] === "'") {
      const quoted = tomlQuotedValue(text, index);
      if (!quoted) return null;
      path.push(quoted.value);
      index = quoted.end;
    } else {
      const bare = /^[A-Za-z0-9_-]+/.exec(text.slice(index));
      if (!bare) return null;
      path.push(bare[0]);
      index += bare[0].length;
    }
    while (text[index] === " " || text[index] === "\t") index += 1;
    if (index === text.length) return path;
    if (text[index] !== ".") return null;
    index += 1;
  }
  return null;
}

export function tomlKeyPathName(path: string[]): string {
  return path.map((part) => /^[A-Za-z0-9_-]+$/.test(part) ? part : JSON.stringify(part)).join(".");
}

export function codexProviderSectionName(provider: string): string {
  return tomlKeyPathName(["model_providers", provider]);
}

export function tomlKeyAssignment(line: string): { path: string[]; valueStart: number } | null {
  for (let index = 0; index < line.length; index += 1) {
    if (line[index] === '"' || line[index] === "'") {
      const quoted = tomlQuotedValue(line, index);
      if (!quoted) return null;
      index = quoted.end - 1;
    } else if (line[index] === "#") return null;
    else if (line[index] === "=") {
      const path = tomlDottedKeyPath(line.slice(0, index));
      return path ? { path, valueStart: index + 1 } : null;
    }
  }
  return null;
}

export function tomlInlineComment(line: string, start: number): string {
  for (let index = start; index < line.length; index += 1) {
    if (line[index] === '"' || line[index] === "'") {
      const quoted = tomlQuotedValue(line, index);
      if (!quoted) return "";
      index = quoted.end - 1;
    } else if (line[index] === "#") {
      const spacing = /[ \t]*$/.exec(line.slice(start, index))?.[0] || "";
      return spacing + line.slice(index);
    }
  }
  return "";
}

export function tomlStringAssignmentValue(line: string, key: string): string | null {
  const assignment = tomlKeyAssignment(line);
  if (!assignment || assignment.path.length !== 1 || assignment.path[0] !== key) return null;
  const text = line.slice(assignment.valueStart).trimStart();
  const quoted = tomlQuotedValue(text, 0);
  if (!quoted) return null;
  const rest = text.slice(quoted.end).trimStart();
  return !rest || rest.startsWith("#") ? quoted.value : null;
}

export function setAuthOpenAiApiKey(contents: string, apiKey: string): string {
  let parsed: Record<string, unknown> = {};
  try {
    const value = JSON.parse(contents || "{}");
    if (value && typeof value === "object" && !Array.isArray(value)) parsed = value as Record<string, unknown>;
  } catch {
    parsed = {};
  }
  parsed.OPENAI_API_KEY = apiKey.trim();
  return `${JSON.stringify(parsed, null, 2)}\n`;
}

export function setRootTomlStringKey(contents: string, key: string, value: string): string {
  const trimmed = value.trim();
  if (!trimmed) return removeRootTomlKey(contents, key);
  return setRootTomlLine(contents, key, `${key} = "${tomlString(trimmed)}"`);
}

export function setManagedOpenAiBaseUrl(contents: string, enabled: boolean): string {
  const current = rootTomlStringValue(contents, "openai_base_url");
  if (enabled) {
    return !current || current === PROTOCOL_PROXY_BASE_URL
      ? setRootTomlStringKey(contents, "openai_base_url", PROTOCOL_PROXY_BASE_URL)
      : contents;
  }
  return current === PROTOCOL_PROXY_BASE_URL ? removeRootTomlKey(contents, "openai_base_url") : contents;
}

export function setRootTomlIntKey(contents: string, key: string, value: string): string {
  const trimmed = value.replace(/[^\d]/g, "");
  if (!trimmed) return removeRootTomlKey(contents, key);
  return setRootTomlLine(contents, key, `${key} = ${trimmed}`);
}

export function setRootTomlLine(contents: string, key: string, lineText: string): string {
  const lines = contents.split(/\r?\n/);
  const structural = tomlStructureMask(lines);
  const firstTable = lines.findIndex((line, index) => structural[index] && tomlSectionName(line) !== null);
  const rootEnd = firstTable >= 0 ? firstTable : lines.length;
  for (let index = 0; index < rootEnd; index += 1) {
    if (!structural[index]) continue;
    const assignment = tomlKeyAssignment(lines[index]);
    if (assignment?.path.length === 1 && assignment.path[0] === key) {
      const replacement = tomlKeyAssignment(lineText);
      lines[index] = replacement
        ? lines[index].slice(0, assignment.valueStart) + " " + lineText.slice(replacement.valueStart).trimStart() + tomlInlineComment(lines[index], assignment.valueStart)
        : lineText;
      return ensureTrailingNewline(lines.join("\n").trimEnd());
    }
  }
  const insertAt = key === "model" ? 0 : rootEnd;
  lines.splice(insertAt, 0, lineText);
  return ensureTrailingNewline(lines.join("\n").trimEnd());
}

export function codexRequiresOpenAiAuthFromConfig(contents: string): boolean {
  const provider = rootTomlStringValue(contents, "model_provider");
  const targetSection = provider ? codexProviderSectionName(provider) : "";
  const lines = contents.split(/\r?\n/);
  const structural = tomlStructureMask(lines);
  let currentSection = "";
  let sawProviderSection = false;
  let rootRequiresAuth = false;

  for (const [index, line] of lines.entries()) {
    if (!structural[index]) continue;
    const section = tomlSectionName(line);
    if (section !== null) {
      currentSection = section;
      if (section.startsWith("model_providers.")) sawProviderSection = true;
      continue;
    }
    const match = /^\s*requires_openai_auth\s*=\s*(true|false)\s*(?:#.*)?$/i.exec(line);
    if (match && currentSection === "") rootRequiresAuth = match[1].toLowerCase() === "true";
    if (!match || !currentSection.startsWith("model_providers.")) continue;
    if (targetSection) {
      if (currentSection === targetSection) return match[1].toLowerCase() === "true";
      continue;
    }
    if (match[1].toLowerCase() === "true") return true;
  }

  return !sawProviderSection && rootRequiresAuth;
}

export function setCodexProviderStringKey(
  contents: string,
  key: string,
  value: string,
  options: { requiresOpenAiAuth?: boolean } = {},
): string {
  const sessionProvider = rootTomlStringValue(contents, "model_provider") || "custom";
  const provider = sessionProvider === "openai" ? "custom" : sessionProvider;
  let next = contents;
  if (!rootTomlStringValue(next, "model_provider")) {
    next = setRootTomlStringKey(next, "model_provider", sessionProvider);
  }
  next = ensureCodexProviderDefaults(next, provider, { requiresOpenAiAuth: options.requiresOpenAiAuth !== false });
  return setTomlSectionStringKey(next, codexProviderSectionName(provider), key, value);
}

export function setCodexExperimentalBearerToken(contents: string, apiKey: string, options: { requiresOpenAiAuth?: boolean } = {}): string {
  const trimmed = apiKey.trim();
  if (trimmed && options.requiresOpenAiAuth === false) {
    // 补纯 API 鉴权时只改目标字段，不覆盖导入模板的名称、协议或登录要求。
    const sessionProvider = rootTomlStringValue(contents, "model_provider") || "custom";
    const provider = sessionProvider === "openai" ? "custom" : sessionProvider;
    const next = rootTomlStringValue(contents, "model_provider")
      ? contents
      : setRootTomlStringKey(contents, "model_provider", sessionProvider);
    return setTomlSectionStringKey(next, codexProviderSectionName(provider), "experimental_bearer_token", trimmed);
  }
  return trimmed
    ? setCodexProviderStringKey(contents, "experimental_bearer_token", trimmed, options)
    : removeCodexExperimentalBearerToken(contents);
}

export function removeCodexExperimentalBearerToken(contents: string): string {
  const sessionProvider = rootTomlStringValue(contents, "model_provider") || "custom";
  const provider = sessionProvider === "openai" ? "custom" : sessionProvider;
  return removeTomlSectionKey(contents, codexProviderSectionName(provider), "experimental_bearer_token");
}

export function ensureCodexProviderDefaults(
  contents: string,
  provider: string,
  options: { requiresOpenAiAuth?: boolean } = {},
): string {
  let next = contents;
  const section = codexProviderSectionName(provider);
  // 已有非空 name 必须原样保留：name 只是展示标签，用户可能改成 "My relay"，
  // 无条件写回 provider id 会在改 Base URL 等字段时把自定义名覆盖掉（issue #2682）。
  next = setTomlSectionStringKey(next, section, "name", resolveProviderName(contents, provider));
  next = setTomlSectionStringKey(next, section, "wire_api", "responses");
  return options.requiresOpenAiAuth === false ? next : setTomlSectionBoolKey(next, section, "requires_openai_auth", true);
}

export function setTomlSectionBoolKey(contents: string, sectionName: string, key: string, value: boolean): string {
  return setTomlSectionRawKey(contents, sectionName, key, value ? "true" : "false");
}

export function setTomlSectionStringKey(contents: string, sectionName: string, key: string, value: string): string {
  return setTomlSectionRawKey(contents, sectionName, key, `"${tomlString(value.trim())}"`);
}

export function setTomlSectionRawKey(contents: string, sectionName: string, key: string, value: string): string {
  const lines = contents.split(/\r?\n/);
  const structural = tomlStructureMask(lines);
  let sectionStart = -1;
  let sectionEnd = lines.length;
  for (let index = 0; index < lines.length; index += 1) {
    if (!structural[index]) continue;
    const section = tomlSectionName(lines[index]);
    if (section === null) continue;
    if (sectionStart >= 0) {
      sectionEnd = index;
      break;
    }
    if (section === sectionName) sectionStart = index;
  }
  if (sectionStart < 0) {
    const prefix = ensureTrailingNewline(lines.join("\n").trimEnd()).trimEnd();
    return joinTomlSections([prefix, `[${sectionName}]\n${key} = ${value}`]);
  }
  const replacement = `${key} = ${value}`;
  for (let index = sectionStart + 1; index < sectionEnd; index += 1) {
    if (!structural[index]) continue;
    const assignment = tomlKeyAssignment(lines[index]);
    if (assignment?.path.length === 1 && assignment.path[0] === key) {
      lines[index] = lines[index].slice(0, assignment.valueStart) + " " + value + tomlInlineComment(lines[index], assignment.valueStart);
      return ensureTrailingNewline(lines.join("\n").trimEnd());
    }
  }
  let insertAt = sectionEnd;
  while (insertAt > sectionStart + 1 && lines[insertAt - 1].trim() === "") insertAt -= 1;
  lines.splice(insertAt, 0, replacement);
  return ensureTrailingNewline(lines.join("\n").trimEnd());
}

export function removeTomlSectionKey(contents: string, sectionName: string, key: string): string {
  const lines = contents.split(/\r?\n/);
  const structural = tomlStructureMask(lines);
  let sectionStart = -1;
  let sectionEnd = lines.length;
  for (let index = 0; index < lines.length; index += 1) {
    if (!structural[index]) continue;
    const section = tomlSectionName(lines[index]);
    if (section === null) continue;
    if (sectionStart >= 0) {
      sectionEnd = index;
      break;
    }
    if (section === sectionName) sectionStart = index;
  }
  if (sectionStart < 0) return contents;
  const next = lines.filter((line, index) => {
    if (index <= sectionStart || index >= sectionEnd || !structural[index]) return true;
    const assignment = tomlKeyAssignment(line);
    return assignment?.path.length !== 1 || assignment.path[0] !== key;
  });
  return ensureTrailingNewline(next.join("\n").trimEnd());
}

export function relayProfileSwitchValidation(profile: RelayProfile, settings: BackendSettings | null = null): string | null {
  const sessionProviderError = relaySessionProviderValidation(profile);
  if (sessionProviderError) return sessionProviderError;
  if (isAggregateRelayProfile(profile)) {
    return aggregateRelayProfileValidation(profile);
  }
  const modelRouteError = relayModelRoutesValidation(profile, settings);
  if (modelRouteError) return modelRouteError;
  if (profile.relayMode === "official" && !profile.officialMixApiKey) return null;
  if (!profile.configContents.trim()) {
    return tf("供应商「{0}」缺少独立 config.toml，已停止切换，避免继续显示上一套配置文件。请先在该供应商详情里保存 config.toml。", [profile.name || profile.id]);
  }
  if (profile.relayMode !== "official" || !authJsonHasOpenAiApiKey(profile.authContents)) return null;
  return t("官方混合 API 不应在 auth.json 中保存 OPENAI_API_KEY。请清理此供应商的 auth.json 后再切换。");
}

export function relayModelRoutesValidation(profile: RelayProfile, settings: BackendSettings | null): string | null {
  const issue = findRelayModelRouteIssue([profile], settings?.relayProfiles ?? [profile]);
  return relayModelRouteIssueMessage(issue);
}

export function relayModelRoutesSettingsValidation(settings: BackendSettings): string | null {
  return relayModelRouteIssueMessage(
    findRelayModelRouteIssue(settings.relayProfiles, settings.relayProfiles),
  );
}

export function relaySettingsValidation(settings: BackendSettings): string | null {
  for (const profile of settings.relayProfiles) {
    const sessionProviderError = relaySessionProviderValidation(profile);
    if (sessionProviderError) return sessionProviderError;
  }
  return relayModelRoutesSettingsValidation(settings);
}

export function relaySessionProviderValidation(profile: RelayProfile): string | null {
  if (relaySessionProvider(profile) === "openai" && profile.protocol !== "responses") {
    return t("OpenAI 会话身份仅支持 Responses API；Chat Completions 不支持 ChatGPT Remote 的远程压缩。请切换协议或改回 Custom。");
  }
  return null;
}

export function relayModelRouteIssueMessage(issue: ReturnType<typeof findRelayModelRouteIssue>): string | null {
  if (!issue) return null;
  switch (issue.kind) {
    case "incomplete":
      return t("单模型协议转换需要填写模型名称和目标供应商。");
    case "duplicate":
      return tf("模型「{0}」存在重复路由。", [issue.model]);
    case "self":
      return tf("模型「{0}」不能路由到当前供应商自身。", [issue.model]);
    case "missingTarget":
      return tf("模型「{0}」的目标供应商不存在。", [issue.model]);
    case "aggregateTarget":
      return tf("模型「{0}」不能路由到聚合供应商。", [issue.model]);
    case "targetProtocol":
      return tf("模型「{0}」的目标供应商必须使用 Responses API。", [issue.model]);
    case "targetCredentials":
      return tf("模型「{0}」的目标供应商缺少 Base URL 或 Key。", [issue.model]);
  }
}

export function relaySettingsWithDraft(
  settings: BackendSettings,
  profileId: string,
  draft: RelayProfile,
  isNew: boolean,
): BackendSettings {
  const normalizedDraft = isAggregateRelayProfile(draft)
    ? normalizeAggregateRelayProfile(draft, settings)
    : deriveRelayProfileFromFiles(draft);
  return isNew
    ? addRelayProfile(settings, normalizedDraft)
    : updateRelayProfile(settings, profileId, normalizedDraft);
}

export function relayProfileUsesLiveFiles(profile: RelayProfile): boolean {
  return profile.relayMode !== "official" || profile.officialMixApiKey || !!profile.configContents.trim();
}

export function authJsonHasOpenAiApiKey(contents: string): boolean {
  const trimmed = contents.trim();
  if (!trimmed) return false;
  try {
    const value = JSON.parse(trimmed);
    return !!value && typeof value === "object" && typeof value.OPENAI_API_KEY === "string" && value.OPENAI_API_KEY.trim().length > 0;
  } catch {
    return /"OPENAI_API_KEY"\s*:/.test(trimmed);
  }
}

export function tomlString(value: string): string {
  return value.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
}

export function syncLegacyRelayFields(settings: BackendSettings): BackendSettings {
  // 先规范化非聚合供应商：聚合成员的"存在性"要用**规范化后仍然在列表里**的那些 id，
  // 否则同一批 profile 里排在聚合后面的成员会被当成不存在而丢掉（顺序相关的老问题）。
  const nonAggregate = settings.relayProfiles
    .filter((profile) => !isAggregateRelayProfile(profile))
    .map((profile) => deriveRelayProfileFromFiles(profile));
  const relayProfiles = settings.relayProfiles.map((profile) =>
    isAggregateRelayProfile(profile)
      ? normalizeAggregateRelayProfile(profile, { ...settings, relayProfiles: [...nonAggregate, ...settings.relayProfiles.filter(isAggregateRelayProfile)] })
      : deriveRelayProfileFromFiles(profile),
  );
  const active = activeRelayProfile({ ...settings, relayProfiles });
  const aggregateRelayProfiles = normalizeAggregateProfilesFromRelayProfiles(relayProfiles);
  const activeAggregateRelayId = isAggregateRelayProfile(active) ? active.id : "";
  return {
    ...settings,
    relayProfiles,
    activeRelayId: active.id,
    relayBaseUrl: isAggregateRelayProfile(active) ? PROTOCOL_PROXY_BASE_URL : active.baseUrl,
    relayApiKey: active.apiKey,
    aggregateRelayProfiles,
    activeAggregateRelayId,
  };
}

export function normalizeAggregateProfilesFromRelayProfiles(profiles: RelayProfile[]): AggregateRelayProfile[] {
  const candidates = profiles.filter((profile) => !isAggregateRelayProfile(profile));
  return profiles.filter(isAggregateRelayProfile).map((profile) => {
    const aggregate = normalizeAggregateConfig(profile.aggregate, candidates);
    const memberIds = new Set(aggregate.members.map((member) => member.profileId));
    return {
      id: profile.id,
      name: profile.name || t("聚合供应商"),
      sessionProvider: normalizeRelaySessionProvider(profile.sessionProvider),
      modelAggregationEnabled: aggregate.modelAggregationEnabled === true,
      strategy: aggregate.strategy,
      members: aggregate.members.map((member) => ({
        relayId: member.profileId,
        weight: clampAggregateWeight(member.weight),
      })),
      routes: normalizeAggregateRoutes(aggregate.routes ?? [], { dropEmptyPattern: true, memberIds }).map((route) => ({
        pattern: route.pattern,
        relayId: route.profileId,
        priority: route.priority,
      })),
      defaultRelayId: aggregate.defaultRelayId ?? "",
      syncDefaultMemberLogin: aggregate.syncDefaultMemberLogin === true,
      autoReviewRelayId: aggregate.autoReviewRelayId ?? "",
    };
  });
}
export function updateRelayProfile(settings: BackendSettings, id: string, patch: Partial<RelayProfile>): BackendSettings {
  if (patch.relayMode === "aggregate" || patch.aggregate) {
    return syncLegacyRelayFields({
      ...settings,
      relayProfiles: settings.relayProfiles.map((profile) =>
        profile.id === id ? normalizeAggregateRelayProfile({ ...profile, ...patch }, settings) : profile,
      ),
    });
  }
  return syncLegacyRelayFields({
    ...settings,
    relayProfiles: settings.relayProfiles.map((profile) => {
      if (profile.id !== id) return profile;
      return deriveRelayProfileFromFiles({ ...profile, ...patch });
    }),
  });
}

export function createRelayProfile(settings: BackendSettings): RelayProfile {
  const id = `relay-${Date.now().toString(36)}`;
  const contextSelection = relayProfileEditingHost.contextSelectionForAllEntries(settings);
  const next = {
    id,
    name: tf("供应商 {0}", [settings.relayProfiles.length + 1]),
    model: "",
    baseUrl: relayProfileEditingHost.defaultRelayBaseUrl(),
    upstreamBaseUrl: relayProfileEditingHost.defaultRelayBaseUrl(),
    apiKey: "",
    protocol: "responses" as RelayProtocol,
    relayMode: "official" as RelayMode,
    sessionProvider: "custom" as RelaySessionProvider,
    officialMixApiKey: false,
    hideOfficialUsageAlert: false,
    testModel: "",
    configContents: "",
    authContents: "",
    useCommonConfig: true,
    contextSelection,
    contextSelectionInitialized: true,
    contextWindow: "",
    autoCompactLimit: "",
    modelList: "",
    modelWindows: "",
    modelAutoCompact: "",
    modelMetadata: "",
    modelVlm: "",
    vlmApiKey: "",
    vlmModel: "",
    vlmBaseUrl: "",
    userAgent: "",
    customHeaders: [],
    sub2apiEnabled: false,
    noAuth: false,
    sub2apiMultiplier: "",
    modelRoutes: [],
    standardOpenaiProtocol: false,
    webSearchHistoryCompat: false,
  };
  return withGeneratedRelayFiles(next);
}

export function createAggregateRelayProfile(settings: BackendSettings): RelayProfile {
  const id = `aggregate-${Date.now().toString(36)}`;
  const contextSelection = relayProfileEditingHost.contextSelectionForAllEntries(settings);
  const candidates = aggregateMemberCandidates(settings.relayProfiles, id);
  return normalizeAggregateRelayProfile(
    {
      id,
      name: tf("聚合供应商 {0}", [settings.relayProfiles.filter(isAggregateRelayProfile).length + 1]),
      model: "",
      baseUrl: "",
      upstreamBaseUrl: "",
      apiKey: "",
      protocol: "responses",
      relayMode: "aggregate",
      sessionProvider: "custom",
      officialMixApiKey: false,
      hideOfficialUsageAlert: false,
      testModel: "",
      configContents: "",
      authContents: "",
      useCommonConfig: true,
      contextSelection,
      contextSelectionInitialized: true,
      contextWindow: "",
      autoCompactLimit: "",
      modelList: "",
      modelWindows: "",
      modelAutoCompact: "",
      modelMetadata: "",
      modelVlm: "",
      vlmApiKey: "",
      vlmModel: "",
      vlmBaseUrl: "",
      userAgent: "",
      customHeaders: [],
      sub2apiEnabled: false,
      noAuth: false,
      sub2apiMultiplier: "",
      modelRoutes: [],
      standardOpenaiProtocol: false,
      webSearchHistoryCompat: false,
      aggregate: {
        strategy: "failover",
        modelAggregationEnabled: false,
        members: candidates.slice(0, 1).map((profile) => ({ profileId: profile.id, weight: 1 })),
        defaultRelayId: "",
      },
    },
    settings,
  );
}

export function addRelayProfile(settings: BackendSettings, profile: RelayProfile): BackendSettings {
  const nextWithFiles = isAggregateRelayProfile(profile)
    ? normalizeAggregateRelayProfile(profile, settings)
    : deriveRelayProfileFromFiles(
        profile.configContents.trim() || profile.authContents.trim() ? profile : withGeneratedRelayFiles(profile),
      );
  const activeId = settings.relayProfiles.some((item) => item.id === settings.activeRelayId)
    ? settings.activeRelayId
    : activeRelayProfile(settings).id;
  return syncLegacyRelayFields({
    ...settings,
    relayProfiles: [...settings.relayProfiles, nextWithFiles],
    activeRelayId: activeId,
  });
}

export function duplicateRelayProfile(settings: BackendSettings, id: string): BackendSettings {
  const sourceIndex = settings.relayProfiles.findIndex((profile) => profile.id === id);
  const source = settings.relayProfiles[sourceIndex] || activeRelayProfile(settings);
  const nextId = `relay-${Date.now().toString(36)}`;
  const next = {
    ...source,
    id: nextId,
    name: tf("{0} 副本", [source.name || t("未命名供应商")]),
  };
  const normalizedNext = isAggregateRelayProfile(next) ? normalizeAggregateRelayProfile(next, settings) : next;
  const relayProfiles = [...settings.relayProfiles];
  relayProfiles.splice(sourceIndex >= 0 ? sourceIndex + 1 : relayProfiles.length, 0, normalizedNext);
  return syncLegacyRelayFields({
    ...settings,
    relayProfiles,
  });
}

export function reorderRelayProfiles(settings: BackendSettings, sourceId: string, targetId: string): BackendSettings {
  if (sourceId === targetId) return settings;
  const sourceIndex = settings.relayProfiles.findIndex((profile) => profile.id === sourceId);
  const targetIndex = settings.relayProfiles.findIndex((profile) => profile.id === targetId);
  if (sourceIndex < 0 || targetIndex < 0) return settings;
  const relayProfiles = [...settings.relayProfiles];
  const [moved] = relayProfiles.splice(sourceIndex, 1);
  relayProfiles.splice(targetIndex, 0, moved);
  return syncLegacyRelayFields({
    ...settings,
    relayProfiles,
  });
}

export function removeRelayProfile(settings: BackendSettings, id: string): BackendSettings {
  if (relayProfileDeleteIssue(settings, id)) return settings;
  const profiles = settings.relayProfiles.filter((profile) => profile.id !== id);
  const scrubbedProfiles = profiles.map((profile) =>
    isAggregateRelayProfile(profile)
      ? normalizeAggregateRelayProfile(
          {
            ...profile,
            aggregate: {
              ...normalizeAggregateConfig(profile.aggregate, [], new Set(profiles.map((item) => item.id))),
              // 用户显式删除的这家必须从成员/路由里移除：存在性判定会保留它
              // （profiles 里已经没有了，其实也会被丢掉），这里再做一次明确的过滤。
              members: normalizeAggregateConfig(profile.aggregate, [], new Set(profiles.map((item) => item.id)))
                .members.filter((member) => member.profileId !== id),
              routes: (normalizeAggregateConfig(profile.aggregate, [], new Set(profiles.map((item) => item.id))).routes ?? [])
                .filter((route) => route.profileId !== id),
            },
          },
          { ...settings, relayProfiles: profiles },
        )
      : {
          ...profile,
          modelRoutes: normalizeRelayModelRoutes(profile.modelRoutes).filter((route) => route.targetRelayId !== id),
        },
  );
  return syncLegacyRelayFields({
    ...settings,
    relayProfiles: scrubbedProfiles.length ? scrubbedProfiles : relayProfileEditingHost.defaultRelayProfiles(),
    activeRelayId: settings.activeRelayId === id ? scrubbedProfiles[0]?.id || "default" : settings.activeRelayId,
  });
}

export function relayProfileDeleteIssue(settings: BackendSettings, id: string): string | null {
  if (settings.activeRelayId === id) {
    return t("当前正在使用该供应商，请先切换到其他供应商后再删除。");
  }
  const existingProfileIds = new Set(settings.relayProfiles.map((profile) => profile.id));
  for (const aggregateProfile of settings.relayProfiles.filter(isAggregateRelayProfile)) {
    const aggregate = normalizeAggregateConfig(aggregateProfile.aggregate, [], existingProfileIds);
    if (!aggregate.members.some((member) => member.profileId === id)) continue;
    if (aggregate.defaultRelayId === id) {
      return tf("供应商「{0}」是聚合配置「{1}」的默认成员，请先调整聚合配置后再删除。", [
        settings.relayProfiles.find((profile) => profile.id === id)?.name || id,
        aggregateProfile.name || aggregateProfile.id,
      ]);
    }
    if (aggregate.members.length <= 1) {
      return tf("供应商「{0}」是聚合配置「{1}」的最后成员，请先调整聚合配置后再删除。", [
        settings.relayProfiles.find((profile) => profile.id === id)?.name || id,
        aggregateProfile.name || aggregateProfile.id,
      ]);
    }
  }
  return null;
}

export const aggregateStrategyOptions: Array<{ value: RelayAggregateStrategy; label: string; description: string }> = [
  {
    value: "failover",
    get label() { return t("失败切换"); },
    get description() { return t("按成员顺序请求，失败后切到下一个供应商。"); },
  },
  {
    value: "conversationRoundRobin",
    get label() { return t("按对话轮转"); },
    get description() { return t("同一对话保持一个成员，不同对话依次分配。"); },
  },
  {
    value: "requestRoundRobin",
    get label() { return t("按请求轮转"); },
    get description() { return t("每次请求按成员顺序切换，适合均匀摊请求量。"); },
  },
  {
    value: "weightedRoundRobin",
    get label() { return t("权重轮转"); },
    get description() { return t("按成员权重分配请求，权重越高承担越多。"); },
  },
];

export function isAggregateRelayProfile(profile: Pick<RelayProfile, "relayMode" | "aggregate">): boolean {
  return isAggregateProfile(profile);
}

export type ProviderViewKind = "direct" | "routing" | "failover" | "modelAggregation";

export function relayProviderView(profile: RelayProfile): ProviderViewKind {
  if (isAggregateRelayProfile(profile)) {
    return profile.aggregate?.modelAggregationEnabled === true ? "modelAggregation" : "failover";
  }
  return profile.protocol !== "responses" || normalizeRelayModelRoutes(profile.modelRoutes).length
    ? "routing"
    : "direct";
}

export function normalizeAggregateRelayProfile(profile: RelayProfile, settings: BackendSettings | null): RelayProfile {
  const candidates = settings ? aggregateMemberCandidates(settings.relayProfiles, profile.id) : [];
  // 传"全部供应商 id"而不是候选：成员的存在性判定不能受"当前是否就绪"影响。
  const existingProfileIds = settings ? new Set(settings.relayProfiles.map((item) => item.id)) : null;
  const aggregate = normalizeAggregateConfig(profile.aggregate, candidates, existingProfileIds);
  return {
    ...profile,
    baseUrl: "",
    upstreamBaseUrl: "",
    apiKey: "",
    protocol: "responses",
    relayMode: "aggregate",
    sessionProvider: normalizeRelaySessionProvider(profile.sessionProvider),
    officialMixApiKey: false,
    hideOfficialUsageAlert: false,
    configContents: "",
    authContents: "",
    sub2apiEnabled: false,
    noAuth: false,
    sub2apiMultiplier: "",
    standardOpenaiProtocol: false,
    webSearchHistoryCompat: false,
    aggregate,
  };
}

// 创建时选定类型；重新创建草稿时不沿用另一类的路由、权重或测试模型。
export function aggregateProfileForMode(profile: RelayProfile, settings: BackendSettings, mode: AggregateMode): RelayProfile {
  const members = (profile.aggregate?.members ?? []).map((member) => ({ profileId: member.profileId, weight: 1 }));
  // 模型聚合总要有一个默认供应商（对标 CC Switch）：沿用仍合法的旧值，否则默认第一个成员。
  const defaultRelayId = mode === "model"
    ? normalizeAggregateDefaultRelayId(
        profile.aggregate?.defaultRelayId,
        new Set(members.map((member) => member.profileId)),
      ) || members[0]?.profileId || ""
    : "";
  return normalizeAggregateRelayProfile({
    ...profile,
    testModel: "",
    aggregate: {
      strategy: "failover",
      modelAggregationEnabled: mode === "model",
      members,
      routes: [],
      defaultRelayId,
    },
  }, settings);
}

export function normalizeAggregateConfig(
  aggregate: RelayAggregateConfig | null | undefined,
  candidates: RelayProfile[],
  /**
   * 当前快照里**全部**供应商 id（不只是可勾选候选）。传了才判定成员的存在性；不传
   * （分页子集、拿不到完整快照）则一律保留成员——宁可留着让用户自己取消勾选，
   * 也不能替用户删配置。
   */
  existingProfileIds: ReadonlySet<string> | null = null,
): RelayAggregateConfig {
  const candidateIds = new Set(candidates.map((profile) => profile.id));
  const seen = new Set<string>();
  const strategy: RelayAggregateStrategy =
    aggregate?.strategy && aggregateStrategyOptions.some((option) => option.value === aggregate.strategy)
      ? aggregate.strategy
      : "failover";
  const modelAggregationEnabled = aggregate?.modelAggregationEnabled === true;
  // 成员去留只看"这家供应商还在不在"：候选集合只决定 UI 能勾选什么，不能用来删掉
  // 已经在聚合里的成员（供应商编辑中缺 Key 时判定会翻转，见 normalizeAggregateMembers）。
  const members = normalizeAggregateMembers(aggregate?.members ?? [], existingProfileIds)
    .map((member) => {
      seen.add(member.profileId);
      return { profileId: member.profileId, weight: modelAggregationEnabled ? 1 : clampAggregateWeight(member.weight) };
    });
  // 路由目标同样只按"供应商还在不在"去留：路由早就配好了，目标一时不就绪（或同一批
  // settings 里排在聚合后面）不代表用户想删掉这条路由。
  const routes = modelAggregationEnabled ? [] : (aggregate?.routes ?? [])
    .filter((route) => route.pattern.trim() !== "" || route.profileId.trim() !== "")
    .filter((route) => !existingProfileIds || !route.profileId.trim() || existingProfileIds.has(route.profileId.trim()))
    .map((route) => ({
      pattern: route.pattern.trim(),
      profileId: route.profileId,
      priority: clampAggregateRoutePriority(route.priority),
    }));
  // 默认供应商必须是当前成员；成员被移出（或值不再合法）时自动清空为「未设置」。
  const defaultRelayId = normalizeAggregateDefaultRelayId(
    aggregate?.defaultRelayId,
    new Set(members.map((member) => member.profileId)),
  );
  // 审批成员同样必须是当前成员；成员被移出时清空为「未设置」（#2703）。
  const autoReviewRelayId = normalizeAggregateDefaultRelayId(
    aggregate?.autoReviewRelayId,
    new Set(members.map((member) => member.profileId)),
  );
  return {
    strategy: modelAggregationEnabled ? "failover" : strategy,
    modelAggregationEnabled,
    members,
    routes,
    defaultRelayId,
    syncDefaultMemberLogin: aggregate?.syncDefaultMemberLogin === true,
    autoReviewRelayId,
  };
}

export function clampAggregateWeight(value: number): number {
  if (!Number.isFinite(value)) return 1;
  return Math.max(1, Math.min(999, Math.round(value)));
}

export function aggregateStrategyLabel(strategy: RelayAggregateStrategy): string {
  return aggregateStrategyOptions.find((option) => option.value === strategy)?.label ?? t("失败切换");
}

export function aggregateStrategyHelp(strategy: RelayAggregateStrategy): string {
  if (strategy === "failover") return t("失败切换会保留成员顺序，优先使用第一个可用供应商。");
  if (strategy === "conversationRoundRobin") return t("按对话轮转会让同一对话尽量保持固定成员，降低上下文漂移。");
  if (strategy === "requestRoundRobin") return t("按请求轮转会逐请求切换成员，适合供应商能力接近的场景。");
  return t("权重轮转会读取每个成员的权重值，权重越高的成员获得更多请求。");
}

export function aggregateRelayProfileValidation(profile: RelayProfile): string | null {
  const aggregate = normalizeAggregateConfig(profile.aggregate, []);
  if (aggregate.members.length < 1) {
    return t("聚合供应商至少需要勾选 1 个可用成员（API 供应商或官方登录账号）。");
  }
  // 模型聚合必须指定默认供应商（不带前缀的请求只有它能接）。
  if (aggregate.modelAggregationEnabled === true) {
    const defaultRelayId = (aggregate.defaultRelayId ?? "").trim();
    const defaultIsMember = !!defaultRelayId
      && aggregate.members.some((member) => member.profileId === defaultRelayId);
    if (!defaultIsMember) {
      return t("模型聚合必须指定「默认供应商」：在成员列表里点某一行的「设为默认」。不带 cpx- 前缀的请求会原样交给它，没设就报错。");
    }
  }
  const issues = validateAggregateRoutes(
    aggregate.routes ?? [],
    new Set(aggregate.members.map((member) => member.profileId)),
  );
  if (!issues) return null;
  const first = issues[0];
  if (first.code === "emptyPattern") {
    return t("路由规则的模型匹配模式不能为空。");
  }
  if (first.code === "invalidPriority") {
    return tf("路由规则「{0}」的优先级必须是大于等于 0 的整数。", [first.pattern]);
  }
  return tf("路由规则「{0}」的目标供应商必须是聚合成员，请先将其勾选为成员。", [first.pattern]);
}

export function ensureTrailingNewline(value: string) {
  return value.trim() ? `${value}\n` : "";
}

export function removeRootTomlKey(contents: string, key: string): string {
  const lines: string[] = [];
  let inRoot = true;
  const sourceLines = contents.split(/\r?\n/);
  const structural = tomlStructureMask(sourceLines);
  for (const [index, line] of sourceLines.entries()) {
    if (structural[index] && tomlSectionName(line) !== null) inRoot = false;
    const assignment = tomlKeyAssignment(line);
    if (structural[index] && inRoot && assignment?.path.length === 1 && assignment.path[0] === key) continue;
    lines.push(line);
  }
  return ensureTrailingNewline(lines.join("\n").trimEnd());
}

export function joinTomlSections(sections: string[]): string {
  return ensureTrailingNewline(
    sections
      .map((section) => section.trim())
      .filter(Boolean)
      .join("\n\n"),
  );
}

export function splitTomlRootAndTables(section: string): { root: string; tables: string } {
  const lines = section.trim().split(/\r?\n/);
  const structural = tomlStructureMask(lines);
  const firstTable = lines.findIndex((line, index) => structural[index] && tomlSectionName(line) !== null);
  if (firstTable < 0) return { root: lines.join("\n"), tables: "" };
  return {
    root: lines.slice(0, firstTable).join("\n"),
    tables: lines.slice(firstTable).join("\n"),
  };
}

export function normalizeRelaySessionProvider(value: string | undefined): RelaySessionProvider {
  return value === "openai" ? "openai" : "custom";
}

export function relaySessionProviderFromConfig(contents: string): RelaySessionProvider {
  return normalizeRelaySessionProvider(rootTomlStringValue(contents, "model_provider"));
}

export function relaySessionProvider(profile: Pick<RelayProfile, "configContents" | "sessionProvider">): RelaySessionProvider {
  const fromConfig = relaySessionProviderFromConfig(profile.configContents);
  return fromConfig === "openai" || profile.sessionProvider === "openai" ? "openai" : "custom";
}

export const CHAT_UPSTREAM_BASE_URL_KEY = "codex_plus_chat_base_url";

export function activeRelayProfile(settings: BackendSettings): RelayProfile {
  return (
    settings.relayProfiles.find((profile) => profile.id === settings.activeRelayId) ||
    settings.relayProfiles[0] ||
    relayProfileEditingHost.defaultRelayProfiles()[0]
  );
}

export const emptyContextSelection = (): RelayContextSelection => ({
  mcpServers: [],
  skills: [],
  plugins: [],
});

export function normalizeContextSelection(
  selection?: Partial<RelayContextSelection>,
  fallback: RelayContextSelection = emptyContextSelection(),
): RelayContextSelection {
  if (!selection) {
    return {
      mcpServers: [...fallback.mcpServers],
      skills: [...fallback.skills],
      plugins: [...fallback.plugins],
    };
  }
  return {
    mcpServers: Array.isArray(selection?.mcpServers) ? selection.mcpServers.map(String) : [],
    skills: Array.isArray(selection?.skills) ? selection.skills.map(String) : [],
    plugins: Array.isArray(selection?.plugins) ? selection.plugins.map(String) : [],
  };
}

export function normalizeRelayMode(mode: RelayMode | undefined): RelayMode {
  if (mode === "aggregate") return mode;
  if (mode === "pureApi") return mode;
  return "official";
}

/** 协议取值归一化：认不出的值一律回退 Responses，避免脏数据把上游写成直连。 */
export function normalizeRelayProtocol(value: RelayProtocol | undefined): RelayProtocol {
  if (value === "chatCompletions" || value === "anthropicMessages") return value;
  return "responses";
}

/** Anthropic 认证字段：只允许 `x-api-key` 与 `authorization`，缺省用前者。 */
export function normalizeAnthropicAuthHeader(value: string | undefined): string {
  const normalized = (value ?? "").trim().toLowerCase();
  if (normalized === "authorization" || normalized === "bearer") return "authorization";
  if (normalized === "x-api-key" || normalized === "x_api_key" || normalized === "api_key") return "x-api-key";
  return "";
}

/** 思考模式：只认四个取值，其余（含空串）按 auto。 */
export function normalizeAnthropicThinkingMode(value: string | undefined): string {
  const normalized = (value ?? "").trim().toLowerCase();
  if (normalized === "adaptive" || normalized === "budget" || normalized === "off") {
    return normalized;
  }
  return "";
}

/** Anthropic 的 `max_tokens` 兜底值：空/非法按 0（用内置 8192），上限 200000。 */
export function normalizeAnthropicMaxTokens(value: number | undefined): number {
  if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) return 0;
  return Math.min(200000, Math.round(value));
}

export function hydrateAggregateRelayProfile(profile: RelayProfile, aggregate: AggregateRelayProfile | undefined): RelayProfile {
  if (!aggregate) return profile;
  return {
    ...profile,
    name: profile.name || aggregate.name,
    relayMode: "aggregate",
    sessionProvider: normalizeRelaySessionProvider(aggregate.sessionProvider),
    aggregate: {
      strategy: aggregate.strategy,
      modelAggregationEnabled: aggregate.modelAggregationEnabled === true,
      members: aggregate.members.map((member) => ({
        profileId: member.relayId,
        weight: clampAggregateWeight(member.weight),
      })),
      routes: normalizeAggregateRoutes(aggregate.routes ?? []),
      defaultRelayId: aggregate.defaultRelayId ?? "",
      syncDefaultMemberLogin: aggregate.syncDefaultMemberLogin === true,
      autoReviewRelayId: aggregate.autoReviewRelayId ?? "",
    },
  };
}

export function normalizeRelayProfile(profile: RelayProfile, defaultContextSelection = emptyContextSelection()): RelayProfile {
  const legacyMixedApi = profile.relayMode === "mixedApi";
  if (profile.relayMode === "aggregate" || profile.aggregate) {
    return normalizeAggregateRelayProfile(
      {
        ...profile,
        model: profile.model || "",
        baseUrl: "",
        upstreamBaseUrl: "",
        apiKey: "",
        protocol: "responses",
        relayMode: "aggregate",
        sessionProvider: normalizeRelaySessionProvider(profile.sessionProvider),
        officialMixApiKey: false,
        hideOfficialUsageAlert: false,
        testModel: profile.testModel || "",
        configContents: "",
        authContents: "",
        useCommonConfig: profile.useCommonConfig !== false,
        contextSelection: profile.contextSelectionInitialized
          ? normalizeContextSelection(profile.contextSelection)
          : normalizeContextSelection(undefined, defaultContextSelection),
        contextSelectionInitialized: true,
        contextWindow: "",
        autoCompactLimit: "",
        modelList: "",
        modelWindows: "",
        modelAutoCompact: "",
        modelMetadata: "",
        modelRoutes: [],
        sub2apiEnabled: false,
        noAuth: false,
        sub2apiMultiplier: "",
        standardOpenaiProtocol: false,
        webSearchHistoryCompat: false,
      },
      null,
    );
  }
  const relayMode = normalizeRelayMode(profile.relayMode);
  const officialMixApiKey = profile.officialMixApiKey === true || legacyMixedApi;
  let normalized: RelayProfile = {
    ...profile,
    model: profile.model || "",
    baseUrl: profile.baseUrl || relayProfileEditingHost.defaultRelayBaseUrl(),
    upstreamBaseUrl: profile.upstreamBaseUrl || profile.baseUrl || "",
    apiKey: profile.apiKey || "",
    protocol: normalizeRelayProtocol(profile.protocol),
    relayMode,
    sessionProvider: relaySessionProvider(profile),
    officialMixApiKey,
    hideOfficialUsageAlert: profile.hideOfficialUsageAlert === true,
    testModel: profile.testModel || "",
    configContents: profile.configContents || "",
    authContents: relayMode === "official" && !officialMixApiKey ? buildOfficialRelayAuthJson(profile.authContents || "") : profile.authContents || "",
    useCommonConfig: profile.useCommonConfig !== false,
    contextSelection: profile.contextSelectionInitialized
      ? normalizeContextSelection(profile.contextSelection)
      : normalizeContextSelection(undefined, defaultContextSelection),
    contextSelectionInitialized: true,
    contextWindow: profile.contextWindow || "",
    autoCompactLimit: profile.autoCompactLimit || "",
    modelList: profile.modelList || "",
    modelWindows: profile.modelWindows || "",
    modelAutoCompact: profile.modelAutoCompact || "",
    modelMetadata: profile.modelMetadata || "",
    modelRoutes: relayMode === "official" && !officialMixApiKey ? [] : normalizeRelayModelRoutes(profile.modelRoutes),
    userAgent: profile.userAgent || "",
    autoReviewModel: profile.autoReviewModel?.trim() || "",
    multiAgent: profile.multiAgent === "v1" || profile.multiAgent === "v2" ? profile.multiAgent : "",
    customHeaders: profile.customHeaders || [],
    sub2apiEnabled: profile.noAuth ? false : profile.sub2apiEnabled === true,
    sub2apiMultiplier: !profile.noAuth && profile.sub2apiEnabled === true ? profile.sub2apiMultiplier || "" : "",
    standardOpenaiProtocol: profile.standardOpenaiProtocol === true,
    webSearchHistoryCompat: profile.webSearchHistoryCompat === true,
    localCompactionPromptAtEnd: profile.localCompactionPromptAtEnd === true,
    trustUpstreamCatalogLite: profile.trustUpstreamCatalogLite === true,
    strictResponsesReasoning: profile.strictResponsesReasoning === true,
    stripImageGenerationTool: profile.stripImageGenerationTool === true,
    stripResponsesImageDetail: profile.stripResponsesImageDetail === true,
    disableToolSearch: profile.disableToolSearch === true,
    pureApiImageGeneration: profile.pureApiImageGeneration === true,
    anthropicAuthHeader: normalizeAnthropicAuthHeader(profile.anthropicAuthHeader),
    anthropicMaxTokens: normalizeAnthropicMaxTokens(profile.anthropicMaxTokens),
    anthropicPromptCache: profile.anthropicPromptCache !== false,
    anthropicImpersonateClaudeCode: profile.anthropicImpersonateClaudeCode === true,
    anthropicThinkingMode: normalizeAnthropicThinkingMode(profile.anthropicThinkingMode),
  };
  const derived = relayProfileUsesLiveFiles(normalized) ? deriveRelayProfileFromFiles(normalized) : normalized;
  return derived.noAuth || (derived.relayMode === "official" && !derived.officialMixApiKey)
    ? derived : relayProfileWithNormalizedApiKeys(derived);
}
