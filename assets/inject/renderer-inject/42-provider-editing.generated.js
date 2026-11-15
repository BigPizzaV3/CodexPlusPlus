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

  // apps/codex-plus-manager/src/relay-profile-editing.ts
  var relay_profile_editing_exports = {};
  __export(relay_profile_editing_exports, {
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
  return __toCommonJS(relay_profile_editing_exports);
})(); // 结束 codexPlusProviderEditing
