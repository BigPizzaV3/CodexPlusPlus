  /**
   * Codex++ 设置页里「语音输入」「下一步建议」两个内置拓展的服务配置：地址、模型、Key、超时等。
   * 以前只在管理器的增强页里能改；现在管理器只留增强总开关，这些都只在这里改。
   *
   * 分工：
   *   - 值存在 Codex++ 后端设置，经 /settings/set 合并写入（dictation 是嵌套对象，只发这次改的字段）。
   *   - Key 不回页面（/settings/get 只给 dictation.apiKeyConfigured / codexAppStepwiseApiKeyConfigured）：
   *     输入框留空表示不改，填了失焦才写；「清除」写空串。
   *   - 本地模型下载、全局热键经 /dictation/local-model/*、/dictation/global-hotkey* 走管理器，
   *     下一步建议的「测试连接」走 /stepwise/test。这些路由都不在拓展白名单里。
   *   - 两节由 renderCodexPlusSettingsCatalogSections 跟着清单分组一起生成（带 data-codex-plus-settings-catalog），
   *     清单刷新时一起重建；下载进度、热键、测试结果这些页面状态放在 codexPlusServiceSettingsState。
   */
  const codexPlusServiceSettingsState = {
    localModel: null,
    localModelError: "",
    localModelTimer: null,
    hotkey: null,
    hotkeyLoading: false,
    hotkeyBusy: false,
    hotkeyMessage: "",
    hotkeyFailed: false,
    stepwiseTestBusy: false,
    stepwiseTestMessage: "",
    stepwiseTestFailed: false,
    nativeBrowser: null,
    nativeBrowserBusy: false,
  };

  // 语音服务预设：只改端点、模型与 provider（默认端点与管理器 dictation-settings.ts 的默认值一致）。
  const codexPlusDictationPresets = {
    sensevoice: { provider: "sensevoice", baseUrl: "local://sensevoice", model: "SenseVoiceSmall (INT8)" },
    groq: { provider: "api", baseUrl: "https://api.groq.com/openai/v1", model: "whisper-large-v3-turbo" },
    openai: { provider: "api", baseUrl: "https://api.openai.com/v1", model: "whisper-1" },
    local: { provider: "api", baseUrl: "http://127.0.0.1:8000/v1", model: "whisper-1" },
  };
  const codexPlusDictationPresetLabels = [
    ["sensevoice", "本地 SenseVoice（离线）"],
    ["groq", "Groq"],
    ["openai", "OpenAI 兼容"],
    ["local", "本地服务"],
    ["custom", "自定义"],
  ];
  const codexPlusDictationLocalModelBytes = 239549735;

  // 数值字段的取值范围与后端 clamp 一致。
  const codexPlusStepwiseNumberFields = {
    codexAppStepwiseMaxItems: { min: 0, max: 6, step: 1 },
    codexAppStepwiseTimeoutMs: { min: 1000, max: 60000, step: 500 },
    codexAppStepwiseMaxInputChars: { min: 1000, max: 24000, step: 500 },
    codexAppStepwiseMaxOutputTokens: { min: 100, max: 4000, step: 50 },
  };

  function codexPlusDictationSettings() {
    const value = codexPlusBackendSettings?.dictation;
    return value && typeof value === "object" ? value : {};
  }

  function codexPlusDictationPreset(dictation) {
    if (dictation.provider === "sensevoice") return "sensevoice";
    const baseUrl = String(dictation.baseUrl || "").trim().replace(/\/+$/, "");
    const match = Object.entries(codexPlusDictationPresets).find(([, preset]) => preset.provider === "api" && preset.baseUrl === baseUrl);
    return match ? match[0] : "custom";
  }

  /** 语音服务配置校验（地址规则与后端 dictation.rs 发请求前的检查一致）；只在开启语音输入时提示。 */
  function codexPlusDictationIssue(dictation) {
    if (dictation.enabled !== true || dictation.provider === "sensevoice") return "";
    try {
      const url = new URL(String(dictation.baseUrl || ""));
      if (!["http:", "https:"].includes(url.protocol) || url.username || url.password || url.search || url.hash) {
        return "Base URL 需要是不带账号、查询参数和锚点的 http(s) 地址。";
      }
    } catch {
      return "Base URL 需要是不带账号、查询参数和锚点的 http(s) 地址。";
    }
    if (!String(dictation.model || "").trim()) return "请填写语音模型。";
    const env = String(dictation.apiKeyEnv || "").trim();
    if (env && !/^[A-Za-z_][A-Za-z0-9_]*$/.test(env)) return "API Key 环境变量名只能包含字母、数字和下划线，且不能以数字开头。";
    return "";
  }

  function renderCodexPlusServiceRow(title, description, control, attrs = "") {
    return `
                <div class="codex-plus-row"${attrs}>
                  <div><div class="codex-plus-row-title">${escapeHtml(title)}</div>${description ? `<div class="codex-plus-row-description">${escapeHtml(description)}</div>` : ""}</div>
                  <div class="codex-plus-service-controls">${control}</div>
                </div>`;
  }

  function renderCodexPlusServiceInput(kind, key, value, options = {}) {
    const type = options.type || "text";
    const range = options.range ? ` min="${options.range.min}" max="${options.range.max}" step="${options.range.step}"` : "";
    return `<input class="codex-plus-provider-input codex-plus-service-input" data-codex-plus-${kind}-field="${escapeHtml(key)}" type="${type}"${range} value="${escapeHtml(value ?? "")}" placeholder="${escapeHtml(options.placeholder || "")}" aria-label="${escapeHtml(options.label || key)}" spellcheck="false" autocomplete="off">`;
  }

  function renderCodexPlusServiceSelect(attr, value, options, label) {
    return `<select class="codex-plus-provider-input codex-plus-service-input" ${attr} aria-label="${escapeHtml(label)}">${options
      .map(([optionValue, optionLabel]) => `<option value="${escapeHtml(optionValue)}"${optionValue === value ? " selected" : ""}>${escapeHtml(optionLabel)}</option>`)
      .join("")}</select>`;
  }

  /** Key 输入框：不回显，只显示「已配置」；已配置时给「清除」。 */
  function renderCodexPlusServiceSecret(kind, key, configured, label) {
    const input = `<input class="codex-plus-provider-input codex-plus-service-input" data-codex-plus-${kind}-secret="${escapeHtml(key)}" type="password" value="" placeholder="${configured ? "已配置（留空不改）" : "未配置"}" aria-label="${escapeHtml(label)}" spellcheck="false" autocomplete="off">`;
    const clear = configured ? `<button type="button" class="codex-plus-action-button" data-codex-plus-${kind}-secret-clear="${escapeHtml(key)}">清除</button>` : "";
    return input + clear;
  }

  function renderCodexPlusServiceStatus(text, failed, attr) {
    return `<span class="codex-plus-service-status" ${attr} role="${failed ? "alert" : "status"}" data-tone="${failed ? "error" : ""}"${text ? "" : " hidden"}>${escapeHtml(text)}</span>`;
  }

  function codexPlusLocalModelText() {
    const state = codexPlusServiceSettingsState;
    const model = state.localModel;
    if (model?.phase === "downloading") {
      const total = Number(model.totalBytes) || codexPlusDictationLocalModelBytes;
      return `正在下载并校验… ${Math.min(100, Math.floor((Number(model.completedBytes) || 0) / total * 100))}%`;
    }
    if (state.localModelError) return state.localModelError;
    if (model?.phase === "failed") return model.message || "本地模型下载失败，请重试。";
    if (!model) return "正在读取本地模型状态…";
    return model.modelReady ? "本地模型已下载" : "本地模型尚未下载";
  }

  function renderCodexPlusDictationServiceSection() {
    const state = codexPlusServiceSettingsState;
    const dictation = codexPlusDictationSettings();
    const preset = codexPlusDictationPreset(dictation);
    const sensevoice = dictation.provider === "sensevoice";
    const rows = [];
    if (codexPlusIsWindowsPlatform || codexPlusIsMacPlatform) {
      const hotkey = state.hotkey || { enabled: false, hotkey: "Ctrl+Shift+Space" };
      const disabled = !state.hotkey || state.hotkeyBusy ? " disabled" : "";
      rows.push(renderCodexPlusServiceRow(
        "全局听写热键",
        "按一次开始录音，再按一次停止并插入文字。热键冲突会在保存时报告。",
        `<input class="codex-plus-provider-input codex-plus-service-input codex-plus-service-input-short" data-codex-plus-dictation-hotkey="true" type="text" value="${escapeHtml(hotkey.hotkey)}" placeholder="Ctrl+Shift+Space" aria-label="全局热键" spellcheck="false" autocomplete="off"${disabled}>`
          + `<button type="button" class="codex-plus-action-button" data-codex-plus-dictation-hotkey-save="true"${disabled}>保存</button>`
          + `<button type="button" class="codex-plus-toggle" data-codex-plus-dictation-hotkey-toggle="true" role="switch" aria-checked="${hotkey.enabled === true}" data-enabled="${hotkey.enabled === true}" aria-label="全局听写热键"${disabled}><span></span></button>`
          + renderCodexPlusServiceStatus(state.hotkeyMessage, state.hotkeyFailed, 'data-codex-plus-dictation-hotkey-status="true"'),
      ));
    }
    rows.push(renderCodexPlusServiceRow(
      "语音服务",
      sensevoice ? "SenseVoiceSmall 在本机 CPU 上运行，音频不会离开本机。" : "转写请求发送到 Base URL 下的 /audio/transcriptions，可改预设的地址和模型。",
      renderCodexPlusServiceSelect('data-codex-plus-dictation-preset="true"', preset, codexPlusDictationPresetLabels, "语音服务预设"),
    ));
    rows.push(renderCodexPlusServiceRow("语音模型", "", renderCodexPlusServiceInput("dictation", "model", dictation.model, { placeholder: "whisper-large-v3-turbo", label: "语音模型" })));
    if (sensevoice) {
      const model = state.localModel;
      const downloading = model?.phase === "downloading";
      const failed = !!state.localModelError || model?.phase === "failed";
      rows.push(renderCodexPlusServiceRow(
        "本地模型",
        "约 229 MiB，需手动下载；下载并校验完成后才能录音转写。",
        renderCodexPlusServiceStatus(codexPlusLocalModelText(), failed && !downloading, 'data-codex-plus-dictation-local-model-status="true"')
          + `<button type="button" class="codex-plus-action-button" data-codex-plus-dictation-local-model-download="true"${downloading ? " disabled" : ""}>${model?.modelReady ? "校验或重新下载" : "下载本地模型"}</button>`,
      ));
    } else {
      rows.push(renderCodexPlusServiceRow("Base URL", "", renderCodexPlusServiceInput("dictation", "baseUrl", dictation.baseUrl, { placeholder: "https://api.groq.com/openai/v1", label: "语音服务 Base URL" })));
      rows.push(renderCodexPlusServiceRow("API Key", "本地服务不要求认证时可留空。", renderCodexPlusServiceSecret("dictation", "apiKey", dictation.apiKeyConfigured === true, "语音服务 API Key")));
      rows.push(renderCodexPlusServiceRow("API Key 环境变量", "没填 API Key 时从这个环境变量读取。", renderCodexPlusServiceInput("dictation", "apiKeyEnv", dictation.apiKeyEnv, { placeholder: "GROQ_API_KEY", label: "API Key 环境变量" })));
    }
    rows.push(renderCodexPlusServiceRow("语音语言", "留空自动识别。", renderCodexPlusServiceInput("dictation", "language", dictation.language, { placeholder: "例如 zh 或 en", label: "语音语言" })));
    rows.push(renderCodexPlusServiceRow("转写超时（秒）", "", renderCodexPlusServiceInput("dictation", "timeoutSeconds", dictation.timeoutSeconds ?? 120, { type: "number", range: { min: 1, max: 600, step: 1 }, label: "转写超时（秒）" })));
    const issue = codexPlusDictationIssue(dictation);
    if (issue) {
      rows.push(`
                <div class="codex-plus-row" role="alert"><div><div class="codex-plus-row-title">配置不完整</div><div class="codex-plus-row-description">${escapeHtml(issue)}</div></div></div>`);
    }
    return `
            <section class="codex-plus-settings-section" aria-label="语音输入服务" data-codex-plus-settings-catalog="section" data-codex-plus-service-section="dictation">
              <h2 class="codex-plus-settings-section-title">语音输入服务</h2>
              <div class="codex-plus-settings-card">${rows.join("")}</div>
            </section>`;
  }

  function renderCodexPlusStepwiseServiceSection() {
    const state = codexPlusServiceSettingsState;
    const settings = codexPlusBackendSettings || {};
    const field = (key, title, description, options = {}) => renderCodexPlusServiceRow(title, description,
      renderCodexPlusServiceInput("stepwise", key, settings[key] ?? "", { label: title, ...options }));
    const number = (key, title) => field(key, title, "", { type: "number", range: codexPlusStepwiseNumberFields[key] });
    const rows = [
      field("codexAppStepwiseBaseUrl", "Base URL", "生成建议与大纲用的模型服务地址，与代码模型的供应商分开。", { placeholder: "https://api.example.com/v1" }),
      field("codexAppStepwiseModel", "模型", "", { placeholder: "例如 gpt-5.4-mini" }),
      renderCodexPlusServiceRow("API Key", "", renderCodexPlusServiceSecret("stepwise", "codexAppStepwiseApiKey", settings.codexAppStepwiseApiKeyConfigured === true, "下一步建议 API Key")),
      renderCodexPlusServiceRow("协议", "", renderCodexPlusServiceSelect('data-codex-plus-stepwise-select="codexAppStepwiseProtocol"', settings.codexAppStepwiseProtocol || "auto", [
        ["auto", "自动兼容"],
        ["chat_completions", "Chat Completions"],
        ["responses", "Responses API"],
        ["anthropic_messages", "Anthropic Messages"],
      ], "协议")),
      renderCodexPlusServiceRow("生成方式", "", renderCodexPlusServiceSelect('data-codex-plus-stepwise-select="codexAppStepwiseGenerationMode"', settings.codexAppStepwiseGenerationMode || "auto", [
        ["auto", "自动生成"],
        ["manual", "手动刷新"],
      ], "生成方式")),
      field("codexAppStepwiseApiKeyEnv", "API Key 环境变量", ""),
      number("codexAppStepwiseMaxItems", "最多建议数"),
      number("codexAppStepwiseTimeoutMs", "超时（毫秒）"),
      number("codexAppStepwiseMaxInputChars", "最大输入字符"),
      number("codexAppStepwiseMaxOutputTokens", "最大输出 tokens"),
      renderCodexPlusServiceRow("测试连接", "用已保存的配置请求一次建议。",
        renderCodexPlusServiceStatus(state.stepwiseTestMessage, state.stepwiseTestFailed, 'data-codex-plus-stepwise-test-status="true"')
          + `<button type="button" class="codex-plus-action-button" data-codex-plus-stepwise-test="true"${state.stepwiseTestBusy ? " disabled" : ""}>${state.stepwiseTestBusy ? "正在测试…" : "测试连接"}</button>`),
    ];
    return `
            <section class="codex-plus-settings-section" aria-label="下一步建议服务" data-codex-plus-settings-catalog="section" data-codex-plus-service-section="stepwise">
              <h2 class="codex-plus-settings-section-title">下一步建议服务</h2>
              <div class="codex-plus-settings-card">${rows.join("")}</div>
            </section>`;
  }

  // 原生浏览器状态文案：连接是否可用、请求标识、兼容补丁状态三者分开说（issue #2209）。
  function codexPlusBrowserConnectionLabel(connection) {
    switch (connection?.state) {
      case "available": {
        const names = [...new Set((connection.browsers || []).map((browser) =>
          browser.family === "edge" ? "Edge" : browser.family === "chrome" ? "Chrome" : ""))].filter(Boolean);
        return names.length ? `${names.join(" / ")} 浏览器可用` : "浏览器状态检测失败";
      }
      case "disconnected": return "浏览器未连接";
      case "checking": return "正在检测浏览器连接";
      case "unsupported": return "此平台不支持浏览器连接检测";
      default: return "浏览器状态检测失败";
    }
  }

  function codexPlusBrowserHeaderLabel(enabled) {
    return enabled === true ? "请求标识：已开启（扩展报告）"
      : enabled === false ? "请求标识：已关闭（扩展报告）"
      : "请求标识：扩展未提供状态";
  }

  function codexPlusNativeBrowserPatchLabel(state) {
    switch (state) {
      case "disabled": return "兼容选项未启用";
      case "not_started": return "等待下次启动 Codex++；保存设置不会自动重启";
      case "waiting_for_runtime": return "等待原生插件生成运行时";
      case "prepared": return "兼容服务文件已准备，等待原生工作进程加载";
      case "restored": return "服务文件已恢复；扩展保留的请求标识不受此操作影响";
      case "runtime_unverified": return "未应用旧版兼容补丁：当前运行时版本不匹配";
      case "blocked": return "兼容补丁文件检查或恢复受阻，已保留文件";
      case "unsupported": return "此平台未提供 Edge / Chrome 请求标识兼容补丁";
      case "stale": return "状态已过期；需启动本版本 Codex++ 后重新检查";
      default: return "无法读取兼容补丁状态";
    }
  }

  /** 「原生 Edge / Chrome 请求标识兼容」的只读状态（只在 Windows 上有这个开关）。 */
  function renderCodexPlusNativeBrowserStatusSection() {
    const state = codexPlusServiceSettingsState;
    const result = state.nativeBrowser;
    const connection = result?.connection || { state: "checking", browsers: [], failedChecks: 0 };
    const compatibility = result?.compatibility || { state: "not_started", detail: "" };
    const lines = [codexPlusBrowserConnectionLabel(connection)];
    for (const browser of connection.browsers || []) {
      lines.push(`${browser.family === "edge" ? "Edge" : "Chrome"} · ${codexPlusBrowserHeaderLabel(browser.headerEnabled ?? null)}${browser.recognized === false ? "（扩展版本未登记）" : ""}`);
    }
    const details = [codexPlusNativeBrowserPatchLabel(compatibility.state)];
    if (compatibility.detail) details.push(compatibility.detail);
    if (connection.failedChecks > 0) details.push(`${connection.failedChecks} 个原生连接端点未通过检测`);
    const busy = state.nativeBrowserBusy ? " disabled" : "";
    return `
            <section class="codex-plus-settings-section" aria-label="原生浏览器状态" data-codex-plus-settings-catalog="section" data-codex-plus-service-section="native-browser">
              <h2 class="codex-plus-settings-section-title">原生浏览器状态</h2>
              <div class="codex-plus-settings-card">${renderCodexPlusServiceRow(
                lines[0],
                [...lines.slice(1), ...details].join("；"),
                `<button type="button" class="codex-plus-action-button" data-codex-plus-native-browser-refresh="true"${busy}>${state.nativeBrowserBusy ? "正在检测…" : "刷新"}</button>`,
                ' data-codex-plus-native-browser-status="true" role="status" aria-live="polite"',
              )}</div>
            </section>`;
  }

  async function refreshCodexPlusNativeBrowserStatus() {
    const state = codexPlusServiceSettingsState;
    if (state.nativeBrowserBusy) return;
    state.nativeBrowserBusy = true;
    refreshCodexPlusServiceSection("native-browser");
    try {
      const result = await postJson("/native-browser/status", {});
      state.nativeBrowser = result?.connection ? result : {
        compatibility: { state: "unavailable", detail: result?.message || "" },
        connection: { state: "check_failed", browsers: [], failedChecks: 1 },
      };
    } catch {
      state.nativeBrowser = {
        compatibility: { state: "unavailable", detail: "" },
        connection: { state: "check_failed", browsers: [], failedChecks: 1 },
      };
    } finally {
      state.nativeBrowserBusy = false;
      refreshCodexPlusServiceSection("native-browser");
    }
  }

  /** 清单分组后面要接的服务配置节；renderCodexPlusSettingsCatalogSections 按分组 id 调用。 */
  function renderCodexPlusServiceSettingsAfterGroup(groupId) {
    if (groupId === "input") {
      codexPlusEnsureServiceSettingsLoaded();
      return renderCodexPlusDictationServiceSection();
    }
    if (groupId === "stepwise") return renderCodexPlusStepwiseServiceSection();
    if (groupId === "plugins" && codexPlusIsWindowsPlatform) {
      // 第一次渲染时检测一次；之后靠「刷新」按钮，不常驻轮询。
      if (!codexPlusServiceSettingsState.nativeBrowser && !codexPlusServiceSettingsState.nativeBrowserBusy) {
        setTimeout(() => void refreshCodexPlusNativeBrowserStatus(), 0);
      }
      return renderCodexPlusNativeBrowserStatusSection();
    }
    return "";
  }

  /** 重建一节；焦点在节内的输入框上时，重建后放回同一个字段。 */
  function refreshCodexPlusServiceSection(section) {
    const render = section === "dictation" ? renderCodexPlusDictationServiceSection
      : section === "native-browser" ? renderCodexPlusNativeBrowserStatusSection
        : renderCodexPlusStepwiseServiceSection;
    document.querySelectorAll(`[data-codex-plus-service-section="${section}"]`).forEach((node) => {
      const active = document.activeElement && node.contains(document.activeElement) ? document.activeElement : null;
      const focusAttr = active ? [...active.attributes].find((attr) => attr.name.startsWith("data-codex-plus-")) : null;
      node.insertAdjacentHTML("afterend", render());
      const next = node.nextElementSibling;
      node.remove();
      if (focusAttr && next) next.querySelector(`[${focusAttr.name}="${CSS.escape(focusAttr.value)}"]`)?.focus({ preventScroll: true });
    });
    codexPlusSyncLocalModelPolling();
  }

  /** 设置页打开时读一次热键配置；选了本地模型时读模型状态。 */
  function codexPlusEnsureServiceSettingsLoaded() {
    const state = codexPlusServiceSettingsState;
    if (!state.hotkey && !state.hotkeyLoading && (codexPlusIsWindowsPlatform || codexPlusIsMacPlatform)) {
      state.hotkeyLoading = true;
      void Promise.resolve()
        .then(() => postJson("/dictation/global-hotkey", {}))
        .then((result) => {
          if (result && typeof result.hotkey === "string") state.hotkey = { enabled: result.enabled === true, hotkey: result.hotkey };
          else {
            state.hotkeyMessage = result?.message || "无法读取全局热键设置";
            state.hotkeyFailed = true;
          }
        }, (error) => {
          state.hotkeyMessage = error?.message || "无法读取全局热键设置";
          state.hotkeyFailed = true;
        })
        .then(() => {
          state.hotkeyLoading = false;
          refreshCodexPlusServiceSection("dictation");
        });
    }
    // 渲染完成后再决定是否轮询（DOM 里有没有这一节）。
    setTimeout(codexPlusSyncLocalModelPolling, 0);
  }

  async function refreshCodexPlusLocalModelStatus() {
    const state = codexPlusServiceSettingsState;
    try {
      const result = await postJson("/dictation/local-model/status", {});
      if (result?.status === "failed") state.localModelError = result.message || "无法读取本地模型状态，请重试。";
      else {
        state.localModel = result;
        state.localModelError = "";
      }
    } catch {
      state.localModelError = "无法读取本地模型状态，请重试。";
    }
    // 只更新状态文字与按钮，不重建整节，免得打断正在输入的字段。
    document.querySelectorAll("[data-codex-plus-dictation-local-model-status]").forEach((node) => {
      const text = codexPlusLocalModelText();
      const downloading = state.localModel?.phase === "downloading";
      const failed = !downloading && (!!state.localModelError || state.localModel?.phase === "failed");
      if (node.textContent !== text) node.textContent = text;
      node.hidden = !text;
      node.dataset.tone = failed ? "error" : "";
      node.setAttribute("role", failed ? "alert" : "status");
      const button = node.parentElement?.querySelector("[data-codex-plus-dictation-local-model-download]");
      if (button) {
        button.disabled = downloading;
        const label = state.localModel?.modelReady ? "校验或重新下载" : "下载本地模型";
        if (button.textContent !== label) button.textContent = label;
      }
    });
    codexPlusSyncLocalModelPolling();
  }

  /**
   * 页面上有本地模型那一行时轮询状态：下载中每秒一次，否则只在出现时读一次。
   * 这一行不在了（关掉设置页、换了服务）就停。
   */
  function codexPlusSyncLocalModelPolling() {
    const state = codexPlusServiceSettingsState;
    const visible = !!document.querySelector("[data-codex-plus-dictation-local-model-status]");
    if (!visible) {
      clearTimeout(state.localModelTimer);
      state.localModelTimer = null;
      return;
    }
    if (state.localModelTimer) return;
    const delay = !state.localModel ? 0 : state.localModel.phase === "downloading" ? 1000 : -1;
    if (delay < 0) return;
    state.localModelTimer = setTimeout(() => {
      state.localModelTimer = null;
      void refreshCodexPlusLocalModelStatus();
    }, delay);
  }

  /**
   * 写一组设置到后端：本地先合并（页面立即按新值显示），后端只收到 patch。
   * 写失败时提示原因，并重新读一次后端设置，页面回到真实值。
   */
  async function writeCodexPlusServiceSettings(localPatch, remotePatch, section) {
    const seq = ++codexPlusBackendSettingsSeq;
    codexPlusBackendSettings = { ...codexPlusBackendSettings, ...localPatch };
    if (section) refreshCodexPlusServiceSection(section);
    let result;
    try {
      result = await postJson("/settings/set", remotePatch);
    } catch (error) {
      result = { status: "failed", message: error?.message || String(error) };
    }
    if (!result || result.status === "failed") {
      showToast(`保存失败：${result?.message || "未连接"}`, { type: "error" });
      await loadBackendSettings();
    } else if (seq === codexPlusBackendSettingsSeq) {
      codexPlusBackendSettings = { ...codexPlusBackendSettings, ...result };
    }
    for (const key of Object.keys(remotePatch)) {
      if (typeof emitCodexPlusSettingsChanged === "function") emitCodexPlusSettingsChanged(key, codexPlusBackendSettings[key], true);
    }
    if (section) refreshCodexPlusServiceSection(section);
  }

  function setCodexPlusDictationFields(patch, rerender) {
    const dictation = { ...codexPlusDictationSettings(), ...patch };
    if (typeof patch.apiKey === "string") {
      delete dictation.apiKey;
      dictation.apiKeyConfigured = patch.apiKey.trim() !== "";
    }
    return writeCodexPlusServiceSettings({ dictation }, { dictation: patch }, rerender ? "dictation" : "");
  }

  function clampCodexPlusServiceNumber(value, range, fallback) {
    const number = Math.round(Number(value));
    if (!Number.isFinite(number)) return fallback;
    return Math.min(range.max, Math.max(range.min, number));
  }

  async function saveCodexPlusGlobalHotkey(enabled) {
    const state = codexPlusServiceSettingsState;
    const input = document.querySelector("[data-codex-plus-dictation-hotkey]");
    const hotkey = String(input?.value ?? state.hotkey?.hotkey ?? "").trim();
    state.hotkeyBusy = true;
    state.hotkeyMessage = "";
    refreshCodexPlusServiceSection("dictation");
    try {
      const result = await postJson("/dictation/global-hotkey/set", { enabled, hotkey });
      if (result?.status === "ok" && typeof result.hotkey === "string") {
        state.hotkey = { enabled: result.enabled === true, hotkey: result.hotkey };
        state.hotkeyMessage = result.message || "已保存";
        state.hotkeyFailed = false;
      } else {
        // 失败时保留用户刚输入的热键，方便改了再存。
        state.hotkey = { enabled: state.hotkey?.enabled === true, hotkey };
        state.hotkeyMessage = result?.message || "保存全局热键失败";
        state.hotkeyFailed = true;
      }
    } catch (error) {
      state.hotkeyMessage = error?.message || "保存全局热键失败";
      state.hotkeyFailed = true;
    } finally {
      state.hotkeyBusy = false;
      refreshCodexPlusServiceSection("dictation");
    }
  }

  async function startCodexPlusLocalModelDownload() {
    const state = codexPlusServiceSettingsState;
    state.localModelError = "";
    state.localModel = { ...(state.localModel || {}), phase: "downloading", completedBytes: 0 };
    refreshCodexPlusServiceSection("dictation");
    try {
      const result = await postJson("/dictation/local-model/download", {});
      if (result?.status === "failed") state.localModelError = result.message || "本地模型下载失败，请重试。";
      else state.localModel = result;
    } catch {
      state.localModelError = "本地模型下载失败，请重试。";
    }
    void refreshCodexPlusLocalModelStatus();
  }

  async function testCodexPlusStepwiseConnection() {
    const state = codexPlusServiceSettingsState;
    if (state.stepwiseTestBusy) return;
    state.stepwiseTestBusy = true;
    state.stepwiseTestMessage = "";
    refreshCodexPlusServiceSection("stepwise");
    try {
      const result = await postJson("/stepwise/test", {});
      const count = Array.isArray(result?.items) ? result.items.length : 0;
      const error = result?.status === "failed" ? result.message || "测试失败" : result?.error || "";
      state.stepwiseTestFailed = !!error;
      state.stepwiseTestMessage = error || (result?.disabled ? "功能已关闭" : `连接正常 · ${count} 条建议`);
    } catch (error) {
      state.stepwiseTestFailed = true;
      state.stepwiseTestMessage = error?.message || "测试失败";
    } finally {
      state.stepwiseTestBusy = false;
      refreshCodexPlusServiceSection("stepwise");
    }
  }

  /** 设置页 change 事件：输入框失焦 / 回车、下拉选择。处理了返回 true。 */
  function handleCodexPlusServiceSettingsChange(target) {
    const dictationField = target?.closest?.("[data-codex-plus-dictation-field]");
    if (dictationField) {
      const key = dictationField.getAttribute("data-codex-plus-dictation-field");
      if (key === "timeoutSeconds") {
        const value = clampCodexPlusServiceNumber(dictationField.value, { min: 1, max: 600 }, codexPlusDictationSettings().timeoutSeconds ?? 120);
        dictationField.value = String(value);
        void setCodexPlusDictationFields({ timeoutSeconds: value }, false);
      } else {
        // 地址、环境变量改了会影响「配置不完整」提示，重建一下这节。
        void setCodexPlusDictationFields({ [key]: dictationField.value.trim() }, key === "baseUrl" || key === "model" || key === "apiKeyEnv");
      }
      return true;
    }
    const presetSelect = target?.closest?.("[data-codex-plus-dictation-preset]");
    if (presetSelect) {
      const preset = codexPlusDictationPresets[presetSelect.value];
      // 自定义保留当前输入，只是让用户自己改地址。
      if (preset) void setCodexPlusDictationFields({ ...preset }, true);
      return true;
    }
    const secret = target?.closest?.("[data-codex-plus-dictation-secret], [data-codex-plus-stepwise-secret]");
    if (secret) {
      const value = secret.value.trim();
      // 留空表示不改。
      if (!value) return true;
      secret.value = "";
      if (secret.hasAttribute("data-codex-plus-dictation-secret")) void setCodexPlusDictationFields({ apiKey: value }, true);
      else void writeCodexPlusServiceSettings({ codexAppStepwiseApiKeyConfigured: true }, { codexAppStepwiseApiKey: value }, "stepwise");
      return true;
    }
    const stepwiseField = target?.closest?.("[data-codex-plus-stepwise-field]");
    if (stepwiseField) {
      const key = stepwiseField.getAttribute("data-codex-plus-stepwise-field");
      const range = codexPlusStepwiseNumberFields[key];
      let value = stepwiseField.value.trim();
      if (range) {
        value = clampCodexPlusServiceNumber(value, range, Number(codexPlusBackendSettings[key]) || range.min);
        stepwiseField.value = String(value);
      }
      void writeCodexPlusServiceSettings({ [key]: value }, { [key]: value }, "");
      return true;
    }
    const stepwiseSelect = target?.closest?.("[data-codex-plus-stepwise-select]");
    if (stepwiseSelect) {
      const key = stepwiseSelect.getAttribute("data-codex-plus-stepwise-select");
      void writeCodexPlusServiceSettings({ [key]: stepwiseSelect.value }, { [key]: stepwiseSelect.value }, "");
      return true;
    }
    return !!target?.closest?.("[data-codex-plus-dictation-hotkey]");
  }

  /** 设置页 click 事件里本节的按钮。处理了返回 true。 */
  function handleCodexPlusServiceSettingsClick(target) {
    if (target?.closest?.("[data-codex-plus-dictation-hotkey-save]")) {
      if (!codexPlusServiceSettingsState.hotkeyBusy) void saveCodexPlusGlobalHotkey(codexPlusServiceSettingsState.hotkey?.enabled === true);
      return true;
    }
    if (target?.closest?.("[data-codex-plus-dictation-hotkey-toggle]")) {
      if (!codexPlusServiceSettingsState.hotkeyBusy) void saveCodexPlusGlobalHotkey(codexPlusServiceSettingsState.hotkey?.enabled !== true);
      return true;
    }
    if (target?.closest?.("[data-codex-plus-dictation-local-model-download]")) {
      if (codexPlusServiceSettingsState.localModel?.phase !== "downloading") void startCodexPlusLocalModelDownload();
      return true;
    }
    if (target?.closest?.("[data-codex-plus-native-browser-refresh]")) {
      void refreshCodexPlusNativeBrowserStatus();
      return true;
    }
    if (target?.closest?.("[data-codex-plus-stepwise-test]")) {
      void testCodexPlusStepwiseConnection();
      return true;
    }
    const clear = target?.closest?.("[data-codex-plus-dictation-secret-clear], [data-codex-plus-stepwise-secret-clear]");
    if (clear) {
      const dictation = clear.hasAttribute("data-codex-plus-dictation-secret-clear");
      void Promise.resolve(codexPlusAsk({
        title: "清除 API Key",
        message: dictation ? "清除后语音输入改用环境变量里的 Key（如果设置了）。" : "清除后下一步建议改用环境变量里的 Key（如果设置了）。",
        options: [{ value: "clear", label: "清除" }],
      })).then((answer) => {
        if (answer !== "clear") return;
        if (dictation) void setCodexPlusDictationFields({ apiKey: "" }, true);
        else void writeCodexPlusServiceSettings({ codexAppStepwiseApiKeyConfigured: false }, { codexAppStepwiseApiKey: "" }, "stepwise");
      });
      return true;
    }
    return false;
  }

  /**
   * 打开 Codex++ 设置页并滚到某一节（悬浮球的「配置」用它，不再打开管理器）。
   * 清单可能还在读，节要等它渲染出来，所以短时间内重试几次。
   */
  function openCodexPlusServiceSettings(section) {
    openCodexPlusPage();
    let attempts = 0;
    const scrollWhenReady = () => {
      const node = document.querySelector(`[data-codex-plus-service-section="${section}"]`);
      if (node) {
        // 设置页按分组分页：先切到这一节所在的分组，再滚过去。
        const group = node.closest("[data-codex-plus-settings-group]")?.getAttribute("data-codex-plus-settings-group");
        if (group && typeof selectCodexPlusSettingsGroup === "function") selectCodexPlusSettingsGroup(group);
        node.scrollIntoView({ block: "start" });
        return;
      }
      if (++attempts < 25) setTimeout(scrollWhenReady, 120);
    };
    setTimeout(scrollWhenReady, 0);
  }
  window.__codexPlusOpenServiceSettings = openCodexPlusServiceSettings;
