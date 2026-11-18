  // 生成文件，勿手改：node scripts/assemble-provider-editing.mjs
  // 来源：apps/codex-plus-manager/src/relay-profile-editing.ts（与管理器共用的供应商编辑纯函数）
var codexPlusProviderEditing = (() => {
  var __defProp = Object.defineProperty;
  var __getOwnPropDesc = Object.getOwnPropertyDescriptor;
  var __getOwnPropNames = Object.getOwnPropertyNames;
  var __hasOwnProp = Object.prototype.hasOwnProperty;
  var __export = (target, all) => {
    for (var name in all)
      __defProp(target, name, { get: all[name], enumerable: true });
  };
  var __copyProps = (to, from, except, desc) => {
    if (from && typeof from === "object" || typeof from === "function") {
      for (let key of __getOwnPropNames(from))
        if (!__hasOwnProp.call(to, key) && key !== except)
          __defProp(to, key, { get: () => from[key], enumerable: !(desc = __getOwnPropDesc(from, key)) || desc.enumerable });
    }
    return to;
  };
  var __toCommonJS = (mod) => __copyProps(__defProp({}, "__esModule", { value: true }), mod);

  // apps/codex-plus-manager/src/provider-editing-entry.ts
  var provider_editing_entry_exports = {};
  __export(provider_editing_entry_exports, {
    CHAT_UPSTREAM_BASE_URL_KEY: () => CHAT_UPSTREAM_BASE_URL_KEY,
    activeRelayProfile: () => activeRelayProfile,
    addRelayProfile: () => addRelayProfile,
    aggregateProfileForMode: () => aggregateProfileForMode,
    aggregateRelayProfileValidation: () => aggregateRelayProfileValidation,
    aggregateStrategyHelp: () => aggregateStrategyHelp,
    aggregateStrategyLabel: () => aggregateStrategyLabel,
    aggregateStrategyOptions: () => aggregateStrategyOptions,
    applyRelayProfilePatchToFiles: () => applyRelayProfilePatchToFiles,
    authJsonHasOpenAiApiKey: () => authJsonHasOpenAiApiKey,
    buildOfficialRelayAuthJson: () => buildOfficialRelayAuthJson,
    buildRelayAuthJson: () => buildRelayAuthJson,
    buildRelayConfigToml: () => buildRelayConfigToml,
    clampAggregateWeight: () => clampAggregateWeight,
    codexApiKeyFromAuth: () => codexApiKeyFromAuth,
    codexBaseUrlFromConfig: () => codexBaseUrlFromConfig,
    codexExperimentalBearerTokenFromConfig: () => codexExperimentalBearerTokenFromConfig,
    codexModelFromConfig: () => codexModelFromConfig,
    codexProviderSectionName: () => codexProviderSectionName,
    codexProviderStringFromConfig: () => codexProviderStringFromConfig,
    codexRequiresOpenAiAuthFromConfig: () => codexRequiresOpenAiAuthFromConfig,
    codexTopLevelIntFromConfig: () => codexTopLevelIntFromConfig,
    configureRelayProfileEditing: () => configureRelayProfileEditing,
    createAggregateRelayProfile: () => createAggregateRelayProfile,
    createRelayProfile: () => createRelayProfile,
    deriveRelayProfileFromFiles: () => deriveRelayProfileFromFiles,
    duplicateRelayProfile: () => duplicateRelayProfile,
    emptyContextSelection: () => emptyContextSelection,
    ensureCodexProviderDefaults: () => ensureCodexProviderDefaults,
    ensureTrailingNewline: () => ensureTrailingNewline,
    gateway: () => gateway_model_exports,
    gatewaySafety: () => gateway_agent_safety_exports,
    hydrateAggregateRelayProfile: () => hydrateAggregateRelayProfile,
    isAggregateRelayProfile: () => isAggregateRelayProfile,
    joinTomlSections: () => joinTomlSections,
    normalizeAggregateConfig: () => normalizeAggregateConfig,
    normalizeAggregateProfilesFromRelayProfiles: () => normalizeAggregateProfilesFromRelayProfiles,
    normalizeAggregateRelayProfile: () => normalizeAggregateRelayProfile,
    normalizeAnthropicAuthHeader: () => normalizeAnthropicAuthHeader,
    normalizeAnthropicMaxTokens: () => normalizeAnthropicMaxTokens,
    normalizeAnthropicThinkingMode: () => normalizeAnthropicThinkingMode,
    normalizeContextSelection: () => normalizeContextSelection,
    normalizeRelayMode: () => normalizeRelayMode,
    normalizeRelayProfile: () => normalizeRelayProfile,
    normalizeRelayProtocol: () => normalizeRelayProtocol,
    normalizeRelaySessionProvider: () => normalizeRelaySessionProvider,
    parseModelSuffix: () => parseModelSuffix,
    relayModelRouteIssueMessage: () => relayModelRouteIssueMessage,
    relayModelRoutesSettingsValidation: () => relayModelRoutesSettingsValidation,
    relayModelRoutesValidation: () => relayModelRoutesValidation,
    relayProfileDeleteIssue: () => relayProfileDeleteIssue,
    relayProfileEditingHost: () => relayProfileEditingHost,
    relayProfileSwitchValidation: () => relayProfileSwitchValidation,
    relayProfileUsesLiveFiles: () => relayProfileUsesLiveFiles,
    relayProviderView: () => relayProviderView,
    relaySessionProvider: () => relaySessionProvider,
    relaySessionProviderFromConfig: () => relaySessionProviderFromConfig,
    relaySessionProviderValidation: () => relaySessionProviderValidation,
    relaySettingsValidation: () => relaySettingsValidation,
    relaySettingsWithDraft: () => relaySettingsWithDraft,
    removeCodexExperimentalBearerToken: () => removeCodexExperimentalBearerToken,
    removeRelayProfile: () => removeRelayProfile,
    removeRootTomlKey: () => removeRootTomlKey,
    removeTomlSectionKey: () => removeTomlSectionKey,
    reorderRelayProfiles: () => reorderRelayProfiles,
    rootTomlStringValue: () => rootTomlStringValue,
    setAuthOpenAiApiKey: () => setAuthOpenAiApiKey,
    setCodexExperimentalBearerToken: () => setCodexExperimentalBearerToken,
    setCodexProviderStringKey: () => setCodexProviderStringKey,
    setManagedOpenAiBaseUrl: () => setManagedOpenAiBaseUrl,
    setRootTomlIntKey: () => setRootTomlIntKey,
    setRootTomlLine: () => setRootTomlLine,
    setRootTomlStringKey: () => setRootTomlStringKey,
    setTomlSectionBoolKey: () => setTomlSectionBoolKey,
    setTomlSectionRawKey: () => setTomlSectionRawKey,
    setTomlSectionStringKey: () => setTomlSectionStringKey,
    splitTomlRootAndTables: () => splitTomlRootAndTables,
    syncLegacyRelayFields: () => syncLegacyRelayFields,
    tomlDottedKeyPath: () => tomlDottedKeyPath,
    tomlInlineComment: () => tomlInlineComment,
    tomlKeyAssignment: () => tomlKeyAssignment,
    tomlKeyPathName: () => tomlKeyPathName,
    tomlQuotedValue: () => tomlQuotedValue,
    tomlSectionName: () => tomlSectionName,
    tomlString: () => tomlString,
    tomlStringAssignmentValue: () => tomlStringAssignmentValue,
    tomlStructureMask: () => tomlStructureMask,
    updateRelayProfile: () => updateRelayProfile,
    withGeneratedRelayFiles: () => withGeneratedRelayFiles
  });

  // scripts/provider-editing-i18n-shim.mjs
  function t(zh) {
    return zh;
  }
  function tf(key, args) {
    return String(key).replace(/\{(\d+)\}/g, (match, index) => {
      const value = args[Number(index)];
      return value === void 0 || value === null ? match : String(value);
    });
  }

  // apps/codex-plus-manager/src/aggregate-members.ts
  function isAggregateProfile(profile) {
    return profile.relayMode === "aggregate" || !!profile.aggregate;
  }
  function isApiMemberProfile(profile) {
    return Boolean((profile.baseUrl ?? "").trim() && (profile.apiKey ?? "").trim());
  }
  function authContentsHaveChatgptTokens(contents) {
    const trimmed = (contents ?? "").trim();
    if (!trimmed) return false;
    try {
      const value = JSON.parse(trimmed);
      const tokens = value && typeof value === "object" ? value.tokens : void 0;
      if (!tokens || typeof tokens !== "object") return false;
      return [tokens.access_token, tokens.refresh_token].some(
        (token) => typeof token === "string" && token.trim().length > 0
      );
    } catch {
      return /"tokens"\s*:/.test(trimmed) && /"(access_token|refresh_token)"\s*:/.test(trimmed);
    }
  }
  function isOfficialLoginMemberProfile(profile) {
    return profile.relayMode === "official" && !profile.officialMixApiKey && (profile.protocol ?? "responses") === "responses" && authContentsHaveChatgptTokens(profile.authContents);
  }
  function aggregateMemberCandidates(profiles, aggregateId) {
    return profiles.filter(
      (profile) => profile.id !== aggregateId && !isAggregateProfile(profile) && (isApiMemberProfile(profile) || isOfficialLoginMemberProfile(profile))
    );
  }
  function normalizeAggregateDefaultRelayId(value, memberIds) {
    const id = (value ?? "").trim();
    return id && memberIds.has(id) ? id : "";
  }
  function normalizeAggregateMembers(members, profileIds) {
    const seen = /* @__PURE__ */ new Set();
    const normalized = [];
    for (const member of members ?? []) {
      const profileId = String(member?.profileId ?? "").trim();
      if (!profileId || seen.has(profileId)) continue;
      if (profileIds && !profileIds.has(profileId)) continue;
      seen.add(profileId);
      normalized.push({ ...member, profileId });
    }
    return normalized;
  }

  // apps/codex-plus-manager/src/aggregate-routes.ts
  function clampAggregateRoutePriority(value) {
    if (!Number.isFinite(value)) return 0;
    return Math.max(0, Math.min(999, Math.round(value)));
  }
  function normalizeAggregateRoutes(routes, options = {}) {
    const { dropEmptyPattern = false, memberIds, clampPriority = true } = options;
    return routes.map((route) => ({
      pattern: route.pattern.trim(),
      profileId: "profileId" in route ? route.profileId : route.relayId,
      priority: clampPriority ? clampAggregateRoutePriority(route.priority) : route.priority
    })).filter((route) => {
      if (dropEmptyPattern && !route.pattern) return false;
      if (memberIds && !memberIds.has(route.profileId)) return false;
      return true;
    });
  }
  function validateAggregateRoutes(routes, memberIds) {
    const issues = [];
    for (const route of routes) {
      const pattern = route.pattern.trim();
      const profileId = "profileId" in route ? route.profileId : route.relayId;
      if (!pattern && !profileId.trim()) continue;
      if (!pattern) {
        issues.push({ code: "emptyPattern", pattern: "" });
        continue;
      }
      if (!Number.isInteger(route.priority) || route.priority < 0 || route.priority > 999) {
        issues.push({ code: "invalidPriority", pattern });
        continue;
      }
      if (!memberIds.has(profileId)) {
        issues.push({ code: "notMember", pattern });
      }
    }
    return issues.length ? issues : null;
  }

  // apps/codex-plus-manager/src/model-metadata.ts
  var MAX_SAFE_INTEGER = BigInt(Number.MAX_SAFE_INTEGER);
  var PERCENT_SCALE = 1000000n;
  var SCALED_PERCENT_DENOMINATOR = 100n * PERCENT_SCALE;
  var MODEL_SUFFIX_PATTERN = /^(.*?)\[\s*(\d+(?:[KkMm])?)\s*\]$/;
  function parseModelRowName(rowName) {
    const rawName = rowName;
    const trimmed = rowName.trim();
    const match = MODEL_SUFFIX_PATTERN.exec(trimmed);
    const suffixWindow = match?.[1].trim() ? suffixWindowTokens(match[2]) : null;
    const canonicalSlug = match && suffixWindow !== null ? match[1].trim() : trimmed;
    return {
      rawName,
      canonicalSlug,
      suffixWindow: suffixWindow === null ? null : String(suffixWindow),
      key: canonicalSlug.toLowerCase()
    };
  }
  function modelSlugFromRowName(rowName) {
    return parseModelRowName(rowName).canonicalSlug;
  }
  function suffixWindowTokens(suffix) {
    const match = /^(\d+)([KkMm])?$/.exec(suffix.trim());
    if (!match) return null;
    const multiplier = match[2] ? match[2].toLowerCase() === "m" ? 1e6 : 1e3 : 1;
    const tokens = Number(match[1]) * multiplier;
    if (!Number.isSafeInteger(tokens) || tokens <= 0) return null;
    return tokens;
  }
  function suffixWindowString(rowName) {
    return parseModelRowName(rowName).suffixWindow;
  }

  // apps/codex-plus-manager/src/model-routes.ts
  var PROTOCOL_PROXY_BASE_URL = "http://127.0.0.1:57321/v1";
  var independentProxyProfiles = /* @__PURE__ */ new Set();
  function independentProxyEnabled(profile) {
    return !!profile.id && independentProxyProfiles.has(profile.id) && (profile.relayMode !== "official" || profile.officialMixApiKey);
  }
  function normalizeRelayModelRoutes(routes) {
    if (!Array.isArray(routes)) return [];
    return routes.map((route) => ({
      model: typeof route?.model === "string" ? route.model : "",
      targetRelayId: typeof route?.targetRelayId === "string" ? route.targetRelayId : "",
      targetModel: typeof route?.targetModel === "string" ? route.targetModel : ""
    }));
  }
  function findRelayModelRouteIssue(sources, allProfiles) {
    for (const source of sources) {
      if (source.relayMode === "aggregate") continue;
      const seenModels = /* @__PURE__ */ new Set();
      for (const route of normalizeRelayModelRoutes(source.modelRoutes)) {
        const model = route.model.trim();
        const targetRelayId = route.targetRelayId.trim();
        if (!model || !targetRelayId) return { kind: "incomplete", model, sourceRelayId: source.id };
        if (seenModels.has(model)) return { kind: "duplicate", model, sourceRelayId: source.id };
        seenModels.add(model);
        if (targetRelayId === source.id) return { kind: "self", model, sourceRelayId: source.id };
        const target = allProfiles.find((candidate) => candidate.id === targetRelayId);
        if (!target) return { kind: "missingTarget", model, sourceRelayId: source.id };
        if (target.relayMode === "aggregate") return { kind: "aggregateTarget", model, sourceRelayId: source.id };
        if (target.protocol !== "responses") return { kind: "targetProtocol", model, sourceRelayId: source.id };
        const targetHasApiCredentials = !(target.relayMode === "official" && !target.officialMixApiKey) && Boolean(
          target.baseUrl.trim() && (target.apiKey.trim() || target.relayMode === "pureApi" && target.noAuth)
        );
        if (!targetHasApiCredentials) return { kind: "targetCredentials", model, sourceRelayId: source.id };
      }
    }
    return null;
  }
  function stripImageGenerationToolEnabled(profile) {
    return profile.stripImageGenerationTool === true && profile.protocol === "responses" && profile.relayMode !== "aggregate" && (profile.relayMode !== "official" || profile.officialMixApiKey);
  }
  function stripResponsesImageDetailEnabled(profile) {
    return profile.stripResponsesImageDetail === true && profile.protocol === "responses" && profile.relayMode !== "aggregate" && (profile.relayMode !== "official" || profile.officialMixApiKey);
  }
  function pureApiImageGenerationEnabled(profile) {
    return profile.pureApiImageGeneration === true && profile.relayMode === "pureApi";
  }
  function requestCompatibilityEnabled(profile) {
    if (profile.relayMode === "official" && !profile.officialMixApiKey) return false;
    if (profile.protocol === "responses" && profile.strictResponsesReasoning === true) return true;
    if (!profile.modelMetadata?.includes("codex_plus_reasoning_parameters")) return false;
    try {
      const map = JSON.parse(profile.modelMetadata);
      return !!map && typeof map === "object" && !Array.isArray(map) && Object.values(map).some((entry) => entry && typeof entry === "object" && ["omit", "passthrough"].includes(entry.codex_plus_reasoning_parameters));
    } catch {
      return false;
    }
  }

  // apps/codex-plus-manager/src/provider-api-keys.ts
  var DEFAULT_KEY_ID = "default";
  function uniqueKeyId(base, used) {
    let id = base;
    let suffix = 2;
    while (used.has(id)) {
      id = `${base}-${suffix}`;
      suffix += 1;
    }
    return id;
  }
  function normalizeRelayApiKeys(profile) {
    const used = /* @__PURE__ */ new Set();
    const apiKeys = (Array.isArray(profile.apiKeys) ? profile.apiKeys : []).map((entry, index) => {
      const requestedId = String(entry?.id || "").trim() || `key-${index + 1}`;
      const id = uniqueKeyId(requestedId, used);
      used.add(id);
      return {
        id,
        name: String(entry?.name || "").trim() || `Key ${index + 1}`,
        apiKey: String(entry?.apiKey || "")
      };
    });
    if (!apiKeys.length) {
      apiKeys.push({ id: DEFAULT_KEY_ID, name: "默认", apiKey: profile.apiKey || "" });
    }
    const requestedActiveId = String(profile.activeApiKeyId || "").trim();
    const activeApiKeyId = apiKeys.some((entry) => entry.id === requestedActiveId) ? requestedActiveId : apiKeys[0]?.id || "";
    return { apiKeys, activeApiKeyId };
  }
  function relayProfileWithNormalizedApiKeys(profile) {
    const normalized = normalizeRelayApiKeys(profile);
    const selected = normalized.apiKeys.find((entry) => entry.id === normalized.activeApiKeyId);
    return {
      ...profile,
      ...normalized,
      apiKey: selected?.apiKey ?? profile.apiKey
    };
  }
  function relayProfileWithUpdatedApiKey(profile, apiKey) {
    if (profile.noAuth || profile.relayMode === "aggregate" || profile.relayMode === "official" && !profile.officialMixApiKey) return profile;
    const normalized = normalizeRelayApiKeys(profile);
    return {
      ...profile,
      ...normalized,
      apiKey,
      apiKeys: normalized.apiKeys.map((entry) => entry.id === normalized.activeApiKeyId ? { ...entry, apiKey } : entry)
    };
  }

  // apps/codex-plus-manager/src/provider-name.ts
  function tomlSectionStringValue(contents, sectionName, key) {
    const lines = contents.split(/\r?\n/);
    let inSection = false;
    const keyPattern = new RegExp(`^\\s*${key}\\s*=\\s*(["'])(.*)\\1\\s*(?:#.*)?$`);
    for (const line of lines) {
      const section = /^\s*\[([^\]]+)\]\s*$/.exec(line);
      if (section) {
        inSection = section[1].trim() === sectionName;
        continue;
      }
      if (!inSection) continue;
      const match = keyPattern.exec(line.trim());
      if (match) return match[2].replace(/\\(["'\\])/g, "$1");
    }
    return "";
  }
  function resolveProviderName(contents, provider) {
    const existing = tomlSectionStringValue(contents, `model_providers.${provider}`, "name").trim();
    return existing || provider;
  }

  // apps/codex-plus-manager/src/relay-session.ts
  function sessionProviderForProtocol(sessionProvider, protocol) {
    if (protocol !== "responses" && sessionProvider === "openai") return "custom";
    return sessionProvider;
  }

  // apps/codex-plus-manager/src/relay-profile-editing.ts
  var relayProfileEditingHost = {
    defaultRelayBaseUrl: () => "",
    defaultRelayProfiles: () => [],
    contextSelectionForAllEntries: () => ({ mcpServers: [], skills: [], plugins: [] })
  };
  function configureRelayProfileEditing(host) {
    Object.assign(relayProfileEditingHost, host);
  }
  function withGeneratedRelayFiles(profile) {
    if (isAggregateRelayProfile(profile)) {
      return { ...profile, configContents: "", authContents: "", aggregate: normalizeAggregateConfig(profile.aggregate, []) };
    }
    if (profile.relayMode === "official") {
      return {
        ...profile,
        configContents: profile.officialMixApiKey ? buildRelayConfigToml(profile, { includeBearerToken: true, requiresOpenAiAuth: true }) : "",
        authContents: profile.authContents || ""
      };
    }
    return {
      ...profile,
      configContents: buildRelayConfigToml(profile, { includeBearerToken: !profile.noAuth, requiresOpenAiAuth: false }),
      authContents: buildRelayAuthJson(profile)
    };
  }
  function buildRelayConfigToml(profile, options) {
    const searchHistoryCompat = profile.webSearchHistoryCompat === true && profile.protocol === "responses" && profile.relayMode !== "aggregate" && (profile.relayMode !== "official" || profile.officialMixApiKey);
    const localCompaction = profile.localCompactionPromptAtEnd === true && (profile.protocol === "responses" || profile.protocol === "chatCompletions") && profile.relayMode !== "aggregate" && (profile.relayMode !== "official" || profile.officialMixApiKey);
    const baseUrl = profile.protocol !== "responses" || independentProxyEnabled(profile) || searchHistoryCompat || localCompaction || requestCompatibilityEnabled(profile) || stripImageGenerationToolEnabled(profile) || stripResponsesImageDetailEnabled(profile) || pureApiImageGenerationEnabled(profile) ? PROTOCOL_PROXY_BASE_URL : profile.baseUrl.trim();
    const apiKey = profile.apiKey.trim();
    const sessionProvider = normalizeRelaySessionProvider(profile.sessionProvider);
    const rootLines = [
      profile.model.trim() ? `model = "${tomlString(profile.model.trim())}"` : null,
      `model_provider = "${sessionProvider}"`,
      sessionProvider === "openai" ? `openai_base_url = "${PROTOCOL_PROXY_BASE_URL}"` : null,
      ""
    ].filter((line) => line !== null);
    const requiresOpenAiAuthLine = options.requiresOpenAiAuth === void 0 ? null : `requires_openai_auth = ${options.requiresOpenAiAuth ? "true" : "false"}`;
    return [
      ...rootLines,
      "[model_providers.custom]",
      'name = "custom"',
      'wire_api = "responses"',
      requiresOpenAiAuthLine,
      `base_url = "${tomlString(baseUrl)}"`,
      options.includeBearerToken && apiKey ? `experimental_bearer_token = "${tomlString(apiKey)}"` : null,
      ""
    ].filter((line) => line !== null).join("\n");
  }
  function buildRelayAuthJson(profile) {
    return `${JSON.stringify({ OPENAI_API_KEY: profile.apiKey.trim() }, null, 2)}
`;
  }
  function buildOfficialRelayAuthJson(contents) {
    const trimmed = contents.trim();
    if (!trimmed) return "";
    try {
      const parsed = JSON.parse(trimmed);
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return contents;
      const knownOAuth = String(parsed.auth_mode || "").toLowerCase() === "chatgpt" && parsed.tokens && typeof parsed.tokens === "object";
      if (!knownOAuth && Object.keys(parsed).some((key) => key !== "OPENAI_API_KEY")) return contents;
      delete parsed.OPENAI_API_KEY;
      return `${JSON.stringify(parsed, null, 2)}
`;
    } catch {
      return contents;
    }
  }
  function deriveRelayProfileFromFiles(profile, editedFile) {
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
    const apiKey = editedFile === "config" ? configApiKey : editedFile === "auth" && profile.relayMode === "pureApi" ? codexApiKeyFromAuth(authContents) : profile.relayMode === "official" ? configApiKey || profile.apiKey || "" : codexApiKeyFromAuth(authContents) || configApiKey || "";
    if (profile.relayMode === "pureApi") {
      configContents = profile.noAuth ? removeCodexExperimentalBearerToken(configContents) : apiKey && configContents.trim() ? setCodexExperimentalBearerToken(configContents, apiKey, { requiresOpenAiAuth: false }) : configContents;
      if (editedFile && !profile.noAuth) {
        if (editedFile === "config") authContents = setAuthOpenAiApiKey(authContents, apiKey);
        if (!apiKey) configContents = removeCodexExperimentalBearerToken(configContents);
      }
    }
    const configModel = codexModelFromConfig(configContents);
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
      authContents
    };
    return editedFile ? relayProfileWithUpdatedApiKey(derived, apiKey) : derived;
  }
  function applyRelayProfilePatchToFiles(profile, patch, options = {}) {
    const protocol = patch.protocol ?? profile.protocol;
    const sessionProvider = "sessionProvider" in patch ? normalizeRelaySessionProvider(patch.sessionProvider) : relaySessionProvider(profile);
    const compatibleSession = sessionProviderForProtocol(sessionProvider, protocol);
    const normalizedPatch = compatibleSession === sessionProvider ? patch : { ...patch, sessionProvider: compatibleSession };
    let next = { ...profile, ...normalizedPatch };
    if (isAggregateRelayProfile(next)) {
      return normalizeAggregateRelayProfile(next, null);
    }
    const shouldHaveFiles = next.relayMode !== "official" || next.officialMixApiKey || next.configContents.trim() || next.authContents.trim();
    const needsAuthFile = next.relayMode === "pureApi";
    if (options.allowGenerateFiles && shouldHaveFiles && (!next.configContents.trim() || needsAuthFile && !next.authContents.trim())) {
      next = withGeneratedRelayFiles(next);
    }
    if ("sessionProvider" in patch) {
      const sessionProvider2 = normalizeRelaySessionProvider(patch.sessionProvider);
      next.sessionProvider = sessionProvider2;
      next.configContents = setRootTomlStringKey(next.configContents, "model_provider", sessionProvider2);
      next.configContents = setManagedOpenAiBaseUrl(next.configContents, sessionProvider2 === "openai");
    }
    if ("model" in patch) {
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
      const searchHistoryCompat = next.webSearchHistoryCompat === true && next.protocol === "responses" && next.relayMode !== "aggregate" && (next.relayMode !== "official" || next.officialMixApiKey);
      const localCompaction = next.localCompactionPromptAtEnd === true && (next.protocol === "responses" || next.protocol === "chatCompletions") && next.relayMode !== "aggregate" && (next.relayMode !== "official" || next.officialMixApiKey);
      const baseUrlForConfig = next.protocol !== "responses" || independentProxyEnabled(next) || normalizeRelayModelRoutes(next.modelRoutes).length > 0 || searchHistoryCompat || localCompaction || requestCompatibilityEnabled(next) || stripImageGenerationToolEnabled(next) || stripResponsesImageDetailEnabled(next) || pureApiImageGenerationEnabled(next) ? PROTOCOL_PROXY_BASE_URL : next.upstreamBaseUrl || next.baseUrl;
      next.configContents = setCodexProviderStringKey(next.configContents, "base_url", baseUrlForConfig, {
        requiresOpenAiAuth: next.relayMode !== "pureApi"
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
        patch.autoCompactLimit || ""
      );
    }
    if ("relayMode" in patch || "officialMixApiKey" in patch) {
      if (next.relayMode === "official" && !next.officialMixApiKey) {
        next.configContents = "";
        next.authContents = buildOfficialRelayAuthJson(next.authContents);
      } else if (options.allowGenerateFiles && (!next.configContents.trim() || next.relayMode === "pureApi" && !next.authContents.trim())) {
        next = withGeneratedRelayFiles(next);
      }
    }
    return deriveRelayProfileFromFiles(next);
  }
  function codexModelFromConfig(contents) {
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
  function parseModelSuffix(raw) {
    const window = suffixWindowString(raw);
    if (window === null) return { slug: raw.trim() };
    return { slug: modelSlugFromRowName(raw), window: Number(window) };
  }
  function codexBaseUrlFromConfig(contents) {
    return codexProviderStringFromConfig(contents, "base_url");
  }
  function codexExperimentalBearerTokenFromConfig(contents) {
    return codexProviderStringFromConfig(contents, "experimental_bearer_token");
  }
  function codexProviderStringFromConfig(contents, key) {
    const provider = rootTomlStringValue(contents, "model_provider");
    const targetSection = codexProviderSectionName(provider || "custom");
    const fallbackSection = provider === "openai" && rootTomlStringValue(contents, "openai_base_url") === PROTOCOL_PROXY_BASE_URL ? codexProviderSectionName("custom") : "";
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
    return fallbackValue || rootValue;
  }
  function codexApiKeyFromAuth(contents) {
    try {
      const parsed = JSON.parse(contents || "{}");
      return typeof parsed.OPENAI_API_KEY === "string" ? parsed.OPENAI_API_KEY : "";
    } catch {
      return "";
    }
  }
  function codexTopLevelIntFromConfig(contents, key) {
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
  function rootTomlStringValue(contents, key) {
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
  function tomlStructureMask(lines) {
    let multiline = null;
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
  function tomlSectionName(line) {
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
        return path ? (array ? "[]" : "") + tomlKeyPathName(path) : null;
      }
    }
    return null;
  }
  function tomlQuotedValue(text, start) {
    const quote = text[start];
    if (quote !== '"' && quote !== "'") return null;
    let value = "";
    for (let index = start + 1; index < text.length; index += 1) {
      const character = text[index];
      if (character === quote) return { value, end: index + 1 };
      if (character.charCodeAt(0) < 32 && character !== "	" || character.charCodeAt(0) === 127) return null;
      if (quote === '"' && character === "\\") {
        const escape = text[++index];
        const escapes = { '"': '"', "\\": "\\", b: "\b", t: "	", n: "\n", f: "\f", r: "\r" };
        if (Object.prototype.hasOwnProperty.call(escapes, escape)) value += escapes[escape];
        else if (escape === "u" || escape === "U") {
          const length = escape === "u" ? 4 : 8;
          const digits = text.slice(index + 1, index + 1 + length);
          if (digits.length !== length || !/^[0-9a-f]+$/i.test(digits)) return null;
          const codepoint = Number.parseInt(digits, 16);
          if (codepoint > 1114111 || codepoint >= 55296 && codepoint <= 57343) return null;
          value += String.fromCodePoint(codepoint);
          index += length;
        } else return null;
      } else value += character;
    }
    return null;
  }
  function tomlDottedKeyPath(text) {
    const path = [];
    let index = 0;
    while (index < text.length) {
      while (text[index] === " " || text[index] === "	") index += 1;
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
      while (text[index] === " " || text[index] === "	") index += 1;
      if (index === text.length) return path;
      if (text[index] !== ".") return null;
      index += 1;
    }
    return null;
  }
  function tomlKeyPathName(path) {
    return path.map((part) => /^[A-Za-z0-9_-]+$/.test(part) ? part : JSON.stringify(part)).join(".");
  }
  function codexProviderSectionName(provider) {
    return tomlKeyPathName(["model_providers", provider]);
  }
  function tomlKeyAssignment(line) {
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
  function tomlInlineComment(line, start) {
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
  function tomlStringAssignmentValue(line, key) {
    const assignment = tomlKeyAssignment(line);
    if (!assignment || assignment.path.length !== 1 || assignment.path[0] !== key) return null;
    const text = line.slice(assignment.valueStart).trimStart();
    const quoted = tomlQuotedValue(text, 0);
    if (!quoted) return null;
    const rest = text.slice(quoted.end).trimStart();
    return !rest || rest.startsWith("#") ? quoted.value : null;
  }
  function setAuthOpenAiApiKey(contents, apiKey) {
    let parsed = {};
    try {
      const value = JSON.parse(contents || "{}");
      if (value && typeof value === "object" && !Array.isArray(value)) parsed = value;
    } catch {
      parsed = {};
    }
    parsed.OPENAI_API_KEY = apiKey.trim();
    return `${JSON.stringify(parsed, null, 2)}
`;
  }
  function setRootTomlStringKey(contents, key, value) {
    const trimmed = value.trim();
    if (!trimmed) return removeRootTomlKey(contents, key);
    return setRootTomlLine(contents, key, `${key} = "${tomlString(trimmed)}"`);
  }
  function setManagedOpenAiBaseUrl(contents, enabled) {
    const current = rootTomlStringValue(contents, "openai_base_url");
    if (enabled) {
      return !current || current === PROTOCOL_PROXY_BASE_URL ? setRootTomlStringKey(contents, "openai_base_url", PROTOCOL_PROXY_BASE_URL) : contents;
    }
    return current === PROTOCOL_PROXY_BASE_URL ? removeRootTomlKey(contents, "openai_base_url") : contents;
  }
  function setRootTomlIntKey(contents, key, value) {
    const trimmed = value.replace(/[^\d]/g, "");
    if (!trimmed) return removeRootTomlKey(contents, key);
    return setRootTomlLine(contents, key, `${key} = ${trimmed}`);
  }
  function setRootTomlLine(contents, key, lineText) {
    const lines = contents.split(/\r?\n/);
    const structural = tomlStructureMask(lines);
    const firstTable = lines.findIndex((line, index) => structural[index] && tomlSectionName(line) !== null);
    const rootEnd = firstTable >= 0 ? firstTable : lines.length;
    for (let index = 0; index < rootEnd; index += 1) {
      if (!structural[index]) continue;
      const assignment = tomlKeyAssignment(lines[index]);
      if (assignment?.path.length === 1 && assignment.path[0] === key) {
        const replacement = tomlKeyAssignment(lineText);
        lines[index] = replacement ? lines[index].slice(0, assignment.valueStart) + " " + lineText.slice(replacement.valueStart).trimStart() + tomlInlineComment(lines[index], assignment.valueStart) : lineText;
        return ensureTrailingNewline(lines.join("\n").trimEnd());
      }
    }
    const insertAt = key === "model" ? 0 : rootEnd;
    lines.splice(insertAt, 0, lineText);
    return ensureTrailingNewline(lines.join("\n").trimEnd());
  }
  function codexRequiresOpenAiAuthFromConfig(contents) {
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
  function setCodexProviderStringKey(contents, key, value, options = {}) {
    const sessionProvider = rootTomlStringValue(contents, "model_provider") || "custom";
    const provider = sessionProvider === "openai" ? "custom" : sessionProvider;
    let next = contents;
    if (!rootTomlStringValue(next, "model_provider")) {
      next = setRootTomlStringKey(next, "model_provider", sessionProvider);
    }
    next = ensureCodexProviderDefaults(next, provider, { requiresOpenAiAuth: options.requiresOpenAiAuth !== false });
    return setTomlSectionStringKey(next, codexProviderSectionName(provider), key, value);
  }
  function setCodexExperimentalBearerToken(contents, apiKey, options = {}) {
    const trimmed = apiKey.trim();
    if (trimmed && options.requiresOpenAiAuth === false) {
      const sessionProvider = rootTomlStringValue(contents, "model_provider") || "custom";
      const provider = sessionProvider === "openai" ? "custom" : sessionProvider;
      const next = rootTomlStringValue(contents, "model_provider") ? contents : setRootTomlStringKey(contents, "model_provider", sessionProvider);
      return setTomlSectionStringKey(next, codexProviderSectionName(provider), "experimental_bearer_token", trimmed);
    }
    return trimmed ? setCodexProviderStringKey(contents, "experimental_bearer_token", trimmed, options) : removeCodexExperimentalBearerToken(contents);
  }
  function removeCodexExperimentalBearerToken(contents) {
    const sessionProvider = rootTomlStringValue(contents, "model_provider") || "custom";
    const provider = sessionProvider === "openai" ? "custom" : sessionProvider;
    return removeTomlSectionKey(contents, codexProviderSectionName(provider), "experimental_bearer_token");
  }
  function ensureCodexProviderDefaults(contents, provider, options = {}) {
    let next = contents;
    const section = codexProviderSectionName(provider);
    next = setTomlSectionStringKey(next, section, "name", resolveProviderName(contents, provider));
    next = setTomlSectionStringKey(next, section, "wire_api", "responses");
    return options.requiresOpenAiAuth === false ? next : setTomlSectionBoolKey(next, section, "requires_openai_auth", true);
  }
  function setTomlSectionBoolKey(contents, sectionName, key, value) {
    return setTomlSectionRawKey(contents, sectionName, key, value ? "true" : "false");
  }
  function setTomlSectionStringKey(contents, sectionName, key, value) {
    return setTomlSectionRawKey(contents, sectionName, key, `"${tomlString(value.trim())}"`);
  }
  function setTomlSectionRawKey(contents, sectionName, key, value) {
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
      return joinTomlSections([prefix, `[${sectionName}]
${key} = ${value}`]);
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
  function removeTomlSectionKey(contents, sectionName, key) {
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
  function relayProfileSwitchValidation(profile, settings = null) {
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
  function relayModelRoutesValidation(profile, settings) {
    const issue = findRelayModelRouteIssue([profile], settings?.relayProfiles ?? [profile]);
    return relayModelRouteIssueMessage(issue);
  }
  function relayModelRoutesSettingsValidation(settings) {
    return relayModelRouteIssueMessage(
      findRelayModelRouteIssue(settings.relayProfiles, settings.relayProfiles)
    );
  }
  function relaySettingsValidation(settings) {
    for (const profile of settings.relayProfiles) {
      const sessionProviderError = relaySessionProviderValidation(profile);
      if (sessionProviderError) return sessionProviderError;
    }
    return relayModelRoutesSettingsValidation(settings);
  }
  function relaySessionProviderValidation(profile) {
    if (relaySessionProvider(profile) === "openai" && profile.protocol !== "responses") {
      return t("OpenAI 会话身份仅支持 Responses API；Chat Completions 不支持 ChatGPT Remote 的远程压缩。请切换协议或改回 Custom。");
    }
    return null;
  }
  function relayModelRouteIssueMessage(issue) {
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
  function relaySettingsWithDraft(settings, profileId, draft, isNew) {
    const normalizedDraft = isAggregateRelayProfile(draft) ? normalizeAggregateRelayProfile(draft, settings) : deriveRelayProfileFromFiles(draft);
    return isNew ? addRelayProfile(settings, normalizedDraft) : updateRelayProfile(settings, profileId, normalizedDraft);
  }
  function relayProfileUsesLiveFiles(profile) {
    return profile.relayMode !== "official" || profile.officialMixApiKey || !!profile.configContents.trim();
  }
  function authJsonHasOpenAiApiKey(contents) {
    const trimmed = contents.trim();
    if (!trimmed) return false;
    try {
      const value = JSON.parse(trimmed);
      return !!value && typeof value === "object" && typeof value.OPENAI_API_KEY === "string" && value.OPENAI_API_KEY.trim().length > 0;
    } catch {
      return /"OPENAI_API_KEY"\s*:/.test(trimmed);
    }
  }
  function tomlString(value) {
    return value.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
  }
  function syncLegacyRelayFields(settings) {
    const nonAggregate = settings.relayProfiles.filter((profile) => !isAggregateRelayProfile(profile)).map((profile) => deriveRelayProfileFromFiles(profile));
    const relayProfiles = settings.relayProfiles.map(
      (profile) => isAggregateRelayProfile(profile) ? normalizeAggregateRelayProfile(profile, { ...settings, relayProfiles: [...nonAggregate, ...settings.relayProfiles.filter(isAggregateRelayProfile)] }) : deriveRelayProfileFromFiles(profile)
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
      activeAggregateRelayId
    };
  }
  function normalizeAggregateProfilesFromRelayProfiles(profiles) {
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
          weight: clampAggregateWeight(member.weight)
        })),
        routes: normalizeAggregateRoutes(aggregate.routes ?? [], { dropEmptyPattern: true, memberIds }).map((route) => ({
          pattern: route.pattern,
          relayId: route.profileId,
          priority: route.priority
        })),
        defaultRelayId: aggregate.defaultRelayId ?? "",
        syncDefaultMemberLogin: aggregate.syncDefaultMemberLogin === true,
        autoReviewRelayId: aggregate.autoReviewRelayId ?? ""
      };
    });
  }
  function updateRelayProfile(settings, id, patch) {
    if (patch.relayMode === "aggregate" || patch.aggregate) {
      return syncLegacyRelayFields({
        ...settings,
        relayProfiles: settings.relayProfiles.map(
          (profile) => profile.id === id ? normalizeAggregateRelayProfile({ ...profile, ...patch }, settings) : profile
        )
      });
    }
    return syncLegacyRelayFields({
      ...settings,
      relayProfiles: settings.relayProfiles.map((profile) => {
        if (profile.id !== id) return profile;
        return deriveRelayProfileFromFiles({ ...profile, ...patch });
      })
    });
  }
  function createRelayProfile(settings) {
    const id = `relay-${Date.now().toString(36)}`;
    const contextSelection = relayProfileEditingHost.contextSelectionForAllEntries(settings);
    const next = {
      id,
      name: tf("供应商 {0}", [settings.relayProfiles.length + 1]),
      model: "",
      baseUrl: relayProfileEditingHost.defaultRelayBaseUrl(),
      upstreamBaseUrl: relayProfileEditingHost.defaultRelayBaseUrl(),
      apiKey: "",
      protocol: "responses",
      relayMode: "official",
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
      webSearchHistoryCompat: false
    };
    return withGeneratedRelayFiles(next);
  }
  function createAggregateRelayProfile(settings) {
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
          defaultRelayId: ""
        }
      },
      settings
    );
  }
  function addRelayProfile(settings, profile) {
    const nextWithFiles = isAggregateRelayProfile(profile) ? normalizeAggregateRelayProfile(profile, settings) : deriveRelayProfileFromFiles(
      profile.configContents.trim() || profile.authContents.trim() ? profile : withGeneratedRelayFiles(profile)
    );
    const activeId = settings.relayProfiles.some((item) => item.id === settings.activeRelayId) ? settings.activeRelayId : activeRelayProfile(settings).id;
    return syncLegacyRelayFields({
      ...settings,
      relayProfiles: [...settings.relayProfiles, nextWithFiles],
      activeRelayId: activeId
    });
  }
  function duplicateRelayProfile(settings, id) {
    const sourceIndex = settings.relayProfiles.findIndex((profile) => profile.id === id);
    const source = settings.relayProfiles[sourceIndex] || activeRelayProfile(settings);
    const nextId = `relay-${Date.now().toString(36)}`;
    const next = {
      ...source,
      id: nextId,
      name: tf("{0} 副本", [source.name || t("未命名供应商")])
    };
    const normalizedNext = isAggregateRelayProfile(next) ? normalizeAggregateRelayProfile(next, settings) : next;
    const relayProfiles = [...settings.relayProfiles];
    relayProfiles.splice(sourceIndex >= 0 ? sourceIndex + 1 : relayProfiles.length, 0, normalizedNext);
    return syncLegacyRelayFields({
      ...settings,
      relayProfiles
    });
  }
  function reorderRelayProfiles(settings, sourceId, targetId) {
    if (sourceId === targetId) return settings;
    const sourceIndex = settings.relayProfiles.findIndex((profile) => profile.id === sourceId);
    const targetIndex = settings.relayProfiles.findIndex((profile) => profile.id === targetId);
    if (sourceIndex < 0 || targetIndex < 0) return settings;
    const relayProfiles = [...settings.relayProfiles];
    const [moved] = relayProfiles.splice(sourceIndex, 1);
    relayProfiles.splice(targetIndex, 0, moved);
    return syncLegacyRelayFields({
      ...settings,
      relayProfiles
    });
  }
  function removeRelayProfile(settings, id) {
    if (relayProfileDeleteIssue(settings, id)) return settings;
    const profiles = settings.relayProfiles.filter((profile) => profile.id !== id);
    const scrubbedProfiles = profiles.map(
      (profile) => isAggregateRelayProfile(profile) ? normalizeAggregateRelayProfile(
        {
          ...profile,
          aggregate: {
            ...normalizeAggregateConfig(profile.aggregate, [], new Set(profiles.map((item) => item.id))),
            // 用户显式删除的这家必须从成员/路由里移除：存在性判定会保留它
            // （profiles 里已经没有了，其实也会被丢掉），这里再做一次明确的过滤。
            members: normalizeAggregateConfig(profile.aggregate, [], new Set(profiles.map((item) => item.id))).members.filter((member) => member.profileId !== id),
            routes: (normalizeAggregateConfig(profile.aggregate, [], new Set(profiles.map((item) => item.id))).routes ?? []).filter((route) => route.profileId !== id)
          }
        },
        { ...settings, relayProfiles: profiles }
      ) : {
        ...profile,
        modelRoutes: normalizeRelayModelRoutes(profile.modelRoutes).filter((route) => route.targetRelayId !== id)
      }
    );
    return syncLegacyRelayFields({
      ...settings,
      relayProfiles: scrubbedProfiles.length ? scrubbedProfiles : relayProfileEditingHost.defaultRelayProfiles(),
      activeRelayId: settings.activeRelayId === id ? scrubbedProfiles[0]?.id || "default" : settings.activeRelayId
    });
  }
  function relayProfileDeleteIssue(settings, id) {
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
          aggregateProfile.name || aggregateProfile.id
        ]);
      }
      if (aggregate.members.length <= 1) {
        return tf("供应商「{0}」是聚合配置「{1}」的最后成员，请先调整聚合配置后再删除。", [
          settings.relayProfiles.find((profile) => profile.id === id)?.name || id,
          aggregateProfile.name || aggregateProfile.id
        ]);
      }
    }
    return null;
  }
  var aggregateStrategyOptions = [
    {
      value: "failover",
      get label() {
        return t("失败切换");
      },
      get description() {
        return t("按成员顺序请求，失败后切到下一个供应商。");
      }
    },
    {
      value: "conversationRoundRobin",
      get label() {
        return t("按对话轮转");
      },
      get description() {
        return t("同一对话保持一个成员，不同对话依次分配。");
      }
    },
    {
      value: "requestRoundRobin",
      get label() {
        return t("按请求轮转");
      },
      get description() {
        return t("每次请求按成员顺序切换，适合均匀摊请求量。");
      }
    },
    {
      value: "weightedRoundRobin",
      get label() {
        return t("权重轮转");
      },
      get description() {
        return t("按成员权重分配请求，权重越高承担越多。");
      }
    }
  ];
  function isAggregateRelayProfile(profile) {
    return isAggregateProfile(profile);
  }
  function relayProviderView(profile) {
    if (isAggregateRelayProfile(profile)) {
      return profile.aggregate?.modelAggregationEnabled === true ? "modelAggregation" : "failover";
    }
    return profile.protocol !== "responses" || normalizeRelayModelRoutes(profile.modelRoutes).length ? "routing" : "direct";
  }
  function normalizeAggregateRelayProfile(profile, settings) {
    const candidates = settings ? aggregateMemberCandidates(settings.relayProfiles, profile.id) : [];
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
      aggregate
    };
  }
  function aggregateProfileForMode(profile, settings, mode) {
    const members = (profile.aggregate?.members ?? []).map((member) => ({ profileId: member.profileId, weight: 1 }));
    const defaultRelayId = mode === "model" ? normalizeAggregateDefaultRelayId(
      profile.aggregate?.defaultRelayId,
      new Set(members.map((member) => member.profileId))
    ) || members[0]?.profileId || "" : "";
    return normalizeAggregateRelayProfile({
      ...profile,
      testModel: "",
      aggregate: {
        strategy: "failover",
        modelAggregationEnabled: mode === "model",
        members,
        routes: [],
        defaultRelayId
      }
    }, settings);
  }
  function normalizeAggregateConfig(aggregate, candidates, existingProfileIds = null) {
    const candidateIds = new Set(candidates.map((profile) => profile.id));
    const seen = /* @__PURE__ */ new Set();
    const strategy = aggregate?.strategy && aggregateStrategyOptions.some((option) => option.value === aggregate.strategy) ? aggregate.strategy : "failover";
    const modelAggregationEnabled = aggregate?.modelAggregationEnabled === true;
    const members = normalizeAggregateMembers(aggregate?.members ?? [], existingProfileIds).map((member) => {
      seen.add(member.profileId);
      return { profileId: member.profileId, weight: modelAggregationEnabled ? 1 : clampAggregateWeight(member.weight) };
    });
    const routes = modelAggregationEnabled ? [] : (aggregate?.routes ?? []).filter((route) => route.pattern.trim() !== "" || route.profileId.trim() !== "").filter((route) => !existingProfileIds || !route.profileId.trim() || existingProfileIds.has(route.profileId.trim())).map((route) => ({
      pattern: route.pattern.trim(),
      profileId: route.profileId,
      priority: clampAggregateRoutePriority(route.priority)
    }));
    const defaultRelayId = normalizeAggregateDefaultRelayId(
      aggregate?.defaultRelayId,
      new Set(members.map((member) => member.profileId))
    );
    const autoReviewRelayId = normalizeAggregateDefaultRelayId(
      aggregate?.autoReviewRelayId,
      new Set(members.map((member) => member.profileId))
    );
    return {
      strategy: modelAggregationEnabled ? "failover" : strategy,
      modelAggregationEnabled,
      members,
      routes,
      defaultRelayId,
      syncDefaultMemberLogin: aggregate?.syncDefaultMemberLogin === true,
      autoReviewRelayId
    };
  }
  function clampAggregateWeight(value) {
    if (!Number.isFinite(value)) return 1;
    return Math.max(1, Math.min(999, Math.round(value)));
  }
  function aggregateStrategyLabel(strategy) {
    return aggregateStrategyOptions.find((option) => option.value === strategy)?.label ?? t("失败切换");
  }
  function aggregateStrategyHelp(strategy) {
    if (strategy === "failover") return t("失败切换会保留成员顺序，优先使用第一个可用供应商。");
    if (strategy === "conversationRoundRobin") return t("按对话轮转会让同一对话尽量保持固定成员，降低上下文漂移。");
    if (strategy === "requestRoundRobin") return t("按请求轮转会逐请求切换成员，适合供应商能力接近的场景。");
    return t("权重轮转会读取每个成员的权重值，权重越高的成员获得更多请求。");
  }
  function aggregateRelayProfileValidation(profile) {
    const aggregate = normalizeAggregateConfig(profile.aggregate, []);
    if (aggregate.members.length < 1) {
      return t("聚合供应商至少需要勾选 1 个可用成员（API 供应商或官方登录账号）。");
    }
    if (aggregate.modelAggregationEnabled === true) {
      const defaultRelayId = (aggregate.defaultRelayId ?? "").trim();
      const defaultIsMember = !!defaultRelayId && aggregate.members.some((member) => member.profileId === defaultRelayId);
      if (!defaultIsMember) {
        return t("模型聚合必须指定「默认供应商」：在成员列表里点某一行的「设为默认」。不带 cpx- 前缀的请求会原样交给它，没设就报错。");
      }
    }
    const issues = validateAggregateRoutes(
      aggregate.routes ?? [],
      new Set(aggregate.members.map((member) => member.profileId))
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
  function ensureTrailingNewline(value) {
    return value.trim() ? `${value}
` : "";
  }
  function removeRootTomlKey(contents, key) {
    const lines = [];
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
  function joinTomlSections(sections) {
    return ensureTrailingNewline(
      sections.map((section) => section.trim()).filter(Boolean).join("\n\n")
    );
  }
  function splitTomlRootAndTables(section) {
    const lines = section.trim().split(/\r?\n/);
    const structural = tomlStructureMask(lines);
    const firstTable = lines.findIndex((line, index) => structural[index] && tomlSectionName(line) !== null);
    if (firstTable < 0) return { root: lines.join("\n"), tables: "" };
    return {
      root: lines.slice(0, firstTable).join("\n"),
      tables: lines.slice(firstTable).join("\n")
    };
  }
  function normalizeRelaySessionProvider(value) {
    return value === "openai" ? "openai" : "custom";
  }
  function relaySessionProviderFromConfig(contents) {
    return normalizeRelaySessionProvider(rootTomlStringValue(contents, "model_provider"));
  }
  function relaySessionProvider(profile) {
    const fromConfig = relaySessionProviderFromConfig(profile.configContents);
    return fromConfig === "openai" || profile.sessionProvider === "openai" ? "openai" : "custom";
  }
  var CHAT_UPSTREAM_BASE_URL_KEY = "codex_plus_chat_base_url";
  function activeRelayProfile(settings) {
    return settings.relayProfiles.find((profile) => profile.id === settings.activeRelayId) || settings.relayProfiles[0] || relayProfileEditingHost.defaultRelayProfiles()[0];
  }
  var emptyContextSelection = () => ({
    mcpServers: [],
    skills: [],
    plugins: []
  });
  function normalizeContextSelection(selection, fallback = emptyContextSelection()) {
    if (!selection) {
      return {
        mcpServers: [...fallback.mcpServers],
        skills: [...fallback.skills],
        plugins: [...fallback.plugins]
      };
    }
    return {
      mcpServers: Array.isArray(selection?.mcpServers) ? selection.mcpServers.map(String) : [],
      skills: Array.isArray(selection?.skills) ? selection.skills.map(String) : [],
      plugins: Array.isArray(selection?.plugins) ? selection.plugins.map(String) : []
    };
  }
  function normalizeRelayMode(mode) {
    if (mode === "aggregate") return mode;
    if (mode === "pureApi") return mode;
    return "official";
  }
  function normalizeRelayProtocol(value) {
    if (value === "chatCompletions" || value === "anthropicMessages") return value;
    return "responses";
  }
  function normalizeAnthropicAuthHeader(value) {
    const normalized = (value ?? "").trim().toLowerCase();
    if (normalized === "authorization" || normalized === "bearer") return "authorization";
    if (normalized === "x-api-key" || normalized === "x_api_key" || normalized === "api_key") return "x-api-key";
    return "";
  }
  function normalizeAnthropicThinkingMode(value) {
    const normalized = (value ?? "").trim().toLowerCase();
    if (normalized === "adaptive" || normalized === "budget" || normalized === "off") {
      return normalized;
    }
    return "";
  }
  function normalizeAnthropicMaxTokens(value) {
    if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) return 0;
    return Math.min(2e5, Math.round(value));
  }
  function hydrateAggregateRelayProfile(profile, aggregate) {
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
          weight: clampAggregateWeight(member.weight)
        })),
        routes: normalizeAggregateRoutes(aggregate.routes ?? []),
        defaultRelayId: aggregate.defaultRelayId ?? "",
        syncDefaultMemberLogin: aggregate.syncDefaultMemberLogin === true,
        autoReviewRelayId: aggregate.autoReviewRelayId ?? ""
      }
    };
  }
  function normalizeRelayProfile(profile, defaultContextSelection = emptyContextSelection()) {
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
          contextSelection: profile.contextSelectionInitialized ? normalizeContextSelection(profile.contextSelection) : normalizeContextSelection(void 0, defaultContextSelection),
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
          webSearchHistoryCompat: false
        },
        null
      );
    }
    const relayMode = normalizeRelayMode(profile.relayMode);
    const officialMixApiKey = profile.officialMixApiKey === true || legacyMixedApi;
    let normalized = {
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
      contextSelection: profile.contextSelectionInitialized ? normalizeContextSelection(profile.contextSelection) : normalizeContextSelection(void 0, defaultContextSelection),
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
      anthropicThinkingMode: normalizeAnthropicThinkingMode(profile.anthropicThinkingMode)
    };
    const derived = relayProfileUsesLiveFiles(normalized) ? deriveRelayProfileFromFiles(normalized) : normalized;
    return derived.noAuth || derived.relayMode === "official" && !derived.officialMixApiKey ? derived : relayProfileWithNormalizedApiKeys(derived);
  }

  // apps/codex-plus-manager/src/gateway-model.ts
  var gateway_model_exports = {};
  __export(gateway_model_exports, {
    BUILTIN_GATEWAY_AGENTS: () => BUILTIN_GATEWAY_AGENTS,
    COMPACTION_FALLBACK_REASON_PREFIX: () => COMPACTION_FALLBACK_REASON_PREFIX,
    COMPACTION_MODEL_REASON_PREFIX: () => COMPACTION_MODEL_REASON_PREFIX,
    DIRECT_PROXY: () => DIRECT_PROXY,
    FOREIGN_COMPACTION_DROPPED_REASON: () => FOREIGN_COMPACTION_DROPPED_REASON,
    GATEWAY_AGENT_NAMES: () => GATEWAY_AGENT_NAMES,
    GATEWAY_CLAUDE_PROVIDER_ID: () => GATEWAY_CLAUDE_PROVIDER_ID,
    GROUP_MODEL_PREFIX: () => GROUP_MODEL_PREFIX,
    MAX_GATEWAY_PORT: () => MAX_GATEWAY_PORT,
    MIN_GATEWAY_PORT: () => MIN_GATEWAY_PORT,
    QUEUE_REASON_PREFIX: () => QUEUE_REASON_PREFIX,
    RECENT_USE_MS: () => RECENT_USE_MS,
    SPONSORS: () => SPONSORS,
    activeAutoGroup: () => activeAutoGroup,
    activeSponsorIds: () => activeSponsorIds,
    agentModels: () => agentModels,
    agentName: () => agentName,
    agentPoolProviders: () => agentPoolProviders,
    averageDuration: () => averageDuration,
    axisLabelIndexes: () => axisLabelIndexes,
    bindingFromDraft: () => bindingFromDraft,
    blankBinding: () => blankBinding,
    cacheHitRate: () => cacheHitRate,
    compactionNote: () => compactionNote,
    connectedAgents: () => connectedAgents,
    costText: () => costText,
    currencySymbol: () => currencySymbol,
    defaultProviderAgents: () => defaultProviderAgents,
    disabledGroup: () => disabledGroup,
    dispatchStatus: () => dispatchStatus,
    draftFromBinding: () => draftFromBinding,
    failureLabel: () => failureLabel,
    filterSessions: () => filterSessions,
    foreignCompactionDropped: () => foreignCompactionDropped,
    formatElapsed: () => formatElapsed,
    formatMoney: () => formatMoney,
    formatQueueWait: () => formatQueueWait,
    formatRemaining: () => formatRemaining,
    formatRequestCost: () => formatRequestCost,
    formatTokens: () => formatTokens,
    gatewayPresets: () => gatewayPresets,
    glanceAgents: () => glanceAgents,
    globMatches: () => globMatches,
    groupDisplayName: () => groupDisplayName,
    groupModeDescription: () => groupModeDescription,
    groupModeLabel: () => groupModeLabel,
    groupModelId: () => groupModelId,
    groupStickyDescription: () => groupStickyDescription,
    groupStickyLabel: () => groupStickyLabel,
    imageModelLabel: () => imageModelLabel,
    importCandidates: () => importCandidates,
    isDirectProxy: () => isDirectProxy,
    isFavoriteModel: () => isFavoriteModel,
    isPattern: () => isPattern,
    keyLabel: () => keyLabel,
    maskAccountName: () => maskAccountName,
    maskAccountText: () => maskAccountText,
    memberLabel: () => memberLabel,
    memberStatus: () => memberStatus,
    memberTarget: () => memberTarget,
    modelDispatch: () => modelDispatch,
    modelDisplay: () => modelDisplay,
    modelRoutable: () => modelRoutable,
    niceCeiling: () => niceCeiling,
    ownPickerSection: () => ownPickerSection,
    parseCompactionNote: () => parseCompactionNote,
    parseContextWindow: () => parseContextWindow,
    parseGatewayPort: () => parseGatewayPort,
    parseKeyLimit: () => parseKeyLimit,
    parseWeight: () => parseWeight,
    patternMatches: () => patternMatches,
    pendingImportCount: () => pendingImportCount,
    pickerSections: () => pickerSections,
    poolExcludeToggle: () => poolExcludeToggle,
    presetAdded: () => presetAdded,
    presetHost: () => presetHost,
    prettyModelName: () => prettyModelName,
    priceFromDraft: () => priceFromDraft,
    protocolLabel: () => protocolLabel,
    providerDraftFromEntry: () => providerDraftFromEntry,
    providerDraftFromPreset: () => providerDraftFromPreset,
    providerEnabled: () => providerEnabled,
    providerInput: () => providerInput,
    proxyAddressError: () => proxyAddressError,
    qualifiedProvider: () => qualifiedProvider,
    queueWaitMs: () => queueWaitMs,
    quotaEntries: () => quotaEntries,
    quotaRemaining: () => quotaRemaining,
    quotaSupported: () => quotaSupported,
    quotaWindowFresh: () => quotaWindowFresh,
    quotaWindowLabel: () => quotaWindowLabel,
    relativeTime: () => relativeTime,
    relayFilterArgs: () => relayFilterArgs,
    relayFilterFromFocus: () => relayFilterFromFocus,
    rememberAgentNames: () => rememberAgentNames,
    routeNarrative: () => routeNarrative,
    servingProviders: () => servingProviders,
    shortDate: () => shortDate,
    shortPath: () => shortPath,
    skippedAgents: () => skippedAgents,
    slugify: () => slugify,
    speedOf: () => speedOf,
    splitModels: () => splitModels,
    subscriptionBlockReason: () => subscriptionBlockReason,
    toggleFavoriteModel: () => toggleFavoriteModel,
    unlistedModels: () => unlistedModels,
    usageDayStats: () => usageDayStats,
    usageRangeStart: () => usageRangeStart,
    validateDraft: () => validateDraft,
    validateProviderDraft: () => validateProviderDraft,
    visibleOwnModels: () => visibleOwnModels
  });

  // apps/codex-plus-manager/src/presets.ts
  var PRESETS = [
    // ── 官方 ──
    {
      id: "openai",
      name: "OpenAI Official",
      category: "official",
      baseUrl: "https://api.openai.com/v1",
      protocol: "responses",
      model: "gpt-5.5",
      websiteUrl: "https://chatgpt.com/codex"
    },
    // ── 中国官方 ──
    {
      id: "deepseek",
      name: "DeepSeek",
      websiteUrl: "https://platform.deepseek.com",
      apiKeyUrl: "https://platform.deepseek.com/api_keys",
      category: "cn_official",
      baseUrl: "https://api.deepseek.com/",
      protocol: "responses",
      model: "deepseek-v4-flash",
      modelList: ["deepseek-v4-flash", "deepseek-v4-pro"]
    },
    {
      id: "zhipu-glm",
      name: "Zhipu GLM",
      websiteUrl: "https://open.bigmodel.cn",
      apiKeyUrl: "https://www.bigmodel.cn/claude-code?ic=RRVJPB5SII",
      category: "cn_official",
      baseUrl: "https://open.bigmodel.cn/api/coding/paas/v4",
      protocol: "chatCompletions",
      model: "glm-5.1",
      modelList: ["glm-5.1"]
    },
    {
      id: "kimi",
      name: "Kimi",
      websiteUrl: "https://platform.moonshot.cn",
      apiKeyUrl: "https://platform.moonshot.cn/console/api-keys",
      category: "cn_official",
      baseUrl: "https://api.moonshot.cn/v1",
      protocol: "chatCompletions",
      model: "kimi-k2.6",
      modelList: ["kimi-k2.6"]
    },
    {
      id: "bailian",
      name: "Bailian (Qwen)",
      websiteUrl: "https://bailian.console.aliyun.com",
      apiKeyUrl: "https://bailian.console.aliyun.com/#/api-key",
      category: "cn_official",
      baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
      protocol: "chatCompletions",
      model: "qwen3-coder-plus",
      modelList: ["qwen3-coder-plus", "qwen3-max"]
    },
    {
      id: "stepfun",
      name: "StepFun",
      websiteUrl: "https://platform.stepfun.com/step-plan",
      apiKeyUrl: "https://platform.stepfun.com/interface-key",
      category: "cn_official",
      baseUrl: "https://api.stepfun.com/step_plan/v1",
      protocol: "chatCompletions",
      model: "step-3.5-flash-2603",
      modelList: ["step-3.5-flash-2603", "step-3.5-flash"]
    },
    {
      id: "minimax",
      name: "MiniMax (China)",
      websiteUrl: "https://platform.minimaxi.com",
      apiKeyUrl: "https://platform.minimaxi.com/subscribe/coding-plan",
      category: "cn_official",
      baseUrl: "https://api.minimaxi.com/v1",
      protocol: "chatCompletions",
      model: "MiniMax-M3",
      modelList: ["MiniMax-M3", "MiniMax-M2.7"]
    },
    {
      id: "minimax-global",
      name: "MiniMax (Global)",
      websiteUrl: "https://platform.minimax.io",
      apiKeyUrl: "https://platform.minimax.io/subscribe/coding-plan",
      category: "official",
      baseUrl: "https://api.minimax.io/v1",
      protocol: "chatCompletions",
      model: "MiniMax-M3",
      modelList: ["MiniMax-M3", "MiniMax-M2.7"]
    },
    {
      id: "volcano-ark",
      name: "火山引擎 Ark",
      websiteUrl: "https://www.volcengine.com/product/ark",
      apiKeyUrl: "https://console.volcengine.com/ark/region:ark+cn-beijing/apiKey",
      category: "cn_official",
      baseUrl: "https://ark.cn-beijing.volces.com/api/coding/v3",
      protocol: "chatCompletions",
      model: "ark-code-latest",
      modelList: ["ark-code-latest"]
    },
    {
      id: "baidu-qianfan",
      name: "百度千帆 Coding Plan",
      category: "cn_official",
      baseUrl: "https://qianfan.baidubce.com/v2/coding",
      protocol: "chatCompletions",
      model: "qianfan-code-latest",
      websiteUrl: "https://cloud.baidu.com/product/qianfan_modelbuilder"
    },
    {
      id: "xiaomi-mimo",
      name: "小米 MiMo",
      category: "cn_official",
      baseUrl: "https://api.xiaomimimo.com/v1",
      protocol: "chatCompletions",
      model: "mimo-v2.5-pro",
      modelList: ["mimo-v2.5-pro"],
      websiteUrl: "https://platform.xiaomimimo.com"
    },
    {
      id: "modelscope",
      name: "ModelScope",
      category: "cn_official",
      baseUrl: "https://api-inference.modelscope.cn/v1",
      protocol: "chatCompletions",
      model: "ZhipuAI/GLM-5.1",
      modelList: ["ZhipuAI/GLM-5.1"],
      websiteUrl: "https://modelscope.cn"
    },
    {
      id: "longcat",
      name: "Longcat",
      category: "cn_official",
      baseUrl: "https://api.longcat.chat/openai/v1",
      protocol: "chatCompletions",
      model: "LongCat-Flash-Chat",
      modelList: ["LongCat-Flash-Chat"],
      websiteUrl: "https://longcat.chat/platform"
    },
    // ── 聚合/中转 ──
    {
      id: "jojocode-max",
      name: "JOJO Code",
      websiteUrl: "https://max2.jojocode.com/",
      apiKeyUrl: "https://max2.jojocode.com/",
      category: "aggregator",
      baseUrl: "https://max2.jojocode.com/v1",
      protocol: "responses",
      // 模型留空：添加时「从上游获取」或手填。
      model: ""
    },
    {
      id: "runapi",
      name: "RunAPI",
      websiteUrl: "https://runapi.host",
      apiKeyUrl: "https://runapi.host",
      category: "aggregator",
      baseUrl: "https://runapi.host/v1",
      protocol: "responses",
      model: "gpt-5.5"
    },
    {
      id: "siliconflow",
      name: "SiliconFlow",
      websiteUrl: "https://siliconflow.cn",
      apiKeyUrl: "https://cloud.siliconflow.cn/i/drGuwc9k",
      category: "aggregator",
      baseUrl: "https://api.siliconflow.cn/v1",
      protocol: "chatCompletions",
      model: "Pro/MiniMaxAI/MiniMax-M2.7",
      modelList: ["Pro/MiniMaxAI/MiniMax-M2.7"]
    },
    {
      id: "openrouter",
      name: "OpenRouter",
      websiteUrl: "https://openrouter.ai",
      apiKeyUrl: "https://openrouter.ai/keys",
      category: "aggregator",
      baseUrl: "https://openrouter.ai/api/v1",
      protocol: "chatCompletions",
      model: "gpt-5.5"
    },
    {
      id: "aihubmix",
      name: "AiHubMix",
      category: "aggregator",
      baseUrl: "https://aihubmix.com/v1",
      protocol: "responses",
      model: "gpt-5.5",
      websiteUrl: "https://aihubmix.com"
    },
    {
      id: "apikeyfun",
      name: "APIKEY.FUN",
      category: "aggregator",
      baseUrl: "https://api.apikey.fun/v1",
      protocol: "responses",
      model: "gpt-5.5",
      modelList: ["gpt-5.5"],
      websiteUrl: "https://apikey.fun"
    },
    {
      id: "cccccc-api",
      name: "ccccccAPI",
      websiteUrl: "https://dzhsmartai.com",
      apiKeyUrl: "https://dzhsmartai.com/register?promo=CODEXPLUSPLUS",
      category: "aggregator",
      baseUrl: "https://dzhsmartai.com/v1",
      protocol: "responses",
      // 模型留空：添加时「从上游获取」或手填。
      model: ""
    },
    {
      id: "tokenmy",
      name: "TokenMy",
      websiteUrl: "https://agent.tokenmy.ai",
      apiKeyUrl: "https://agent.tokenmy.ai",
      category: "aggregator",
      baseUrl: "https://agent.tokenmy.ai/v1",
      protocol: "responses",
      // 模型留空：添加时「从上游获取」或手填。
      model: ""
    },
    {
      id: "vegaapi",
      name: "VegaAPI",
      websiteUrl: "https://vegaapi.com",
      apiKeyUrl: "https://vegaapi.com/register",
      category: "aggregator",
      baseUrl: "https://vegaapi.com/v1",
      protocol: "responses",
      // 模型留空：添加时「从上游获取」或手填。
      model: ""
    },
    {
      id: "novita",
      name: "Novita AI",
      category: "aggregator",
      baseUrl: "https://api.novita.ai/openai/v1",
      protocol: "chatCompletions",
      model: "zai-org/glm-5.1",
      modelList: ["zai-org/glm-5.1"],
      websiteUrl: "https://novita.ai"
    },
    // ── 第三方 ──
    {
      id: "azure",
      name: "Azure OpenAI",
      category: "third_party",
      baseUrl: "https://YOUR_RESOURCE_NAME.openai.azure.com/openai",
      protocol: "responses",
      model: "gpt-5.5",
      websiteUrl: "https://learn.microsoft.com/en-us/azure/ai-foundry/openai/how-to/codex"
    }
  ];

  // apps/codex-plus-manager/src/subscription-policy.ts
  function defaultSubscriptionPolicy() {
    return {
      defaultUsedPercentLimit: null,
      windowUsedPercentLimits: {},
      onLimit: "stop",
      unknownUsage: "allow",
      maxAgeSeconds: 600
    };
  }
  function normalizeSubscriptionPolicy(value) {
    const defaults = defaultSubscriptionPolicy();
    return {
      defaultUsedPercentLimit: value?.defaultUsedPercentLimit ?? defaults.defaultUsedPercentLimit,
      windowUsedPercentLimits: { ...value?.windowUsedPercentLimits },
      onLimit: value?.onLimit ?? defaults.onLimit,
      unknownUsage: value?.unknownUsage ?? defaults.unknownUsage,
      maxAgeSeconds: value?.maxAgeSeconds ?? defaults.maxAgeSeconds
    };
  }

  // apps/codex-plus-manager/src/relay-headers.ts
  var FORBIDDEN_RELAY_HEADERS = [
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "keep-alive",
    "proxy-connection",
    "te",
    "trailer",
    "upgrade",
    "expect"
  ];
  function normalizeRelayHeaders(rows) {
    return rows.map((row) => ({ key: row.key.trim(), value: row.value.trim() })).filter((row) => row.key.length > 0 || row.value.length > 0);
  }
  function isToken(name) {
    return /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/.test(name);
  }
  function relayHeadersValidationMessage(rows) {
    const seen = /* @__PURE__ */ new Set();
    for (const row of normalizeRelayHeaders(rows)) {
      const name = row.key;
      if (!name) {
        return "自定义请求头存在空的名称";
      }
      if (!isToken(name)) {
        return `自定义请求头「${name}」不是合法的 HTTP 头名称`;
      }
      const lowered = name.toLowerCase();
      if (FORBIDDEN_RELAY_HEADERS.includes(lowered)) {
        return `自定义请求头「${name}」由协议层掌控，不允许覆盖（Host、Content-Length 等传输头）`;
      }
      if (seen.has(lowered)) {
        return `自定义请求头「${name}」重复配置`;
      }
      seen.add(lowered);
      if (/[\r\n]/.test(row.value)) {
        return `自定义请求头「${name}」的值不能包含换行`;
      }
    }
    return null;
  }
  function providerHeaderRows(headers) {
    return (headers ?? []).map(
      (header) => header.masked ? { key: header.key, value: "", masked: header.masked, savedKey: header.key } : { key: header.key, value: header.value }
    );
  }
  function keepsSaved(row) {
    return Boolean(row.masked && row.savedKey) && row.value.trim() === "" && row.key.trim().toLowerCase() === (row.savedKey ?? "").trim().toLowerCase();
  }
  function providerHeaderInputs(rows) {
    return rows.map((row) => ({ key: row.key.trim(), value: row.value.trim(), keep: keepsSaved(row) })).filter((row) => row.key.length > 0 || row.value.length > 0);
  }
  function providerHeadersValidationMessage(rows) {
    const renamed = rows.find((row) => row.masked && row.savedKey && row.value.trim() === "" && row.key.trim() && !keepsSaved(row));
    if (renamed) return `改了请求头「${renamed.key.trim()}」的名称，请重新填写它的值`;
    return relayHeadersValidationMessage(rows);
  }

  // apps/codex-plus-manager/src/gateway-model.ts
  function keyLabel(keys, keyId) {
    const list = keys ?? [];
    const found = list.find((key) => key.id === keyId) ?? (keyId === "primary" && list.length === 1 ? list[0] : void 0);
    return found ? found.masked : keyId;
  }
  function imageModelLabel(model, vendorPicks) {
    return vendorPicks ? "GPT Image" : model;
  }
  function blankBinding() {
    return {
      enabled: false,
      model: "",
      extraModels: [],
      claudeRoles: { opus: "", sonnet: "", haiku: "", subagent: "" },
      unknownModel: "mapToDefault",
      contextWindow: null,
      claudeEnableEffort: false,
      token: ""
    };
  }
  function draftFromBinding(binding) {
    const source = binding ?? blankBinding();
    return {
      model: source.model,
      extraModels: source.extraModels.join("\n"),
      opus: source.claudeRoles.opus,
      sonnet: source.claudeRoles.sonnet,
      haiku: source.claudeRoles.haiku,
      subagent: source.claudeRoles.subagent,
      unknownModel: source.unknownModel,
      autoReviewModel: source.autoReviewModel ?? "",
      titleModel: source.titleModel ?? "",
      compactionModel: source.compactionModel ?? "",
      compactionReasoningEffort: source.compactionReasoningEffort ?? "",
      contextWindow: source.contextWindow ? String(source.contextWindow) : "",
      claudeEnableEffort: source.claudeEnableEffort,
      poolExclude: [...source.poolExclude ?? []],
      autoJoinProviders: source.autoJoinProviders !== false
    };
  }
  function defaultProviderAgents(payload) {
    return glanceAgents(payload?.agents).map((agent) => agent.agent).filter((agent) => payload?.gateway.bindings[agent]?.autoJoinProviders !== false);
  }
  function skippedAgents(payload, picked) {
    return glanceAgents(payload?.agents).map((agent) => agent.agent).filter((agent) => !picked.includes(agent));
  }
  function poolExcludeToggle(previous, providerId, on) {
    const exclude = previous ?? [];
    if (on) return exclude.filter((id) => id !== providerId);
    return exclude.includes(providerId) ? exclude : [...exclude, providerId];
  }
  function parseContextWindow(value) {
    const text = value.trim().toLowerCase();
    if (!text) return null;
    const match = /^(\d+(?:\.\d+)?)\s*([km]?)$/.exec(text);
    if (!match) return Number.NaN;
    const base = Number(match[1]);
    const scale = match[2] === "m" ? 1e6 : match[2] === "k" ? 1e3 : 1;
    return Math.round(base * scale);
  }
  function parseKeyLimit(value) {
    const text = value.trim();
    if (!text) return null;
    if (!/^\d+$/.test(text)) return Number.NaN;
    const limit = Number(text);
    return limit > 0 && limit <= 1e6 ? limit : Number.NaN;
  }
  var QUEUE_REASON_PREFIX = "local_rate_limit_wait_ms:";
  var FOREIGN_COMPACTION_DROPPED_REASON = "foreign_compaction_dropped";
  var COMPACTION_MODEL_REASON_PREFIX = "compaction_model:";
  var COMPACTION_FALLBACK_REASON_PREFIX = "compaction_fallback:";
  function parseCompactionNote(reason) {
    if (reason.startsWith(COMPACTION_MODEL_REASON_PREFIX)) {
      return { kind: "model", model: reason.slice(COMPACTION_MODEL_REASON_PREFIX.length) };
    }
    if (reason.startsWith(COMPACTION_FALLBACK_REASON_PREFIX)) {
      const rest = reason.slice(COMPACTION_FALLBACK_REASON_PREFIX.length);
      const at = rest.indexOf(":");
      return at < 0 ? { kind: "fallback", reason: rest, model: "" } : { kind: "fallback", reason: rest.slice(0, at), model: rest.slice(at + 1) };
    }
    return null;
  }
  function compactionNote(reasons) {
    for (const reason of reasons ?? []) {
      const note = parseCompactionNote(reason);
      if (note) return note;
    }
    return null;
  }
  function foreignCompactionDropped(reasons) {
    return (reasons ?? []).includes(FOREIGN_COMPACTION_DROPPED_REASON);
  }
  function queueWaitMs(reasons) {
    const reason = (reasons ?? []).find((item) => item.startsWith(QUEUE_REASON_PREFIX));
    if (reason === void 0) return null;
    const ms = Number(reason.slice(QUEUE_REASON_PREFIX.length));
    return Number.isFinite(ms) && ms >= 0 ? ms : null;
  }
  function formatQueueWait(ms) {
    return ms < 1e4 ? `${(ms / 1e3).toFixed(1)}s` : `${Math.round(ms / 1e3)}s`;
  }
  function splitModels(text) {
    const seen = /* @__PURE__ */ new Set();
    return text.split(/[\n,]/).map((line) => line.trim()).filter((line) => {
      if (!line || seen.has(line)) return false;
      seen.add(line);
      return true;
    });
  }
  function bindingFromDraft(draft, previous, item) {
    const base = previous ?? blankBinding();
    const window = parseContextWindow(draft.contextWindow);
    const model = draft.model.trim();
    return {
      ...base,
      model,
      defaultModelSelection: item ? item.defaultModelSelection ?? null : model.toLowerCase() === base.model.trim().toLowerCase() ? base.defaultModelSelection ?? null : null,
      extraModels: splitModels(draft.extraModels),
      claudeRoles: {
        opus: draft.opus.trim(),
        sonnet: draft.sonnet.trim(),
        haiku: draft.haiku.trim(),
        subagent: draft.subagent.trim()
      },
      unknownModel: draft.unknownModel,
      autoReviewModel: draft.autoReviewModel.trim(),
      titleModel: draft.titleModel.trim(),
      compactionModel: draft.compactionModel.trim(),
      compactionReasoningEffort: draft.compactionReasoningEffort.trim(),
      contextWindow: window && Number.isFinite(window) ? window : null,
      claudeEnableEffort: draft.claudeEnableEffort,
      poolExclude: [...draft.poolExclude],
      autoJoinProviders: draft.autoJoinProviders
    };
  }
  var GROUP_MODEL_PREFIX = "group/";
  function groupModelId(id) {
    return `${GROUP_MODEL_PREFIX}${id}`;
  }
  function disabledGroup(payload, model, selection) {
    const key = modelSlugFromRowName(model).toLowerCase();
    if (selection && key === `${selection.providerId}/${modelSlugFromRowName(selection.upstreamModel)}`.toLowerCase()) return void 0;
    return payload?.groups.find((group) => group.disabled && groupModelId(group.id).toLowerCase() === key);
  }
  function slugify(name) {
    const slug = name.trim().toLowerCase().replace(/[\s_/]+/g, "-").replace(/^-+|-+$/g, "");
    return slug || "unnamed";
  }
  function providerEnabled(entry) {
    return entry.enabled !== false;
  }
  function agentPoolProviders(payload, exclude = []) {
    return (payload?.pool ?? []).filter((entry) => providerEnabled(entry) && !exclude.includes(entry.id));
  }
  function agentModels(payload, exclude = []) {
    const models = [];
    const seen = /* @__PURE__ */ new Set();
    for (const entry of agentPoolProviders(payload, exclude)) {
      for (const listed of entry.models) {
        const model = disabledGroup(payload, listed) ? `${entry.id}/${listed}` : listed;
        const key = model.toLowerCase();
        if (seen.has(key)) continue;
        seen.add(key);
        models.push(model);
      }
    }
    for (const group of payload?.groups ?? []) {
      if (!group.disabled && group.members.length) models.push(groupModelId(group.id));
    }
    return models;
  }
  function qualifiedProvider(payload, model, exclude = []) {
    if (model.startsWith(GROUP_MODEL_PREFIX)) return null;
    if (servingProviders(payload, model, exclude).length) return null;
    const index = model.indexOf("/");
    if (index <= 0 || !model.slice(index + 1).trim()) return null;
    const prefix = model.slice(0, index).toLowerCase();
    const entry = agentPoolProviders(payload, exclude).find((item) => item.id.toLowerCase() === prefix);
    return entry ? { entry, model: model.slice(index + 1).trim() } : null;
  }
  function modelRoutable(payload, model, exclude = []) {
    if (disabledGroup(payload, model)) return false;
    const lower = model.trim().toLowerCase();
    return agentModels(payload, exclude).some((known) => known.toLowerCase() === lower) || qualifiedProvider(payload, model.trim(), exclude) !== null;
  }
  function validateDraft(draft, payload, retainedModel, retainedSelection) {
    if (payload) {
      if (!agentPoolProviders(payload, draft.poolExclude).length) return "供应商池是空的，先到「供应商」里添加";
      const model = draft.model.trim();
      const retainedDisabledModel = draft.model === retainedModel && disabledGroup(payload, model);
      const retainedProvider = retainedSelection && payload.pool.find((entry) => entry.id === retainedSelection.providerId);
      const retainedUnavailableModel = draft.model === retainedModel && retainedSelection && modelSlugFromRowName(model).toLowerCase() === `${retainedSelection.providerId}/${modelSlugFromRowName(retainedSelection.upstreamModel)}`.toLowerCase() && (retainedSelection.providerRemoved || !retainedProvider || !providerEnabled(retainedProvider) || draft.poolExclude.includes(retainedProvider.id));
      if (model && !retainedDisabledModel && !retainedUnavailableModel && !modelRoutable(payload, model, draft.poolExclude)) return "供应商池里没有这个模型";
    }
    const window = parseContextWindow(draft.contextWindow);
    if (window !== null && (!Number.isFinite(window) || window < 1e3)) return "上下文窗口格式不对，例如 200000、200k 或 1M";
    return null;
  }
  function protocolLabel(protocol) {
    switch (protocol) {
      case "chatCompletions":
        return "Chat";
      case "anthropicMessages":
        return "Anthropic";
      default:
        return "Responses";
    }
  }
  function failureLabel(value) {
    switch (value) {
      case "rate_limited":
        return "被限流";
      case "quota_exhausted":
        return "额度耗尽";
      case "auth_invalid":
        return "Key 无效";
      case "server_error":
        return "上游出错";
      case "failover":
        return "已切换";
      case "client_error":
        return "请求被拒";
      default:
        return "";
    }
  }
  function formatRemaining(ms) {
    if (!ms || ms <= 0) return "";
    const seconds = Math.ceil(ms / 1e3);
    if (seconds < 60) return `${seconds}s`;
    const minutes = Math.ceil(seconds / 60);
    if (minutes < 60) return `${minutes}m`;
    const hours = Math.floor(minutes / 60);
    if (hours >= 24) return `${Math.floor(hours / 24)}d${hours % 24 ? `${hours % 24}h` : ""}`;
    return `${hours}h${minutes % 60 ? `${minutes % 60}m` : ""}`;
  }
  var GATEWAY_CLAUDE_PROVIDER_ID = "codex-plus-gateway";
  var GATEWAY_AGENT_NAMES = { claude: "Claude Code", grok: "Grok", codex: "Codex" };
  var BUILTIN_GATEWAY_AGENTS = ["codex", "claude", "grok"];
  function glanceAgents(agents) {
    return (agents ?? []).filter((agent) => agent.enabled || BUILTIN_GATEWAY_AGENTS.includes(agent.agent));
  }
  function agentName(agent) {
    return GATEWAY_AGENT_NAMES[agent] ?? agent;
  }
  function rememberAgentNames(agents) {
    for (const item of agents ?? []) {
      if (item.name && !GATEWAY_AGENT_NAMES[item.agent]) GATEWAY_AGENT_NAMES[item.agent] = item.name;
    }
  }
  function matches(query, ...values) {
    if (!query) return true;
    return values.some((value) => value.toLowerCase().includes(query));
  }
  function pickerSections(payload, rawQuery, avatar, exclude = [], skipGroup, sectionOnly = null, hasOwnMatch = false) {
    if (!payload) return [];
    const query = rawQuery.trim().toLowerCase();
    const sections = [];
    const providers = agentPoolProviders(payload, exclude);
    const unlisted = unlistedModels(payload);
    const shared = [];
    for (const entry of providers) {
      const providerHit = matches(query, entry.name, entry.id);
      const items = entry.models.filter((model) => providerHit || matches(query, model, `${entry.id}/${model}`)).flatMap((model) => {
        const groupDisabled = Boolean(disabledGroup(payload, model));
        const multi = servingProviders(payload, model, exclude).length > 1;
        if (multi && !groupDisabled && !shared.some((known) => known.toLowerCase() === model.toLowerCase())) shared.push(model);
        const value = multi || groupDisabled ? `${entry.id}/${model}` : model;
        if (unlisted.has(value.toLowerCase())) return [];
        return [{
          model: value,
          providerModel: `${entry.id}/${model}`,
          defaultModelSelection: { providerId: entry.id, upstreamModel: model, providerSource: entry.source ?? "" },
          label: prettyModelName(model),
          sub: maskAccountName(entry.name),
          avatar: avatar(entry.name)
        }];
      });
      if (items.length) sections.push({ key: `provider:${entry.id}`, title: maskAccountName(entry.name), avatar: avatar(entry.name), items });
    }
    const autoItems = shared.filter((model) => activeAutoGroup(payload, model)).map((model) => ({
      model,
      label: prettyModelName(model),
      sub: servingProviders(payload, model, exclude).map((entry) => maskAccountName(entry.name)).join(" → "),
      avatar: ""
    }));
    if (autoItems.length) sections.push({ key: "auto", title: "自动组", avatar: "", items: autoItems });
    const groups = payload.groups.filter((group) => !group.disabled && group.id !== skipGroup && group.members.length && matches(query, group.name, groupModelId(group.id))).map((group) => ({ model: groupModelId(group.id), label: group.name, sub: `${GROUP_MODEL_PREFIX}${group.name}`, avatar: avatar(group.name) }));
    if (groups.length) sections.push({ key: "group", title: "我的组", avatar: "", items: groups });
    const typed = rawQuery.trim();
    const visible = sectionOnly ? sections.filter((section) => section.key === sectionOnly) : sections;
    const qualified = qualifiedProvider(payload, typed, exclude);
    const canOfferCustom = !sectionOnly || sectionOnly.startsWith("provider:");
    if (typed && !/\s/.test(typed) && canOfferCustom && !disabledGroup(payload, typed) && !visible.length && !hasOwnMatch && (!modelRoutable(payload, typed, exclude) || qualified)) {
      const targets = providers.filter((entry) => (!sectionOnly || sectionOnly === `provider:${entry.id}`) && (!qualified || entry.id === qualified.entry.id)).slice(0, 6);
      if (!targets.length) return sections;
      sections.push({
        key: "custom",
        title: "自定义模型",
        avatar: "",
        items: targets.map((entry) => ({
          model: qualified ? typed : `${entry.id}/${typed}`,
          defaultModelSelection: { providerId: entry.id, upstreamModel: modelSlugFromRowName(qualified ? qualified.model : typed), providerSource: entry.source ?? "" },
          label: typed,
          sub: maskAccountName(entry.name),
          avatar: avatar(entry.name),
          custom: true
        }))
      });
    }
    return sections;
  }
  function visibleOwnModels(agent, payload) {
    const own = agent.ownModels;
    if (!own) return null;
    if (!(payload?.gateway.hideOwnModels ?? []).includes(agent.agent)) return own;
    const current = own.current.trim().toLowerCase();
    const models = current ? own.models.filter((model) => model.id.toLowerCase() === current) : [];
    return models.length ? { ...own, models } : null;
  }
  function ownPickerSection(own, rawQuery, avatar) {
    if (!own || !own.models.length) return null;
    const query = rawQuery.trim().toLowerCase();
    const vendorHit = matches(query, own.vendor);
    const items = own.models.filter((model) => vendorHit || matches(query, model.id, model.label)).map((model) => ({ model: model.id, label: model.label, sub: model.label === model.id ? "" : model.id, avatar: avatar(own.vendor), direct: own.vendor }));
    return items.length ? { key: "own", title: own.vendor, avatar: avatar(own.vendor), items } : null;
  }
  function servingProviders(payload, model, exclude = []) {
    const name = model.trim().toLowerCase();
    return agentPoolProviders(payload, exclude).filter((entry) => entry.models.some((known) => known.toLowerCase() === name));
  }
  function groupDisplayName(payload, model) {
    const key = model.trim().toLowerCase();
    return payload?.groups.find((group) => groupModelId(group.id).toLowerCase() === key)?.name;
  }
  function modelDisplay(payload, model, exclude = [], selection) {
    const title = model.trim() || agentModels(payload, exclude)[0] || "";
    const selectedModel = modelSlugFromRowName(title);
    if (selection && selectedModel.toLowerCase() === `${selection.providerId}/${modelSlugFromRowName(selection.upstreamModel)}`.toLowerCase()) {
      const provider = payload?.pool.find((entry2) => entry2.id === selection.providerId);
      const unavailable = selection.providerRemoved || !provider || !providerEnabled(provider) || exclude.includes(provider.id) || (provider.source ?? "") !== selection.providerSource;
      const conflict = selectedModel.toLowerCase().startsWith(GROUP_MODEL_PREFIX) || selectedModel.includes("*") || servingProviders(payload, selectedModel, exclude).length > 0;
      if (unavailable || conflict) {
        return {
          title: prettyModelName(selection.upstreamModel),
          sub: provider ? maskAccountName(provider.name) : selection.providerId,
          group: false,
          issue: unavailable ? "unavailable" : "conflict"
        };
      }
    }
    if (title.startsWith(GROUP_MODEL_PREFIX)) {
      const name = groupDisplayName(payload, title);
      return { title: name ?? title, sub: name === void 0 ? title : `${GROUP_MODEL_PREFIX}${name}`, group: true };
    }
    const qualified = qualifiedProvider(payload, title, exclude);
    if (qualified) return { title: prettyModelName(qualified.model), sub: maskAccountName(qualified.entry.name), group: false };
    const serving = servingProviders(payload, title, exclude);
    if (serving.length > 1) {
      return { title: prettyModelName(title), sub: serving.map((entry2) => maskAccountName(entry2.name)).join(" → "), group: false, auto: true };
    }
    const entry = serving[0];
    return { title: prettyModelName(title), sub: entry ? maskAccountName(entry.name) : "", group: false };
  }
  var MODEL_WORDS = {
    gpt: "GPT",
    glm: "GLM",
    deepseek: "DeepSeek",
    minimax: "MiniMax",
    qwen: "Qwen",
    qwq: "QwQ",
    kimi: "Kimi",
    grok: "Grok",
    claude: "Claude",
    gemini: "Gemini",
    gemma: "Gemma",
    llama: "Llama",
    mistral: "Mistral",
    codestral: "Codestral",
    doubao: "Doubao",
    ernie: "ERNIE",
    hunyuan: "Hunyuan",
    step: "Step",
    mimo: "MiMo",
    longcat: "LongCat"
  };
  function prettyModelToken(token) {
    const lower = token.toLowerCase();
    if (MODEL_WORDS[lower]) return MODEL_WORDS[lower];
    if (/^o\d/.test(lower)) return lower;
    if (/^[a-z]\d+(\.\d+)?[a-z]?$/.test(lower)) return lower.toUpperCase();
    if (/^\d+(\.\d+)?[bkm]$/.test(lower)) return lower.toUpperCase();
    if (/^\d/.test(lower)) return lower;
    const word = /^([a-z]+)(.*)$/.exec(lower);
    if (!word) return token;
    const head = MODEL_WORDS[word[1]] ?? word[1].charAt(0).toUpperCase() + word[1].slice(1);
    return head + word[2];
  }
  function prettyModelName(model) {
    const match = /^(.*\/)?([^/[]+)(\[.*\])?$/.exec(model.trim());
    if (!match) return model;
    const [, prefix = "", name, suffix = ""] = match;
    if (/[A-Z]/.test(name)) return model.trim();
    return prefix + name.split(/([-_])/).map((part) => part === "-" || part === "_" ? part : prettyModelToken(part)).join("") + suffix;
  }
  function groupModeLabel(mode) {
    switch (mode) {
      case "rotate":
        return "轮换";
      case "weighted":
        return "按权重";
      case "usage":
        return "额度优先";
      case "fast":
        return "速度优先";
      case "manual":
        return "手动";
      default:
        return "依次";
    }
  }
  function groupModeDescription(mode) {
    switch (mode) {
      case "rotate":
        return "轮换：每个新会话从下一个成员开始；成员答不了时换下一个，每个成员再按自己的 Key 轮换。";
      case "weighted":
        return "按权重：按成员权重决定先试谁；成员答不了时换下一个，每个成员再按自己的 Key 轮换。";
      case "usage":
        return "额度优先：订阅账号里剩余额度最多的先试；没有额度数据的排在后面，额度用完的排最后。";
      case "fast":
        return "速度优先：最近响应最快的先试；还没测过的先试一次，之后按实测速度排。";
      case "manual":
        return "手动：只发给选中的那个成员，它答不了也不换；在成员列表里点一下就换成它。";
      default:
        return "依次：按成员顺序，前一个答不了才换下一个，每个成员再按自己的 Key 轮换。";
    }
  }
  function groupStickyLabel(sticky) {
    switch (sticky) {
      case "session":
        return "整个会话固定";
      case "off":
        return "不固定";
      default:
        return "缓存期内固定";
    }
  }
  function groupStickyDescription(sticky) {
    switch (sticky) {
      case "session":
        return "同一会话一直先试上次回答的成员，它答不了才换，最省缓存。";
      case "off":
        return "每个请求都按模式重新排，会话中途可能换成员，上游的提示词缓存会失效。";
      default:
        return "同一会话 10 分钟内继续先试上次回答的成员，保住上游的提示词缓存；空闲超过 10 分钟再按模式重新排。";
    }
  }
  function parseWeight(text) {
    const trimmed = text.trim();
    if (!trimmed) return null;
    const value = Number(trimmed);
    if (!Number.isFinite(value)) return null;
    return Math.max(1, Math.floor(value));
  }
  var MIN_GATEWAY_PORT = 1024;
  var MAX_GATEWAY_PORT = 65535;
  function parseGatewayPort(text) {
    const trimmed = text.trim();
    if (!/^\d{1,5}$/.test(trimmed)) return null;
    const value = Number(trimmed);
    return value >= MIN_GATEWAY_PORT && value <= MAX_GATEWAY_PORT ? value : null;
  }
  function isPattern(model) {
    return model.includes("*");
  }
  function globMatches(pattern, text) {
    const escaped = pattern.toLowerCase().split("*").map((part) => part.replace(/[.+?^${}()|[\]\\]/g, "\\$&"));
    return new RegExp(`^${escaped.join(".*")}$`, "s").test(text.toLowerCase());
  }
  function patternMatches(payload, pattern) {
    const matches2 = [];
    for (const entry of agentPoolProviders(payload)) {
      for (const model of entry.models) {
        const qualified = `${entry.id}/${model}`;
        if (globMatches(pattern, qualified) || globMatches(pattern, model)) matches2.push(qualified);
      }
    }
    return matches2;
  }
  function speedOf(payload, relayId, model) {
    const found = payload?.speeds?.find((item) => item.relayId === relayId && item.model === model.toLowerCase());
    return found ? found.ms : null;
  }
  function quotaWindowLabel(window) {
    const minutes = window.windowMinutes;
    if (minutes === null) return { text: "本月", args: [] };
    if (minutes === 300) return { text: "5 小时", args: [] };
    if (minutes === 10080) return { text: "每周", args: [] };
    if (minutes % 1440 === 0) return { text: "{0} 天", args: [minutes / 1440] };
    if (minutes % 60 === 0) return { text: "{0} 小时", args: [minutes / 60] };
    return { text: "{0} 分钟", args: [minutes] };
  }
  function quotaSupported(entry) {
    return entry.quotaApi ?? (entry.account === "chatgpt" || entry.account === "copilot" || entry.account === "grok");
  }
  function quotaEntries(pool) {
    return (pool ?? []).filter((entry) => entry.enabled !== false && quotaSupported(entry));
  }
  function quotaRemaining(quota, nowMs) {
    if (quota.officialAllowed === false) return 0;
    return quota.windows.reduce((least, window) => {
      const left = window.resetsAtMs !== null && window.resetsAtMs <= nowMs ? 0 : Math.min(1, Math.max(0, 1 - window.usedPercent / 100));
      return Math.min(least, left);
    }, 1);
  }
  function quotaWindowFresh(quota, window, nowMs, maxAgeSeconds = 600) {
    const observed = window.observedAtMs ?? quota.updatedAtMs;
    return observed <= nowMs && nowMs - observed <= maxAgeSeconds * 1e3 && Number.isFinite(window.usedPercent) && window.usedPercent >= 0 && window.usedPercent <= 100 && (window.resetsAtMs === null || window.resetsAtMs > nowMs);
  }
  function subscriptionBlockReason(entry, nowMs = Date.now()) {
    if (entry.account !== "chatgpt" || !entry.subscriptionPolicy) return null;
    const policy = normalizeSubscriptionPolicy(entry.subscriptionPolicy);
    if (policy.defaultUsedPercentLimit === 0) return "订阅额度已用上限设为 0%，已停止新请求";
    const unknown = (text) => policy.unknownUsage === "allow" ? null : text;
    const quota = entry.quota;
    if (!quota) return entry.subscriptionBlockReason || unknown("官方额度未知，请刷新额度");
    const fresh = quota.updatedAtMs <= nowMs && nowMs - quota.updatedAtMs <= policy.maxAgeSeconds * 1e3;
    if (fresh && (quota.officialAllowed === false || quota.officialLimitReached === true && quota.officialAllowed !== true)) return "官方订阅额度受限，请等待重置或刷新额度";
    let uncertain = !fresh || !quota.windowsComplete && Object.keys(policy.windowUsedPercentLimits).some((key) => !quota.windows.some((window) => String(window.windowMinutes) === key)) || !quota.windows.length && !(quota.windowsComplete && quota.officialAllowed === true);
    for (const window of quota.windows) {
      if (!quotaWindowFresh(quota, window, nowMs, policy.maxAgeSeconds)) {
        uncertain = true;
        continue;
      }
      const cap = policy.windowUsedPercentLimits[String(window.windowMinutes)] ?? policy.defaultUsedPercentLimit ?? 100;
      if (window.usedPercent >= cap) {
        const label = window.windowMinutes === 300 ? "5h" : window.windowMinutes === 10080 ? "7d" : `${window.windowMinutes ?? ""} 分钟`;
        return `${label} 已用 ${window.usedPercent}%，达到 ${cap}% 上限`;
      }
    }
    return entry.subscriptionBlockReason || (uncertain ? unknown("官方额度待刷新，请先刷新额度") : null);
  }
  function memberLabel(payload, model) {
    if (isPattern(model)) return model;
    const view = modelDisplay(payload, model);
    if (view.group) return view.title;
    if (view.auto) return `${view.title} · ${view.sub}`;
    return view.sub ? `${view.sub} · ${view.title}` : view.title;
  }
  var RECENT_USE_MS = 10 * 60 * 1e3;
  function memberStatus(payload, relayId, nowMs = Date.now()) {
    const activity = payload?.activity?.agents ?? [];
    if (activity.some((agent) => agent.inFlight.includes(relayId))) {
      return { state: "busy", text: "正在回答…", args: [] };
    }
    const breaker = payload?.health.providers.find((item) => item.relayId === relayId && item.breakerOpen);
    if (breaker) {
      const left = formatRemaining(breaker.remainingMs);
      return left ? { state: "cooling", text: "连续出错 · {0} 后恢复", args: [left] } : { state: "cooling", text: "连续出错", args: [] };
    }
    const entry = payload?.pool.find((item) => item.id === relayId);
    const keyCount = entry?.keyCount ?? 1;
    const unhealthy = (payload?.health.keys ?? []).filter((key) => key.relayId === relayId && key.state !== "ok");
    if (unhealthy.length >= keyCount && unhealthy.length > 0) {
      const cooling = unhealthy.filter((key) => key.state === "cooling");
      if (!cooling.length) return { state: "cooling", text: "Key 都已停用", args: [] };
      const waits = cooling.map((key) => key.remainingMs ?? 0).filter((ms) => ms > 0);
      const left = waits.length ? formatRemaining(Math.min(...waits)) : "";
      const quota = cooling.some((key) => key.lastFailure === "quota_exhausted");
      if (!left) return { state: "cooling", text: quota ? "额度耗尽" : "被限流", args: [] };
      return { state: "cooling", text: quota ? "额度耗尽 · {0} 后恢复" : "被限流 · {0} 后恢复", args: [left] };
    }
    if (entry?.account && entry.quota) {
      const quota = entry.quota;
      const maxAge = entry.subscriptionPolicy?.maxAgeSeconds ?? 600;
      const exhausted = quota.windows.filter((window) => quotaWindowFresh(quota, window, nowMs, maxAge) && window.usedPercent >= 100);
      if (exhausted.length) {
        const resets = exhausted.map((window) => window.resetsAtMs).filter((reset) => reset !== null);
        const left = resets.length === exhausted.length ? formatRemaining(Math.max(...resets) - nowMs) : "";
        return left ? { state: "cooling", text: "额度耗尽 · {0} 后恢复", args: [left] } : { state: "cooling", text: "额度耗尽", args: [] };
      }
      const fresh = quota.updatedAtMs <= nowMs && nowMs - quota.updatedAtMs <= maxAge * 1e3;
      if (fresh && (quota.officialAllowed === false || quota.officialLimitReached === true && quota.officialAllowed !== true)) {
        return { state: "cooling", text: "订阅额度受限", args: [] };
      }
    }
    const answered = activity.filter((agent) => agent.last?.result === "answered" && agent.last.steps.at(-1)?.relayId === relayId && nowMs - agent.last.finishedAtMs < RECENT_USE_MS).sort((left, right) => (right.last?.finishedAtMs ?? 0) - (left.last?.finishedAtMs ?? 0))[0];
    if (answered) return { state: "answered", text: "{0} 刚用过", args: [agentName(answered.agent)] };
    if (unhealthy.length) return { state: "idle", text: "{0} 个 Key · {1} 个冷却中", args: [keyCount, unhealthy.length] };
    return { state: "idle", text: "{0} 个 Key", args: [Math.max(keyCount, 1)] };
  }
  function dispatchStatus(payload, relayId, model, nowMs = Date.now()) {
    const status = memberStatus(payload, relayId, nowMs);
    if (status.state === "cooling") return status;
    const used = (payload?.activity?.agents ?? []).filter((agent) => {
      const step = agent.last?.result === "answered" ? agent.last.steps.at(-1) : void 0;
      return step?.relayId === relayId && step.model.toLowerCase() === model.toLowerCase() && nowMs - (agent.last?.finishedAtMs ?? 0) < RECENT_USE_MS;
    }).sort((left, right) => (right.last?.finishedAtMs ?? 0) - (left.last?.finishedAtMs ?? 0))[0];
    return used ? { state: "answered", text: "{0} 刚用过", args: [agentName(used.agent)] } : null;
  }
  function formatElapsed(ms) {
    if (ms < 1e4) return `${(ms / 1e3).toFixed(1)} s`;
    return `${Math.round(ms / 1e3)} s`;
  }
  function routeNarrative(agent, record, payload) {
    const lines = [];
    const who = agentName(agent);
    const requested = groupDisplayName(payload, record.requestedModel || record.model) ?? (record.requestedModel || record.model);
    const plan = record.planned.map((member) => `${maskAccountName(member.relayName)}/${groupDisplayName(payload, member.model) ?? member.model}`).join(", ");
    if (record.groupId) {
      const group = payload?.groups.find((item) => item.id === record.groupId);
      lines.push({ tone: "muted", text: "{0} 请求 {1}，交给模型组「{2}」：{3}", args: [who, requested, group?.name ?? record.groupId, plan] });
      if (record.planned[0]) lines.push({ tone: "step", text: "{0} 排在第一个。", args: [maskAccountName(record.planned[0].relayName)] });
    } else {
      lines.push({ tone: "muted", text: "{0} 请求 {1}，转发到 {2}", args: [who, requested, plan] });
    }
    record.steps.forEach((step, index) => {
      const last = index === record.steps.length - 1;
      if (step.outcome === "answered") {
        lines.push({ tone: "ok", text: "{0}（{1}）回答，用时 {2}。", args: [maskAccountName(step.relayName), groupDisplayName(payload, step.model) ?? step.model, formatElapsed(record.elapsedMs)] });
        return;
      }
      const reason = { zh: step.outcome === "transport" ? "连不上" : failureLabel(step.failure) || "出错" };
      const status = step.status ? ` · HTTP ${step.status}` : "";
      lines.push({
        tone: last ? "fail" : "step",
        text: last ? "{0} 的 Key {1} {2}{3}，错误交回给 {4}。" : "{0} 的 Key {1} {2}{3}，换下一个。",
        args: last ? [maskAccountName(step.relayName), step.keyId, reason, status, who] : [maskAccountName(step.relayName), step.keyId, reason, status]
      });
    });
    if (record.result === "cooling") {
      lines.push({ tone: "fail", text: "所有 Key 都在冷却，约 {0} 后恢复。", args: [formatRemaining(record.coolingMs ?? 0) || "1s"] });
    }
    return lines;
  }
  function connectedAgents(payload) {
    return (payload?.agents ?? []).filter((view) => view.enabled).map((view) => view.agent);
  }
  function modelDispatch(payload) {
    const connected = connectedAgents(payload);
    const rows = [];
    const byModel = /* @__PURE__ */ new Map();
    for (const entry of agentPoolProviders(payload)) {
      const excludedBy = connected.filter((agent) => payload?.gateway.bindings[agent]?.poolExclude?.includes(entry.id));
      for (const model of entry.models) {
        const key = model.toLowerCase();
        let slot = byModel.get(key);
        if (!slot) {
          slot = { row: { model, providers: [] }, ids: /* @__PURE__ */ new Set() };
          byModel.set(key, slot);
          rows.push(slot.row);
        }
        if (!slot.ids.has(entry.id)) {
          slot.ids.add(entry.id);
          slot.row.providers.push({ id: entry.id, name: maskAccountName(entry.name), baseUrl: entry.baseUrl, excludedBy });
        }
      }
    }
    return rows.sort((left, right) => Number(right.providers.length > 1) - Number(left.providers.length > 1));
  }
  var GATEWAY_EXTRA_PRESETS = [
    {
      key: "anthropic",
      name: "Anthropic",
      baseUrl: "https://api.anthropic.com",
      protocol: "anthropicMessages",
      model: "claude-sonnet-5-5",
      modelList: ["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"],
      section: "vendor",
      apiKeyUrl: "https://console.anthropic.com/settings/keys"
    },
    {
      key: "xai",
      name: "xAI",
      baseUrl: "https://api.x.ai/v1",
      protocol: "chatCompletions",
      model: "",
      modelList: [],
      section: "vendor",
      apiKeyUrl: "https://console.x.ai"
    },
    // OpenCode 的模型网关：Go 是按月订阅的开源编码模型，Zen 按量计费。每个模型只在一种协议上提供，
    // 网关按 models.dev 目录逐个模型选协议，这里的协议只是占位。
    {
      key: "opencode-go",
      name: "OpenCode Go",
      baseUrl: "https://opencode.ai/zen/go/v1",
      protocol: "chatCompletions",
      model: "",
      modelList: [],
      section: "relay",
      apiKeyUrl: "https://opencode.ai/auth"
    },
    {
      key: "opencode-zen",
      name: "OpenCode Zen",
      baseUrl: "https://opencode.ai/zen/v1",
      protocol: "chatCompletions",
      model: "",
      modelList: [],
      section: "relay",
      apiKeyUrl: "https://opencode.ai/auth"
    },
    {
      key: "ollama",
      name: "Ollama",
      baseUrl: "http://127.0.0.1:11434/v1",
      protocol: "chatCompletions",
      model: "",
      modelList: [],
      section: "local",
      defaultKey: "ollama"
    },
    {
      key: "lm-studio",
      name: "LM Studio",
      baseUrl: "http://127.0.0.1:1234/v1",
      protocol: "chatCompletions",
      model: "",
      modelList: [],
      section: "local",
      defaultKey: "lm-studio"
    }
  ];
  var SPONSORS = [
    { id: "jojocode-max", expiresAt: "2029-06-15" },
    { id: "runapi", expiresAt: "2026-12-07" },
    { id: "apikeyfun", expiresAt: "2026-11-22" },
    { id: "cccccc-api", expiresAt: "2026-12-01" },
    { id: "tokenmy", expiresAt: "2026-11-20" },
    { id: "vegaapi", expiresAt: "2027-03-18" }
  ];
  function localDay(now) {
    const pad = (value) => String(value).padStart(2, "0");
    return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
  }
  function activeSponsorIds(now = /* @__PURE__ */ new Date()) {
    const today = localDay(now);
    return SPONSORS.filter((sponsor) => sponsor.expiresAt >= today).map((sponsor) => sponsor.id);
  }
  function gatewayPresets(now = /* @__PURE__ */ new Date()) {
    const sponsorIds = activeSponsorIds(now);
    const expired = new Set(SPONSORS.map((sponsor) => sponsor.id).filter((id) => !sponsorIds.includes(id)));
    const shared = PRESETS.filter((preset) => !expired.has(preset.id)).map((preset) => ({
      key: preset.id,
      name: preset.name,
      baseUrl: preset.baseUrl,
      protocol: preset.protocol,
      model: preset.model,
      modelList: preset.modelList ?? (preset.model ? [preset.model] : []),
      section: sponsorIds.includes(preset.id) ? "sponsor" : preset.category === "official" || preset.category === "cn_official" ? "vendor" : "relay",
      apiKeyUrl: preset.apiKeyUrl
    }));
    const vendors = [...GATEWAY_EXTRA_PRESETS.filter((preset) => preset.section === "vendor"), ...shared.filter((preset) => preset.section === "vendor")];
    const relays = [...GATEWAY_EXTRA_PRESETS.filter((preset) => preset.section === "relay"), ...shared.filter((preset) => preset.section === "relay")];
    const sponsors = sponsorIds.flatMap((id) => shared.filter((preset) => preset.key === id));
    return [...sponsors, ...vendors, ...relays, ...GATEWAY_EXTRA_PRESETS.filter((preset) => preset.section === "local")];
  }
  function normalizeUrl(url) {
    return url.trim().replace(/\/+$/, "").toLowerCase();
  }
  function presetHost(baseUrl) {
    try {
      const url = new URL(baseUrl);
      return url.hostname === "127.0.0.1" || url.hostname === "localhost" ? `localhost:${url.port}` : url.hostname;
    } catch {
      return baseUrl;
    }
  }
  function presetAdded(payload, preset) {
    const target = normalizeUrl(preset.baseUrl);
    return Boolean(payload?.pool.some((entry) => entry.source === `preset:${preset.key}` || normalizeUrl(entry.baseUrl) === target));
  }
  var NO_PRICE = { priceCurrency: "CNY", priceInput: "", priceCached: "", priceOutput: "", proxy: "", stripImageGenerationTool: false, stripResponsesImageDetail: false, disableToolSearch: false };
  function providerDraftFromPreset(preset) {
    if (!preset) return { name: "", baseUrl: "", keys: "", protocol: "chatCompletions", model: "", modelList: "", ...NO_PRICE };
    return {
      name: preset.name,
      baseUrl: preset.baseUrl,
      keys: preset.defaultKey ?? "",
      protocol: preset.protocol,
      model: preset.model,
      modelList: preset.modelList.join("\n"),
      ...NO_PRICE
    };
  }
  function providerDraftFromEntry(entry) {
    return {
      name: entry.name,
      baseUrl: entry.baseUrl,
      keys: "",
      protocol: entry.protocol,
      model: entry.models[0] ?? "",
      modelList: entry.models.join("\n"),
      priceCurrency: entry.price?.currency || "CNY",
      priceInput: entry.price ? String(entry.price.input) : "",
      priceCached: entry.price ? String(entry.price.cachedInput) : "",
      priceOutput: entry.price ? String(entry.price.output) : "",
      proxy: entry.proxy ?? "",
      stripImageGenerationTool: entry.stripImageGenerationTool === true,
      subscriptionPolicy: entry.subscriptionPolicy == null ? entry.subscriptionPolicy : normalizeSubscriptionPolicy(entry.subscriptionPolicy),
      stripResponsesImageDetail: entry.stripResponsesImageDetail === true,
      disableToolSearch: entry.disableToolSearch === true,
      customHeaders: providerHeaderRows(entry.customHeaders)
    };
  }
  function providerInput(draft) {
    return {
      name: draft.name.trim(),
      baseUrl: draft.baseUrl.trim(),
      protocol: draft.protocol,
      keys: draft.keys.split(/\n/).map((line) => line.trim()).filter(Boolean),
      model: draft.model.trim(),
      modelList: splitModels(draft.modelList).join("\n"),
      price: priceFromDraft(draft),
      modelPrices: draft.modelPrices ?? null,
      proxy: draft.proxy.trim(),
      stripImageGenerationTool: draft.stripImageGenerationTool === true,
      ...draft.subscriptionPolicy !== void 0 ? { subscriptionPolicy: draft.subscriptionPolicy } : {},
      stripResponsesImageDetail: draft.stripResponsesImageDetail === true,
      disableToolSearch: draft.disableToolSearch === true,
      ...draft.customHeaders !== void 0 ? { customHeaders: providerHeaderInputs(draft.customHeaders) } : {}
    };
  }
  function priceNumber(text) {
    const value = Number(text.trim());
    return Number.isFinite(value) && value > 0 ? value : 0;
  }
  function priceFromDraft(draft) {
    const price = { currency: draft.priceCurrency || "CNY", input: priceNumber(draft.priceInput), cachedInput: priceNumber(draft.priceCached), output: priceNumber(draft.priceOutput) };
    return price.input || price.cachedInput || price.output ? price : null;
  }
  var DIRECT_PROXY = "direct";
  function isDirectProxy(proxy) {
    return (proxy ?? "").trim().toLowerCase() === DIRECT_PROXY;
  }
  function proxyAddressError(proxy) {
    const value = proxy.trim();
    if (!value || isDirectProxy(value)) return null;
    return /^(https?|socks5h?):\/\/\S+$/i.test(value) ? null : "代理地址要写成 http://主机:端口（也支持 https:// 与 socks5://）";
  }
  function validateProviderDraft(draft, editing = false) {
    if (!draft.name.trim()) return "请填写名称";
    if (!/^https?:\/\/\S+$/i.test(draft.baseUrl.trim())) return "地址要以 http:// 或 https:// 开头";
    if (!editing && !draft.keys.trim()) return "请填写 API Key";
    if (!draft.model.trim() && splitModels(draft.modelList).length === 0) return "至少填一个模型";
    return proxyAddressError(draft.proxy) ?? providerHeadersValidationMessage(draft.customHeaders ?? []);
  }
  function usageRangeStart(range, now = /* @__PURE__ */ new Date()) {
    const days = range === "today" ? 1 : range === "7d" ? 7 : range === "30d" ? 30 : 0;
    if (!days) return void 0;
    const start = new Date(now.getFullYear(), now.getMonth(), now.getDate());
    start.setDate(start.getDate() - (days - 1));
    return start.getTime();
  }
  function relayFilterFromFocus(focus) {
    if (focus.relayUnknown) return { id: "", label: focus.relayLabel || "", unknown: true };
    return focus.relayId ? { id: focus.relayId, label: focus.relayLabel || focus.relayId } : null;
  }
  function relayFilterArgs(filter) {
    return filter?.unknown ? { relayId: null, relayUnknown: true } : { relayId: filter?.id || null, relayUnknown: false };
  }
  function formatRequestCost(cost) {
    if (cost.amount <= 0) return "—";
    if (cost.amount >= 0.01) return formatMoney(cost.amount, cost.currency);
    return `${currencySymbol(cost.currency)}${cost.amount.toPrecision(2)}`;
  }
  function formatTokens(value) {
    if (value >= 1e9) return `${trimNumber(value / 1e9)}B`;
    if (value >= 1e6) return `${trimNumber(value / 1e6)}M`;
    if (value >= 1e3) return `${trimNumber(value / 1e3)}K`;
    return String(value);
  }
  function trimNumber(value) {
    return value >= 100 ? value.toFixed(0) : value >= 10 ? value.toFixed(0) : value.toFixed(1).replace(/\.0$/, "");
  }
  function currencySymbol(currency) {
    switch (currency.toUpperCase()) {
      case "CNY":
        return "¥";
      case "USD":
        return "$";
      case "EUR":
        return "€";
      default:
        return `${currency.toUpperCase()} `;
    }
  }
  function formatMoney(amount, currency) {
    return `${currencySymbol(currency)}${amount.toFixed(2)}`;
  }
  function cacheHitRate(counts) {
    return counts.inputTokens > 0 ? counts.cachedTokens / counts.inputTokens : null;
  }
  function averageDuration(counts) {
    return counts.timedCalls ? (counts.durationMs ?? 0) / counts.timedCalls : null;
  }
  function costText(costs) {
    return (costs ?? []).filter((cost) => cost.amount > 0).map((cost) => formatMoney(cost.amount, cost.currency)).join(" + ");
  }
  function niceCeiling(value) {
    if (value <= 0) return 1;
    const power = 10 ** Math.floor(Math.log10(value));
    for (const step of [1, 2, 5, 10]) {
      if (step * power >= value) return step * power;
    }
    return 10 * power;
  }
  function usageDayStats(days) {
    const active = days.filter((day) => day.calls > 0);
    const total = (day) => day.inputTokens + day.outputTokens;
    const peak = active.reduce((best, day) => !best || total(day) > total(best) ? day : best, null);
    const sum = active.reduce((acc, day) => acc + total(day), 0);
    return { activeDays: active.length, peak, dailyAverage: active.length ? sum / active.length : 0 };
  }
  function axisLabelIndexes(length, count = 7) {
    if (length <= 0) return [];
    if (length <= count) return Array.from({ length }, (_, index) => index);
    const step = (length - 1) / (count - 1);
    return Array.from(new Set(Array.from({ length: count }, (_, index) => Math.round(index * step))));
  }
  function shortDate(date, english = false) {
    const [, month, day] = date.split("-").map(Number);
    if (!month || !day) return date;
    if (english) return `${["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][month - 1]} ${day}`;
    return `${month}月${day}日`;
  }
  function filterSessions(sessions, agent, query) {
    const needle = query.trim().toLowerCase();
    return sessions.filter((session) => agent === "all" || session.agent === agent).filter((session) => !needle || [session.title, session.cwd, session.id].some((value) => value.toLowerCase().includes(needle))).sort((left, right) => right.updatedAtMs - left.updatedAtMs);
  }
  function shortPath(path, home = "") {
    let text = path.trim().replace(/^\\\\\?\\UNC\\/, "\\\\").replace(/^\\\\\?\\(?=[A-Za-z]:)/, "");
    if (home && text.startsWith(home)) text = `~${text.slice(home.length)}`;
    else text = text.replace(/^\/Users\/[^/]+/, "~").replace(/^\/home\/[^/]+/, "~");
    const separator = !text.includes("/") && text.includes("\\") ? "\\" : "/";
    const parts = text.split(/[\\/]/).filter(Boolean);
    return parts.length > 4 ? `…${separator}${parts.slice(-2).join(separator)}` : text;
  }
  function relativeTime(ms, now) {
    const diff = Math.max(0, now - ms);
    const minute = 6e4;
    if (diff < minute) return { text: "刚刚", args: [] };
    if (diff < 60 * minute) return { text: "{0} 分钟前", args: [Math.floor(diff / minute)] };
    if (diff < 24 * 60 * minute) return { text: "{0} 小时前", args: [Math.floor(diff / (60 * minute))] };
    if (diff < 30 * 24 * 60 * minute) return { text: "{0} 天前", args: [Math.floor(diff / (24 * 60 * minute))] };
    const date = new Date(ms);
    return { text: "{0}", args: [`${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`] };
  }
  var EMAIL_PATTERN = /(^|[^A-Za-z0-9._%+*-])([A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,})/g;
  function maskEmailsIn(text) {
    return text.replace(EMAIL_PATTERN, (_match, lead, email) => lead + maskEmail(email));
  }
  function maskAccountName(name) {
    const masked = maskEmailsIn(name);
    const match = /^((?:ChatGPT|Copilot|SuperGrok|GitHub Copilot)[^·]*·\s*)(.+)$/.exec(masked);
    if (!match || match[2].includes("@") || match[2].includes("*")) return masked;
    return match[1] + maskToken(match[2].trim());
  }
  function maskAccountText(text) {
    return text.split(/(「[^」]+」)/).map((part) => part.startsWith("「") && part.endsWith("」") ? `「${maskAccountName(part.slice(1, -1))}」` : maskEmailsIn(part)).join("");
  }
  function maskEmail(email) {
    const at = email.indexOf("@");
    return `${maskToken(email.slice(0, at))}${email.slice(at)}`;
  }
  function maskToken(value) {
    const chars = [...value];
    if (chars.length <= 4) return `${chars[0] ?? ""}***`;
    return `${chars.slice(0, 2).join("")}****${chars.slice(-2).join("")}`;
  }
  function importCandidates(payload) {
    const pooled = (source, id) => Boolean(payload?.pool.some((entry) => entry.source === `${source}:${id}`));
    const plain = (source, items) => (items ?? []).map((item) => ({ source, id: item.id, name: item.name, imported: pooled(source, item.id) }));
    return [
      ...plain("codex", payload?.codexProviders),
      ...(payload?.codexAggregates ?? []).map((item) => ({ source: "aggregate", id: item.id, name: item.name, imported: item.imported })),
      ...plain("claude", payload?.claudeProviders),
      ...plain("grok", payload?.grokProfiles),
      ...plain("opencode", payload?.opencodeKeys)
    ];
  }
  function pendingImportCount(payload) {
    return importCandidates(payload).filter((item) => !item.imported).length;
  }
  function pinnedBare(payload, model) {
    const slash = model.indexOf("/");
    if (slash <= 0) return null;
    const id = model.slice(0, slash).toLowerCase();
    const rest = model.slice(slash + 1);
    const entry = agentPoolProviders(payload).find((item) => item.id.toLowerCase() === id);
    return entry && entry.models.some((known) => known.toLowerCase() === rest.toLowerCase()) ? rest : null;
  }
  function pinnedCopies(payload, bare) {
    const serving = servingProviders(payload, bare);
    return serving.length > 1 ? serving.map((entry) => `${entry.id}/${bare}`) : [];
  }
  function isFavoriteModel(favorites, model, payload) {
    if (favorites.includes(model)) return true;
    const bare = pinnedBare(payload, model);
    if (!bare || !favorites.some((item) => item.toLowerCase() === bare.toLowerCase())) return false;
    return !pinnedCopies(payload, bare).some((copy) => favorites.includes(copy));
  }
  function toggleFavoriteModel(favorites, model, payload) {
    if (!isFavoriteModel(favorites, model, payload)) return [...favorites, model];
    const bare = pinnedBare(payload, model);
    if (bare && !favorites.includes(model)) {
      const others = pinnedCopies(payload, bare).filter((copy) => copy !== model && !favorites.includes(copy));
      return [...favorites, ...others];
    }
    const next = favorites.filter((item) => item !== model);
    const copies = pinnedCopies(payload, model);
    if (copies.length && !copies.some((copy) => favorites.includes(copy))) return [...next, ...copies];
    return next;
  }
  function activeAutoGroup(payload, model) {
    if (!payload || payload.gateway.autoModels === false) return null;
    const found = payload.autoGroups?.find((group) => group.name.toLowerCase() === model.toLowerCase());
    return found && !found.hidden ? found : null;
  }
  function unlistedModels(payload) {
    const hidden = /* @__PURE__ */ new Set();
    const add = (group) => {
      for (const member of group.members) {
        const model = member.model.trim();
        if (model && !model.includes("*") && !model.startsWith(GROUP_MODEL_PREFIX)) hidden.add(model.toLowerCase());
      }
    };
    for (const group of payload?.groups ?? []) if (!group.disabled && group.hideMembers) add(group);
    if (payload?.gateway.autoModels !== false) {
      for (const group of payload?.autoGroups ?? []) if (group.hideMembers && !group.hidden) add(group);
    }
    return hidden;
  }
  function memberTarget(payload, model) {
    const qualified = qualifiedProvider(payload, model.trim());
    if (qualified) return qualified;
    const serving = servingProviders(payload, model);
    return serving.length === 1 ? { entry: serving[0], model: model.trim() } : null;
  }

  // apps/codex-plus-manager/src/gateway-agent-safety.ts
  var gateway_agent_safety_exports = {};
  __export(gateway_agent_safety_exports, {
    agentDiagnosticPresentation: () => agentDiagnosticPresentation,
    canReconnectAgent: () => canReconnectAgent,
    diagnosticErrorLabel: () => diagnosticErrorLabel,
    diagnosticEvidenceLabel: () => diagnosticEvidenceLabel,
    disconnectFileLabel: () => disconnectFileLabel,
    disconnectPreviewReady: () => disconnectPreviewReady,
    fullSwitchReady: () => fullSwitchReady
  });
  function canReconnectAgent(agent, relayProfilesEnabled) {
    return agent.agent !== "codex" || relayProfilesEnabled !== false;
  }
  function agentDiagnosticPresentation(agent, helperRunning) {
    if (!agent.enabled) return { badge: "未接入", tone: "off", reconnect: false };
    const state = agent.diagnostics?.state;
    if (state === "configUnreadable") return { badge: "配置无法读取", tone: "error", reconnect: false };
    if (state === "configUnverified") return { badge: "配置无法验证", tone: "error", reconnect: false };
    if (state === "configMissing") return { badge: "配置缺失", tone: "error", reconnect: true };
    if (state === "configChanged" || !state && agent.drift) return { badge: "配置被改动", tone: "error", reconnect: true };
    if (!helperRunning || state === "gatewayStopped") return { badge: "网关未运行", tone: "error", reconnect: false };
    if (state === "upstreamError") return { badge: "上游请求失败", tone: "error", reconnect: false };
    if (state === "active") return { badge: "请求已到达", tone: "on", reconnect: false };
    if (state === "disabled") return { badge: "未接入", tone: "off", reconnect: false };
    if (state && state !== "awaitingTraffic") return { badge: "状态待确认", tone: "pending", reconnect: false };
    return { badge: "等待请求验证", tone: "pending", reconnect: false };
  }
  function disconnectPreviewReady(preview, agent) {
    return Boolean(preview?.canDisconnect && preview.agent === agent && preview.previewToken.trim());
  }
  function fullSwitchReady(preview, agent) {
    return Boolean(preview && !preview.canDisconnect && preview.fullSwitch && preview.agent === agent && preview.previewToken.trim());
  }
  function disconnectFileLabel(kind) {
    switch (kind) {
      case "create":
        return "将创建";
      case "remove":
        return "将移除";
      case "modify":
        return "将修改";
    }
  }
  function diagnosticEvidenceLabel(code) {
    const labels = {
      matches: "与网关配置一致",
      matched: "与网关配置一致",
      missing: "配置项缺失",
      missingFile: "文件不存在",
      missing_file: "文件不存在",
      changed: "配置项已改变",
      mismatch: "与网关配置不符",
      endpointChanged: "地址已改变",
      endpoint_changed: "地址已改变",
      unreadable: "无法读取",
      parseError: "无法解析",
      parse_error: "无法解析",
      credentialChanged: "本地凭据已改变",
      credential_changed: "本地凭据已改变",
      credentialMissing: "本地凭据缺失",
      credential_missing: "本地凭据缺失",
      endpointMatched: "与网关配置一致",
      endpointPortMismatch: "端口与网关配置不符",
      endpointHostMismatch: "主机与网关配置不符",
      endpointPathMismatch: "路径与网关配置不符",
      endpointMissing: "地址缺失",
      selectionChanged: "当前供应商或模型已改变",
      fileMissing: "文件不存在",
      fileUnreadable: "无法读取",
      invalidConfig: "配置格式无效",
      unsupportedConfig: "此配置暂无法验证",
      endpoint_match: "与网关配置一致",
      credential_match: "与网关配置一致",
      credential_sealed: "凭据已由系统安全存储加密保存",
      selection_match: "与网关配置一致",
      managed_config_match: "与网关配置一致",
      config_missing: "配置缺失",
      config_unreadable: "无法读取",
      config_invalid: "配置格式无效",
      config_unverified: "此配置暂无法验证",
      unsupported_agent: "此 Agent 暂无法验证",
      endpoint_missing: "地址缺失",
      endpoint_invalid: "地址格式无效",
      endpoint_host_mismatch: "主机与网关配置不符",
      endpoint_port_mismatch: "端口与网关配置不符",
      endpoint_path_mismatch: "路径与网关配置不符",
      selection_changed: "当前供应商或模型已改变",
      managed_config_changed: "托管配置已改变"
    };
    return labels[code] ?? "诊断信息待确认";
  }
  function diagnosticErrorLabel(kind) {
    const labels = {
      authInvalid: "认证已失效",
      rateLimited: "请求频率受限",
      quotaExhausted: "可用额度不足",
      serverError: "上游服务异常",
      clientError: "上游拒绝请求",
      transport: "上游连接失败",
      cooling: "上游暂时冷却",
      upstreamError: "上游请求失败",
      auth_invalid: "认证已失效",
      rate_limited: "请求频率受限",
      quota_exhausted: "可用额度不足",
      server_error: "上游服务异常",
      client_error: "上游拒绝请求",
      upstream_error: "上游请求失败"
    };
    return labels[kind] ?? "错误原因待确认";
  }
  return __toCommonJS(provider_editing_entry_exports);
})(); // 结束 codexPlusProviderEditing
