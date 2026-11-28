  /**
   * 「供应商」页的本地服务页签：在 Codex 里管理 Codex++ 本地服务的共享供应商，并在本地服务 / 直连之间切换。
   *
   * - 全部操作经 /providers/gateway 转给管理器自己的本地服务命令（core provider_page 做白名单与脱敏），
   *   所以生效方式与管理器完全一致：路由每次请求重读 gateway.json，模型变化由 refresh_agents 重写
   *   Codex 的本地服务目录、心跳指纹变化后页面自动重载 model/list；已打开的对话下一条请求就用新的池。
   * - 表单逻辑用与管理器共用的 gateway-model.ts（42 分片的 codexPlusProviderEditing.gateway）。
   * - Key 只有打码视图：编辑时 Key 留空表示不改，新增 / 替换 / 删除各走单独命令。
   * - 账号 / 订阅类、Codex 原生登录、拓展提供的条目只读（管理器同样不给编辑），去 Codex++ 里管。
   */
  const codexPlusGatewayModel = codexPlusProviderEditing.gateway;
  const codexPlusGatewaySafety = codexPlusProviderEditing.gatewaySafety;
  const codexPlusGatewayPageState = {
    payload: null,
    selectedId: "",
    draft: null,
    isNew: false,
    dirty: false,
    busy: "",
    status: "",
    statusTone: "",
    fetchedModels: [],
    disconnect: null,
    signature: "",
    // 聚合列表是否也列别的 Agent（Claude）的供应商；和管理器一样默认不列。
    showOthers: false,
    // 「Codex 登录身份」：get_native_auth_state 的结果。
    login: null,
    loginBusy: false,
  };

  function codexPlusGatewayCall(command, args = {}) {
    return postJson("/providers/gateway", { command, args });
  }

  function codexPlusGatewayEntries() {
    return codexPlusGatewayPageState.payload?.pool || [];
  }

  /** 聚合列表要列的：不勾「显示其他 Agent 的供应商」时去掉别的 Agent 的（和管理器 entriesForPage 一致）。 */
  function codexPlusGatewayVisibleEntries() {
    const others = codexPlusGatewayPageState.showOthers;
    return codexPlusGatewayEntries().filter((entry) => others || !entry.owner || entry.owner === "codex");
  }

  function codexPlusGatewaySelected() {
    return codexPlusGatewayEntries().find((entry) => entry.id === codexPlusGatewayPageState.selectedId) || null;
  }

  /** 与管理器一致：账号 / 订阅（ChatGPT 只能改额度策略）、官方登录、原生登录与拓展条目不在页面里编辑。 */
  function codexPlusGatewayEditable(entry) {
    if (!entry) return false;
    const source = String(entry.source || "");
    return !entry.account && !entry.official && !source.startsWith("native:") && !source.startsWith("extension:");
  }

  function codexPlusGatewaySetStatus(message, tone = "") {
    codexPlusGatewayPageState.status = String(message || "");
    codexPlusGatewayPageState.statusTone = tone;
  }

  function codexPlusGatewaySelect(id) {
    const state = codexPlusGatewayPageState;
    const entry = codexPlusGatewayEntries().find((item) => item.id === id) || null;
    state.selectedId = entry?.id || "";
    state.isNew = false;
    state.dirty = false;
    state.fetchedModels = [];
    state.draft = entry && codexPlusGatewayEditable(entry) ? codexPlusGatewayModel.providerDraftFromEntry(entry) : null;
  }

  /** 本地服务命令的返回（CommandResult 里平铺了 GatewayPayload）。没带 payload 的结果不覆盖现有列表。 */
  function codexPlusGatewayApplyResult(result, { keepSelection = true } = {}) {
    const state = codexPlusGatewayPageState;
    if (result?.gateway && Array.isArray(result.pool)) {
      state.payload = result;
      state.signature = codexPlusGatewaySignature(result);
      const ids = codexPlusGatewayVisibleEntries().map((entry) => entry.id);
      const keep = keepSelection && ids.includes(state.selectedId) ? state.selectedId : ids[0] || "";
      if (!state.isNew) codexPlusGatewaySelect(keep);
    }
  }

  function codexPlusGatewaySignature(payload) {
    return JSON.stringify([
      (payload?.pool || []).map((entry) => [entry.id, entry.name, entry.baseUrl, entry.protocol, entry.enabled,
        entry.models, entry.keys, entry.modelWindows, entry.contextWindow]),
      payload?.gateway?.bindings?.codex?.enabled,
      payload?.gateway?.bindings?.codex?.model,
      payload?.gateway?.bindings?.codex?.poolExclude,
    ]);
  }

  async function codexPlusGatewayLoad({ quiet = false } = {}) {
    const state = codexPlusGatewayPageState;
    if (!quiet) {
      state.busy = "load";
      renderCodexPlusProviderPage();
    }
    const [result] = await Promise.all([codexPlusGatewayCall("load_gateway"), codexPlusLoginIdentityLoad()]);
    state.busy = "";
    if (result?.status !== "ok" || !result.gateway) {
      codexPlusGatewaySetStatus(result?.message || "读取共享供应商失败", "error");
    } else {
      codexPlusGatewayApplyResult(result);
    }
    renderCodexPlusProviderPage();
  }

  /** 执行一条会改动池子的命令：成功后用返回的最新池子刷新，并提示生效方式。 */
  async function codexPlusGatewayMutate(command, args, { busy = "save", success = "" } = {}) {
    const state = codexPlusGatewayPageState;
    state.busy = busy;
    renderCodexPlusProviderPage();
    const result = await codexPlusGatewayCall(command, args);
    state.busy = "";
    const ok = result?.status === "ok" || result?.status === "warning";
    if (ok) {
      state.isNew = false;
      codexPlusGatewayApplyResult(result);
      codexPlusGatewaySetStatus(`${success || result.message || "已保存。"} 已打开的对话下一条请求就用新的共享供应商。`, "ok");
      showToast(success || "共享供应商已更新", { type: "success" });
    } else {
      codexPlusGatewaySetStatus(result?.message || "操作失败", "error");
    }
    renderCodexPlusProviderPage();
    return ok ? result : null;
  }

  function codexPlusGatewayStartNew() {
    const state = codexPlusGatewayPageState;
    state.draft = codexPlusGatewayModel.providerDraftFromPreset(null);
    state.selectedId = "";
    state.isNew = true;
    state.dirty = true;
    state.fetchedModels = [];
    codexPlusGatewaySetStatus("");
    renderCodexPlusProviderPage();
  }

  async function codexPlusGatewaySave() {
    const state = codexPlusGatewayPageState;
    const draft = state.draft;
    if (!draft) return;
    const issue = codexPlusGatewayModel.validateProviderDraft(draft, !state.isNew);
    if (issue) {
      codexPlusGatewaySetStatus(issue, "error");
      return renderCodexPlusProviderPage();
    }
    // providerDraftFromEntry 带着原价格、代理与开关：update_provider_in 会无条件写价格，必须原样回传。
    const input = codexPlusGatewayModel.providerInput(draft);
    if (state.isNew) {
      const before = new Set(codexPlusGatewayEntries().map((entry) => entry.id));
      const result = await codexPlusGatewayMutate("create_gateway_provider", { input, source: "custom", skipAgents: null }, { success: `已添加「${input.name}」。` });
      const created = result?.pool?.find((entry) => !before.has(entry.id));
      if (created) {
        codexPlusGatewaySelect(created.id);
        renderCodexPlusProviderPage();
      }
      return;
    }
    await codexPlusGatewayMutate("update_gateway_provider", { id: state.selectedId, input }, { success: `已保存「${input.name}」。` });
  }

  async function codexPlusGatewayDelete(entry) {
    const answer = await codexPlusAsk({
      title: "从共享供应商删除供应商",
      message: `确定从共享供应商删除「${entry.name}」？它的 Key、模型和用量设置一起删除，模型组和各 Agent 里对它的引用也会清掉。`,
      options: [{ value: "delete", label: "删除" }],
    });
    if (!answer) return;
    await codexPlusGatewayMutate("delete_gateway_provider", { id: entry.id }, { busy: "delete", success: `已删除「${entry.name}」。` });
  }

  async function codexPlusGatewayReplaceKey(entry, keyId) {
    const answer = await codexPlusAsk({
      title: "替换 Key",
      message: `为「${entry.name}」填入新的 Key，替换掉打码显示的这一把。`,
      input: { placeholder: "sk-..." },
      options: [{ value: "replace", label: "替换" }],
    });
    const key = String(answer?.text || "").trim();
    if (!key) return;
    await codexPlusGatewayMutate("replace_gateway_provider_key", { id: entry.id, keyId, key }, { success: "Key 已替换。" });
  }

  async function codexPlusGatewayDeleteKey(entry, keyId) {
    const answer = await codexPlusAsk({ title: "删除 Key", message: `从「${entry.name}」删除这把 Key？`, options: [{ value: "delete", label: "删除" }] });
    if (!answer) return;
    await codexPlusGatewayMutate("delete_gateway_provider_key", { id: entry.id, keyId }, { success: "Key 已删除。" });
  }

  async function codexPlusGatewayProbe(kind) {
    const state = codexPlusGatewayPageState;
    const entry = codexPlusGatewaySelected();
    state.busy = kind;
    renderCodexPlusProviderPage();
    let result;
    if (kind === "test") {
      const model = String(document.querySelector("[data-codex-gateway-test-model]")?.dataset.value || "").trim();
      result = await codexPlusGatewayCall("test_gateway_provider", { id: entry?.id, model: model || null, keyId: null });
    } else {
      const draft = state.draft;
      const firstKey = String(draft?.keys || "").split(/\n/).map((line) => line.trim()).find(Boolean) || "";
      result = await codexPlusGatewayCall("fetch_provider_models", {
        baseUrl: String(draft?.baseUrl || "").trim(),
        protocol: draft?.protocol || "chatCompletions",
        apiKey: firstKey,
        providerId: state.isNew ? null : entry?.id || null,
      });
      if (result?.status === "ok") state.fetchedModels = Array.isArray(result.models) ? result.models : [];
    }
    state.busy = "";
    codexPlusGatewaySetStatus(result?.message || "请求失败", result?.status === "ok" ? "ok" : "error");
    renderCodexPlusProviderPage();
  }

  /** 让 Codex 经共享供应商接入：与管理器「连接与供应商」的「共享」分页同一条命令，沿用上次的接入设置；
   * 传了 `model`（`<供应商 id>/<模型>`）就把默认模型换成它。 */
  async function codexPlusGatewayConnect(model = "") {
    const state = codexPlusGatewayPageState;
    const payload = state.payload;
    const previous = payload?.gateway?.bindings?.codex;
    const draft = { ...codexPlusGatewayModel.draftFromBinding(previous), ...(model ? { model } : {}) };
    const issue = codexPlusGatewayModel.validateDraft(draft, payload);
    if (issue) {
      codexPlusGatewaySetStatus(`${issue}（请先在共享供应商里添加可用的供应商，或在 Codex++「Agent 接入」里调整默认模型）`, "error");
      return renderCodexPlusProviderPage();
    }
    const binding = { ...codexPlusGatewayModel.bindingFromDraft(draft, previous), enabled: true };
    const result = await codexPlusGatewayMutate("save_gateway_binding", { agent: "codex", binding, reconnect: true }, { busy: "connect", success: "Codex 已经经共享供应商接入。" });
    if (result) await codexPlusProviderLoad({ quiet: true });
  }

  /** 断开并还原：先拿预览（token 只在管理器里，5 分钟有效），把要改回的文件列给用户确认。 */
  async function codexPlusGatewayOpenDisconnect(then = null) {
    const state = codexPlusGatewayPageState;
    state.disconnect = { loading: true, preview: null, error: "", then };
    renderCodexPlusProviderPage();
    const result = await codexPlusGatewayCall("preview_gateway_disconnect", { agent: "codex" });
    if (!state.disconnect) return;
    if (result?.status !== "ok" || !result.preview || result.preview.agent !== "codex") {
      state.disconnect = { loading: false, preview: null, error: result?.message || "无法获取还原预览", then };
    } else {
      state.disconnect = { loading: false, preview: result.preview, error: "", then };
    }
    renderCodexPlusProviderPage();
  }

  async function codexPlusGatewayApplyDisconnect() {
    const state = codexPlusGatewayPageState;
    const preview = state.disconnect?.preview;
    if (!preview) return;
    const fullSwitch = codexPlusGatewaySafety.fullSwitchReady(preview, "codex");
    if (!codexPlusGatewaySafety.disconnectPreviewReady(preview, "codex") && !fullSwitch) return;
    const then = state.disconnect.then;
    state.disconnect = { ...state.disconnect, applying: true };
    renderCodexPlusProviderPage();
    const command = fullSwitch ? "disable_gateway_binding_full_switch" : "disable_gateway_binding";
    const result = await codexPlusGatewayMutate(command, { agent: "codex", previewToken: preview.previewToken }, { busy: "disconnect", success: "已断开本地服务，Codex 改回直连。" });
    state.disconnect = null;
    await codexPlusProviderLoad({ quiet: true });
    if (result && typeof then === "function") await then();
    renderCodexPlusProviderPage();
  }

  function renderCodexPlusGatewayDisconnectDialog() {
    const dialog = codexPlusGatewayPageState.disconnect;
    if (!dialog) return "";
    const preview = dialog.preview;
    const fullSwitch = preview && codexPlusGatewaySafety.fullSwitchReady(preview, "codex") ? preview.fullSwitch : null;
    const ready = preview && (codexPlusGatewaySafety.disconnectPreviewReady(preview, "codex") || fullSwitch);
    const files = (preview?.files || []).map((file) => `<section class="codex-plus-gateway-diff">
        <div class="codex-plus-gateway-diff-head"><code>${escapeHtml(file.path)}</code><span>${escapeHtml(codexPlusGatewaySafety.disconnectFileLabel(file.kind))}</span></div>
        <pre>${file.diff ? file.diff.split("\n").map((line) => `<span data-kind="${line.startsWith("+") && !line.startsWith("+++") ? "add" : line.startsWith("-") && !line.startsWith("---") ? "remove" : "context"}">${escapeHtml(line)}\n</span>`).join("") : "文件内容无需修改"}</pre>
      </section>`).join("");
    return `<div class="codex-plus-gateway-modal" data-codex-plus-ext="builtin-provider" role="dialog" aria-modal="true" aria-label="断开本地服务并还原">
      <div class="codex-plus-gateway-modal-box">
        <div class="codex-plus-provider-title">断开本地服务并还原为直连</div>
        <div class="codex-plus-provider-help">Codex 会恢复接入本地服务之前的设置，下面是要改回的内容。</div>
        ${dialog.loading ? '<div class="codex-plus-provider-note">正在检查文件与还原差异…</div>' : ""}
        ${dialog.error ? `<div class="codex-plus-provider-status" data-tone="error">${escapeHtml(dialog.error)}</div>` : ""}
        ${preview?.warnings?.length ? `<ul class="codex-plus-gateway-warnings">${preview.warnings.map((warning) => `<li>${escapeHtml(warning)}</li>`).join("")}</ul>` : ""}
        <div class="codex-plus-gateway-diffs">${files}${preview?.canDisconnect && !preview.files.length ? '<div class="codex-plus-provider-help">没有需要改写的配置文件。</div>' : ""}</div>
        ${fullSwitch ? `<div class="codex-plus-provider-note">确认后 Codex 会完整切换到直连供应商「${escapeHtml(fullSwitch.name)}」：先备份当前配置、收回借给 Codex 的登录，再写入。</div>`
          : preview && !preview.canDisconnect ? '<div class="codex-plus-provider-status" data-tone="error">当前无法安全还原，请先处理上面的文件问题。</div>' : ""}
        <div class="codex-plus-provider-actions">
          <button type="button" class="codex-plus-action-button" data-codex-gateway-action="disconnect-cancel" ${dialog.applying ? "disabled" : ""}>取消</button>
          <button type="button" class="codex-plus-action-button" data-primary="true" data-codex-gateway-action="disconnect-apply" ${ready && !dialog.applying ? "" : "disabled"}>${dialog.applying ? "正在还原…" : fullSwitch ? "完整切换并断开" : "断开并还原"}</button>
        </div>
      </div></div>`;
  }

  /**
   * 供应商列表页标题行的连接方式（和管理器同一个「聚合 / 直连」切换）。aggregate 是列表看的模式，gatewayMode 是 Codex 实际的连接方式。
   * 和管理器一样：点「直连」只把列表换成直连视图，点某家「使用」才真的切过去（自动断开本地服务）；
   * 点「聚合」时已经经本地服务就只换视图，没接入就接入。
   */
  function renderCodexPlusProviderModeSwitch(aggregate, gatewayMode = aggregate) {
    const busy = Boolean(codexPlusGatewayPageState.busy || codexPlusProviderPageState.busy);
    const canSwitch = codexPlusProviderPageState.settings?.relayProfilesEnabled !== false;
    const button = (mode, label, title, active, enabled) => `<button type="button" role="radio" aria-checked="${active}" aria-selected="${active}" data-codex-gateway-action="mode-${mode}" title="${escapeHtml(title)}" ${active || busy || !enabled ? "disabled" : ""}>${escapeHtml(label)}</button>`;
    return `<div class="codex-plus-provider-tabs codex-plus-provider-mode-switch">
      <div class="codex-plus-provider-tab-group" role="radiogroup" aria-label="Codex 的连接方式">
        ${button("aggregate", "聚合", canSwitch ? "聚合：Codex 经 Codex++ 本地服务，下面打开的供应商合在一起，模型都列进 Codex 的模型选择器，不用切换；需要 Codex++ 在后台运行" : "先在 Codex++「连接与供应商」的「更多设置」里打开供应商配置切换", aggregate, canSwitch)}
        ${button("direct", "直连", gatewayMode ? "查看直连供应商列表，点任一家「使用」即可直接切换过去" : "直连：Codex 直接连一家供应商，关掉 Codex++ 也能用（需协议转换的除外）；换供应商点「使用」", !aggregate, true)}
      </div>
    </div>`;
  }

  /** 当前连接方式的一句说明（供应商列表页的副标题）。 */
  function codexPlusProviderModeHint(gatewayMode) {
    const connecting = codexPlusGatewayPageState.busy === "connect" || codexPlusGatewayPageState.busy === "disconnect";
    return connecting
      ? "正在切换…"
      : gatewayMode
        ? "打开的供应商合在一起给 Codex 用，模型都列进模型选择器。需要 Codex++ 在后台运行。"
        : "Codex 直接连一家，关掉 Codex++ 也能用（需协议转换的除外）；换一家点「使用」。";
  }

  /** 当前连接方式的一句说明；聚合时上面再放「默认模型」。 */
  function renderCodexPlusProviderModeInfo(gatewayMode) {
    const busy = Boolean(codexPlusGatewayPageState.busy || codexPlusProviderPageState.busy);
    const model = codexPlusGatewayPageState.payload?.gateway?.bindings?.codex?.model || "";
    const hint = codexPlusProviderModeHint(gatewayMode);
    return `<div class="codex-plus-provider-mode-info">
      ${gatewayMode && codexPlusGatewayPageState.payload ? renderCodexPlusGatewayDefaultModel(model, busy) : ""}
      <div class="codex-plus-provider-help">${escapeHtml(hint)}</div>
    </div>`;
  }

  /**
   * 聚合时的「默认模型」：和管理器顶部同一份候选（打开、没排除的供应商的模型 + 模型组），选了就改 Codex 绑定的默认模型。
   * 候选跟列表一致：不勾「显示其他 Agent 的供应商」时不列别的 Agent 的供应商的模型；已选的模型能不能用仍按真实排除名单判断。
   */
  function renderCodexPlusGatewayDefaultModel(model, busy) {
    const payload = codexPlusGatewayPageState.payload;
    const exclude = payload?.gateway?.bindings?.codex?.poolExclude || [];
    const visible = new Set(codexPlusGatewayVisibleEntries().map((entry) => entry.id));
    const hidden = codexPlusGatewayEntries().filter((entry) => !visible.has(entry.id)).map((entry) => entry.id);
    const models = codexPlusGatewayModel.agentModels(payload, [...exclude, ...hidden]);
    const options = [
      ...(model ? [] : [["", "选择默认模型"]]),
      // 「供应商 id/模型」这种带前缀的写法不在候选里，但能路由就照常显示。
      ...(model && !models.some((item) => item.toLowerCase() === model.toLowerCase())
        ? [[model, codexPlusGatewayModel.modelRoutable(payload, model, exclude) ? model : `${model}（不可用）`]]
        : []),
      ...models.map((item) => [item, item]),
    ];
    if (!options.length || (!model && !models.length)) return "";
    const current = models.find((item) => item.toLowerCase() === model.toLowerCase()) || model;
    return `<div class="codex-plus-provider-default-model"><span>默认模型</span>${codexPlusProviderMenu("default-model", current, options, "Codex 默认模型", busy ? "disabled" : "")}</div>`;
  }

  /** 浏览器登录、能在 Codex 里直接加账号的种类；Copilot（验证码）和各订阅（要先选登录方式）去管理器加。 */
  const codexPlusGatewayBrowserLogins = { chatgpt: "login_gateway_chatgpt", "chatgpt-api": "login_gateway_chatgpt_api", grok: "login_gateway_grok" };

  /** 额度的一句话：每个窗口「5 小时 32%」，用逗号分开。 */
  function codexPlusGatewayQuotaText(quota) {
    const windows = quota?.windows || [];
    return windows.map((window) => {
      const minutes = window.windowMinutes;
      const name = minutes === 300 ? "5 小时" : minutes === 10080 ? "每周" : minutes ? `${Math.round(minutes / 60)} 小时` : "本期";
      return `${name}已用 ${Math.round(Number(window.usedPercent) || 0)}%`;
    }).join("，");
  }

  /** 一家里的多个账号 / 多个 Key：分配方式、逐个停用、移除和「添加账号」。账号只有一个、Key 不到两个时不显示。 */
  function codexPlusGatewayCredentialsCard(entry) {
    const credentials = entry?.credentials || [];
    const accounts = Boolean(entry?.account);
    if (!entry || (!accounts && credentials.length < 2)) return "";
    const busy = Boolean(codexPlusGatewayPageState.busy);
    const model = codexPlusGatewayModel;
    const globalRotation = codexPlusGatewayPageState.payload?.gateway?.keyPolicy?.mode === "roundRobin" ? "rotate" : "inOrder";
    const rotation = entry.rotationSet && entry.rotation ? entry.rotation : "";
    const rotationOptions = [["", `跟随全局（${model.credentialRotationLabel(globalRotation)}）`], ...model.CREDENTIAL_ROTATIONS.map((value) => [value, model.credentialRotationLabel(value)])];
    const removable = accounts && credentials.length > 1;
    const rows = credentials.map((item) => {
      const on = !item.settings?.disabled;
      const settings = item.settings || {};
      const meta = [
        item.plan,
        codexPlusGatewayQuotaText(item.quota),
        settings.usedPercentLimit ? `用到 ${settings.usedPercentLimit}% 先不用` : "",
        settings.weight && settings.weight !== 1 ? `权重 ${settings.weight}` : "",
        settings.models?.length ? `只给 ${settings.models.join("、")}` : "",
        on ? "" : "已停用",
      ].filter(Boolean).join(" · ");
      const label = item.kind === "key" ? `<code>${escapeHtml(item.label)}</code>` : escapeHtml(item.label);
      return `<div class="codex-plus-provider-line" data-kind="action" data-codex-gateway-credential="${escapeHtml(item.id)}"><span class="codex-plus-provider-line-copy"><span class="codex-plus-provider-line-title">${label}</span>${meta ? `<span class="codex-plus-provider-line-help">${escapeHtml(meta)}</span>` : ""}</span><span class="codex-plus-provider-line-control">
        ${credentials.length > 1 ? codexPlusProviderToggleHtml(on, `使用「${item.label}」`, `data-codex-gateway-credential-toggle="${escapeHtml(item.id)}" ${busy ? "disabled" : ""}`) : ""}
        ${removable ? `<button type="button" class="codex-plus-action-button" data-codex-gateway-account-remove="${escapeHtml(item.id)}" data-label="${escapeHtml(item.label)}" ${busy ? "disabled" : ""}>移除</button>` : ""}
      </span></div>`;
    }).join("");
    const loggingIn = codexPlusGatewayPageState.busy === "add-account";
    const addLine = accounts
      ? codexPlusProviderActionLine("添加账号", codexPlusGatewayBrowserLogins[entry.account]
        ? "再登录一个账号加进这一家，轮着用各个账号的额度。会打开浏览器登录。"
        : "这种账号要在 Codex++ 管理器里登录（验证码或选登录方式），登录后加进这一家。",
        loggingIn
          ? '<button type="button" class="codex-plus-action-button" data-codex-gateway-action="cancel-login">取消登录</button>'
          : `<button type="button" class="codex-plus-action-button" data-codex-gateway-action="add-account" ${busy ? "disabled" : ""}>${codexPlusGatewayBrowserLogins[entry.account] ? "添加账号" : "在 Codex++ 中添加"}</button>`)
      : "";
    const rotationLine = credentials.length > 1
      ? codexPlusProviderActionLine("分配方式", model.credentialRotationDescription(rotation || globalRotation) + "；同一个对话固定用一个，它答不了才换。",
        codexPlusProviderMenu("gw-rotation", rotation, rotationOptions, "分配方式", busy ? "disabled" : ""))
      : "";
    return codexPlusProviderCard(accounts ? "账号" : "Key 分配", `${rotationLine}${rows}${addLine}`,
      credentials.length < 2 ? "" : accounts
        ? "权重、只给哪些模型、额度用到多少先不用，在 Codex++ 管理器的「分配方式」里设。"
        : "权重、只给哪些模型，在 Codex++ 管理器的「分配方式」里设。");
  }

  async function codexPlusGatewayAddAccount(entry) {
    const command = codexPlusGatewayBrowserLogins[entry.account];
    if (!command) {
      void postJson("/manager/open", { page: "gatewayProviders" });
      return;
    }
    showToast("已在浏览器打开登录页，登录完成后回到这里。", { type: "info" });
    await codexPlusGatewayMutate(command, { providerId: entry.id }, { busy: "add-account" });
  }

  function renderCodexPlusGatewayDetail() {
    const state = codexPlusGatewayPageState;
    const entry = state.isNew ? null : codexPlusGatewaySelected();
    if (!state.isNew && !entry) return '<div class="codex-plus-provider-empty">选择左侧的供应商，或新建一个。</div>';
    const busy = Boolean(state.busy);
    const editable = state.isNew || codexPlusGatewayEditable(entry);
    const enabled = entry ? entry.enabled !== false : true;
    const header = `<div class="codex-plus-provider-detail-header">
      <div><div class="codex-plus-provider-title">${escapeHtml(state.isNew ? (state.draft?.name || "新供应商") : entry.name)}</div>
        <div class="codex-plus-provider-subtitle">共享供应商${entry && !enabled ? " · 已停用" : ""}${state.dirty ? " · 有未保存的修改" : ""}</div></div>
      <div class="codex-plus-provider-actions">
        ${entry ? `<button type="button" class="codex-plus-action-button" data-codex-gateway-action="toggle" ${busy ? "disabled" : ""}>${enabled ? "停用" : "启用"}</button>` : ""}
        ${editable ? `<button type="button" class="codex-plus-action-button" data-primary="true" data-codex-gateway-action="save" ${busy || !state.dirty ? "disabled" : ""}>${state.busy === "save" ? "保存中…" : "保存"}</button>` : ""}
        ${entry ? `<button type="button" class="codex-plus-action-button" data-codex-gateway-action="test" ${busy ? "disabled" : ""}>${state.busy === "test" ? "测试中…" : "测试"}</button>` : ""}
        ${state.isNew ? '<button type="button" class="codex-plus-action-button" data-codex-gateway-action="cancel-new">取消</button>' : ""}
      </div></div>`;
    // 结果提示贴在标题行下面（和直连供应商的详情一样），不再只在底部状态栏的小字里。
    const statusBanner = state.status
      ? `<div class="codex-plus-provider-banner" data-tone="${escapeHtml(state.statusTone || "info")}" role="${state.statusTone === "error" ? "alert" : "status"}">${escapeHtml(state.status)}</div>`
      : "";
    // 删除放到页面底部的「危险操作」卡片，与直连页一致。
    const dangerCard = entry ? codexPlusProviderCard("危险操作", codexPlusProviderActionLine("从共享供应商删除", "它的 Key 和模型配置会一起删掉；只想暂时不用可以点右上角「停用」。",
      `<button type="button" class="codex-plus-action-button" data-danger="true" data-codex-gateway-action="delete" ${busy ? "disabled" : ""}>删除</button>`)) : "";
    // 与直连页同一套卡片：小节标题 + 圆角卡片，卡片里一行一项（左标题右控件）。
    const testRow = entry
      ? codexPlusProviderActionLine("测试用模型", "点右上角「测试」时用这个模型发一次请求。",
        codexPlusProviderMenu("gw-test-model", "", [["", "默认"], ...(entry.models || []).map((model) => [model, model])], "测试用模型", 'data-codex-gateway-test-model="true"'))
      : "";
    const managerRow = (title, help) => codexPlusProviderCard("更多设置", codexPlusProviderActionLine(title, help,
      '<button type="button" class="codex-plus-action-button" data-codex-gateway-action="open-manager">在 Codex++ 中打开</button>'));
    if (!editable) {
      return `${header}${statusBanner}
        ${codexPlusGatewayCredentialsCard(entry)}
        ${codexPlusProviderCard("模型", [
          codexPlusProviderActionLine("模型列表", (entry.models || []).join("、") || "（无）", ""),
          testRow,
        ].join(""), entry.account
          ? "登录得到的账号（订阅额度、Token 刷新都由本地服务维护）；模型列表在 Codex++ 管理器里改。"
          : "这一项由 Codex 原生登录或拓展提供，在 Codex 里只能启停、排序和测试。")}
        ${managerRow("账号与订阅", "登录、额度和刷新在 Codex++ 管理器里管理。")}
        ${dangerCard}`;
    }
    const draft = state.draft;
    const keys = entry?.keys || [];
    const keyRows = keys.map((key) => `<div class="codex-plus-provider-key"><label>${escapeHtml(key.id)}</label><code>${escapeHtml(key.masked)}</code>
        <button type="button" data-codex-gateway-key-replace="${escapeHtml(key.id)}">替换</button>
        ${keys.length > 1 ? `<button type="button" data-codex-gateway-key-delete="${escapeHtml(key.id)}">删除</button>` : ""}</div>`).join("");
    const fetched = state.fetchedModels.length
      ? `上游有 ${state.fetchedModels.length} 个模型：${escapeHtml(state.fetchedModels.slice(0, 12).join("、"))}${state.fetchedModels.length > 12 ? "…" : ""} <button type="button" class="codex-plus-provider-link" data-codex-gateway-action="fill-models">全部填入</button>`
      : "";
    return `${header}${statusBanner}
      ${codexPlusProviderCard("连接", [
        codexPlusProviderLine("名称", `<input type="text" class="codex-plus-provider-input" data-codex-gateway-field="name" value="${escapeHtml(draft.name)}">`),
        codexPlusProviderActionLine("协议", "", codexPlusProviderMenu("gw-protocol", draft.protocol, codexPlusProviderProtocolOptions, "协议")),
        codexPlusProviderLine("接口地址", `<input type="text" class="codex-plus-provider-input" data-codex-gateway-field="baseUrl" value="${escapeHtml(draft.baseUrl)}" placeholder="https://example.com/v1" spellcheck="false">`),
      ].join(""))}
      ${codexPlusProviderCard("API Key", state.isNew
        ? codexPlusProviderLine("Key", `<textarea class="codex-plus-provider-input codex-plus-provider-code" rows="2" data-codex-gateway-field="keys" placeholder="一行一个，多个 Key 自动轮换" spellcheck="false" autocomplete="off">${escapeHtml(draft.keys)}</textarea>`, "", true)
        : `${keyRows || '<div class="codex-plus-provider-help">没有已保存的 Key。</div>'}
            <div class="codex-plus-provider-key-add"><input type="password" class="codex-plus-provider-input" data-codex-gateway-new-key="true" placeholder="sk-..." autocomplete="off" spellcheck="false">
            <button type="button" class="codex-plus-action-button" data-codex-gateway-action="add-key" ${busy ? "disabled" : ""}>添加 Key</button></div>`,
        "Key 只显示首尾几位；多个 Key 会自动轮换。")}
      ${state.isNew ? "" : codexPlusGatewayCredentialsCard(entry)}
      ${codexPlusProviderCard("模型", [
        codexPlusProviderLine("默认模型", `<input type="text" class="codex-plus-provider-input" data-codex-gateway-field="model" value="${escapeHtml(draft.model)}" spellcheck="false">`),
        codexPlusProviderLine("模型列表", `<textarea class="codex-plus-provider-input codex-plus-provider-code" rows="5" data-codex-gateway-field="modelList" spellcheck="false" placeholder="一行一个">${escapeHtml(draft.modelList)}</textarea>`, "一行一个模型名。", true),
        codexPlusProviderActionLine("从上游获取模型", fetched || "读取供应商的 /models，可一键填入上面的列表。",
          `<button type="button" class="codex-plus-action-button" data-codex-gateway-action="models" ${busy ? "disabled" : ""}>${state.busy === "models" ? "获取中…" : "获取"}</button>`, true),
        testRow,
      ].join(""))}
      ${entry?.models?.length ? `<details class="codex-plus-provider-card"><summary>上下文窗口</summary>
        ${entry.models.map((model) => {
          const window = entry.modelWindows?.[model] ?? entry.contextWindow;
          return codexPlusProviderActionLine(model, window ? codexPlusProviderWindowLabel(window) : "默认",
            `<button type="button" class="codex-plus-action-button" aria-label="${escapeHtml(`编辑 ${model} 的上下文窗口`)}"
                 data-codex-gateway-action="edit-window" data-model="${escapeHtml(model)}" ${busy ? "disabled" : ""}>编辑</button>`);
        }).join("")}</details>` : ""}
      ${managerRow("价格、代理、限速、模型组", "这些高级项和订阅登录在 Codex++ 管理器里设置。")}
      ${dangerCard}`;
  }

  async function codexPlusGatewayConfirmDiscard() {
    if (!codexPlusGatewayPageState.dirty) return true;
    const answer = await codexPlusAsk({ title: "放弃修改？", message: "当前共享供应商有未保存的修改。", options: [{ value: "discard", label: "放弃修改" }] });
    return Boolean(answer);
  }

  /** 本地服务页签的输入：只改草稿，不重绘（保持焦点）。 */
  function codexPlusGatewayHandleInput(event) {
    const target = event.target;
    const field = target?.dataset?.codexGatewayField;
    const state = codexPlusGatewayPageState;
    if (!field || !state.draft) return false;
    state.draft = { ...state.draft, [field]: target.value };
    state.dirty = true;
    const saveButton = document.querySelector('[data-codex-gateway-action="save"]');
    if (saveButton && !state.busy) saveButton.disabled = false;
    return true;
  }

  /** 本地服务页签的点击；处理了返回 true。 */
  async function codexPlusGatewayHandleClick(target) {
    const state = codexPlusGatewayPageState;
    const member = target.closest("[data-codex-gateway-member]");
    if (member) {
      const id = member.dataset.codexGatewayMember;
      const entry = codexPlusGatewayEntries().find((item) => item.id === id);
      if (!member.disabled && entry) {
        const used = member.getAttribute("aria-checked") !== "true";
        await codexPlusGatewayMutate("set_gateway_pool_member", { agents: ["codex"], id, used }, {
          busy: `member:${id}`,
          success: used ? `Codex 开始用「${entry.name}」。` : `Codex 不再用「${entry.name}」。`,
        });
      }
      return true;
    }
    const entry = codexPlusGatewaySelected();
    const credentialToggle = target.closest("[data-codex-gateway-credential-toggle]");
    if (credentialToggle && entry) {
      if (credentialToggle.disabled) return true;
      const id = credentialToggle.dataset.codexGatewayCredentialToggle;
      const item = (entry.credentials || []).find((credential) => credential.id === id);
      if (item) {
        const disabled = !item.settings?.disabled;
        await codexPlusGatewayMutate("set_gateway_credential_settings", { id: entry.id, credentialId: id, settings: { ...item.settings, disabled } }, {
          busy: `credential:${id}`,
          success: disabled ? `已停用「${item.label}」。` : `已启用「${item.label}」。`,
        });
      }
      return true;
    }
    const removeAccount = target.closest("[data-codex-gateway-account-remove]");
    if (removeAccount && entry) {
      if (removeAccount.disabled) return true;
      const label = removeAccount.dataset.label || "";
      const answer = await codexPlusAsk({ title: "移除账号？", message: `从「${entry.name}」移除账号「${label}」。`, options: [{ value: "remove", label: "移除" }] });
      if (answer) await codexPlusGatewayMutate("remove_gateway_provider_account", { id: entry.id, credentialId: removeAccount.dataset.codexGatewayAccountRemove }, { success: `已移除「${label}」。` });
      return true;
    }
    const replaceKey = target.closest("[data-codex-gateway-key-replace]");
    if (replaceKey && entry) {
      await codexPlusGatewayReplaceKey(entry, replaceKey.dataset.codexGatewayKeyReplace);
      return true;
    }
    const deleteKey = target.closest("[data-codex-gateway-key-delete]");
    if (deleteKey && entry) {
      await codexPlusGatewayDeleteKey(entry, deleteKey.dataset.codexGatewayKeyDelete);
      return true;
    }
    const actionNode = target.closest("[data-codex-gateway-action]");
    if (!actionNode) return false;
    if (actionNode.disabled) return true;
    switch (actionNode.dataset.codexGatewayAction) {
      case "reload": await codexPlusGatewayLoad(); break;
      case "new":
        if (await codexPlusGatewayConfirmDiscard()) codexPlusGatewayStartNew();
        break;
      case "cancel-new":
        state.isNew = false;
        codexPlusGatewaySelect(codexPlusGatewayEntries()[0]?.id || "");
        renderCodexPlusProviderPage();
        break;
      case "save": await codexPlusGatewaySave(); break;
      case "edit-window": {
        if (!await codexPlusGatewayConfirmDiscard()) break;
        const model = actionNode.dataset.model;
        const current = entry.modelWindows?.[model];
        const answer = await codexPlusAsk({
          title: "供应商上下文窗口",
          message: `${model}\n适用于使用此供应商模型的会话。留空恢复供应商或模型的默认窗口。`,
          input: { default: current ? String(current) : "", placeholder: "400K" },
          options: [{ value: "save", label: "保存" }],
        });
        if (!answer) break;
        const window = codexPlusGatewayModel.parseContextWindow(answer.text);
        if (window !== null && (!Number.isSafeInteger(window) || window < 1000)) {
          codexPlusGatewaySetStatus("请输入至少 1000 tokens，例如 400K 或 1M。", "error");
          break;
        }
        const savedEntry = codexPlusGatewaySelected();
        const windows = { ...savedEntry.modelWindows };
        if (window === null) delete windows[model];
        else windows[model] = window;
        const saved = await codexPlusGatewayMutate("save_gateway_provider_windows",
          { id: entry.id, windows, fallback: savedEntry.contextWindow ?? null });
        if (saved) await loadCodexModelCatalog(true);
        break;
      }
      case "toggle":
        if (entry) await codexPlusGatewayMutate("set_gateway_provider_enabled", { id: entry.id, enabled: entry.enabled === false }, { success: entry.enabled === false ? `已启用「${entry.name}」。` : `已停用「${entry.name}」。` });
        break;
      case "delete": if (entry) await codexPlusGatewayDelete(entry); break;
      case "test": await codexPlusGatewayProbe("test"); break;
      case "models": await codexPlusGatewayProbe("models"); break;
      case "fill-models": {
        const lines = String(state.draft?.modelList || "").split(/\n/).map((line) => line.trim()).filter(Boolean);
        state.fetchedModels.forEach((model) => { if (!lines.includes(model)) lines.push(model); });
        state.draft = { ...state.draft, modelList: lines.join("\n"), model: state.draft.model || lines[0] || "" };
        state.fetchedModels = [];
        state.dirty = true;
        renderCodexPlusProviderPage();
        break;
      }
      case "add-key": {
        const key = String(document.querySelector("[data-codex-gateway-new-key]")?.value || "").trim();
        if (entry && key) await codexPlusGatewayMutate("add_gateway_provider_key", { id: entry.id, key }, { success: "Key 已添加。" });
        break;
      }
      case "add-account": if (entry) await codexPlusGatewayAddAccount(entry); break;
      case "cancel-login": void codexPlusGatewayCall("cancel_gateway_login"); break;
      case "connect": await codexPlusGatewayConnect(); break;
      case "disconnect": await codexPlusGatewayOpenDisconnect(); break;
      case "mode-aggregate":
        // 已经经本地服务：只把列表换回聚合视图；还没接入：接入。
        if (codexPlusProviderPageState.data?.gatewayMode === true) {
          codexPlusProviderPageState.viewMode = "aggregate";
          renderCodexPlusProviderPage();
        } else if (await codexPlusProviderConfirmDiscard()) await codexPlusGatewayConnect();
        break;
      case "mode-direct":
        // 只换成直连视图；点某家「使用」才真的切过去（codexPlusProviderSwitchDirect）。
        codexPlusProviderPageState.viewMode = "direct";
        renderCodexPlusProviderPage();
        break;
      // 本地账号的名字、用户名、邮箱在管理器里改（和管理器「编辑资料」同一处）。
      case "edit-local-profile": void postJson("/manager/open", { page: "relay" }); break;
      case "disconnect-cancel":
        state.disconnect = null;
        renderCodexPlusProviderPage();
        break;
      case "disconnect-apply": await codexPlusGatewayApplyDisconnect(); break;
      case "open-manager": void postJson("/manager/open", { page: "gatewayProviders" }); break;
      default: return false;
    }
    return true;
  }

  /** 下拉的值：local 本地账号、none 不登录、account:<id> ChatGPT 账号、"" 尚未指定（和管理器 nativeAuthSelection 一致）。 */
  function codexPlusLoginIdentityValue(login) {
    if (login?.localLogin) return "local";
    if (login?.selection?.mode === "none") return "none";
    if (login?.selection?.mode === "account") return `account:${login.selection.accountRef}`;
    return "";
  }

  async function codexPlusLoginIdentityLoad() {
    const result = await codexPlusGatewayCall("get_native_auth_state");
    if (result?.status === "ok") codexPlusGatewayPageState.login = result;
  }

  /** 选了就写：后端一起改原生登录选择和解锁开关（先还原假登录再写真账号），重启 Codex 后生效。 */
  async function codexPlusLoginIdentityChoose(value) {
    const state = codexPlusGatewayPageState;
    if (state.loginBusy || value === codexPlusLoginIdentityValue(state.login) || !value) return;
    const args = value.startsWith("account:")
      ? { kind: "account", accountRef: value.slice("account:".length) }
      : { kind: value === "local" ? "local" : "none", accountRef: null };
    state.loginBusy = true;
    renderCodexPlusProviderPage();
    const result = await codexPlusGatewayCall("set_codex_login_identity", args);
    state.loginBusy = false;
    if (result?.status === "ok") {
      state.login = result;
      showToast("登录身份已保存，重启 Codex 后生效", { type: "success" });
    } else {
      await codexPlusLoginIdentityLoad();
      showToast(result?.message || "保存登录身份失败", { type: "error" });
    }
    renderCodexPlusProviderPage();
  }

  /** 侧栏底部的「Codex 登录身份」：ChatGPT 账号 / 本地账号 / 不登录，和管理器同一个选择。 */
  function renderCodexPlusLoginIdentity() {
    const state = codexPlusGatewayPageState;
    const login = state.login;
    if (!login) return "";
    const value = codexPlusLoginIdentityValue(login);
    const blocked = login.localLoginBlockedReason || "";
    const accounts = login.accounts || [];
    const options = [
      ...(value ? [] : [["", login.actualLabel || "尚未指定"]]),
      ...accounts.map((account) => [`account:${account.id}`, account.name || account.email || account.id]),
      ...(value.startsWith("account:") && !accounts.some((account) => `account:${account.id}` === value) ? [[value, "已选账号不可用"]] : []),
      // 官方登录供应商下模型请求要用真实登录：虚拟账号置灰，原因写在下面。
      ["local", "虚拟账号", Boolean(blocked) && value !== "local"],
      ["none", "不登录"],
    ];
    const help = value === "local"
      ? `代码审查、图像、资料库等入口已解锁，模型仍走所选供应商。${login.localLoginLaunchHint ? "直接打开 Codex 前，先在 Codex++ 首页打开「自动接管官方 App」。" : ""}<button type="button" class="codex-plus-provider-link" data-codex-gateway-action="edit-local-profile">编辑资料</button>`
      : escapeHtml(login.drift || login.restartBlockedReason || (blocked ? `虚拟账号暂不可用：${blocked}` : "选「虚拟账号」可以用 API Key 解锁 ChatGPT 专属入口。"));
    return `<div class="codex-plus-provider-login">
      <div class="codex-plus-provider-sidebar-header"><span>Codex 登录身份</span></div>
      ${codexPlusProviderMenu("login-identity", value, options, "Codex 登录身份", state.loginBusy ? "disabled" : "")}
      <div class="codex-plus-provider-help">${state.loginBusy ? "正在保存…" : help}${state.loginBusy ? "" : " 改完重启 Codex 后生效。"}</div>
    </div>`;
  }

  /** 别处（管理器）改了登录身份：变了才重绘；正在保存或菜单开着时不打断。 */
  function codexPlusLoginIdentityPoll() {
    const state = codexPlusGatewayPageState;
    if (state.loginBusy || document.hidden) return;
    void codexPlusGatewayCall("get_native_auth_state").then((result) => {
      if (result?.status !== "ok" || state.loginBusy) return;
      const pick = (login) => JSON.stringify([login?.selection, login?.localLogin, login?.localLoginBlockedReason, login?.localLoginLaunchHint, login?.drift, login?.restartBlockedReason, login?.actualLabel, (login?.accounts || []).map((account) => account.id)]);
      if (pick(result) === pick(state.login)) return;
      state.login = result;
      if (document.querySelector(".codex-plus-provider-menu-popover")) return;
      renderCodexPlusProviderPage();
    });
  }

  /** 别处改了池子：没有未保存修改、也没有弹窗时静默刷新。 */
  function codexPlusGatewayPoll() {
    const state = codexPlusGatewayPageState;
    if (state.dirty || state.busy || state.disconnect || document.hidden) return;
    void codexPlusGatewayCall("load_gateway").then((result) => {
      if (result?.status !== "ok" || !result.gateway) return;
      if (state.dirty || state.busy || state.disconnect) return;
      if (codexPlusGatewaySignature(result) === state.signature) return;
      codexPlusGatewayApplyResult(result);
      renderCodexPlusProviderPage();
    });
  }
