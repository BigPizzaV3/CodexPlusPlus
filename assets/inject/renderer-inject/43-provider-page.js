  /**
   * Codex 左侧图标栏的「供应商」页：不离开 Codex 就能增删改切 Codex 的供应商。
   *
   * 分工：
   *   - 字段 ↔ config.toml / auth.json 的联动用 42 分片里与管理器共用的纯函数
   *     （codexPlusProviderEditing，来源 apps/codex-plus-manager/src/relay-profile-editing.ts），
   *     行为与管理器的「连接与供应商」页一致；
   *   - 保存 / 切换经 bridge 路由 /providers/*（core provider_page），最终跑的是管理器自己的
   *     save_settings / switch_relay_profile：归一化、切换锁、live 文件、网关绑定都只有一份。
   *   - 命名 Key 只下发打码预览，提交时由服务端按 id 还原；选中别的已存 Key 不改文件，
   *     由服务端归一化时按选中的那把重写，所以页面里永远拿不到备用 Key 的明文。
   *
   * 本页编辑直连供应商（官方 / 官方+API / 纯 API）与聚合供应商（故障转移 / 模型聚合：成员、权重、
   * 策略、路由、默认成员、审批成员，同样用共用函数归一化与校验）；网关接管条目只能「使用」。
   * 直连供应商的模型路由、请求头、兼容选项仍在管理器里编辑（「在 Codex++ 中打开」）。
   *
   * 页面 DOM 每次打开重建，状态只放在 codexPlusProviderPageState 里（见 01-registry 的生命周期约定）。
   */
  const codexPlusProviderEditingApi = codexPlusProviderEditing;
  const codexPlusProviderPageState = {
    data: null,
    settings: null,
    selectedId: "",
    draft: null,
    isNew: false,
    dirty: false,
    busy: "",
    status: "",
    statusTone: "",
    keySources: {},
    pollTimer: null,
    fetchedModels: [],
    // "gateway" | "direct"；打开页面时按 Codex 当前的接入方式选。
    tab: "",
  };

  codexPlusProviderEditingApi.configureRelayProfileEditing({
    defaultRelayBaseUrl: () => String(codexPlusProviderPageState.data?.defaults?.relayBaseUrl || ""),
    defaultRelayProfiles: () => codexPlusProviderPageState.data?.defaults?.relayProfiles || [],
    // contextSelection 已是遗留字段（MCP / Skill 的启停由条目自己的开关决定），给空值即可。
    contextSelectionForAllEntries: () => ({ mcpServers: [], skills: [], plugins: [] }),
  });

  function codexPlusProviderClone(value) {
    return value == null ? value : JSON.parse(JSON.stringify(value));
  }

  function codexPlusProviderIsAggregate(profile) {
    return codexPlusProviderEditingApi.isAggregateRelayProfile(profile || {});
  }

  /** 直连供应商：字段与 config.toml / auth.json 联动的那一套编辑。 */
  function codexPlusProviderEditable(profile) {
    return !!profile && !profile.gatewayManaged && !codexPlusProviderIsAggregate(profile);
  }

  /** 能在本页保存的条目：直连与聚合都行，网关接管条目不行。 */
  function codexPlusProviderCanEdit(profile) {
    return !!profile && !profile.gatewayManaged;
  }

  function codexPlusProviderModeKey(profile) {
    if (!profile) return "official";
    if (profile.relayMode === "pureApi") return "pureApi";
    if (profile.relayMode === "mixedApi" || profile.officialMixApiKey) return "mixedApi";
    return profile.relayMode === "aggregate" ? "aggregate" : "official";
  }

  function codexPlusProviderModeLabel(profile) {
    if (profile?.gatewayManaged) return "网关";
    const mode = codexPlusProviderModeKey(profile);
    if (mode === "aggregate") return profile.aggregate?.modelAggregationEnabled === true ? "模型聚合" : "故障转移";
    return { pureApi: "纯 API", mixedApi: "官方 + API", official: "官方登录" }[mode];
  }

  /** 进入编辑：与管理器一致，先按文件回读出 baseUrl / model / apiKey 等派生字段。 */
  function codexPlusProviderDraftFrom(profile) {
    const copy = codexPlusProviderClone(profile);
    return codexPlusProviderEditable(copy) ? codexPlusProviderEditingApi.deriveRelayProfileFromFiles(copy) : copy;
  }

  function codexPlusProviderSelected() {
    const state = codexPlusProviderPageState;
    return state.settings?.relayProfiles?.find((profile) => profile.id === state.selectedId) || null;
  }

  function codexPlusProviderSelect(id) {
    const state = codexPlusProviderPageState;
    state.selectedId = id;
    state.isNew = false;
    state.dirty = false;
    state.fetchedModels = [];
    state.draft = codexPlusProviderDraftFrom(codexPlusProviderSelected());
  }

  function codexPlusProviderSetStatus(message, tone = "") {
    codexPlusProviderPageState.status = String(message || "");
    codexPlusProviderPageState.statusTone = tone;
    // 同步底部状态栏（如果当前是直连页签）
    const state = codexPlusProviderPageState;
    if (state.tab !== "gateway") {
      const footer = document.querySelector("[data-codex-provider-footer]");
      if (footer) {
        footer.textContent = codexPlusProviderPageState.status;
        footer.dataset.tone = tone;
      }
    }
  }

  /**
   * 与管理器 normalizeSettings 读取时一致：聚合配置回填进供应商、补齐缺省字段并按文件回读派生字段
   * （服务端不序列化 model / baseUrl / apiKey，共用函数假定它们总是字符串）。
   */
  function codexPlusProviderNormalizeSettings(raw) {
    const settings = codexPlusProviderClone(raw);
    const aggregates = new Map((settings.aggregateRelayProfiles || []).map((aggregate) => [aggregate.id, aggregate]));
    settings.relayProfiles = (settings.relayProfiles || []).map((profile) => {
      const gatewayManaged = profile.gatewayManaged === true;
      const normalized = codexPlusProviderEditingApi.normalizeRelayProfile(
        codexPlusProviderEditingApi.hydrateAggregateRelayProfile(profile, aggregates.get(profile.id)),
      );
      // 打码 Key 的标记要保留到提交，服务端靠它还原真值。
      normalized.apiKeys = (normalized.apiKeys || []).map((entry) => {
        const original = (profile.apiKeys || []).find((item) => item.id === entry.id);
        return original ? { ...entry, stored: original.stored === true, preview: original.preview || "" } : entry;
      });
      return gatewayManaged ? { ...normalized, gatewayManaged } : normalized;
    });
    return settings;
  }

  function codexPlusProviderApplyList(result, { keepSelection = true } = {}) {
    const state = codexPlusProviderPageState;
    state.data = result;
    state.settings = codexPlusProviderNormalizeSettings(result.settings);
    state.keySources = {};
    const ids = state.settings.relayProfiles.map((profile) => profile.id);
    if (!keepSelection || !ids.includes(state.selectedId)) state.selectedId = state.settings.activeRelayId || ids[0] || "";
    codexPlusProviderSelect(state.selectedId);
  }

  async function codexPlusProviderLoad({ keepSelection = true, quiet = false } = {}) {
    const state = codexPlusProviderPageState;
    if (!quiet) {
      state.busy = "load";
      renderCodexPlusProviderPage();
    }
    const result = await postJson("/providers/list", {});
    state.busy = "";
    if (result?.status !== "ok" || !result.settings) {
      codexPlusProviderSetStatus(result?.message || "读取供应商失败", "error");
      renderCodexPlusProviderPage();
      return false;
    }
    codexPlusProviderApplyList(result, { keepSelection });
    if (!state.tab) state.tab = result.gatewayMode === true ? "gateway" : "direct";
    renderCodexPlusProviderPage();
    if (state.tab === "gateway" && !codexPlusGatewayPageState.payload && !codexPlusGatewayPageState.busy) void codexPlusGatewayLoad();
    return true;
  }

  /**
   * 提交整份供应商列表。服务端：当前供应商没变只保存；切换或改了当前供应商时走管理器的切换流程。
   * 切换遇到 live 配置被外部改过时返回 conflict，这里问用户三选一后带着选择重提。
   */
  async function codexPlusProviderCommit(settings, { activate = "", busy = "save", successMessage = "" } = {}) {
    const state = codexPlusProviderPageState;
    state.busy = busy;
    renderCodexPlusProviderPage();
    let conflictResolution = "stop";
    let result;
    const relayProfiles = codexPlusProviderProfilesForCommit(settings.relayProfiles);
    for (;;) {
      result = await postJson("/providers/commit", {
        revision: state.data?.revision || "",
        relayProfiles,
        aggregateRelayProfiles: settings.aggregateRelayProfiles,
        activeAggregateRelayId: settings.activeAggregateRelayId,
        keySources: state.keySources,
        ...(activate ? { activate } : {}),
        conflictResolution,
      });
      if (result?.status !== "conflict" || conflictResolution !== "stop") break;
      const name = result.liveConflict?.profileName || "当前供应商";
      const answer = await codexPlusAsk({
        title: "Codex 的实时配置被改过",
        message: `「${name}」在 config.toml / auth.json 里的地址或 Key 和保存的不一致，可能是你或别的工具手动改过。要怎么处理？`,
        options: [
          { value: "discardLiveChanges", label: "放弃实时改动" },
          { value: "importAsNewProfile", label: "导入为新供应商" },
        ],
      });
      if (!answer) {
        result = { status: "failed", message: "已取消切换，未改动任何文件。" };
        break;
      }
      conflictResolution = answer.value;
    }
    state.busy = "";
    if (result?.status !== "ok" || !result.settings) {
      const conflict = String(result?.message || "").includes("settings_revision_conflict");
      codexPlusProviderSetStatus(conflict ? "供应商配置已在别处被修改，已重新读取，请再改一次。" : (result?.message || "保存失败"), "error");
      if (conflict) await codexPlusProviderLoad({ quiet: true });
      else renderCodexPlusProviderPage();
      return false;
    }
    const selectedId = state.selectedId;
    codexPlusProviderApplyList(result);
    if (result.settings.relayProfiles.some((profile) => profile.id === selectedId)) codexPlusProviderSelect(selectedId);
    const switched = result.switched === true;
    const message = successMessage || result.message || "已保存。";
    codexPlusProviderSetStatus(switched ? `${message} 新对话立即生效，已打开的对话在下一次发送时切换。` : message, "ok");
    showToast(switched ? "供应商已切换" : "供应商已保存", { type: "success" });
    renderCodexPlusProviderPage();
    return true;
  }

  /**
   * 没改过的供应商原样提交服务端下发的版本：页面读取时做过与管理器相同的归一化，
   * 原样回传归一化结果可能和库里有细微差别，服务端会误判「当前供应商变了」而去重写 live 文件
   * （网关接管时那次切换还会被拒，连带改别的供应商也保存不了）。
   */
  function codexPlusProviderProfilesForCommit(profiles) {
    const state = codexPlusProviderPageState;
    // 共用函数（syncLegacyRelayFields 等）会把每个供应商按文件重新派生一遍，所以两边都先派生、再按键排序比较。
    const fingerprint = (profile) => codexPlusProviderStableJson(codexPlusProviderEditable(profile)
      ? codexPlusProviderEditingApi.deriveRelayProfileFromFiles(codexPlusProviderClone(profile))
      : profile);
    const loaded = new Map((state.settings?.relayProfiles || []).map((profile) => [profile.id, fingerprint(profile)]));
    const raw = new Map((state.data?.settings?.relayProfiles || []).map((profile) => [profile.id, profile]));
    return profiles.map((profile) => (raw.has(profile.id) && loaded.get(profile.id) === fingerprint(profile) ? raw.get(profile.id) : profile));
  }

  function codexPlusProviderStableJson(value) {
    if (Array.isArray(value)) return `[${value.map(codexPlusProviderStableJson).join(",")}]`;
    if (value && typeof value === "object") {
      return `{${Object.keys(value).sort().filter((key) => value[key] !== undefined)
        .map((key) => `${JSON.stringify(key)}:${codexPlusProviderStableJson(value[key])}`).join(",")}}`;
    }
    return JSON.stringify(value);
  }

  /** 当前编辑结果放回列表；返回待提交的设置，校验不过时返回 null 并提示。 */
  function codexPlusProviderSettingsWithDraft() {
    const state = codexPlusProviderPageState;
    const draft = state.draft;
    if (!draft || !codexPlusProviderCanEdit(draft)) return codexPlusProviderClone(state.settings);
    if (codexPlusProviderIsAggregate(draft)) {
      // 与管理器保存聚合草稿一致：先按会话身份 / 成员 / 默认成员 / 路由校验，再归一化后放回列表
      // （syncLegacyRelayFields 会重新生成 aggregateRelayProfiles，服务端按它保存）。
      const aggregateIssue = !String(draft.name || "").trim()
        ? "请填写供应商名称。"
        : codexPlusProviderEditingApi.relaySessionProviderValidation(draft)
          || codexPlusProviderEditingApi.aggregateRelayProfileValidation(draft);
      if (aggregateIssue) {
        codexPlusProviderSetStatus(aggregateIssue, "error");
        return null;
      }
      const next = codexPlusProviderEditingApi.relaySettingsWithDraft(state.settings, draft.id, draft, state.isNew);
      const settingsIssue = codexPlusProviderEditingApi.relaySettingsValidation(next);
      if (settingsIssue) {
        codexPlusProviderSetStatus(settingsIssue, "error");
        return null;
      }
      return next;
    }
    const issue = codexPlusProviderEditingApi.relaySessionProviderValidation(draft);
    if (issue) {
      codexPlusProviderSetStatus(issue, "error");
      return null;
    }
    if (!String(draft.name || "").trim()) {
      codexPlusProviderSetStatus("请填写供应商名称。", "error");
      return null;
    }
    const normalized = codexPlusProviderEditingApi.deriveRelayProfileFromFiles(draft);
    const next = state.isNew
      ? codexPlusProviderEditingApi.addRelayProfile(state.settings, normalized)
      : codexPlusProviderEditingApi.updateRelayProfile(state.settings, draft.id, normalized);
    const settingsIssue = codexPlusProviderEditingApi.relaySettingsValidation(next);
    if (settingsIssue) {
      codexPlusProviderSetStatus(settingsIssue, "error");
      return null;
    }
    return next;
  }

  async function codexPlusProviderSave() {
    const next = codexPlusProviderSettingsWithDraft();
    if (!next) return renderCodexPlusProviderPage();
    await codexPlusProviderCommit(next);
  }

  async function codexPlusProviderUse(id) {
    const state = codexPlusProviderPageState;
    if (state.data?.gatewayMode === true) {
      // 管理器在网关接管时会拒绝直接切换：先给出还原预览，确认断开后再切到这个供应商。
      await codexPlusGatewayOpenDisconnect(() => codexPlusProviderUse(id));
      return;
    }
    let next = codexPlusProviderClone(state.settings);
    if (state.dirty && state.draft?.id === id) {
      next = codexPlusProviderSettingsWithDraft();
      if (!next) return renderCodexPlusProviderPage();
    }
    const target = next.relayProfiles.find((profile) => profile.id === id);
    const issue = target ? codexPlusProviderEditingApi.relayProfileSwitchValidation(target, next) : "供应商不存在";
    if (issue) {
      codexPlusProviderSetStatus(issue, "error");
      return renderCodexPlusProviderPage();
    }
    await codexPlusProviderCommit(next, { activate: id, busy: "switch" });
  }

  async function codexPlusProviderDelete(id) {
    const state = codexPlusProviderPageState;
    const profile = state.settings.relayProfiles.find((item) => item.id === id);
    if (!profile) return;
    const issue = codexPlusProviderEditingApi.relayProfileDeleteIssue(state.settings, id);
    if (issue) {
      codexPlusProviderSetStatus(issue, "error");
      return renderCodexPlusProviderPage();
    }
    const answer = await codexPlusAsk({
      title: "删除供应商",
      message: `确定删除「${profile.name || id}」？它的 Key 和配置快照会一起删掉。`,
      options: [{ value: "delete", label: "删除" }],
    });
    if (!answer) return;
    const next = codexPlusProviderEditingApi.removeRelayProfile(state.settings, id);
    if (state.selectedId === id) state.selectedId = state.settings.activeRelayId;
    await codexPlusProviderCommit(next, { successMessage: `已删除「${profile.name || id}」。` });
  }

  async function codexPlusProviderDuplicate(id) {
    const state = codexPlusProviderPageState;
    const before = new Set(state.settings.relayProfiles.map((profile) => profile.id));
    const next = codexPlusProviderEditingApi.duplicateRelayProfile(state.settings, id);
    const copy = next.relayProfiles.find((profile) => !before.has(profile.id));
    if (!copy) return;
    // 打码的 Key 由服务端按来源供应商还原。
    state.keySources[copy.id] = id;
    state.selectedId = copy.id;
    await codexPlusProviderCommit(next, { successMessage: `已复制为「${copy.name}」。` });
  }

  async function codexPlusProviderMove(id, offset) {
    const state = codexPlusProviderPageState;
    const ids = state.settings.relayProfiles.map((profile) => profile.id);
    const index = ids.indexOf(id);
    const target = ids[index + offset];
    if (index < 0 || !target) return;
    await codexPlusProviderCommit(codexPlusProviderEditingApi.reorderRelayProfiles(state.settings, id, target), { busy: "reorder", successMessage: "顺序已更新。" });
  }

  /** 新建：kind 为 "direct"（纯 API 直连）、"failover"（故障转移）或 "model"（模型聚合），与管理器的新建入口一致。 */
  function codexPlusProviderStartNew(kind = "direct") {
    const state = codexPlusProviderPageState;
    const api = codexPlusProviderEditingApi;
    if (kind === "failover" || kind === "model") {
      const memberSettings = codexPlusProviderMemberSettings();
      state.draft = api.aggregateProfileForMode(api.createAggregateRelayProfile(memberSettings), memberSettings, kind === "model" ? "model" : "provider");
    } else {
      const draft = api.createRelayProfile(state.settings);
      state.draft = api.applyRelayProfilePatchToFiles(draft, { name: "自定义供应商", relayMode: "pureApi", officialMixApiKey: false }, { allowGenerateFiles: true });
    }
    state.selectedId = state.draft.id;
    state.isNew = true;
    state.dirty = true;
    state.fetchedModels = [];
    codexPlusProviderSetStatus("");
    renderCodexPlusProviderPage();
    // 重绘会沿用上一个供应商的滚动位置；新建从头看起。
    const detail = document.querySelector("[data-codex-provider-panel] .codex-plus-provider-detail");
    if (detail) detail.scrollTop = 0;
  }

  /**
   * 判定聚合成员用的设置视图：页面里已存的命名 Key 只有打码预览，派生出的 apiKey 是空的，
   * 共用的成员判定（URL + Key 都填才算 API 成员）会把它们全当成不可用。这里给「当前 Key 已存」的
   * 供应商填一个占位 Key，只用来判定成员与新建聚合的初始成员，从不提交。
   */
  function codexPlusProviderMemberSettings() {
    const settings = codexPlusProviderPageState.settings;
    const relayProfiles = (settings?.relayProfiles || []).map((profile) => {
      if (String(profile.apiKey || "").trim()) return profile;
      const stored = (profile.apiKeys || []).some((entry) => entry.id === profile.activeApiKeyId && entry.stored === true);
      return stored ? { ...profile, apiKey: "stored" } : profile;
    });
    return { ...settings, relayProfiles };
  }

  /** 可勾选的聚合成员：API 供应商或已完成官方登录的账号（与管理器同一判定）。 */
  function codexPlusProviderAggregateCandidates(draft) {
    return codexPlusProviderEditingApi.aggregateMemberCandidates(codexPlusProviderMemberSettings().relayProfiles, draft.id);
  }

  function codexPlusProviderAggregateConfig(draft) {
    return codexPlusProviderEditingApi.normalizeAggregateConfig(draft.aggregate, codexPlusProviderAggregateCandidates(draft));
  }

  /** 改聚合配置：与管理器 AggregateRelayProfileEditor 的 updateAggregate 一样，改完按当前列表归一化。 */
  function codexPlusProviderUpdateAggregate(patch, profilePatch = {}) {
    const state = codexPlusProviderPageState;
    if (!state.draft) return;
    const aggregate = codexPlusProviderAggregateConfig(state.draft);
    state.draft = codexPlusProviderEditingApi.normalizeAggregateRelayProfile(
      { ...state.draft, ...profilePatch, aggregate: { ...aggregate, ...patch } },
      state.settings,
    );
    state.dirty = true;
    codexPlusProviderSyncFileFields();
  }

  function codexPlusProviderToggleMember(profileId) {
    const aggregate = codexPlusProviderAggregateConfig(codexPlusProviderPageState.draft);
    const members = aggregate.members.some((member) => member.profileId === profileId)
      ? aggregate.members.filter((member) => member.profileId !== profileId)
      : [...aggregate.members, { profileId, weight: 1 }];
    codexPlusProviderUpdateAggregate({ members });
  }

  function codexPlusProviderUpdateRoute(index, patch) {
    const routes = codexPlusProviderAggregateConfig(codexPlusProviderPageState.draft).routes || [];
    codexPlusProviderUpdateAggregate({ routes: routes.map((route, routeIndex) => (routeIndex === index ? { ...route, ...patch } : route)) });
  }

  /** 与管理器 updateDraft 一致：字段改动按需同步进 config.toml / auth.json。 */
  function codexPlusProviderPatch(patch) {
    const state = codexPlusProviderPageState;
    if (!state.draft) return;
    state.draft = codexPlusProviderEditingApi.applyRelayProfilePatchToFiles(state.draft, patch, { allowGenerateFiles: state.isNew });
    state.dirty = true;
    codexPlusProviderSyncFileFields();
  }

  function codexPlusProviderSyncFileFields() {
    const draft = codexPlusProviderPageState.draft;
    const config = document.querySelector('[data-codex-provider-file="config"]');
    const auth = document.querySelector('[data-codex-provider-file="auth"]');
    if (config && document.activeElement !== config) config.value = draft?.configContents || "";
    if (auth && document.activeElement !== auth) auth.value = draft?.authContents || "";
    const dirty = document.querySelector("[data-codex-provider-dirty]");
    if (dirty) dirty.hidden = !codexPlusProviderPageState.dirty;
    // 输入不重绘整页：「保存」按钮的可用状态在这里跟着改。
    const save = document.querySelector('[data-codex-provider-panel] [data-codex-provider-action="save"]');
    if (save) save.disabled = Boolean(codexPlusProviderPageState.busy) || !codexPlusProviderPageState.dirty;
  }

  /** 模型列表：一行一个，`slug[1M]` 后缀写窗口（与管理器的 model_list 语法相同）。 */
  function codexPlusProviderModelLines(draft) {
    let windows = {};
    try {
      windows = JSON.parse(draft?.modelWindows || "{}") || {};
    } catch {}
    return String(draft?.modelList || "")
      .split("\n")
      .map((line) => line.trim())
      .filter(Boolean)
      .map((slug) => (windows[slug] && !slug.includes("[") ? `${slug}[${windows[slug]}]` : slug))
      .join("\n");
  }

  function codexPlusProviderApplyModelLines(text) {
    const state = codexPlusProviderPageState;
    if (!state.draft) return;
    const windows = {};
    const slugs = [];
    String(text || "").split("\n").map((line) => line.trim()).filter(Boolean).forEach((line) => {
      const parsed = codexPlusProviderEditingApi.parseModelSuffix(line);
      if (!parsed.slug || slugs.includes(parsed.slug)) return;
      slugs.push(parsed.slug);
      if (parsed.window) windows[parsed.slug] = String(parsed.window);
    });
    state.draft = { ...state.draft, modelList: slugs.join("\n"), modelWindows: JSON.stringify(windows) };
    state.dirty = true;
    codexPlusProviderSyncFileFields();
  }

  function codexPlusProviderAddKey(name, value) {
    const state = codexPlusProviderPageState;
    const key = String(value || "").trim();
    if (!key) return;
    const keys = Array.isArray(state.draft.apiKeys) ? state.draft.apiKeys.slice() : [];
    const id = `key-${Date.now().toString(36)}`;
    keys.push({ id, name: String(name || "").trim() || `Key ${keys.length + 1}`, apiKey: key });
    // 新填的 Key 是明文，按管理器的 updateApiKeys 同步进文件并设为当前。
    codexPlusProviderPatch({ apiKeys: keys, activeApiKeyId: id, apiKey: key });
    renderCodexPlusProviderPage();
  }

  function codexPlusProviderSelectKey(id) {
    const state = codexPlusProviderPageState;
    const keys = state.draft.apiKeys || [];
    const entry = keys.find((item) => item.id === id);
    if (!entry) return;
    if (entry.apiKey) {
      codexPlusProviderPatch({ apiKeys: keys, activeApiKeyId: id, apiKey: entry.apiKey });
    } else {
      // 已存 Key 只有打码预览：只改选中项，文件由服务端按真值重写。
      state.draft = { ...state.draft, activeApiKeyId: id };
      state.dirty = true;
    }
    renderCodexPlusProviderPage();
  }

  function codexPlusProviderRemoveKey(id) {
    const state = codexPlusProviderPageState;
    const keys = (state.draft.apiKeys || []).filter((item) => item.id !== id);
    if (state.draft.activeApiKeyId === id) {
      const next = keys[0];
      if (next?.apiKey) codexPlusProviderPatch({ apiKeys: keys, activeApiKeyId: next.id, apiKey: next.apiKey });
      else state.draft = { ...state.draft, apiKeys: keys, activeApiKeyId: next?.id || "" };
    } else {
      state.draft = { ...state.draft, apiKeys: keys };
    }
    state.dirty = true;
    renderCodexPlusProviderPage();
  }

  async function codexPlusProviderProbe(kind) {
    const state = codexPlusProviderPageState;
    if (!state.draft) return;
    state.busy = kind;
    renderCodexPlusProviderPage();
    const profile = codexPlusProviderEditingApi.deriveRelayProfileFromFiles(state.draft);
    const result = await postJson(kind === "test" ? "/providers/test" : "/providers/models", {
      profile,
      keySourceId: state.keySources[profile.id] || "",
    });
    state.busy = "";
    if (kind === "models" && result?.status === "ok") state.fetchedModels = Array.isArray(result.models) ? result.models : [];
    codexPlusProviderSetStatus(result?.message || "请求失败", result?.status === "ok" ? "ok" : result?.status === "warning" ? "warn" : "error");
    renderCodexPlusProviderPage();
  }

  function renderCodexPlusProviderList() {
    const state = codexPlusProviderPageState;
    const profiles = state.settings?.relayProfiles || [];
    const rows = profiles.map((profile, index) => {
      const active = profile.id === state.settings.activeRelayId;
      const selected = profile.id === state.selectedId && !state.isNew;
      return `<div class="codex-plus-provider-item" role="listitem" data-selected="${selected}" data-codex-provider-select="${escapeHtml(profile.id)}">
        <div class="codex-plus-provider-item-main">
          <div class="codex-plus-provider-item-name">${active ? '<span class="codex-plus-provider-item-dot" aria-hidden="true"></span>' : ""}${escapeHtml(profile.name || profile.id)}</div>
          <div class="codex-plus-provider-item-meta"><span class="codex-plus-provider-badge">${escapeHtml(codexPlusProviderModeLabel(profile))}</span>${active ? '<span class="codex-plus-provider-badge" data-tone="active">使用中</span>' : ""}</div>
          ${profile.model || profile.baseUrl ? `<div class="codex-plus-provider-item-sub">${escapeHtml(profile.model || "")}${profile.model && profile.baseUrl ? " · " : ""}${escapeHtml(String(profile.baseUrl || "").replace(/^https?:\/\//, "").replace(/\/.*$/, ""))}</div>` : ""}
        </div>
        <div class="codex-plus-provider-item-actions">
          <button type="button" title="上移" aria-label="上移" data-codex-provider-move="-1" data-id="${escapeHtml(profile.id)}" ${index === 0 ? "disabled" : ""}>↑</button>
          <button type="button" title="下移" aria-label="下移" data-codex-provider-move="1" data-id="${escapeHtml(profile.id)}" ${index === profiles.length - 1 ? "disabled" : ""}>↓</button>
        </div>
      </div>`;
    }).join("");
    const draftRow = state.isNew && state.draft
      ? `<div class="codex-plus-provider-item" role="listitem" data-selected="true"><div class="codex-plus-provider-item-main"><div class="codex-plus-provider-item-name">${escapeHtml(state.draft.name || "新供应商")}</div><div class="codex-plus-provider-item-meta"><span class="codex-plus-provider-badge" data-tone="new">未保存</span></div></div></div>`
      : "";
    return `<div class="codex-plus-provider-list" role="list" aria-label="供应商">${rows}${draftRow}</div>`;
  }

  function codexPlusProviderInput(key, value, attrs = "") {
    return `<input type="text" class="codex-plus-provider-input" data-codex-provider-field="${key}" value="${escapeHtml(value ?? "")}" ${attrs}>`;
  }

  /** 详情页的一张分组卡片：小标题 + 圆角卡片（行间细线），照 Codex 原生设置页；note 是卡片下面的小字。 */
  function codexPlusProviderCard(title, rows, note = "") {
    return `<section class="codex-plus-provider-group"><div class="codex-plus-provider-section-title">${escapeHtml(title)}</div><div class="codex-plus-provider-card">${rows}</div>${note ? `<div class="codex-plus-provider-group-note">${escapeHtml(note)}</div>` : ""}</section>`;
  }

  /** 卡片里的一行：左边标题（+ 说明），右边控件；stack 为 true 时控件换到下一行铺满（多行文本框用）。 */
  function codexPlusProviderLine(label, control, help = "", stack = false) {
    return `<label class="codex-plus-provider-line"${stack ? ' data-stack="true"' : ""}><span class="codex-plus-provider-line-copy"><span class="codex-plus-provider-line-title">${escapeHtml(label)}</span>${help ? `<span class="codex-plus-provider-line-help">${escapeHtml(help)}</span>` : ""}</span><span class="codex-plus-provider-line-control">${control}</span></label>`;
  }

  /**
   * 卡片里的按钮行：左边标题 + 说明，右边按钮。用 div 而不是 label（label 会把整行的点击转给第一个按钮）。
   * helpHtml 为 true 时 help 已是拼好的 HTML（里面带链接按钮），调用方负责转义。
   */
  function codexPlusProviderActionLine(label, help, control, helpHtml = false) {
    // data-kind="action"：右边只有按钮，控件区按内容收窄，说明文字才不会被挤成两行。
    return `<div class="codex-plus-provider-line" data-kind="action"><span class="codex-plus-provider-line-copy"><span class="codex-plus-provider-line-title">${escapeHtml(label)}</span>${help ? `<span class="codex-plus-provider-line-help">${helpHtml ? help : escapeHtml(help)}</span>` : ""}</span><span class="codex-plus-provider-line-control">${control}</span></div>`;
  }

  /**
   * 和 Codex 原生一样的下拉：胶囊按钮 + 自绘菜单（复用 10-style 的 .codex-plus-menu-*，与拓展页排序同一套）。
   * 不用原生 select：它的弹出层不跟主题，样子也和 Codex 不一样。
   * 当前值放在按钮的 data-value 上；菜单只在 DOM 里开合、不重绘整页——模型行里还没填名字的新行只存在于 DOM，整页重绘会丢。
   * kind 决定选中后做什么（见 codexPlusProviderMenuChanged），attrs 给读值的地方留钩子（如 data-codex-provider-model-window）。
   */
  function codexPlusProviderMenu(kind, value, options, label, attrs = "") {
    const current = options.find(([id]) => id === value) || options[0] || ["", ""];
    return `<div class="codex-plus-provider-menu"><button type="button" class="codex-plus-menu-trigger" data-codex-provider-menu-toggle="${escapeHtml(kind)}" data-value="${escapeHtml(current[0])}" data-options="${escapeHtml(JSON.stringify(options))}" data-menu-label="${escapeHtml(label)}" aria-haspopup="menu" aria-expanded="false" aria-label="${escapeHtml(label)}：${escapeHtml(current[1])}" ${attrs}><span class="codex-plus-provider-menu-label">${escapeHtml(current[1])}</span>${codexPlusMenuIcons.chevron}</button></div>`;
  }

  /** 动作菜单（如「+ 新建」）：按钮文字固定，不显示当前值，菜单里也没有勾选项。 */
  function codexPlusProviderActionMenu(kind, text, options, attrs = "") {
    return `<div class="codex-plus-provider-menu"><button type="button" class="codex-plus-menu-trigger" data-codex-provider-menu-toggle="${escapeHtml(kind)}" data-fixed-label="true" data-value="" data-options="${escapeHtml(JSON.stringify(options))}" data-menu-label="${escapeHtml(text)}" aria-haspopup="menu" aria-expanded="false" aria-label="${escapeHtml(text)}" ${attrs}><span class="codex-plus-provider-menu-label">${escapeHtml(text)}</span>${codexPlusMenuIcons.chevron}</button></div>`;
  }

  function codexPlusProviderOpenMenu(trigger) {
    codexPlusProviderCloseMenu();
    let options = [];
    try {
      options = JSON.parse(trigger.dataset.options || "[]");
    } catch {}
    const value = trigger.dataset.value || "";
    trigger.setAttribute("aria-expanded", "true");
    trigger.insertAdjacentHTML("afterend", `<div class="codex-plus-menu-popover codex-plus-provider-menu-popover" role="menu" aria-label="${escapeHtml(trigger.dataset.menuLabel || "")}">${options
      .map(([id, text]) => `<button type="button" class="codex-plus-menu-item" role="menuitemradio" aria-checked="${String(id === value)}" data-codex-provider-menu-value="${escapeHtml(id)}"><span class="codex-plus-menu-item-label">${escapeHtml(text)}</span>${id === value ? codexPlusMenuIcons.check : ""}</button>`)
      .join("")}</div>`);
    // Codex 在 document 捕获阶段把按键送进输入框，菜单的方向键 / Esc 要在 window 捕获阶段先拿到。
    window.addEventListener("keydown", codexPlusProviderMenuKeydown, true);
    window.addEventListener("pointerdown", codexPlusProviderMenuPointerdown, true);
    const menu = trigger.nextElementSibling;
    (menu?.querySelector('[aria-checked="true"]') || menu?.querySelector(".codex-plus-menu-item"))?.focus({ preventScroll: true });
  }

  /** 收起菜单；`refocus` 时焦点回到按钮（键盘操作关掉菜单后不丢焦点）。 */
  function codexPlusProviderCloseMenu(refocus = false) {
    window.removeEventListener("keydown", codexPlusProviderMenuKeydown, true);
    window.removeEventListener("pointerdown", codexPlusProviderMenuPointerdown, true);
    const menu = document.querySelector(".codex-plus-provider-menu-popover");
    if (!menu) return;
    const trigger = menu.previousElementSibling;
    menu.remove();
    trigger?.setAttribute("aria-expanded", "false");
    if (refocus) trigger?.focus({ preventScroll: true });
  }

  /** 点菜单外面收起；点的是菜单自己或它的按钮就交给点击处理。 */
  function codexPlusProviderMenuPointerdown(event) {
    const target = event.target instanceof Element ? event.target : null;
    if (target?.closest(".codex-plus-provider-menu")) return;
    codexPlusProviderCloseMenu();
  }

  /** 菜单键盘操作：上下移动、Home / End、回车 / 空格选中、Esc / Tab 收起（与拓展页排序菜单一致）。 */
  function codexPlusProviderMenuKeydown(event) {
    const menu = document.querySelector(".codex-plus-provider-menu-popover");
    if (!menu) return codexPlusProviderCloseMenu();
    const items = Array.from(menu.querySelectorAll(".codex-plus-menu-item"));
    const index = items.indexOf(document.activeElement);
    let next = null;
    if (event.key === "ArrowDown") next = items[(index + 1) % items.length];
    else if (event.key === "ArrowUp") next = items[(index - 1 + items.length) % items.length];
    else if (event.key === "Home") next = items[0];
    else if (event.key === "End") next = items[items.length - 1];
    else if (event.key === "Enter" || event.key === " ") {
      if (index < 0) return;
      event.preventDefault();
      event.stopPropagation();
      codexPlusProviderChooseMenuItem(items[index]);
      return;
    } else if (event.key === "Escape" || event.key === "Tab") {
      event.preventDefault();
      event.stopPropagation();
      codexPlusProviderCloseMenu(event.key === "Escape");
      return;
    } else {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    next?.focus({ preventScroll: true });
  }

  /** 选中一项：按钮上的值和文字就地更新，再按 kind 写回草稿。 */
  function codexPlusProviderChooseMenuItem(item) {
    const trigger = item.closest(".codex-plus-provider-menu-popover")?.previousElementSibling;
    if (!trigger) return;
    const value = item.dataset.codexProviderMenuValue || "";
    const text = item.textContent.trim();
    if (trigger.dataset.fixedLabel !== "true") {
      trigger.dataset.value = value;
      const label = trigger.querySelector(".codex-plus-provider-menu-label");
      if (label) label.textContent = text;
      trigger.setAttribute("aria-label", `${trigger.dataset.menuLabel || ""}：${text}`);
    }
    codexPlusProviderCloseMenu(true);
    codexPlusProviderMenuChanged(trigger.dataset.codexProviderMenuToggle, value, trigger);
  }

  function codexPlusProviderMenuChanged(kind, value, trigger) {
    if (kind === "mode") {
      if (value === "official") codexPlusProviderPatch({ relayMode: "official", officialMixApiKey: false });
      else if (value === "mixedApi") codexPlusProviderPatch({ relayMode: "official", officialMixApiKey: true });
      else codexPlusProviderPatch({ relayMode: "pureApi", officialMixApiKey: false });
      return renderCodexPlusProviderPage();
    }
    if (kind === "protocol") {
      codexPlusProviderPatch({ protocol: value });
      return renderCodexPlusProviderPage();
    }
    if (kind === "model-window") {
      const list = trigger.closest("[data-codex-provider-model-list]");
      if (list) codexPlusProviderApplyModelLines(codexPlusProviderModelRowsText(list));
      return;
    }
    if (kind === "new") {
      void codexPlusProviderConfirmDiscard().then((ok) => { if (ok) codexPlusProviderStartNew(value); });
      return;
    }
    if (kind === "agg-strategy") codexPlusProviderUpdateAggregate({ strategy: value });
    else if (kind === "agg-session") codexPlusProviderUpdateAggregate({}, { sessionProvider: value });
    else if (kind === "agg-review") codexPlusProviderUpdateAggregate({ autoReviewRelayId: value });
    else if (kind === "agg-route-target") codexPlusProviderUpdateRoute(Number(trigger.dataset.aggRouteIndex), { profileId: value });
    if (kind.startsWith("agg-")) return renderCodexPlusProviderPage();
    // 网关页签：协议写进草稿（与输入框一样不重绘）；测试用模型只在点「测试」时读按钮上的值。
    if (kind === "gw-protocol") codexPlusGatewayHandleInput({ target: { dataset: { codexGatewayField: "protocol" }, value } });
  }

  /** 菜单的点击（开合、选中）；是它的事件就处理并返回 true。 */
  function codexPlusProviderHandleMenuClick(target) {
    const item = target.closest(".codex-plus-provider-menu-popover .codex-plus-menu-item");
    if (item) {
      codexPlusProviderChooseMenuItem(item);
      return true;
    }
    const trigger = target.closest("[data-codex-provider-menu-toggle]");
    if (!trigger) return false;
    if (trigger.getAttribute("aria-expanded") === "true") codexPlusProviderCloseMenu();
    else codexPlusProviderOpenMenu(trigger);
    return true;
  }

  const codexPlusProviderProtocolOptions = [["responses", "Responses"], ["chatCompletions", "Chat Completions"], ["anthropicMessages", "Anthropic Messages"]];

  /** 上下文窗口的显示名：1000000 → 1M，272000 → 272K。 */
  function codexPlusProviderWindowLabel(value) {
    const n = Number(value);
    if (!n) return String(value || "");
    if (n >= 1000000 && n % 1000000 === 0) return `${n / 1000000}M`;
    return n >= 1000 ? `${Math.round(n / 1000)}K` : String(n);
  }

  /**
   * 模型行编辑器：一行一个模型（默认星标、名称、上下文窗口、移除）。
   * 写回仍走 codexPlusProviderApplyModelLines，`slug[窗口]` 的序列化与管理器、文本框完全一致。
   */
  function codexPlusProviderModelEditor(draft) {
    const rows = codexPlusProviderModelLines(draft).split("\n").filter(Boolean)
      .map((line) => codexPlusProviderEditingApi.parseModelSuffix(line));
    const rowHtml = (row) => {
      const slug = row.slug || "";
      return codexPlusProviderModelRowHtml(slug, row.window ? String(row.window) : "", Boolean(slug) && slug === draft.model);
    };
    return `<div class="codex-plus-provider-line" data-stack="true"><span class="codex-plus-provider-line-copy"><span class="codex-plus-provider-line-title">模型列表</span><span class="codex-plus-provider-line-help">星标是默认模型；窗口选「默认」表示沿用 Codex 默认长度。Agent 请求这些模型名时会路由到这个供应商。</span></span><div class="codex-plus-provider-line-control"><div class="codex-plus-provider-model-list" data-codex-provider-model-list>${rows.length ? rows.map(rowHtml).join("") : '<div class="codex-plus-provider-help">还没有模型。</div>'}<button type="button" class="codex-plus-action-button codex-plus-provider-model-add" data-codex-provider-action="add-model">+ 添加模型</button></div></div></div>`;
  }

  function renderCodexPlusProviderDetail() {
    const state = codexPlusProviderPageState;
    const draft = state.draft;
    if (!draft) return '<div class="codex-plus-provider-empty">选择左侧的供应商，或新建一个。</div>';
    const isActive = !state.isNew && draft.id === state.settings.activeRelayId;
    const busy = Boolean(state.busy);
    const editable = codexPlusProviderEditable(draft);
    const canEdit = codexPlusProviderCanEdit(draft);
    const header = `<div class="codex-plus-provider-detail-header">
      <div><div class="codex-plus-provider-title">${escapeHtml(draft.name || "新供应商")}</div>
        <div class="codex-plus-provider-subtitle">${escapeHtml(codexPlusProviderModeLabel(draft))}${isActive ? " · 使用中" : ""}<span data-codex-provider-dirty ${state.dirty ? "" : "hidden"}> · 有未保存的修改</span></div></div>
      <div class="codex-plus-provider-actions">
        ${isActive || state.isNew ? "" : `<button type="button" class="codex-plus-action-button" data-codex-provider-action="use" ${busy ? "disabled" : ""}>${state.busy === "switch" ? "切换中…" : "使用"}</button>`}
        ${canEdit ? `<button type="button" class="codex-plus-action-button" data-codex-provider-action="save" data-primary="true" ${busy || !state.dirty ? "disabled" : ""}>${state.busy === "save" ? "保存中…" : isActive ? "保存并应用" : "保存"}</button>` : ""}
        ${editable ? `<button type="button" class="codex-plus-action-button" data-codex-provider-action="test" ${busy ? "disabled" : ""}>${state.busy === "test" ? "测试中…" : "测试"}</button>` : ""}
        ${state.isNew ? `<button type="button" class="codex-plus-action-button" data-codex-provider-action="cancel-new">取消</button>` : `
          ${canEdit ? `<button type="button" class="codex-plus-action-button" data-codex-provider-action="duplicate" ${busy ? "disabled" : ""}>复制</button>` : ""}`}
      </div></div>`;
    const statusBanner = state.status
      ? `<div class="codex-plus-provider-banner" data-tone="${escapeHtml(state.statusTone || "info")}" role="${state.statusTone === "error" ? "alert" : "status"}">${escapeHtml(state.status)}</div>`
      : "";
    const dangerCard = draft.gatewayManaged || state.isNew ? "" : codexPlusProviderCard("危险操作", codexPlusProviderActionLine("删除这个供应商",
      isActive ? "当前使用的供应商不能删除，先切换到别的供应商。" : "它的 Key 和配置快照会一起删掉。",
      `<button type="button" class="codex-plus-action-button" data-codex-provider-action="delete" data-danger="true" ${busy || isActive ? "disabled" : ""}>删除</button>`));
    if (!canEdit) {
      return `${header}${statusBanner}${codexPlusProviderCard("编辑", codexPlusProviderActionLine("由网关维护",
        "这是 Codex++ 网关接管 Codex 时生成的条目，由网关维护，这里只能切换。",
        '<button type="button" class="codex-plus-action-button" data-codex-provider-action="open-manager">在 Codex++ 中打开</button>'))}`;
    }
    if (!editable) return `${header}${statusBanner}${renderCodexPlusProviderAggregateDetail(draft)}${dangerCard}`;
    const mode = codexPlusProviderModeKey(draft);
    const showApi = mode !== "official";
    const keys = Array.isArray(draft.apiKeys) ? draft.apiKeys : [];
    const keyRows = keys.map((entry) => {
      const selected = entry.id === draft.activeApiKeyId;
      const preview = entry.apiKey ? codexPlusProviderPreviewKey(entry.apiKey) : (entry.preview || "（空）");
      return `<div class="codex-plus-provider-key" data-selected="${selected}">
        <label><input type="radio" name="codex-provider-key" data-codex-provider-key-select="${escapeHtml(entry.id)}" ${selected ? "checked" : ""}> ${escapeHtml(entry.name || entry.id)}</label>
        <code>${escapeHtml(preview)}</code>${entry.apiKey ? '<span class="codex-plus-provider-badge" data-tone="new">新</span>' : ""}
        <button type="button" data-codex-provider-key-remove="${escapeHtml(entry.id)}" aria-label="删除这把 Key">移除</button>
      </div>`;
    }).join("");
    // 获取结果直接作为「从上游获取模型」那一行的说明文字（行内 span，不再套一层带内边距的 help 块）。
    const fetched = state.fetchedModels.length
      ? `上游有 ${state.fetchedModels.length} 个模型：${escapeHtml(state.fetchedModels.slice(0, 12).join("、"))}${state.fetchedModels.length > 12 ? "…" : ""} <button type="button" class="codex-plus-provider-link" data-codex-provider-action="fill-models">全部填入</button>`
      : "";
    return `${header}
      ${statusBanner}
      ${codexPlusProviderCard("基本", [
        codexPlusProviderLine("名称", codexPlusProviderInput("name", draft.name)),
        codexPlusProviderActionLine("模式", "", codexPlusProviderMenu("mode", mode, [["official", "官方登录"], ["mixedApi", "官方登录 + API"], ["pureApi", "纯 API"]], "模式")),
        showApi ? codexPlusProviderActionLine("协议", "", codexPlusProviderMenu("protocol", draft.protocol, codexPlusProviderProtocolOptions, "协议")) : "",
        showApi ? codexPlusProviderLine("接口地址", codexPlusProviderInput("baseUrl", draft.baseUrl, 'placeholder="https://example.com/v1" spellcheck="false"')) : "",
      ].join(""))}
      ${showApi ? codexPlusProviderCard("API Key", `${keyRows || '<div class="codex-plus-provider-help">还没有 Key。</div>'}
        <div class="codex-plus-provider-key-add">
          <input type="text" class="codex-plus-provider-input" data-codex-provider-new-key-name="true" placeholder="名称（可选）">
          <input type="password" class="codex-plus-provider-input" data-codex-provider-new-key="true" placeholder="sk-..." autocomplete="off" spellcheck="false">
          <button type="button" class="codex-plus-action-button" data-codex-provider-action="add-key">添加</button>
        </div>`, "已保存的 Key 只显示首尾几位；选中哪一把，保存后 Codex 就用哪一把。") : ""}
      ${codexPlusProviderCard("模型", [
        codexPlusProviderLine("默认模型", codexPlusProviderInput("model", draft.model, 'spellcheck="false"')),
        codexPlusProviderModelEditor(draft),
        showApi ? codexPlusProviderActionLine("从上游获取模型", fetched || "读取供应商的 /models，可一键填入上面的列表。",
          `<button type="button" class="codex-plus-action-button" data-codex-provider-action="models" ${busy ? "disabled" : ""}>${state.busy === "models" ? "获取中…" : "获取"}</button>`, true) : "",
        codexPlusProviderLine("上下文窗口", codexPlusProviderInput("contextWindow", draft.contextWindow, 'inputmode="numeric" placeholder="留空用模型默认值"')),
        codexPlusProviderLine("自动压缩阈值", codexPlusProviderInput("autoCompactLimit", draft.autoCompactLimit, 'inputmode="numeric" placeholder="留空用默认值"')),
      ].join(""))}
      <details class="codex-plus-provider-group"><summary class="codex-plus-provider-section-title">配置文件</summary>
        <div class="codex-plus-provider-card">
          <div class="codex-plus-provider-help">直接编辑这个供应商的 config.toml / auth.json 快照；上面的字段会同步回读。</div>
          ${codexPlusProviderLine("config.toml", `<textarea class="codex-plus-provider-input codex-plus-provider-code" rows="10" data-codex-provider-file="config" spellcheck="false">${escapeHtml(draft.configContents || "")}</textarea>`, "", true)}
          ${codexPlusProviderLine("auth.json", `<textarea class="codex-plus-provider-input codex-plus-provider-code" rows="4" data-codex-provider-file="auth" spellcheck="false">${escapeHtml(draft.authContents || "")}</textarea>`, "", true)}
        </div>
      </details>
      ${codexPlusProviderCard("更多设置", codexPlusProviderActionLine("模型路由、请求头、兼容选项", "这些高级项在 Codex++ 管理器里编辑。",
        '<button type="button" class="codex-plus-action-button" data-codex-provider-action="open-manager">在 Codex++ 中打开</button>'))}
      ${dangerCard}`;
  }

  function codexPlusProviderToggleHtml(on, label, attrs) {
    return `<button type="button" class="codex-plus-toggle" role="switch" aria-label="${escapeHtml(label)}" aria-checked="${String(on)}" data-enabled="${String(on)}" ${attrs}><span></span></button>`;
  }

  /** 成员的模型名：模型列表 + 默认模型去重（与管理器的成员卡片同一口径）。 */
  function codexPlusProviderMemberModels(profile) {
    const names = String(profile.modelList || "").split(/[\r\n,]+/).map((line) => line.trim()).filter(Boolean);
    const model = String(profile.model || "").trim();
    return [...new Set(model ? [...names, model] : names)];
  }

  function codexPlusProviderMemberBrief(profile) {
    if (codexPlusProviderEditingApi.isOfficialLoginMemberProfile(profile)) return "使用已登录的 ChatGPT 账号凭据";
    const host = String(profile.baseUrl || "").replace(/^https?:\/\//, "").replace(/\/.*$/, "");
    return [profile.model, host].filter(Boolean).join(" · ") || "未填写地址";
  }

  /**
   * 聚合供应商的编辑区：照管理器 AggregateRelayProfileEditor 的字段与文案，排成本页的分组卡片。
   * 类型（故障转移 / 模型聚合）创建时选定，之后不能改，与管理器一致。
   */
  function renderCodexPlusProviderAggregateDetail(draft) {
    const state = codexPlusProviderPageState;
    const api = codexPlusProviderEditingApi;
    const candidates = codexPlusProviderAggregateCandidates(draft);
    const aggregate = codexPlusProviderAggregateConfig(draft);
    const modelMode = aggregate.modelAggregationEnabled === true;
    const memberIds = new Set(aggregate.members.map((member) => member.profileId));
    const nameOf = (id) => (state.settings.relayProfiles.find((profile) => profile.id === id)?.name || "未命名供应商");
    const memberOptions = aggregate.members.map((member) => [member.profileId, nameOf(member.profileId)]);
    const sessionProvider = api.normalizeRelaySessionProvider(draft.sessionProvider);
    const typeControl = state.isNew
      ? `<div class="codex-plus-provider-segmented" role="radiogroup" aria-label="类型">${[["provider", "故障转移"], ["model", "模型聚合"]]
        .map(([value, text]) => `<button type="button" role="radio" aria-checked="${String((value === "model") === modelMode)}" data-codex-provider-agg-mode="${value}">${text}</button>`).join("")}</div>`
      : `<span class="codex-plus-provider-badge">${modelMode ? "模型聚合" : "故障转移"}</span>`;
    const basic = codexPlusProviderCard("基本", [
      codexPlusProviderLine("名称", codexPlusProviderInput("name", draft.name, 'placeholder="例如 主力聚合池"')),
      codexPlusProviderActionLine("类型", modelMode
        ? "把成员模型合并到 Codex 列表，选择模型后固定发送到对应供应商。"
        : "多个供应商组成请求池，支持失败切换、会话轮转、请求轮转和权重。", typeControl),
      modelMode ? "" : codexPlusProviderActionLine("聚合策略", api.aggregateStrategyHelp(aggregate.strategy),
        codexPlusProviderMenu("agg-strategy", aggregate.strategy, api.aggregateStrategyOptions.map((option) => [option.value, option.label]), "聚合策略")),
      codexPlusProviderActionLine("Codex 会话身份", modelMode
        ? "模型聚合请求仍由本地 Responses 代理转发；OpenAI 身份用于让 ChatGPT Remote 识别会话。"
        : "聚合请求仍由本地 Responses 代理轮转成员；OpenAI 身份用于让 ChatGPT Remote 识别会话。",
        codexPlusProviderMenu("agg-session", sessionProvider, [["custom", "Custom（默认）"], ["openai", "OpenAI（兼容 ChatGPT Remote）"]], "Codex 会话身份")),
    ].join(""));

    const selected = candidates.filter((candidate) => memberIds.has(candidate.id));
    const defaultCandidate = selected.find((candidate) => candidate.id === aggregate.defaultRelayId);
    const bareCount = defaultCandidate ? codexPlusProviderMemberModels(defaultCandidate).length : 0;
    const prefixedCount = selected.filter((candidate) => candidate.id !== defaultCandidate?.id)
      .reduce((sum, candidate) => sum + codexPlusProviderMemberModels(candidate).length, 0);
    const summary = !modelMode || !selected.length ? ""
      : defaultCandidate && bareCount === 0 && api.isOfficialLoginMemberProfile(defaultCandidate)
        ? `Codex 将看到 ${defaultCandidate.name || "未命名供应商"} 的官方模型目录（裸名）+ ${prefixedCount} 个 cpx- 前缀模型。`
        : defaultCandidate
          ? `Codex 将看到 ${bareCount} 个裸名模型（来自 ${defaultCandidate.name || "未命名供应商"}）+ ${prefixedCount} 个 cpx- 前缀模型。`
          : `Codex 将看到 ${prefixedCount} 个 cpx- 前缀模型；还没指定默认成员，不带前缀的请求会报错。`;
    const memberRows = candidates.map((candidate) => {
      const checked = memberIds.has(candidate.id);
      const member = aggregate.members.find((item) => item.profileId === candidate.id);
      const name = candidate.name || "未命名供应商";
      let side;
      if (modelMode) {
        const models = codexPlusProviderMemberModels(candidate);
        const count = models.length ? `${models.length} 个模型`
          : api.isOfficialLoginMemberProfile(candidate) ? "未填写：用 Codex 的官方模型目录" : "未填写模型";
        const isDefault = checked && candidate.id === aggregate.defaultRelayId;
        side = `<span class="codex-plus-provider-agg-label" title="${escapeHtml(models.join("\n"))}">${escapeHtml(count)}</span>${!checked ? ""
          : isDefault ? '<span class="codex-plus-provider-badge" data-tone="active">默认</span>'
            : `<button type="button" class="codex-plus-provider-link" data-codex-provider-agg-default="${escapeHtml(candidate.id)}" title="不带 cpx- 前缀的请求（旧会话、自动压缩带的模型名）会原样交给它">设为默认</button>`}`;
      } else {
        side = `<span class="codex-plus-provider-agg-label">权重</span><input type="text" inputmode="numeric" class="codex-plus-provider-input codex-plus-provider-agg-number" data-codex-provider-agg-weight="${escapeHtml(candidate.id)}" value="${escapeHtml(String(member?.weight ?? 1))}" aria-label="「${escapeHtml(name)}」的权重" ${checked ? "" : "disabled"}>`;
      }
      const protocol = candidate.protocol && candidate.protocol !== "responses"
        ? `<span class="codex-plus-provider-badge">${escapeHtml((codexPlusProviderProtocolOptions.find(([id]) => id === candidate.protocol) || [, candidate.protocol])[1])}</span>` : "";
      return `<div class="codex-plus-provider-line codex-plus-provider-agg-member" data-kind="action" data-selected="${checked}">
        <span class="codex-plus-provider-line-copy"><span class="codex-plus-provider-line-title">${escapeHtml(name)}<span class="codex-plus-provider-badge">${escapeHtml(codexPlusProviderModeLabel(candidate))}</span>${protocol}</span>
          <span class="codex-plus-provider-line-help">${escapeHtml(codexPlusProviderMemberBrief(candidate))}</span></span>
        <span class="codex-plus-provider-line-control">${side}${codexPlusProviderToggleHtml(checked, `把「${name}」设为成员`, `data-codex-provider-agg-member="${escapeHtml(candidate.id)}"`)}</span>
      </div>`;
    }).join("");
    const members = codexPlusProviderCard(`成员供应商（${aggregate.members.length} / ${candidates.length}）`,
      memberRows || '<div class="codex-plus-provider-help">先添加至少 1 个可用的 API 供应商或官方登录账号，再把它加为成员。</div>',
      [modelMode
        ? "默认成员的模型以裸名发布，其他成员带 cpx- 前缀。本模式不使用轮转策略或通配符路由。"
        : "可选 API 供应商或已完成 ChatGPT 登录的官方账号；聚合供应商不会作为成员。", summary].filter(Boolean).join(" "));

    let defaults = "";
    if (modelMode) {
      const defaultIsOfficialLogin = !!defaultCandidate && defaultCandidate.relayMode === "official" && !defaultCandidate.officialMixApiKey;
      defaults = codexPlusProviderCard("默认供应商", [
        codexPlusProviderActionLine(defaultCandidate ? `默认：${defaultCandidate.name || defaultCandidate.id}` : "还没指定默认供应商",
          defaultCandidate
            ? "不带 cpx- 前缀的请求（旧会话、自动压缩带的模型名等）会原样交给它；带 cpx- 前缀但解析不出来的 ID 一律报错，不回落。"
            : "在上方成员行点「设为默认」。不带 cpx- 前缀的请求会原样交给它，没设就报错。", ""),
        defaultIsOfficialLogin
          ? codexPlusProviderActionLine("把默认成员的官方登录同步到 Codex",
            "开启后切换到本聚合时，会把默认成员的 ChatGPT 凭据一并写进 Codex（代理 Key 保留），账号在 Codex 里就是登录态。",
            codexPlusProviderToggleHtml(aggregate.syncDefaultMemberLogin === true, "把默认成员的官方登录同步到 Codex", "data-codex-provider-agg-sync"))
          : codexPlusProviderActionLine("同步官方登录", "当前默认成员不是官方登录账号，无法把登录同步到 Codex；只有「官方登录」类型的成员才有 ChatGPT 凭据。", ""),
      ].join(""));
    }

    const review = memberOptions.length ? codexPlusProviderCard("审批", codexPlusProviderActionLine("审批成员",
      "Codex「替我审批」的请求（codex-auto-review）只发给这个成员，模型取它的「自动审批模型」，没填用它的默认模型。默认成员是官方登录时，指定一个 API 成员可避免审批消耗官方额度。",
      codexPlusProviderMenu("agg-review", aggregate.autoReviewRelayId || "", [["", "不单独指定"], ...memberOptions], "审批成员"))) : "";

    let routes = "";
    if (!modelMode) {
      const routeRows = (aggregate.routes || []).map((route, index) => {
        const isMember = memberIds.has(route.profileId);
        const options = isMember ? memberOptions : [[route.profileId, `${nameOf(route.profileId)}（不是成员）`], ...memberOptions];
        return `<div class="codex-plus-provider-agg-route">
          <input type="text" class="codex-plus-provider-input" data-codex-provider-agg-route-pattern="${index}" value="${escapeHtml(route.pattern)}" placeholder="例如 deepseek-*" spellcheck="false" aria-label="模型匹配">
          ${codexPlusProviderMenu("agg-route-target", route.profileId, options, "目标成员", `data-agg-route-index="${index}"`)}
          <span class="codex-plus-provider-agg-label">优先级</span>
          <input type="text" inputmode="numeric" class="codex-plus-provider-input codex-plus-provider-agg-number" data-codex-provider-agg-route-priority="${index}" value="${escapeHtml(String(route.priority))}" aria-label="优先级">
          <button type="button" class="codex-plus-provider-model-remove" data-codex-provider-agg-route-remove="${index}" aria-label="删除规则">移除</button>
          ${isMember ? "" : '<span class="codex-plus-provider-agg-route-error">路由目标必须是已勾选的聚合成员，请先在成员供应商中勾选。</span>'}
        </div>`;
      }).join("");
      routes = codexPlusProviderCard(`路由规则（${(aggregate.routes || []).length}）`,
        `${routeRows || '<div class="codex-plus-provider-help">暂无路由规则，未匹配的模型会按聚合策略选择成员。</div>'}
        <div class="codex-plus-provider-agg-route-add"><button type="button" class="codex-plus-action-button" data-codex-provider-action="agg-route-add" ${aggregate.members.length ? "" : "disabled"}>+ 添加规则</button></div>`,
        "按模型名自动路由到指定成员；仅支持 * 通配符，数字越大越优先，chat/completions 协议不走路由。");
    }
    return `${basic}${members}${defaults}${review}${routes}`;
  }

  function codexPlusProviderPreviewKey(key) {
    const text = String(key || "").trim();
    return text.length <= 8 ? "••••" : `${text.slice(0, 3)}…${text.slice(-4)}`;
  }

  function codexPlusProviderSyncFooter() {
    const footer = document.querySelector("[data-codex-provider-footer]");
    if (!footer) return;
    const state = codexPlusProviderPageState;
    const isGateway = state.tab === "gateway";
    const msg = isGateway ? codexPlusGatewayPageState.status : state.status;
    const tone = isGateway ? codexPlusGatewayPageState.statusTone : state.statusTone;
    footer.textContent = msg || "";
    footer.dataset.tone = tone || "";
  }

  function renderCodexPlusProviderPage() {
    const panel = document.querySelector("[data-codex-provider-panel]");
    if (!panel) return;
    const state = codexPlusProviderPageState;
    if (!state.settings) {
      panel.innerHTML = `<div class="codex-plus-provider-empty">${state.busy === "load" ? "正在读取供应商…" : escapeHtml(state.status || "读取供应商失败")} <button type="button" class="codex-plus-provider-link" data-codex-provider-action="reload">重试</button></div>`;
      return;
    }
    const scroll = panel.querySelector(".codex-plus-provider-detail")?.scrollTop || 0;
    const gatewayMode = state.data?.gatewayMode === true;
    const tabs = `<div class="codex-plus-provider-tabs" role="tablist" aria-label="供应商类型">
      <div class="codex-plus-provider-tab-group">
        <button type="button" role="tab" data-codex-provider-tab="gateway" aria-selected="${state.tab === "gateway"}">网关供应商池${gatewayMode ? '<span class="codex-plus-provider-badge" data-tone="active">使用中</span>' : ""}</button>
        <button type="button" role="tab" data-codex-provider-tab="direct" aria-selected="${state.tab !== "gateway"}">直连供应商${gatewayMode ? "" : '<span class="codex-plus-provider-badge" data-tone="active">使用中</span>'}</button>
      </div>
    </div>`;
    const footer = `<div class="codex-plus-provider-footer" data-codex-provider-footer></div>`;
    if (state.tab === "gateway") {
      panel.innerHTML = `<div class="codex-plus-provider-shell">${tabs}${renderCodexPlusGatewayPane()}${footer}</div>${renderCodexPlusGatewayDisconnectDialog()}`;
      const gatewayDetail = panel.querySelector(".codex-plus-provider-detail");
      if (gatewayDetail) gatewayDetail.scrollTop = scroll;
      codexPlusProviderSyncFooter();
      return;
    }
    panel.innerHTML = `<div class="codex-plus-provider-shell">${tabs}<div class="codex-plus-provider-layout">
      <aside class="codex-plus-provider-sidebar">
        <div class="codex-plus-provider-sidebar-header"><span>供应商</span>
          ${codexPlusProviderActionMenu("new", "+ 新建", [["direct", "直连供应商"], ["failover", "故障转移（聚合）"], ["model", "模型聚合"]], state.busy ? "disabled" : "")}</div>
        ${state.settings.relayProfilesEnabled ? "" : '<div class="codex-plus-provider-note">供应商配置总开关已关闭：可以编辑，但不会写入 Codex。</div>'}
        ${renderCodexPlusProviderList()}
      </aside>
      <section class="codex-plus-provider-detail">
        ${gatewayMode ? '<div class="codex-plus-provider-note">Codex 正经网关接入，直连供应商暂时不起作用；在这里点「使用」会先断开网关并还原。</div>' : ""}
        ${renderCodexPlusProviderDetail()}
      </section></div>${footer}</div>${renderCodexPlusGatewayDisconnectDialog()}`;
    const detail = panel.querySelector(".codex-plus-provider-detail");
    if (detail) detail.scrollTop = scroll;
    codexPlusProviderSyncFooter();
  }

  async function codexPlusProviderConfirmDiscard() {
    const state = codexPlusProviderPageState;
    if (!state.dirty) return true;
    const answer = await codexPlusAsk({ title: "放弃修改？", message: "当前供应商有未保存的修改。", options: [{ value: "discard", label: "放弃修改" }] });
    return Boolean(answer);
  }

  /** 模型行里的一行：星标 | 模型名 | 窗口下拉 | 移除。「添加模型」插入新行用，样子与 codexPlusProviderModelEditor 里的一致。 */
  function codexPlusProviderModelRowHtml(slug, win, isDefault) {
    const presets = ["", "128000", "200000", "256000", "400000", "1000000"];
    const values = presets.includes(win) ? presets : [...presets, win];
    return `<div class="codex-plus-provider-model-row" data-codex-provider-model-row>
        <button type="button" class="codex-plus-provider-star" data-active="${isDefault}" data-codex-provider-model-default="${escapeHtml(slug)}" title="${isDefault ? "默认模型" : "设为默认模型"}" aria-label="${isDefault ? "默认模型" : "设为默认模型"}">${isDefault ? "★" : "☆"}</button>
        <input type="text" class="codex-plus-provider-input" data-codex-provider-model-name value="${escapeHtml(slug)}" placeholder="模型名，例如 deepseek-v4-pro" spellcheck="false" aria-label="模型名">
        ${codexPlusProviderMenu("model-window", win, values.map((value) => [value, value ? codexPlusProviderWindowLabel(value) : "窗口：默认"]), "上下文窗口", "data-codex-provider-model-window")}
        <button type="button" class="codex-plus-provider-model-remove" data-codex-provider-model-remove aria-label="移除这个模型">移除</button>
      </div>`;
  }

  /** 从模型行编辑器读出当前所有行，拼成 `slug[窗口]` 文本（空名字的行丢掉）。 */
  function codexPlusProviderModelRowsText(list) {
    return Array.from(list.querySelectorAll("[data-codex-provider-model-row]")).map((row) => {
      const name = String(row.querySelector("[data-codex-provider-model-name]")?.value || "").trim();
      const win = String(row.querySelector("[data-codex-provider-model-window]")?.dataset.value || "");
      return name && win && !name.includes("[") ? `${name}[${win}]` : name;
    }).filter(Boolean).join("\n");
  }

  /** 模型行编辑器的输入 / 选择 / 点击；是它的事件就处理并返回 true。 */
  function codexPlusProviderHandleModelEditor(event) {
    const target = event.target instanceof Element ? event.target : event.target?.parentElement;
    const state = codexPlusProviderPageState;
    if (!target || !state.draft) return false;
    const list = target.closest("[data-codex-provider-model-list]");
    if (!list) return false;
    const row = target.closest("[data-codex-provider-model-row]");
    if (event.type === "input" && row && target.matches("[data-codex-provider-model-name]")) {
      const name = target.value.trim();
      const star = row.querySelector("[data-codex-provider-model-default]");
      const prev = star?.getAttribute("data-codex-provider-model-default") || "";
      codexPlusProviderApplyModelLines(codexPlusProviderModelRowsText(list));
      if (name) {
        // 改的是默认模型那一行：默认模型跟着改名，星标不丢。
        if (prev && prev === state.draft.model) {
          codexPlusProviderPatch({ model: name });
          const modelInput = document.querySelector('[data-codex-provider-field="model"]');
          if (modelInput && document.activeElement !== modelInput) modelInput.value = name;
        }
        star?.setAttribute("data-codex-provider-model-default", name);
      }
      return true;
    }
    if (event.type !== "click") return false;
    const star = target.closest("[data-codex-provider-model-default]");
    if (star) {
      const slug = star.getAttribute("data-codex-provider-model-default") || "";
      if (slug && slug !== state.draft.model) {
        codexPlusProviderPatch({ model: slug });
        renderCodexPlusProviderPage();
      }
      return true;
    }
    if (target.closest("[data-codex-provider-model-remove]")) {
      row?.remove();
      codexPlusProviderApplyModelLines(codexPlusProviderModelRowsText(list));
      renderCodexPlusProviderPage();
      return true;
    }
    if (target.closest('[data-codex-provider-action="add-model"]')) {
      // 新行先只存在于 DOM：名字还空着，状态里不记，填了名字才写回。
      list.querySelector(":scope > .codex-plus-provider-help")?.remove();
      list.querySelector('[data-codex-provider-action="add-model"]')?.insertAdjacentHTML("beforebegin", codexPlusProviderModelRowHtml("", "", false));
      const names = list.querySelectorAll("[data-codex-provider-model-name]");
      names[names.length - 1]?.focus();
      return true;
    }
    return false;
  }

  /** 聚合编辑区的点击（成员开关、设为默认、同步登录、类型、删除路由）；是它的事件就处理并返回 true。 */
  function codexPlusProviderHandleAggregateClick(target) {
    const state = codexPlusProviderPageState;
    if (!state.draft || !codexPlusProviderIsAggregate(state.draft)) return false;
    const member = target.closest("[data-codex-provider-agg-member]");
    const makeDefault = target.closest("[data-codex-provider-agg-default]");
    const mode = target.closest("[data-codex-provider-agg-mode]");
    const removeRoute = target.closest("[data-codex-provider-agg-route-remove]");
    if (member) codexPlusProviderToggleMember(member.dataset.codexProviderAggMember);
    else if (makeDefault) codexPlusProviderUpdateAggregate({ defaultRelayId: makeDefault.dataset.codexProviderAggDefault });
    else if (target.closest("[data-codex-provider-agg-sync]")) {
      codexPlusProviderUpdateAggregate({ syncDefaultMemberLogin: codexPlusProviderAggregateConfig(state.draft).syncDefaultMemberLogin !== true });
    } else if (mode) {
      const next = mode.dataset.codexProviderAggMode === "model" ? "model" : "provider";
      state.draft = codexPlusProviderEditingApi.aggregateProfileForMode(state.draft, state.settings, next);
      state.dirty = true;
    } else if (removeRoute) {
      const index = Number(removeRoute.dataset.codexProviderAggRouteRemove);
      const routes = codexPlusProviderAggregateConfig(state.draft).routes || [];
      codexPlusProviderUpdateAggregate({ routes: routes.filter((_, routeIndex) => routeIndex !== index) });
    } else {
      return false;
    }
    renderCodexPlusProviderPage();
    return true;
  }

  function codexPlusProviderHandleInput(event) {
    if (codexPlusGatewayHandleInput(event)) return;
    if (codexPlusProviderHandleModelEditor(event)) return;
    const target = event.target;
    const state = codexPlusProviderPageState;
    if (!state.draft || !(target instanceof HTMLElement)) return;
    // 聚合的权重 / 路由：就地写回草稿不重绘（重绘会丢焦点）；清空时先不写，等填了数字再写。
    const weightFor = target.dataset.codexProviderAggWeight;
    const routeIndex = target.dataset.codexProviderAggRoutePattern ?? target.dataset.codexProviderAggRoutePriority;
    if (weightFor !== undefined || routeIndex !== undefined) {
      const number = Number.parseInt(target.value, 10);
      if (target.dataset.codexProviderAggRoutePattern !== undefined) {
        codexPlusProviderUpdateRoute(Number(routeIndex), { pattern: target.value });
      } else if (Number.isFinite(number) && weightFor !== undefined) {
        const aggregate = codexPlusProviderAggregateConfig(state.draft);
        codexPlusProviderUpdateAggregate({
          members: aggregate.members.map((member) => (member.profileId === weightFor
            ? { ...member, weight: codexPlusProviderEditingApi.clampAggregateWeight(number) } : member)),
        });
      } else if (Number.isFinite(number)) {
        codexPlusProviderUpdateRoute(Number(routeIndex), { priority: codexPlusProviderEditingApi.clampAggregateRoutePriority(number) });
      }
      return;
    }
    const field = target.dataset.codexProviderField;
    if (field && field !== "protocol") {
      if (field === "name") {
        state.draft = { ...state.draft, name: target.value };
        state.dirty = true;
        codexPlusProviderSyncFileFields();
      } else {
        codexPlusProviderPatch({ [field]: target.value });
      }
      return;
    }
    if (target.dataset.codexProviderModels) return codexPlusProviderApplyModelLines(target.value);
    const file = target.dataset.codexProviderFile;
    if (file) {
      const key = file === "config" ? "configContents" : "authContents";
      state.draft = codexPlusProviderEditingApi.deriveRelayProfileFromFiles({ ...state.draft, [key]: target.value }, file);
      state.dirty = true;
      codexPlusProviderSyncFileFields();
    }
  }

  function codexPlusProviderHandleChange(event) {
    const target = event.target;
    if (!(target instanceof HTMLElement)) return;
    if (codexPlusProviderHandleModelEditor(event)) return;
    if (target.dataset.codexProviderKeySelect) codexPlusProviderSelectKey(target.dataset.codexProviderKeySelect);
  }

  async function codexPlusProviderHandleClick(event) {
    const target = event.target instanceof Element ? event.target : event.target?.parentElement;
    const state = codexPlusProviderPageState;
    if (!target) return;
    if (codexPlusProviderHandleMenuClick(target)) return;
    const tab = target.closest("[data-codex-provider-tab]");
    if (tab) {
      const next = tab.dataset.codexProviderTab === "gateway" ? "gateway" : "direct";
      if (next === state.tab) return;
      state.tab = next;
      renderCodexPlusProviderPage();
      if (next === "gateway" && !codexPlusGatewayPageState.payload && !codexPlusGatewayPageState.busy) void codexPlusGatewayLoad();
      return;
    }
    if (target.closest("[data-codex-gateway-select], [data-codex-gateway-move], [data-codex-gateway-action], [data-codex-gateway-key-replace], [data-codex-gateway-key-delete]")) {
      await codexPlusGatewayHandleClick(target);
      return;
    }
    const move = target.closest("[data-codex-provider-move]");
    if (move && !move.disabled) {
      event.stopPropagation();
      if (!(await codexPlusProviderConfirmDiscard())) return;
      return codexPlusProviderMove(move.dataset.id, Number(move.dataset.codexProviderMove));
    }
    const select = target.closest("[data-codex-provider-select]");
    if (select) {
      const id = select.dataset.codexProviderSelect;
      if (id === state.selectedId && !state.isNew) return;
      if (!(await codexPlusProviderConfirmDiscard())) return;
      codexPlusProviderSelect(id);
      codexPlusProviderSetStatus("");
      return renderCodexPlusProviderPage();
    }
    if (codexPlusProviderHandleModelEditor(event)) return;
    if (codexPlusProviderHandleAggregateClick(target)) return;
    const removeKey = target.closest("[data-codex-provider-key-remove]");
    if (removeKey) return codexPlusProviderRemoveKey(removeKey.dataset.codexProviderKeyRemove);
    const action = target.closest("[data-codex-provider-action]")?.dataset.codexProviderAction;
    if (!action || target.closest("button")?.disabled) return;
    const id = state.draft?.id;
    switch (action) {
      case "reload": return codexPlusProviderLoad();
      case "agg-route-add": {
        const aggregate = codexPlusProviderAggregateConfig(state.draft);
        codexPlusProviderUpdateAggregate({ routes: [...(aggregate.routes || []), { pattern: "", profileId: aggregate.members[0]?.profileId ?? "", priority: 0 }] });
        renderCodexPlusProviderPage();
        const patterns = document.querySelectorAll("[data-codex-provider-agg-route-pattern]");
        patterns[patterns.length - 1]?.focus();
        return;
      }
      case "cancel-new":
        codexPlusProviderSelect(state.settings.activeRelayId);
        return renderCodexPlusProviderPage();
      case "save": return codexPlusProviderSave();
      case "use": return codexPlusProviderUse(id);
      case "delete": return codexPlusProviderDelete(id);
      case "duplicate":
        if (!(await codexPlusProviderConfirmDiscard())) return;
        return codexPlusProviderDuplicate(id);
      case "test": return codexPlusProviderProbe("test");
      case "models": return codexPlusProviderProbe("models");
      case "fill-models":
        codexPlusProviderApplyModelLines([codexPlusProviderModelLines(state.draft), ...state.fetchedModels].join("\n"));
        state.fetchedModels = [];
        return renderCodexPlusProviderPage();
      case "add-key": {
        const panel = target.closest("[data-codex-provider-panel]");
        return codexPlusProviderAddKey(
          panel?.querySelector("[data-codex-provider-new-key-name]")?.value,
          panel?.querySelector("[data-codex-provider-new-key]")?.value,
        );
      }
      case "open-manager":
        void postJson("/manager/open", { page: "relay" });
        return;
      default:
        return;
    }
  }

  /** 别处（管理器、另一个窗口）改了供应商：没有未保存修改时静默刷新。 */
  function codexPlusProviderPoll() {
    const state = codexPlusProviderPageState;
    if (!document.querySelector("[data-codex-provider-panel]")) return;
    if (state.tab === "gateway") codexPlusGatewayPoll();
    if (state.dirty || state.busy || document.hidden) return;
    void postJson("/providers/list", {}).then((result) => {
      if (result?.status !== "ok" || result.revision === state.data?.revision) return;
      if (state.dirty || state.busy) return;
      codexPlusProviderApplyList(result);
      renderCodexPlusProviderPage();
    });
  }

  function openCodexPlusProviderPage() {
    closeCodexPlusPage();
    document.querySelectorAll('.codex-plus-modal-overlay, [data-codex-plus-dialog="true"]').forEach((node) => node.remove());
    const overlay = document.createElement("div");
    overlay.className = `${codexPlusPageClass} codex-plus-provider-page`;
    overlay.dataset.codexPlusPage = "true";
    markCodexPlusExtensionNode(overlay, "builtin-provider");
    applyCodexPlusTheme(overlay);
    applyCodexPlusZoom(overlay);
    overlay.innerHTML = `<div class="codex-plus-modal-content" role="region" aria-label="供应商" data-codex-plus-active-tab="${codexPlusProviderTab}">
      <div class="codex-plus-modal-body"><div class="codex-plus-panel codex-plus-provider-panel" data-codex-provider-panel="true"></div></div></div>`;
    overlay.addEventListener("input", codexPlusProviderHandleInput);
    overlay.addEventListener("change", codexPlusProviderHandleChange);
    overlay.addEventListener("click", (event) => { void codexPlusProviderHandleClick(event); });
    document.body.appendChild(overlay);
    positionCodexPlusPage(overlay);
    observeCodexPlusPageLayout(overlay);
    document.querySelectorAll('[data-codex-plus-ext-rail-active="true"]').forEach((node) => {
      node.removeAttribute("data-codex-plus-ext-rail-active");
      const button = node.querySelector("button") || node;
      button.dataset.active = "false";
      button.removeAttribute("data-selected");
      button.removeAttribute("aria-current");
    });
    setCodexPlusSidebarNavActive(true, "provider");
    window.removeEventListener("resize", window.__codexPlusPageResizeHandler);
    window.__codexPlusPageResizeHandler = () => positionCodexPlusPage(document.querySelector(`.${codexPlusPageClass}`));
    window.addEventListener("resize", window.__codexPlusPageResizeHandler);
    const state = codexPlusProviderPageState;
    clearInterval(state.pollTimer);
    state.pollTimer = setInterval(codexPlusProviderPoll, 5000);
    // 关页时由 closeCodexPlusPage 调用（92-extension-host.js）。
    window.__codexPlusExtensionPageCleanup = () => {
      codexPlusProviderCloseMenu();
      clearInterval(state.pollTimer);
      state.pollTimer = null;
    };
    state.dirty = false;
    state.isNew = false;
    state.tab = "";
    Object.assign(codexPlusGatewayPageState, { payload: null, isNew: false, dirty: false, busy: "", status: "", disconnect: null, fetchedModels: [] });
    codexPlusProviderSetStatus("");
    renderCodexPlusProviderPage();
    void codexPlusProviderLoad({ keepSelection: Boolean(state.selectedId) });
  }
