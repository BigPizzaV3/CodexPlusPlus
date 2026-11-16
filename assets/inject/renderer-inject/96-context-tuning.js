  /**
   * 内置拓展「上下文与压缩」。
   *
   * Codex 自带、但原生界面没有入口的几项会话设置：压缩提示词、回合结束后压缩、工具输出上限，
   * 以及给第三方模型精简系统提示的 include_* 开关。实测（CLI 0.169）都写进 config.toml、不用重启，
   * 新会话立即生效；已打开的会话保持原配置。
   *
   * 写入分两步：
   * 1. 经 Codex 的 `config/batchWrite` 写 live config.toml，再 `config/read` 回读。batchWrite 只查类型，
   *    超范围的值（阈值 150）照样写进去，之后每次 `thread/start` 都失败——所以先在这里校验范围，
   *    回读失败或值对不上就把改过的键写回原值。
   * 2. 同步进 Codex++ 的通用配置（`/session-tuning/write`）。否则切换供应商重写 config.toml 时，
   *    这些键会被回存进旧供应商的快照、换到别的供应商就没了。
   */
  const codexPlusContextTuningFields = [
    { key: "compact_prompt", kind: "text", maxChars: 20000 },
    { key: "model_post_turn_compact_threshold_percent", kind: "int", min: 1, max: 100 },
    { key: "tool_output_token_limit", kind: "int", min: 1, max: 10000000 },
    // include_* 默认是 true；界面上勾选「不发送」= 写 false，取消勾选 = 删掉键回到默认。
    { key: "include_environment_context", kind: "drop", label: "环境信息", hint: "工作目录、shell、日期等，约 900 字" },
    { key: "include_environment_context_time", kind: "drop", label: "环境信息里的当前时间", hint: "时间每轮都变，去掉后提示前缀更稳定，更容易命中缓存" },
    { key: "include_permissions_instructions", kind: "drop", label: "沙箱与审批说明", hint: "约 580 字" },
    { key: "include_apps_instructions", kind: "drop", label: "应用（连接器）使用说明", hint: "没装连接器时本来就很短" },
    { key: "include_collaboration_mode_instructions", kind: "drop", label: "协作模式说明", hint: "计划模式下约 8.8K 字；去掉后计划模式的行为约束也一起没了" },
  ];

  const codexPlusContextTuningTemplates = [
    {
      id: "handoff",
      label: "交接摘要：决定与待办",
      text: [
        "你在为同一任务的下一段工作写交接摘要。读者是接手的模型，它看不到之前的对话。",
        "按下面的小节输出，没有内容的小节写「无」：",
        "1. 目标：用户要完成什么，验收标准是什么。",
        "2. 已做的决定和理由，包括否掉的方案及原因。",
        "3. 当前进度：已完成的、正在做的。",
        "4. 待办：按顺序列出下一步，标出卡住的地方。",
        "5. 关键事实：文件路径、命令、报错原文、版本号、用户明确提出的要求和偏好。",
        "只写接手需要的信息，不要寒暄，不要复述工具输出全文。",
      ].join("\n"),
    },
    {
      id: "files",
      label: "交接摘要：文件与命令",
      text: [
        "把这段对话总结成交接说明，读者是接手的模型，它看不到之前的对话。",
        "必须包含：",
        "- 用户的目标和约束；",
        "- 新建、修改、删除过的每个文件：路径 + 一句话说明改了什么；",
        "- 有用的命令（构建、测试、启动）和它们最近一次的结果；",
        "- 还没解决的报错，原样摘录；",
        "- 剩下的步骤，按顺序。",
        "简洁，不要寒暄，不要贴完整的工具输出。",
      ].join("\n"),
    },
  ];

  function codexPlusContextTuningField(key) {
    return codexPlusContextTuningFields.find((field) => field.key === key) || null;
  }

  /** config/read 里的生效值 → 规范化的「写入值」：null 表示没设置（用 Codex 默认）。 */
  function codexPlusContextTuningValueFromConfig(field, raw) {
    if (field.kind === "drop") return raw === false ? false : null;
    if (field.kind === "int") return Number.isInteger(raw) ? raw : null;
    return typeof raw === "string" && raw.trim() ? raw : null;
  }

  function codexPlusContextTuningValuesFromConfig(config) {
    const values = {};
    for (const field of codexPlusContextTuningFields) {
      values[field.key] = codexPlusContextTuningValueFromConfig(field, config?.[field.key]);
    }
    return values;
  }

  /** 规范化的值 → 表单草稿：文本 / 数字是输入框里的字符串，drop 是「不发送」勾选状态。 */
  function codexPlusContextTuningDraftFromValues(values) {
    const draft = {};
    for (const field of codexPlusContextTuningFields) {
      const value = values?.[field.key] ?? null;
      if (field.kind === "drop") draft[field.key] = value === false;
      else draft[field.key] = value == null ? "" : String(value);
    }
    return draft;
  }

  /** 草稿 → { values, errors }。errors 按键给出中文原因。 */
  function codexPlusContextTuningParseDraft(draft) {
    const values = {};
    const errors = {};
    for (const field of codexPlusContextTuningFields) {
      const raw = draft?.[field.key];
      if (field.kind === "drop") {
        values[field.key] = raw === true ? false : null;
        continue;
      }
      const text = String(raw ?? "").trim();
      if (!text) {
        values[field.key] = null;
        continue;
      }
      if (field.kind === "text") {
        if ([...text].length > field.maxChars) errors[field.key] = `太长了（最多 ${field.maxChars} 字）`;
        values[field.key] = text;
        continue;
      }
      if (!/^\d+$/.test(text)) {
        errors[field.key] = "请填整数";
        values[field.key] = null;
        continue;
      }
      const number = Number(text);
      if (number < field.min || number > field.max) errors[field.key] = `范围 ${field.min}–${field.max}`;
      values[field.key] = number;
    }
    return { values, errors };
  }

  /** 和已读到的值比，只返回改过的键：[{ key, value }]。 */
  function codexPlusContextTuningEdits(loaded, values) {
    return codexPlusContextTuningFields
      .filter((field) => (loaded?.[field.key] ?? null) !== (values?.[field.key] ?? null))
      .map((field) => ({ key: field.key, value: values?.[field.key] ?? null }));
  }

  /** [{ key, value }] → config/batchWrite 的 edits；null 用 replace 删键。 */
  function codexPlusContextTuningBatchEdits(edits) {
    return edits.map(({ key, value }) => ({
      keyPath: key,
      value: value ?? null,
      mergeStrategy: value == null ? "replace" : "upsert",
    }));
  }

  /** 回读的 config 和期望不一致的键。 */
  function codexPlusContextTuningMismatches(config, edits) {
    return edits
      .filter(({ key, value }) => {
        const field = codexPlusContextTuningField(key);
        return field && codexPlusContextTuningValueFromConfig(field, config?.[key]) !== (value ?? null);
      })
      .map(({ key }) => key);
  }

  async function codexPlusContextTuningBackend(path, payload) {
    if (typeof postJson !== "function") throw new Error("Codex++ 后端还没就绪");
    const result = await postJson(path, payload || {});
    if (!result || result.status !== "ok") throw new Error(result?.message || "Codex++ 后端没有响应");
    return result;
  }

  async function codexPlusContextTuningConfigRead(client) {
    const result = await client.sendRequest("config/read", { includeLayers: false });
    return result?.config || {};
  }

  /**
   * 写 live 并回读；回读失败或值对不上就把这些键写回 previous 里的原值。返回回读到的 config。
   */
  async function codexPlusContextTuningApply(client, edits, previous) {
    const write = (items) => client.sendRequest("config/batchWrite", {
      edits: codexPlusContextTuningBatchEdits(items),
      filePath: null,
      expectedVersion: null,
      reloadUserConfig: true,
    });
    await write(edits);
    let config = null;
    let failure = "";
    try {
      config = await codexPlusContextTuningConfigRead(client);
      const wrong = codexPlusContextTuningMismatches(config, edits);
      if (wrong.length) failure = `写入后读到的值不一致：${wrong.join("、")}`;
    } catch (error) {
      failure = `Codex 读不了写入后的配置：${error?.message || error}`;
    }
    if (!failure) return config;
    try {
      await write(edits.map(({ key }) => ({ key, value: previous?.[key] ?? null })));
    } catch (error) {
      failure += `；恢复原值也失败了：${error?.message || error}`;
      throw new Error(failure);
    }
    throw new Error(`${failure}。已恢复原值。`);
  }

  // 改下面任何 CSS 都要把它 +1：已注入过的页面靠它判断旧 <style> 是否过期。
  const codexPlusContextTuningStyleVersion = "1";

  function codexPlusContextTuningStyles() {
    return `
      .codex-plus-context-tuning { display: flex; flex-direction: column; gap: 12px; width: 100%; box-sizing: border-box;
        color: var(--codex-plus-text); font-size: 13px; line-height: 18px; }
      .codex-plus-context-tuning * { box-sizing: border-box; }
      .codex-plus-context-tuning h3 { margin: 4px 0 0; font-size: 13px; font-weight: 500; }
      .codex-plus-context-tuning p { margin: 0; }
      .codex-plus-context-tuning .cct-sub { color: var(--codex-plus-text-secondary); }
      .codex-plus-context-tuning .cct-card { display: flex; flex-direction: column; padding: 0 16px;
        border: 1px solid var(--codex-plus-settings-card-border); border-radius: var(--codex-plus-card-radius);
        background: var(--codex-plus-settings-card-bg); }
      .codex-plus-context-tuning .cct-card > * + * { border-top: 1px solid var(--codex-plus-settings-card-border); }
      .codex-plus-context-tuning .cct-row { display: flex; flex-direction: column; gap: 6px; padding: 12px 0; }
      .codex-plus-context-tuning .cct-line { display: flex; gap: 12px; align-items: center; }
      .codex-plus-context-tuning .cct-label { flex: 1 1 auto; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
      .codex-plus-context-tuning .cct-title { font-weight: 500; }
      .codex-plus-context-tuning .cct-hint { color: var(--codex-plus-text-tertiary); font-size: 12px; line-height: 16px; }
      .codex-plus-context-tuning .cct-error { color: var(--codex-plus-danger); font-size: 12px; line-height: 16px; }
      .codex-plus-context-tuning .cct-error:empty { display: none; }
      .codex-plus-context-tuning input[type="text"], .codex-plus-context-tuning textarea {
        min-height: 28px; padding: 4px 10px; border-radius: var(--codex-plus-control-radius);
        border: 1px solid var(--codex-plus-control-border); background: var(--codex-plus-control-bg);
        color: inherit; font: inherit; outline: none; }
      .codex-plus-context-tuning input:focus, .codex-plus-context-tuning textarea:focus { border-color: var(--codex-plus-focus); }
      .codex-plus-context-tuning input[data-invalid="true"], .codex-plus-context-tuning textarea[data-invalid="true"] { border-color: var(--codex-plus-danger); }
      .codex-plus-context-tuning .cct-number { flex: 0 0 150px; width: 150px; }
      .codex-plus-context-tuning textarea { width: 100%; min-height: 120px; resize: vertical; font: 12px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace; }
      .codex-plus-context-tuning input[type="checkbox"] { appearance: auto !important; margin: 0; flex: 0 0 auto; }
      .codex-plus-context-tuning .cct-toolbar { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
      .codex-plus-context-tuning button { height: 28px; padding: 0 12px; border: 0; border-radius: var(--codex-plus-control-radius);
        background: var(--codex-plus-button-bg); color: inherit; font: inherit; cursor: pointer; }
      .codex-plus-context-tuning button:hover:not(:disabled) { background: var(--codex-plus-button-hover-bg); }
      .codex-plus-context-tuning button:disabled { opacity: .55; cursor: default; }
      .codex-plus-context-tuning button.cct-primary, .codex-plus-context-tuning button.cct-primary:hover:not(:disabled) {
        background: var(--codex-plus-text); color: var(--codex-plus-bg-primary); }
      .codex-plus-context-tuning button.cct-primary:hover:not(:disabled) { opacity: .85; }
      .codex-plus-context-tuning .cct-status { min-height: 16px; font-size: 12px; line-height: 16px; color: var(--codex-plus-text-secondary); white-space: pre-wrap; }
      .codex-plus-context-tuning .cct-status:empty { display: none; }
      .codex-plus-context-tuning [data-tone="error"] { color: var(--codex-plus-danger); }
      .codex-plus-context-tuning [data-tone="ok"] { color: var(--codex-plus-success); }
      .codex-plus-context-tuning [data-tone="warn"] { color: var(--codex-plus-warning); }
      .codex-plus-context-tuning .cct-note { font-size: 12px; line-height: 16px; color: var(--codex-plus-warning); }
      .codex-plus-context-tuning .cct-note:empty { display: none; }
    `;
  }

  function codexPlusContextTuningEnsureStyles() {
    const id = "codex-plus-context-tuning-style";
    const existing = document.getElementById(id);
    if (existing?.dataset.codexPlusStyleVersion === codexPlusContextTuningStyleVersion) return;
    existing?.remove();
    const style = document.createElement("style");
    style.id = id;
    style.dataset.codexPlusStyleVersion = codexPlusContextTuningStyleVersion;
    style.setAttribute("data-codex-plus-ext", "context-tuning");
    style.textContent = codexPlusContextTuningStyles();
    document.head.appendChild(style);
  }

  function codexPlusContextTuningEl(tag, props = {}, ...children) {
    const node = document.createElement(tag);
    for (const [key, value] of Object.entries(props)) {
      if (value === undefined || value === null || value === false) continue;
      if (key === "className") node.className = value;
      else if (key === "text") node.textContent = value;
      else if (key.startsWith("on") && typeof value === "function") node.addEventListener(key.slice(2), value);
      else if (key in node && typeof value !== "string") node[key] = value;
      else node.setAttribute(key, value === true ? "" : String(value));
    }
    for (const child of children.flat()) {
      if (child === null || child === undefined || child === false) continue;
      node.append(child instanceof Node ? child : String(child));
    }
    return node;
  }

  function codexPlusContextTuningRender({ container }) {
    if (!container) return;
    codexPlusContextTuningEnsureStyles();
    const el = codexPlusContextTuningEl;
    const state = { loaded: null, busy: false };
    const inputs = {};
    const errors = {};

    const status = el("div", { className: "cct-status", role: "status" });
    const scopeNote = el("div", { className: "cct-note" });
    const setStatus = (message, tone = "") => {
      status.textContent = message || "";
      status.dataset.tone = tone;
    };

    const errorLine = (key) => (errors[key] = el("div", { className: "cct-error" }));
    const numberRow = (key, title, hint, placeholder, unit) => {
      inputs[key] = el("input", { type: "text", inputMode: "numeric", className: "cct-number", placeholder, "aria-label": title });
      return el(
        "div",
        { className: "cct-row" },
        el(
          "div",
          { className: "cct-line" },
          el("div", { className: "cct-label" }, el("span", { className: "cct-title", text: title }), el("span", { className: "cct-hint", text: hint })),
          inputs[key],
          unit ? el("span", { className: "cct-hint", text: unit }) : null,
        ),
        errorLine(key),
      );
    };

    inputs.compact_prompt = el("textarea", {
      placeholder: "留空 = 用 Codex 内置的压缩提示词",
      "aria-label": "压缩提示词",
      spellcheck: "false",
    });
    const promptRow = el(
      "div",
      { className: "cct-row" },
      el(
        "div",
        { className: "cct-label" },
        el("span", { className: "cct-title", text: "压缩提示词" }),
        el("span", { className: "cct-hint", text: "压缩（自动或 /compact）时让模型按这段要求写摘要，摘要会替换掉之前的对话。想让第三方模型用中文摘要、或者保住决定和待办时用得上。" }),
      ),
      inputs.compact_prompt,
      errorLine("compact_prompt"),
      el(
        "div",
        { className: "cct-toolbar" },
        el("span", { className: "cct-hint", text: "套用模板：" }),
        codexPlusContextTuningTemplates.map((template) => el("button", {
          type: "button",
          text: template.label,
          onclick: () => {
            inputs.compact_prompt.value = template.text;
            validate();
          },
        })),
        el("button", { type: "button", text: "清空", onclick: () => {
          inputs.compact_prompt.value = "";
          validate();
        } }),
      ),
    );

    const dropRows = codexPlusContextTuningFields
      .filter((field) => field.kind === "drop")
      .map((field) => {
        inputs[field.key] = el("input", { type: "checkbox", "aria-label": `不发送${field.label}` });
        return el(
          "label",
          { className: "cct-row" },
          el(
            "div",
            { className: "cct-line" },
            inputs[field.key],
            el("div", { className: "cct-label" }, el("span", { className: "cct-title", text: `不发送${field.label}` }), el("span", { className: "cct-hint", text: field.hint })),
          ),
        );
      });

    const saveButton = el("button", { type: "button", className: "cct-primary", text: "保存", onclick: () => void save() });
    const reloadButton = el("button", { type: "button", text: "重新读取", onclick: () => void load() });

    const root = el(
      "div",
      { className: "codex-plus-context-tuning" },
      el("p", {
        className: "cct-sub",
        text: "Codex 自带、但设置里没有入口的几项。保存后新会话立即生效，已打开的会话保持原样；同时写进 Codex++ 通用配置，切换供应商不会丢。",
      }),
      scopeNote,
      el("h3", { text: "压缩" }),
      el(
        "div",
        { className: "cct-card" },
        promptRow,
        numberRow(
          "model_post_turn_compact_threshold_percent",
          "回合结束后压缩",
          "一轮结束时上下文用量超过模型窗口的这个比例，就先压缩再等下一条消息，避免下一轮做到一半才压缩。和供应商里的「自动压缩阈值」（回合中途触发）是两个机制。",
          "不设置（关闭）",
          "%",
        ),
        numberRow(
          "tool_output_token_limit",
          "工具输出上限",
          "命令输出、读文件等单次工具结果进上下文前截到多少 token。调小更省上下文，但模型可能看不到完整输出。",
          "默认（随模型）",
          "token",
        ),
      ),
      el("h3", { text: "精简系统提示" }),
      el("p", { className: "cct-hint", text: "每轮都会随请求发出去的说明。官方模型建议保留；第三方模型上下文小、或者不认这些说明时再去掉。" }),
      el("div", { className: "cct-card" }, dropRows),
      el("div", { className: "cct-toolbar" }, saveButton, reloadButton),
      status,
    );
    container.replaceChildren(root);

    const readDraft = () => {
      const draft = {};
      for (const field of codexPlusContextTuningFields) {
        draft[field.key] = field.kind === "drop" ? inputs[field.key].checked : inputs[field.key].value;
      }
      return draft;
    };
    const fillDraft = (values) => {
      const draft = codexPlusContextTuningDraftFromValues(values);
      for (const field of codexPlusContextTuningFields) {
        if (field.kind === "drop") inputs[field.key].checked = draft[field.key];
        else inputs[field.key].value = draft[field.key];
      }
    };
    const validate = () => {
      const parsed = codexPlusContextTuningParseDraft(readDraft());
      for (const [key, node] of Object.entries(errors)) {
        node.textContent = parsed.errors[key] || "";
        inputs[key].dataset.invalid = parsed.errors[key] ? "true" : "false";
      }
      const edits = state.loaded ? codexPlusContextTuningEdits(state.loaded, parsed.values) : [];
      saveButton.disabled = state.busy || !state.loaded || Object.keys(parsed.errors).length > 0 || edits.length === 0;
      return { parsed, edits };
    };
    root.addEventListener("input", validate);
    root.addEventListener("change", validate);

    const showScope = (tuning) => {
      scopeNote.textContent = tuning && tuning.commonConfigApplies === false
        ? `当前供应商「${tuning.activeProfileName || "未命名"}」没开「使用通用配置」：这里的设置只跟着它走，换到别的供应商不生效。`
        : "";
    };

    const load = async () => {
      state.busy = true;
      validate();
      setStatus("读取中…");
      try {
        const client = await codexPlusAppServerClient("local");
        const config = await codexPlusContextTuningConfigRead(client);
        state.loaded = codexPlusContextTuningValuesFromConfig(config);
        fillDraft(state.loaded);
        setStatus("");
        try {
          showScope(await codexPlusContextTuningBackend("/session-tuning/read", {}));
        } catch (error) {
          scopeNote.textContent = `读不到 Codex++ 通用配置（${error?.message || error}），保存后切换供应商可能丢失这些设置。`;
        }
      } catch (error) {
        setStatus(`读取 Codex 配置失败：${error?.message || error}`, "error");
      } finally {
        state.busy = false;
        validate();
      }
    };

    const save = async () => {
      const { parsed, edits } = validate();
      if (!edits.length || Object.keys(parsed.errors).length) return;
      state.busy = true;
      validate();
      setStatus("保存中…");
      try {
        const client = await codexPlusAppServerClient("local");
        const config = await codexPlusContextTuningApply(client, edits, state.loaded);
        state.loaded = codexPlusContextTuningValuesFromConfig(config);
        fillDraft(state.loaded);
        try {
          showScope(await codexPlusContextTuningBackend("/session-tuning/write", { edits }));
          setStatus("已保存。新会话立即生效，已打开的会话保持原样。", "ok");
        } catch (error) {
          setStatus(`已对 Codex 生效，但没能同步进 Codex++ 通用配置（${error?.message || error}）：切换供应商后可能丢失。`, "warn");
        }
      } catch (error) {
        setStatus(`没有保存：${error?.message || error}`, "error");
      } finally {
        state.busy = false;
        validate();
      }
    };

    void load();
  }

  function codexPlusContextTuningExtension(ctx) {
    ctx.registerSettingsSection({
      id: "context-tuning",
      title: "上下文与压缩",
      order: 70,
      render: ({ container }) => codexPlusContextTuningRender({ container, ctx }),
    });
  }
