  /**
   * 「供应商」页的网关页签：在 Codex 里管理 Codex++ 网关的供应商池，并在网关 / 直连之间切换。
   *
   * - 全部操作经 /providers/gateway 转给管理器自己的网关命令（core provider_page 做白名单与脱敏），
   *   所以生效方式与管理器完全一致：路由每次请求重读 gateway.json，模型变化由 refresh_agents 重写
   *   Codex 的网关目录、心跳指纹变化后页面自动重载 model/list；已打开的对话下一条请求就用新的池。
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
  };

  function codexPlusGatewayCall(command, args = {}) {
    return postJson("/providers/gateway", { command, args });
  }

  function codexPlusGatewayEntries() {
    return codexPlusGatewayPageState.payload?.pool || [];
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
    // 同步底部状态栏（如果当前是网关页签）
    if (codexPlusProviderPageState.tab === "gateway") {
      const footer = document.querySelector("[data-codex-provider-footer]");
      if (footer) {
        footer.textContent = codexPlusGatewayPageState.status;
        footer.dataset.tone = tone;
      }
    }
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

  /** 网关命令的返回（CommandResult 里平铺了 GatewayPayload）。没带 payload 的结果不覆盖现有列表。 */
  function codexPlusGatewayApplyResult(result, { keepSelection = true } = {}) {
    const state = codexPlusGatewayPageState;
    if (result?.gateway && Array.isArray(result.pool)) {
      state.payload = result;
      state.signature = codexPlusGatewaySignature(result);
      const ids = result.pool.map((entry) => entry.id);
      const keep = keepSelection && ids.includes(state.selectedId) ? state.selectedId : ids[0] || "";
      if (!state.isNew) codexPlusGatewaySelect(keep);
    }
  }

  function codexPlusGatewaySignature(payload) {
    return JSON.stringify([
      (payload?.pool || []).map((entry) => [entry.id, entry.name, entry.baseUrl, entry.protocol, entry.enabled, entry.models, entry.keys]),
      payload?.gateway?.bindings?.codex?.enabled,
    ]);
  }

  async function codexPlusGatewayLoad({ quiet = false } = {}) {
    const state = codexPlusGatewayPageState;
    if (!quiet) {
      state.busy = "load";
      renderCodexPlusProviderPage();
    }
    const result = await codexPlusGatewayCall("load_gateway");
    state.busy = "";
    if (result?.status !== "ok" || !result.gateway) {
      codexPlusGatewaySetStatus(result?.message || "读取网关供应商池失败", "error");
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
      codexPlusGatewaySetStatus(`${success || result.message || "已保存。"} 已打开的对话下一条请求就用新的供应商池。`, "ok");
      showToast(success || "网关供应商池已更新", { type: "success" });
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
      title: "从网关删除供应商",
      message: `确定从网关供应商池删除「${entry.name}」？它的 Key、模型和用量设置一起删除，模型组和各 Agent 里对它的引用也会清掉。`,
      options: [{ value: "delete", label: "删除" }],
    });
    if (!answer) return;
    await codexPlusGatewayMutate("delete_gateway_provider", { id: entry.id }, { busy: "delete", success: `已删除「${entry.name}」。` });
  }

  async function codexPlusGatewayMove(id, offset) {
    const ids = codexPlusGatewayEntries().map((entry) => entry.id);
    const index = ids.indexOf(id);
    const target = index + offset;
    if (index < 0 || target < 0 || target >= ids.length) return;
    [ids[index], ids[target]] = [ids[target], ids[index]];
    await codexPlusGatewayMutate("reorder_gateway_providers", { ids }, { busy: "reorder", success: "顺序已更新（排在前面的先用）。" });
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

  /** 让 Codex 经网关接入：与管理器「连接与供应商 → 经网关」同一条命令，沿用上次的接入设置。 */
  async function codexPlusGatewayConnect() {
    const state = codexPlusGatewayPageState;
    const payload = state.payload;
    const previous = payload?.gateway?.bindings?.codex;
    const draft = codexPlusGatewayModel.draftFromBinding(previous);
    const issue = codexPlusGatewayModel.validateDraft(draft, payload);
    if (issue) {
      codexPlusGatewaySetStatus(`${issue}（请先在供应商池里添加可用的供应商，或在 Codex++「Agent 接入」里调整默认模型）`, "error");
      return renderCodexPlusProviderPage();
    }
    const binding = { ...codexPlusGatewayModel.bindingFromDraft(draft, previous), enabled: true };
    const result = await codexPlusGatewayMutate("save_gateway_binding", { agent: "codex", binding, reconnect: true }, { busy: "connect", success: "Codex 已经由网关接入。" });
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
    const result = await codexPlusGatewayMutate(command, { agent: "codex", previewToken: preview.previewToken }, { busy: "disconnect", success: "已断开网关，Codex 改回直连。" });
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
    return `<div class="codex-plus-gateway-modal" data-codex-plus-ext="builtin-provider" role="dialog" aria-modal="true" aria-label="断开网关并还原">
      <div class="codex-plus-gateway-modal-box">
        <div class="codex-plus-provider-title">断开网关并还原为直连</div>
        <div class="codex-plus-provider-help">Codex 会恢复接入网关之前的设置，下面是要改回的内容。</div>
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

  function renderCodexPlusGatewayModeBar() {
    const gatewayMode = codexPlusProviderPageState.data?.gatewayMode === true;
    const busy = Boolean(codexPlusGatewayPageState.busy);
    const model = codexPlusGatewayPageState.payload?.gateway?.bindings?.codex?.model || "";
    return gatewayMode
      ? `<div class="codex-plus-provider-mode-bar" data-codex-gateway-mode="gateway">
          <span>Codex 正经 Codex++ 网关接入${model ? `，默认模型 ${escapeHtml(model)}` : ""}。已打开的对话下一条请求就生效。</span>
          <button type="button" class="codex-plus-provider-link" data-codex-gateway-action="disconnect" ${busy ? "disabled" : ""}>断开并还原为直连</button>
        </div>`
      : `<div class="codex-plus-provider-mode-bar" data-codex-gateway-mode="direct">
          <span>Codex 现在是直连，这里的供应商池暂时不起作用。</span>
          <button type="button" class="codex-plus-provider-link" data-codex-gateway-action="connect" ${busy ? "disabled" : ""}>${codexPlusGatewayPageState.busy === "connect" ? "正在接入…" : "让 Codex 经网关接入"}</button>
        </div>`;
  }

  function renderCodexPlusGatewayList() {
    const state = codexPlusGatewayPageState;
    const entries = codexPlusGatewayEntries();
    const gatewayMode = codexPlusProviderPageState.data?.gatewayMode === true;
    // 当前 Codex 绑定的模型（格式 "providerId/model"），用来标出活跃供应商。
    const activeModel = state.payload?.gateway?.bindings?.codex?.model || "";
    const activeProviderId = gatewayMode && activeModel.includes("/") ? activeModel.split("/")[0] : "";
    const rows = entries.map((entry, index) => {
      const selected = entry.id === state.selectedId && !state.isNew;
      const enabled = entry.enabled !== false;
      const isActive = gatewayMode && entry.id === activeProviderId;
      const tag = entry.account ? (entry.plan || "账号") : String(entry.source || "").startsWith("extension:") ? "拓展" : String(entry.source || "").startsWith("native:") ? "原生登录" : ({ responses: "Responses", chatCompletions: "Chat", anthropicMessages: "Anthropic" }[entry.protocol] || entry.protocol);
      return `<div class="codex-plus-provider-item" role="listitem" data-selected="${selected}" data-disabled="${!enabled}" data-codex-gateway-select="${escapeHtml(entry.id)}">
        <div class="codex-plus-provider-item-main">
          <div class="codex-plus-provider-item-name">${isActive ? '<span class="codex-plus-provider-item-dot" aria-hidden="true"></span>' : ""}${escapeHtml(entry.name || entry.id)}</div>
          <div class="codex-plus-provider-item-meta"><span class="codex-plus-provider-badge">${escapeHtml(tag)}</span><span class="codex-plus-provider-badge">${entry.models?.length || 0} 个模型</span>${!enabled ? '<span class="codex-plus-provider-badge" data-tone="off">已停用</span>' : isActive ? '<span class="codex-plus-provider-badge" data-tone="active">使用中</span>' : ""}</div>
        </div>
        <div class="codex-plus-provider-item-actions">
          <button type="button" title="上移" aria-label="上移" data-codex-gateway-move="-1" data-id="${escapeHtml(entry.id)}" ${index === 0 ? "disabled" : ""}>↑</button>
          <button type="button" title="下移" aria-label="下移" data-codex-gateway-move="1" data-id="${escapeHtml(entry.id)}" ${index === entries.length - 1 ? "disabled" : ""}>↓</button>
        </div>
      </div>`;
    }).join("");
    const draftRow = state.isNew
      ? `<div class="codex-plus-provider-item" role="listitem" data-selected="true"><div class="codex-plus-provider-item-main"><div class="codex-plus-provider-item-name">${escapeHtml(state.draft?.name || "新供应商")}</div><div class="codex-plus-provider-item-meta"><span class="codex-plus-provider-badge" data-tone="new">未保存</span></div></div></div>`
      : "";
    return `<div class="codex-plus-provider-list" role="list" aria-label="网关供应商池">${rows}${draftRow}${rows || draftRow ? "" : '<div class="codex-plus-provider-help">供应商池是空的。</div>'}</div>`;
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
        <div class="codex-plus-provider-subtitle">网关供应商${entry && !enabled ? " · 已停用" : ""}${state.dirty ? " · 有未保存的修改" : ""}</div></div>
      <div class="codex-plus-provider-actions">
        ${entry ? `<button type="button" class="codex-plus-action-button" data-codex-gateway-action="toggle" ${busy ? "disabled" : ""}>${enabled ? "停用" : "启用"}</button>` : ""}
        ${editable ? `<button type="button" class="codex-plus-action-button" data-primary="true" data-codex-gateway-action="save" ${busy || !state.dirty ? "disabled" : ""}>${state.busy === "save" ? "保存中…" : "保存"}</button>` : ""}
        ${entry ? `<button type="button" class="codex-plus-action-button" data-codex-gateway-action="test" ${busy ? "disabled" : ""}>${state.busy === "test" ? "测试中…" : "测试"}</button>` : ""}
        ${state.isNew ? '<button type="button" class="codex-plus-action-button" data-codex-gateway-action="cancel-new">取消</button>' : ""}
      </div></div>`;
    // 删除放到页面底部的「危险操作」卡片，与直连页一致。
    const dangerCard = entry ? codexPlusProviderCard("危险操作", codexPlusProviderActionLine("从供应商池删除", "它的 Key 和模型配置会一起删掉；只想暂时不用可以点右上角「停用」。",
      `<button type="button" class="codex-plus-action-button" data-danger="true" data-codex-gateway-action="delete" ${busy ? "disabled" : ""}>删除</button>`)) : "";
    // 与直连页同一套卡片：小节标题 + 圆角卡片，卡片里一行一项（左标题右控件）。
    const testRow = entry
      ? codexPlusProviderActionLine("测试用模型", "点右上角「测试」时用这个模型发一次请求。",
        codexPlusProviderMenu("gw-test-model", "", [["", "默认"], ...(entry.models || []).map((model) => [model, model])], "测试用模型", 'data-codex-gateway-test-model="true"'))
      : "";
    const managerRow = (title, help) => codexPlusProviderCard("更多设置", codexPlusProviderActionLine(title, help,
      '<button type="button" class="codex-plus-action-button" data-codex-gateway-action="open-manager">在 Codex++ 中打开</button>'));
    if (!editable) {
      return `${header}
        ${codexPlusProviderCard("模型", [
          codexPlusProviderActionLine("模型列表", (entry.models || []).join("、") || "（无）", ""),
          testRow,
        ].join(""), entry.account
          ? "登录得到的账号（订阅额度、Token 刷新都由网关维护），在 Codex 里只能启停、排序和测试。"
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
    return `${header}
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
      ${codexPlusProviderCard("模型", [
        codexPlusProviderLine("默认模型", `<input type="text" class="codex-plus-provider-input" data-codex-gateway-field="model" value="${escapeHtml(draft.model)}" spellcheck="false">`),
        codexPlusProviderLine("模型列表", `<textarea class="codex-plus-provider-input codex-plus-provider-code" rows="5" data-codex-gateway-field="modelList" spellcheck="false" placeholder="一行一个">${escapeHtml(draft.modelList)}</textarea>`, "一行一个模型名。", true),
        codexPlusProviderActionLine("从上游获取模型", fetched || "读取供应商的 /models，可一键填入上面的列表。",
          `<button type="button" class="codex-plus-action-button" data-codex-gateway-action="models" ${busy ? "disabled" : ""}>${state.busy === "models" ? "获取中…" : "获取"}</button>`, true),
        testRow,
      ].join(""))}
      ${managerRow("价格、代理、限速、窗口、模型组", "这些高级项和订阅登录在 Codex++ 管理器里设置。")}
      ${dangerCard}`;
  }

  function renderCodexPlusGatewayPane() {
    const state = codexPlusGatewayPageState;
    if (!state.payload) {
      return `<div class="codex-plus-provider-empty">${state.busy === "load" ? "正在读取网关供应商池…" : escapeHtml(state.status || "还没有读取网关供应商池")}
        <button type="button" class="codex-plus-provider-link" data-codex-gateway-action="reload">重试</button></div>`;
    }
    return `<div class="codex-plus-provider-layout">
      <aside class="codex-plus-provider-sidebar">
        <div class="codex-plus-provider-sidebar-header"><span>供应商池</span>
          <button type="button" class="codex-plus-action-button" data-codex-gateway-action="new" ${state.busy ? "disabled" : ""}>+ 新建</button></div>
        ${renderCodexPlusGatewayList()}
      </aside>
      <section class="codex-plus-provider-detail">
        ${renderCodexPlusGatewayModeBar()}
        ${renderCodexPlusGatewayDetail()}
      </section></div>${renderCodexPlusGatewayDisconnectDialog()}`;
  }

  async function codexPlusGatewayConfirmDiscard() {
    if (!codexPlusGatewayPageState.dirty) return true;
    const answer = await codexPlusAsk({ title: "放弃修改？", message: "当前网关供应商有未保存的修改。", options: [{ value: "discard", label: "放弃修改" }] });
    return Boolean(answer);
  }

  /** 网关页签的输入：只改草稿，不重绘（保持焦点）。 */
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

  /** 网关页签的点击；处理了返回 true。 */
  async function codexPlusGatewayHandleClick(target) {
    const state = codexPlusGatewayPageState;
    const move = target.closest("[data-codex-gateway-move]");
    if (move) {
      if (move.disabled || !(await codexPlusGatewayConfirmDiscard())) return true;
      await codexPlusGatewayMove(move.dataset.id, Number(move.dataset.codexGatewayMove));
      return true;
    }
    const select = target.closest("[data-codex-gateway-select]");
    if (select) {
      const id = select.dataset.codexGatewaySelect;
      if ((id !== state.selectedId || state.isNew) && (await codexPlusGatewayConfirmDiscard())) {
        codexPlusGatewaySelect(id);
        codexPlusGatewaySetStatus("");
        renderCodexPlusProviderPage();
      }
      return true;
    }
    const entry = codexPlusGatewaySelected();
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
      case "connect": await codexPlusGatewayConnect(); break;
      case "disconnect": await codexPlusGatewayOpenDisconnect(); break;
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
