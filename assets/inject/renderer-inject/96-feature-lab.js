  /**
   * 内置拓展「Feature Lab」。
   *
   * Codex 的 app-server 已经暴露了 `experimentalFeature/list`：按 stage（stable / beta /
   * underDevelopment / deprecated / removed）、默认值、说明列出所有 feature。原生设置页只把
   * `stage === "beta"` 的一小部分画出来，而且排除了 multi_agent / plugins / remote_control /
   * realtime_* 等；这里补一个完整面板，方便排查「原生有、界面不出现」的功能。
   *
   * 写开关走 `config/batchWrite` 的 `features.<name>`（和原生 Fast 补丁同一路径），
   * 不直接改 config.toml。改动需要重启 Codex 才生效，面板里会明确提示。
   */
  const codexPlusFeatureLabStageMeta = {
    stable: { label: "稳定", tone: "stable", order: 0 },
    beta: { label: "Beta", tone: "beta", order: 1 },
    underDevelopment: { label: "开发中", tone: "dev", order: 2 },
    deprecated: { label: "已弃用", tone: "deprecated", order: 3 },
    removed: { label: "已移除", tone: "removed", order: 4 },
  };
  const codexPlusFeatureLabStageOrder = ["stable", "beta", "underDevelopment", "deprecated", "removed"];
  const codexPlusFeatureLabRiskyStages = new Set(["underDevelopment", "deprecated", "removed"]);
  const codexPlusFeatureLabNamePattern = /^[A-Za-z][A-Za-z0-9_]*$/;

  function codexPlusFeatureLabStage(raw) {
    const stage = String(raw || "");
    return Object.prototype.hasOwnProperty.call(codexPlusFeatureLabStageMeta, stage) ? stage : "underDevelopment";
  }

  /** 把 app-server 的 ExperimentalFeature 归一化；字段以后可能增加，缺的给空值。 */
  function codexPlusFeatureLabNormalizeFeature(raw) {
    if (!raw || typeof raw !== "object" || typeof raw.name !== "string") return null;
    const name = raw.name.trim();
    if (!codexPlusFeatureLabNamePattern.test(name)) return null;
    const stage = codexPlusFeatureLabStage(raw.stage);
    const title = typeof raw.displayName === "string" && raw.displayName.trim()
      ? raw.displayName.trim()
      : typeof raw.title === "string" && raw.title.trim()
        ? raw.title.trim()
        : name;
    const description = typeof raw.description === "string" && raw.description.trim()
      ? raw.description.trim()
      : typeof raw.announcement === "string" ? raw.announcement.trim() : "";
    return {
      name,
      title,
      stage,
      description,
      enabled: raw.enabled === true,
      defaultEnabled: raw.defaultEnabled === true,
      risky: codexPlusFeatureLabRiskyStages.has(stage),
      locked: stage === "removed",
    };
  }

  function codexPlusFeatureLabCompare(left, right) {
    const stage = codexPlusFeatureLabStageOrder.indexOf(left.stage) - codexPlusFeatureLabStageOrder.indexOf(right.stage);
    if (stage) return stage;
    return left.title.localeCompare(right.title) || left.name.localeCompare(right.name);
  }

  /** 拉全量 feature（分页），按 stage / 名称排序。同一个客户端可以重复调用。 */
  async function codexPlusFeatureLabFetch(client) {
    if (!client || typeof client.sendRequest !== "function") {
      throw new Error("还没有连上 Codex 的 app-server");
    }
    const features = new Map();
    let cursor = null;
    for (let page = 0; page < 20; page += 1) {
      const result = await client.sendRequest("experimentalFeature/list", { cursor, limit: 100 });
      const data = Array.isArray(result?.data) ? result.data : Array.isArray(result) ? result : [];
      for (const raw of data) {
        const feature = codexPlusFeatureLabNormalizeFeature(raw);
        if (feature && !features.has(feature.name)) features.set(feature.name, feature);
      }
      const next = typeof result?.nextCursor === "string" && result.nextCursor ? result.nextCursor : null;
      if (!next || next === cursor) break;
      cursor = next;
    }
    return [...features.values()].sort(codexPlusFeatureLabCompare);
  }

  /** 写入 `features.<name>`。原生 Fast 补丁用同一条 config/batchWrite 路径。 */
  async function codexPlusFeatureLabWrite(client, feature, enabled) {
    if (!feature || !codexPlusFeatureLabNamePattern.test(String(feature.name || ""))) {
      throw new Error("feature 名称不合法");
    }
    return client.sendRequest("config/batchWrite", {
      edits: [{
        keyPath: `features.${feature.name}`,
        value: enabled === true,
        mergeStrategy: "upsert",
      }],
      filePath: null,
      expectedVersion: null,
      reloadUserConfig: true,
    });
  }

  function codexPlusFeatureLabStyles() {
    return `
      .codex-plus-feature-lab {
        display: flex; flex-direction: column; gap: 12px;
        box-sizing: border-box; width: 100%; max-width: 920px;
        min-height: 100%; padding: 18px 22px 36px;
        color: light-dark(#1f2328, #e8e8e8);
        background: transparent;
        font: 13px/1.5 -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      }
      .codex-plus-feature-lab * { box-sizing: border-box; }
      .codex-plus-feature-lab h2 { margin: 0; font-size: 17px; font-weight: 600; }
      .codex-plus-feature-lab p { margin: 0; }
      .codex-plus-feature-lab .cpl-sub { color: light-dark(#57606a, #9aa4ad); }
      .codex-plus-feature-lab .cpl-toolbar { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
      .codex-plus-feature-lab input[type="search"],
      .codex-plus-feature-lab select {
        height: 30px; padding: 0 9px; border-radius: 7px;
        border: 1px solid light-dark(#d0d7de, #3a3f45);
        background: light-dark(#fff, #1f2328);
        color: inherit; font: inherit; outline: none;
      }
      .codex-plus-feature-lab input[type="search"] { flex: 1 1 240px; min-width: 160px; }
      .codex-plus-feature-lab button {
        height: 30px; padding: 0 11px; border-radius: 7px; cursor: pointer;
        border: 1px solid light-dark(#d0d7de, #3a3f45);
        background: light-dark(#f6f8fa, #262b31);
        color: inherit; font: inherit;
      }
      .codex-plus-feature-lab button:hover:not(:disabled) { background: light-dark(#eef1f4, #2f353c); }
      .codex-plus-feature-lab button:disabled { cursor: default; opacity: .55; }
      .codex-plus-feature-lab .cpl-status { min-height: 18px; color: light-dark(#57606a, #9aa4ad); }
      .codex-plus-feature-lab .cpl-status[data-tone="error"] { color: light-dark(#b42318, #ff8a80); }
      .codex-plus-feature-lab .cpl-status[data-tone="ok"] { color: light-dark(#1a7f37, #7ee2a8); }
      .codex-plus-feature-lab .cpl-list { display: flex; flex-direction: column; gap: 7px; }
      .codex-plus-feature-lab .cpl-row {
        display: flex; gap: 14px; align-items: flex-start;
        padding: 11px 13px; border-radius: 9px;
        border: 1px solid light-dark(#e1e7ec, #333940);
        background: light-dark(#fff, #20252b);
      }
      .codex-plus-feature-lab .cpl-row[data-enabled="true"] { border-color: light-dark(#b7dfc4, #2f6b45); }
      .codex-plus-feature-lab .cpl-main { min-width: 0; flex: 1 1 auto; display: flex; flex-direction: column; gap: 3px; }
      .codex-plus-feature-lab .cpl-title { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; font-weight: 600; }
      .codex-plus-feature-lab .cpl-name { font: 11px/1.4 ui-monospace, SFMono-Regular, Menlo, monospace; color: light-dark(#6e7781, #8b949e); }
      .codex-plus-feature-lab .cpl-desc { color: light-dark(#57606a, #b6bec6); }
      .codex-plus-feature-lab .cpl-meta { font-size: 11px; color: light-dark(#8c959f, #8b949e); }
      .codex-plus-feature-lab .cpl-stage {
        display: inline-flex; align-items: center; height: 18px; padding: 0 6px;
        border-radius: 999px; font-size: 11px; font-weight: 500;
        border: 1px solid transparent;
      }
      .codex-plus-feature-lab .cpl-stage[data-tone="stable"] { color: light-dark(#1a7f37, #7ee2a8); background: light-dark(#dafbe1, #1b3326); }
      .codex-plus-feature-lab .cpl-stage[data-tone="beta"] { color: light-dark(#8a6d00, #f2cc60); background: light-dark(#fff8c5, #3a2f10); }
      .codex-plus-feature-lab .cpl-stage[data-tone="dev"] { color: light-dark(#9a3412, #ffb27d); background: light-dark(#ffebe9, #421f18); }
      .codex-plus-feature-lab .cpl-stage[data-tone="deprecated"] { color: light-dark(#6e7781, #b6bec6); background: light-dark(#eef1f4, #2c3238); }
      .codex-plus-feature-lab .cpl-stage[data-tone="removed"] { color: light-dark(#b42318, #ff8a80); background: light-dark(#ffebe9, #421f18); }
      .codex-plus-feature-lab .cpl-switch {
        position: relative; flex: 0 0 auto; width: 38px; height: 22px; padding: 0;
        border-radius: 999px; border: 1px solid light-dark(#c8d0d8, #454b52);
        background: light-dark(#eaeef2, #333940); transition: background .15s ease;
      }
      .codex-plus-feature-lab .cpl-switch::after {
        content: ""; position: absolute; top: 2px; left: 2px; width: 16px; height: 16px;
        border-radius: 50%; background: light-dark(#fff, #d8dde2);
        box-shadow: 0 1px 2px rgb(0 0 0 / .28); transition: transform .15s ease;
      }
      .codex-plus-feature-lab .cpl-switch[aria-checked="true"] {
        border-color: transparent; background: light-dark(#2da44e, #3fb950);
      }
      .codex-plus-feature-lab .cpl-switch[aria-checked="true"]::after { transform: translateX(16px); background: #fff; }
      .codex-plus-feature-lab .cpl-empty { padding: 22px 0; color: light-dark(#57606a, #9aa4ad); }
      .codex-plus-feature-lab .cpl-note {
        padding: 9px 11px; border-radius: 8px;
        background: light-dark(#f6f8fa, #262b31);
        color: light-dark(#57606a, #b6bec6); font-size: 12px;
      }
    `;
  }

  function codexPlusFeatureLabEnsureStyles() {
    const id = "codex-plus-feature-lab-style";
    if (document.getElementById(id)) return;
    const style = document.createElement("style");
    style.id = id;
    style.textContent = codexPlusFeatureLabStyles();
    document.head.appendChild(style);
  }

  /** 设置页分区本体。渲染是瞬态：每次打开都重新拉一次列表。 */
  function codexPlusFeatureLabRender({ container, ctx }) {
    if (!container) return;
    codexPlusFeatureLabEnsureStyles();
    const root = document.createElement("div");
    root.className = "codex-plus-feature-lab";
    root.innerHTML = `
      <div>
        <h2>Feature Lab</h2>
        <p class="cpl-sub">Codex app-server 暴露的全部实验开关。改动写入 config.toml 的 <code>features.&lt;name&gt;</code>，重启 Codex 后生效。</p>
      </div>
      <div class="cpl-toolbar">
        <input type="search" placeholder="搜索名称或说明" aria-label="搜索 feature" />
        <select aria-label="按阶段过滤">
          <option value="all">全部阶段</option>
          <option value="stable">稳定</option>
          <option value="beta">Beta</option>
          <option value="underDevelopment">开发中</option>
          <option value="deprecated">已弃用</option>
          <option value="removed">已移除</option>
        </select>
        <button type="button" class="cpl-reload">重新读取</button>
        <span class="cpl-count"></span>
      </div>
      <div class="cpl-status" role="status"></div>
      <div class="cpl-list"></div>
      <div class="cpl-note">开发中 / 已弃用的 feature 可能让会话异常或丢失数据。服务端校验的能力（云端任务、真实配额）不受这里影响。</div>
    `;
    container.replaceChildren(root);

    const search = root.querySelector("input[type=search]");
    const stageSelect = root.querySelector("select");
    const reload = root.querySelector(".cpl-reload");
    const status = root.querySelector(".cpl-status");
    const count = root.querySelector(".cpl-count");
    const list = root.querySelector(".cpl-list");
    const state = { features: [], query: "", stage: "all", loading: false };

    const setStatus = (message, tone = "") => {
      if (!status.isConnected) return;
      status.textContent = message || "";
      status.dataset.tone = tone;
    };

    const visibleFeatures = () => {
      const query = state.query.trim().toLowerCase();
      return state.features.filter((feature) => {
        if (state.stage !== "all" && feature.stage !== state.stage) return false;
        if (!query) return true;
        return `${feature.name} ${feature.title} ${feature.description}`.toLowerCase().includes(query);
      });
    };

    const draw = () => {
      if (!list.isConnected) return;
      const features = visibleFeatures();
      count.textContent = state.loading
        ? ""
        : `${features.length} / ${state.features.length} 项`;
      if (!features.length) {
        list.innerHTML = `<div class="cpl-empty">${state.loading ? "正在读取…" : "没有匹配的 feature"}</div>`;
        return;
      }
      const fragment = document.createDocumentFragment();
      for (const feature of features) {
        const row = document.createElement("div");
        row.className = "cpl-row";
        row.dataset.enabled = feature.enabled ? "true" : "false";
        row.dataset.name = feature.name;

        const main = document.createElement("div");
        main.className = "cpl-main";
        const title = document.createElement("div");
        title.className = "cpl-title";
        const titleText = document.createElement("span");
        titleText.textContent = feature.title;
        const name = document.createElement("span");
        name.className = "cpl-name";
        name.textContent = feature.name;
        const stage = document.createElement("span");
        stage.className = "cpl-stage";
        stage.dataset.tone = codexPlusFeatureLabStageMeta[feature.stage]?.tone || "dev";
        stage.textContent = codexPlusFeatureLabStageMeta[feature.stage]?.label || feature.stage;
        title.append(titleText, name, stage);

        const desc = document.createElement("div");
        desc.className = "cpl-desc";
        desc.textContent = feature.description || "（没有说明）";

        const meta = document.createElement("div");
        meta.className = "cpl-meta";
        const defaultText = feature.defaultEnabled ? "开" : "关";
        meta.textContent = `默认${defaultText}${feature.locked ? " · 已移除的 feature 不能启用" : ""}`;

        main.append(title, desc, meta);

        const toggle = document.createElement("button");
        toggle.type = "button";
        toggle.className = "cpl-switch";
        toggle.setAttribute("role", "switch");
        toggle.setAttribute("aria-checked", feature.enabled ? "true" : "false");
        toggle.setAttribute("aria-label", `${feature.title} 开关`);
        toggle.disabled = feature.locked;
        toggle.addEventListener("click", () => {
          void toggleFeature(feature, toggle);
        });

        row.append(main, toggle);
        fragment.appendChild(row);
      }
      list.replaceChildren(fragment);
    };

    const toggleFeature = async (feature, button) => {
      if (state.loading || feature.locked) return;
      const next = !feature.enabled;
      if (next && feature.risky) {
        const answer = await ctx.ask({
          title: `开启「${feature.title}」？`,
          message: `该 feature 标记为「${codexPlusFeatureLabStageMeta[feature.stage]?.label || feature.stage}」，可能让会话异常或丢失数据。确定要打开吗？`,
          options: [{ value: "enable", label: "仍然开启" }],
        });
        if (!answer || answer.value !== "enable") return;
      }
      button.disabled = true;
      setStatus(`正在写入 features.${feature.name}…`);
      try {
        const client = await codexPlusAppServerClient("local");
        await codexPlusFeatureLabWrite(client, feature, next);
        feature.enabled = next;
        button.setAttribute("aria-checked", next ? "true" : "false");
        button.closest(".cpl-row")?.setAttribute("data-enabled", next ? "true" : "false");
        setStatus(`已${next ? "开启" : "关闭"} ${feature.name}，重启 Codex 后生效。`, "ok");
      } catch (error) {
        setStatus(`写入失败：${error?.message || error}`, "error");
      } finally {
        button.disabled = feature.locked || state.loading;
      }
    };

    const load = async () => {
      state.loading = true;
      reload.disabled = true;
      setStatus("正在读取 feature 列表…");
      draw();
      try {
        const client = await codexPlusAppServerClient("local");
        state.features = await codexPlusFeatureLabFetch(client);
        state.loading = false;
        setStatus(state.features.length ? "" : "app-server 没有返回 feature。", state.features.length ? "" : "error");
      } catch (error) {
        state.loading = false;
        state.features = [];
        setStatus(`读取失败：${error?.message || error}`, "error");
      } finally {
        reload.disabled = false;
        draw();
      }
    };

    search.addEventListener("input", () => {
      state.query = search.value;
      draw();
    });
    stageSelect.addEventListener("change", () => {
      state.stage = stageSelect.value;
      draw();
    });
    reload.addEventListener("click", () => void load());
    void load();
  }

  function codexPlusFeatureLabExtension(ctx) {
    ctx.registerSettingsSection({
      id: "feature-lab",
      title: "Feature Lab",
      order: 90,
      render: ({ container }) => codexPlusFeatureLabRender({ container, ctx }),
    });
  }
