  // 拓展详情页的主体（形态对齐 VS Code 的扩展详情）：「细节」显示拓展自己的 README，「设置」显示拓展登记的设置面板
  // 或按清单 config 生成的表单，「功能」列出拓展接管了哪些能力；右侧栏是安装与市场信息。
  // 头部（图标、名称、开关、卸载）仍在 renderCodexPlusExtensionsDetail 里，这里只管头部下面的部分。

  /** 详情当前页签：readme / settings / features。换选中项时回到「细节」。 */
  let codexPlusExtensionsDetailTab = "readme";
  /** README 缓存：`<key>@<版本>` → { state: loading | ok | failed, markdown, assets, truncated, message }。 */
  const codexPlusExtensionReadmes = new Map();

  function resetCodexPlusExtensionsDetailTab() {
    codexPlusExtensionsDetailTab = "readme";
  }

  /** 「刷新」时清掉 README 缓存，改过的 README 下次打开重新读。 */
  function clearCodexPlusExtensionReadmes() {
    codexPlusExtensionReadmes.clear();
  }

  function codexPlusExtensionReadmeCacheKey(local) {
    return `${local.key}@${local.package?.version || ""}`;
  }

  async function loadCodexPlusExtensionReadme(local) {
    const cacheKey = codexPlusExtensionReadmeCacheKey(local);
    if (codexPlusExtensionReadmes.has(cacheKey)) return;
    codexPlusExtensionReadmes.set(cacheKey, { state: "loading" });
    let entry;
    try {
      const result = await postJson("/user-scripts/readme", { key: local.key });
      entry = result?.status === "ok"
        ? { state: "ok", markdown: String(result.markdown || ""), assets: result.assets && typeof result.assets === "object" ? result.assets : {}, truncated: !!result.truncated }
        : { state: "failed", message: result?.message || "README 读取失败" };
    } catch (error) {
      entry = { state: "failed", message: String(error?.message || error || "README 读取失败") };
    }
    codexPlusExtensionReadmes.set(cacheKey, entry);
    // 只换 README 那一块，不重画整个详情：设置面板、滚动位置都不动。
    document.querySelectorAll("[data-codex-extensions-readme]").forEach((node) => {
      if (node.getAttribute("data-codex-extensions-readme") === local.key) node.innerHTML = renderCodexPlusExtensionReadmeBody(local, entry);
    });
  }

  /** 「细节」页签：有 README 显示 README；没有就显示市场清单里的使用要求、已知限制。 */
  function renderCodexPlusExtensionReadmeBody(local, entry) {
    if (!entry || entry.state === "loading") return '<div class="codex-plus-extensions-readme-hint" role="status">正在读取 README…</div>';
    if (entry.state === "failed") return `<div class="codex-plus-extensions-detail-error">${escapeHtml(entry.message)}</div>`;
    if (!entry.markdown.trim()) return "";
    const truncated = entry.truncated ? '<div class="codex-plus-extensions-readme-hint">README 太长，只显示了前 512 KB。</div>' : "";
    return `<div class="codex-plus-markdown">${renderCodexPlusMarkdown(entry.markdown, { assets: entry.assets })}</div>${truncated}`;
  }

  function renderCodexPlusExtensionDetailsTab(local, marketItem) {
    const list = (title, items) => items?.length
      ? `<div class="codex-plus-extensions-detail-section"><div class="codex-plus-extensions-detail-section-title">${escapeHtml(title)}</div><ul>${items.map((item) => `<li>${escapeHtml(item)}</li>`).join("")}</ul></div>`
      : "";
    const facts = `${list("使用要求", marketItem?.requirements)}${list("已知限制", marketItem?.limitations)}`;
    if (local?.package?.readme) {
      const entry = codexPlusExtensionReadmes.get(codexPlusExtensionReadmeCacheKey(local));
      if (!entry) void loadCodexPlusExtensionReadme(local);
      return `<div data-codex-extensions-readme="${escapeHtml(local.key)}">${renderCodexPlusExtensionReadmeBody(local, entry)}</div>${facts}`;
    }
    const empty = local
      ? local.kind === "package" ? "这个拓展没有提供 README。在拓展包目录里放一个 README.md，就会显示在这里。" : "单文件脚本没有 README，说明见上方简介。"
      : "";
    return facts || `<div class="codex-plus-extensions-readme-hint">${escapeHtml(empty || "这个拓展没有更多说明。")}</div>`;
  }

  /** 多语言文本（字符串或 `{ "zh-CN", zh, en }`）取中文优先。 */
  function codexPlusExtensionText(value) {
    if (value && typeof value === "object") {
      return value["zh-CN"] || value.zh || value.en || Object.values(value).find((item) => typeof item === "string") || "";
    }
    return value == null ? "" : String(value);
  }

  /** 拓展有没有可设置的内容：登记了设置面板、声明了设置页或 config。 */
  function codexPlusExtensionHasSettings(local) {
    if (!local || local.kind !== "package") return false;
    const panel = typeof codexPlusSettingsPanelFor === "function" ? codexPlusSettingsPanelFor(local.key) : null;
    return !!(panel || local.package?.settingsPage || local.package?.configSchema?.length);
  }

  /**
   * 「设置」页签：拓展在运行且登记了设置面板（ctx.registerSettingsPanel）就留挂载点给它画；
   * 否则按清单 config 生成表单（停用的拓展也能改），声明了自己的设置界面的再提示一句启用后能看到。
   */
  function renderCodexPlusExtensionSettingsTab(local) {
    const panel = typeof codexPlusSettingsPanelFor === "function" ? codexPlusSettingsPanelFor(local.key) : null;
    if (panel && local.enabled) {
      return `<div class="codex-plus-extensions-detail-settings-host" data-codex-plus-settings-panel="${escapeHtml(local.key)}" ${codexPlusExtensionConstants.extensionAttribute}="${escapeHtml(local.key)}"></div>`;
    }
    const ownUi = panel || local.package?.settingsPage;
    const hint = ownUi
      ? `<div class="codex-plus-extensions-detail-settings-hint">${local.enabled ? "这个拓展加载后，这里会显示它自己的设置界面。" : "启用这个拓展后，这里会显示它自己的设置界面。"}${local.package?.configSchema?.length ? "下面是按清单生成的通用设置。" : ""}</div>`
      : "";
    return `${hint}${local.package?.configSchema?.length ? renderCodexPlusExtensionConfigForm(local) : ""}`;
  }

  /** 按清单 config 生成的设置表单。改一项就把整份配置存回去（后台按整份覆盖）。 */
  function renderCodexPlusExtensionConfigForm(local) {
    const config = local.package?.config && typeof local.package.config === "object" ? local.package.config : {};
    const rows = (local.package?.configSchema || []).map((field, index) => {
      const id = `codex-plus-ext-config-${index}`;
      const value = config[field.key] !== undefined ? config[field.key] : field.default;
      const attrs = `id="${id}" data-codex-ext-config-field="${escapeHtml(field.key)}" data-codex-ext-config-type="${escapeHtml(field.type)}"`;
      let control;
      if (field.type === "boolean") {
        control = `<button type="button" class="codex-plus-toggle" ${attrs} data-enabled="${String(value === true)}" role="switch" aria-checked="${String(value === true)}"><span></span></button>`;
      } else if (field.type === "select") {
        const options = (field.options || []).map((option) => {
          const raw = JSON.stringify(option?.value ?? null);
          const selected = raw === JSON.stringify(value ?? null) ? " selected" : "";
          return `<option value="${escapeHtml(raw)}"${selected}>${escapeHtml(codexPlusExtensionText(option?.label) || String(option?.value ?? ""))}</option>`;
        }).join("");
        control = `<select class="codex-plus-ext-config-input" ${attrs}>${options}</select>`;
      } else if (field.type === "number") {
        const min = field.min == null ? "" : ` min="${escapeHtml(field.min)}"`;
        const max = field.max == null ? "" : ` max="${escapeHtml(field.max)}"`;
        control = `<input type="number" step="any" class="codex-plus-ext-config-input codex-plus-ext-config-number" ${attrs}${min}${max} value="${escapeHtml(value ?? "")}" />`;
      } else if (field.type === "text") {
        control = `<textarea class="codex-plus-ext-config-input" rows="4" spellcheck="false" ${attrs}>${escapeHtml(value ?? "")}</textarea>`;
      } else if (field.type === "array" || field.type === "object") {
        control = `<textarea class="codex-plus-ext-config-input codex-plus-ext-config-json" rows="6" spellcheck="false" ${attrs}>${escapeHtml(JSON.stringify(value ?? (field.type === "array" ? [] : {}), null, 2))}</textarea>`;
      } else {
        control = `<input type="${field.secret ? "password" : "text"}" autocomplete="off" spellcheck="false" class="codex-plus-ext-config-input" ${attrs} value="${escapeHtml(value ?? "")}" />`;
      }
      const description = codexPlusExtensionText(field.description);
      const wide = ["text", "array", "object"].includes(field.type);
      return `
        <div class="codex-plus-ext-config-row" data-wide="${String(wide)}">
          <div class="codex-plus-ext-config-label">
            <label for="${id}">${escapeHtml(codexPlusExtensionText(field.title) || field.key)}</label>
            ${description ? `<div class="codex-plus-ext-config-description">${escapeHtml(description)}</div>` : ""}
            ${field.type === "array" || field.type === "object" ? '<div class="codex-plus-ext-config-description">JSON 格式，失焦后保存。</div>' : ""}
          </div>
          <div class="codex-plus-ext-config-control">${control}</div>
        </div>
      `;
    }).join("");
    return `<div class="codex-plus-ext-config" data-codex-ext-config="${escapeHtml(local.key)}">${rows}<div class="codex-plus-ext-config-status" role="status" data-codex-ext-config-status="true"></div></div>`;
  }

  /** 从控件读出新值；读不出（数字不合法、JSON 写错）时抛错给状态行显示。 */
  function codexPlusExtensionConfigValue(control) {
    const type = control.getAttribute("data-codex-ext-config-type");
    if (type === "boolean") return control.getAttribute("data-enabled") !== "true";
    if (type === "select") return JSON.parse(control.value);
    if (type === "number") {
      if (control.value.trim() === "" || !Number.isFinite(Number(control.value))) throw new Error("请填一个数字");
      return Number(control.value);
    }
    if (type === "array" || type === "object") {
      let value;
      try {
        value = JSON.parse(control.value || (type === "array" ? "[]" : "{}"));
      } catch {
        throw new Error("JSON 格式不对");
      }
      if (type === "array" ? !Array.isArray(value) : !value || typeof value !== "object" || Array.isArray(value)) {
        throw new Error(type === "array" ? "要填一个 JSON 数组" : "要填一个 JSON 对象");
      }
      return value;
    }
    return control.value;
  }

  async function saveCodexPlusExtensionConfigField(control) {
    const form = control.closest("[data-codex-ext-config]");
    const key = form?.getAttribute("data-codex-ext-config") || "";
    const local = (codexPlusUserScripts.scripts || []).find((script) => script.key === key);
    const status = form?.querySelector("[data-codex-ext-config-status]");
    if (!local || !status) return;
    const setStatus = (text, kind) => {
      status.textContent = text;
      status.dataset.kind = kind;
    };
    let value;
    try {
      value = codexPlusExtensionConfigValue(control);
    } catch (error) {
      setStatus(error.message, "error");
      return;
    }
    const field = control.getAttribute("data-codex-ext-config-field");
    const next = { ...(local.package?.config || {}), [field]: value };
    if (control.getAttribute("data-codex-ext-config-type") === "boolean") {
      control.setAttribute("data-enabled", String(value));
      control.setAttribute("aria-checked", String(value));
    }
    setStatus("正在保存…", "pending");
    try {
      await saveCodexPlusExtensionConfig(key, next);
      setStatus("已保存", "ok");
    } catch (error) {
      setStatus(String(error?.message || error), "error");
    }
  }

  /** 「功能」页签：拓展清单里声明接管的能力（对应 VS Code 的 Feature Contributions）。 */
  function codexPlusExtensionFeatures(local) {
    const pkg = local?.package;
    if (!pkg) return [];
    const names = (items) => (Array.isArray(items) ? items : []).map((item) => (typeof item === "string" ? item : item?.name || item?.id || "")).filter(Boolean);
    const rows = [];
    const add = (label, value) => {
      if (Array.isArray(value) ? value.length : value) rows.push([label, Array.isArray(value) ? value.join("、") : String(value)]);
    };
    add("页面脚本", pkg.renderer ? `${pkg.renderer} 个文件` : "");
    add("模型工具", names(pkg.tools));
    add("Codex 钩子", names(pkg.hookEvents));
    const pipeline = pkg.toolPipeline || {};
    add("工具调用管线", [
      pipeline.preExecute ? `执行前 ${pipeline.preExecute} 个` : "",
      pipeline.execute ? `接管执行 ${pipeline.execute} 个` : "",
      pipeline.postExecute ? `结果改写 ${pipeline.postExecute} 个` : "",
    ].filter(Boolean));
    add("网关中间件", [
      pkg.gatewayRequest ? `请求 ${pkg.gatewayRequest} 个` : "",
      pkg.gatewayResponse ? `响应流 ${pkg.gatewayResponse} 个` : "",
      pkg.gatewayError ? `失败重试 ${pkg.gatewayError} 个` : "",
    ].filter(Boolean));
    add("关掉的内置网关环节", names(pkg.gatewayReplaces));
    add("模型供应商", (pkg.providers || []).map((provider) => provider?.name || provider?.id).filter(Boolean));
    add("常驻后台", pkg.backend ? [pkg.http ? "后台脚本（处理 HTTP 请求）" : "后台脚本"] : []);
    add("提供的服务", names(pkg.services));
    add("定时任务", (pkg.schedules || []).length ? `${pkg.schedules.length} 个` : "");
    const codex = pkg.codex || {};
    add("Skills", names(codex.skills));
    add("子 Agent", names(codex.agents));
    add("MCP 服务器", names(codex.mcpServers));
    add("提示词", names(codex.prompts));
    add("全局指令", codex.instructions ? "有" : "");
    add("顶替的内置拓展", names(pkg.replaces));
    add("管理器面板", pkg.managerPage ? [pkg.managerPage.title || "面板"] : []);
    add("页面最早期脚本", pkg.preload?.scripts ? `${pkg.preload.scripts} 个` : "");
    add("启动前后", [
      pkg.launch?.before ? "启动前脚本" : "",
      pkg.launch?.after ? "退出后脚本" : "",
      pkg.launch?.args?.length ? `启动参数 ${pkg.launch.args.join(" ")}` : "",
    ].filter(Boolean));
    add("dsh 插件", pkg.dsh ? "有" : "");
    add("设置项", (pkg.configSchema || []).map((field) => codexPlusExtensionText(field.title) || field.key));
    add("可调用的数据路由", names(pkg.routes));
    return rows;
  }

  function renderCodexPlusExtensionFeaturesTab(rows) {
    return `<table class="codex-plus-extensions-features"><tbody>${rows.map(([label, value]) => `<tr><th scope="row">${escapeHtml(label)}</th><td>${escapeHtml(value)}</td></tr>`).join("")}</tbody></table>`;
  }

  /** 右侧栏：安装信息、市场信息、链接、标签。 */
  function renderCodexPlusExtensionAside(local, marketItem, sourceLabel) {
    const table = (title, rows) => {
      const shown = rows.filter(([, value]) => value);
      return shown.length
        ? `<section><h3>${escapeHtml(title)}</h3><dl>${shown.map(([label, value]) => `<dt>${escapeHtml(label)}</dt><dd>${escapeHtml(value)}</dd>`).join("")}</dl></section>`
        : "";
    };
    const install = local ? table("安装", [
      ["标识符", local.key],
      ["版本", local.package?.version || local.version || ""],
      ["来源", sourceLabel],
      ["状态", userScriptStatusLabel(local.status)],
    ]) : "";
    const market = marketItem ? table("市场", [
      ["作者", marketItem.author || ""],
      ["最新版本", marketItem.version ? `v${marketItem.version}` : ""],
      ["更新时间", marketItem.updated_at || ""],
    ]) : "";
    const links = [
      [marketItem?.homepage || local?.homepage, "主页"],
      [local?.source_url || marketItem?.script_url, "源文件"],
    ].filter(([url]) => /^https?:\/\//i.test(String(url || "")));
    const resources = links.length
      ? `<section><h3>资源</h3><ul class="codex-plus-extensions-aside-links">${links.map(([url, label]) => `<li><a href="${escapeHtml(url)}" target="_blank" rel="noreferrer">${escapeHtml(label)}</a></li>`).join("")}</ul></section>`
      : "";
    const tags = marketItem?.tags?.length
      ? `<section><h3>标签</h3><div class="codex-plus-extensions-detail-tags">${marketItem.tags.map((tag) => `<span>${escapeHtml(tag)}</span>`).join("")}</div></section>`
      : "";
    return `${install}${market}${resources}${tags}`;
  }

  /** 头部下面的部分：页签 + 内容 + 右侧栏。 */
  function renderCodexPlusExtensionDetailTabs({ local, marketItem, sourceLabel }) {
    const tabs = [{ id: "readme", label: "细节" }];
    if (codexPlusExtensionHasSettings(local)) tabs.push({ id: "settings", label: "设置" });
    const features = codexPlusExtensionFeatures(local);
    if (features.length) tabs.push({ id: "features", label: "功能" });
    const active = tabs.some((tab) => tab.id === codexPlusExtensionsDetailTab) ? codexPlusExtensionsDetailTab : "readme";
    const body = (id) => {
      if (id === "settings") return renderCodexPlusExtensionSettingsTab(local);
      if (id === "features") return renderCodexPlusExtensionFeaturesTab(features);
      return renderCodexPlusExtensionDetailsTab(local, marketItem);
    };
    const aside = renderCodexPlusExtensionAside(local, marketItem, sourceLabel);
    return `
      <div class="codex-plus-extensions-detail-tabs" role="tablist" aria-label="拓展详情">
        ${tabs.map((tab) => `<button type="button" role="tab" id="codex-plus-extensions-tab-${tab.id}" aria-controls="codex-plus-extensions-tabpanel-${tab.id}" aria-selected="${String(tab.id === active)}" data-codex-extensions-tab="${tab.id}">${tab.label}</button>`).join("")}
      </div>
      <div class="codex-plus-extensions-detail-columns">
        <div class="codex-plus-extensions-detail-main">
          ${tabs.map((tab) => `<div class="codex-plus-extensions-detail-tabpanel" role="tabpanel" id="codex-plus-extensions-tabpanel-${tab.id}" aria-labelledby="codex-plus-extensions-tab-${tab.id}" data-codex-extensions-tabpanel="${tab.id}"${tab.id === active ? "" : " hidden"}>${body(tab.id)}</div>`).join("")}
          ${local?.error ? `<div class="codex-plus-extensions-detail-error">${escapeHtml(local.error)}</div>` : ""}
        </div>
        ${aside ? `<aside class="codex-plus-extensions-detail-aside">${aside}</aside>` : ""}
      </div>
    `;
  }

  /** 切页签只改显隐，不重画：设置面板与 README 都留在原地。 */
  function selectCodexPlusExtensionsDetailTab(root, tab) {
    const tabs = Array.from(root?.querySelectorAll?.("[data-codex-extensions-tab]") || []).filter((button) => button.getAttribute("role") === "tab");
    if (!tabs.some((button) => button.getAttribute("data-codex-extensions-tab") === tab)) return false;
    codexPlusExtensionsDetailTab = tab;
    tabs.forEach((button) => button.setAttribute("aria-selected", String(button.getAttribute("data-codex-extensions-tab") === tab)));
    root.querySelectorAll("[data-codex-extensions-tabpanel]").forEach((panel) => {
      panel.hidden = panel.getAttribute("data-codex-extensions-tabpanel") !== tab;
    });
    return true;
  }

  /** 详情里的点击：页签、头部的「设置」、配置开关、README 里的页内锚点。处理了返回 true。 */
  function handleCodexPlusExtensionDetailClick(target, event) {
    const tabButton = target?.closest?.("[data-codex-extensions-tab]");
    if (tabButton) {
      const root = tabButton.closest("[data-codex-plus-extensions-detail]");
      selectCodexPlusExtensionsDetailTab(root, tabButton.getAttribute("data-codex-extensions-tab"));
      if (tabButton.getAttribute("role") !== "tab") root?.querySelector(".codex-plus-extensions-detail-tabs")?.scrollIntoView?.({ block: "nearest" });
      return true;
    }
    const toggle = target?.closest?.('[data-codex-ext-config-type="boolean"]');
    if (toggle) {
      void saveCodexPlusExtensionConfigField(toggle);
      return true;
    }
    const anchor = target?.closest?.("[data-codex-plus-md-anchor]");
    if (anchor) {
      // 不能让页内锚点改 Codex 页面的地址，自己滚到 README 里对应的标题。
      event?.preventDefault?.();
      const id = `codex-plus-md-${anchor.getAttribute("data-codex-plus-md-anchor")}`;
      const heading = Array.from(anchor.closest(".codex-plus-markdown")?.querySelectorAll("[id]") || []).find((node) => node.id === id);
      heading?.scrollIntoView?.({ block: "start", behavior: "smooth" });
      return true;
    }
    return false;
  }

  /** 配置表单里输入框 / 下拉 / 文本框的 change（失焦或回车）就保存。 */
  function handleCodexPlusExtensionConfigChange(target) {
    const control = target?.closest?.("[data-codex-ext-config-field]");
    if (!control || control.getAttribute("data-codex-ext-config-type") === "boolean") return false;
    void saveCodexPlusExtensionConfigField(control);
    return true;
  }

  /**
   * 拓展 README 的 Markdown 渲染。原样先转义，原始 HTML 一律当文字显示（HTML 注释直接去掉）；
   * 链接只放行 http(s)、mailto 和页内锚点，图片只放行 https、常见图片 data URL 和包内 `assets/` 资源
   * （`options.assets`：后台随 README 给的 路径 → data URL）。覆盖 README 常用语法：标题（含 Setext）、段落、
   * 无序/有序/任务列表（可嵌套）、引用、围栏/缩进代码块、表格、分隔线，行内代码、粗体、斜体、删除线、链接、图片、自动链接、换行。
   */
  function renderCodexPlusMarkdown(source, options = {}) {
    const assets = options.assets && typeof options.assets === "object" ? options.assets : {};
    const text = String(source || "").replace(/\r\n?/g, "\n").replace(/<!--[\s\S]*?-->/g, "").replace(/\t/g, "    ");
    const slugs = new Map();
    const slug = (value) => {
      const base = String(value).toLowerCase().replace(/<[^>]*>/g, "").replace(/&[a-z#0-9]+;/g, "").trim()
        .replace(/[^\p{L}\p{N}\s-]/gu, "").replace(/\s+/g, "-") || "section";
      const count = slugs.get(base) || 0;
      slugs.set(base, count + 1);
      return count ? `${base}-${count}` : base;
    };
    const safeHref = (url) => {
      const value = String(url || "").trim();
      if (/^(https?:|mailto:)/i.test(value)) return { href: value, external: true };
      if (/^#[^\s]*$/.test(value)) return { anchor: value.slice(1) };
      return null;
    };
    const safeSrc = (url) => {
      const value = String(url || "").trim();
      if (/^https:\/\//i.test(value) || /^data:image\/(png|jpe?g|gif|webp);base64,[a-z0-9+/=]+$/i.test(value)) return value;
      const relative = value.replace(/^\.?\//, "");
      return typeof assets[relative] === "string" ? assets[relative] : "";
    };

    const inline = (raw) => {
      const stash = [];
      const keep = (html) => `\u0000${stash.push(html) - 1}\u0000`;
      let value = String(raw);
      value = value.replace(/(`+)([\s\S]*?[^`])\1(?!`)/g, (_, _ticks, code) => keep(`<code>${escapeHtml(code.trim())}</code>`));
      value = value.replace(/\\([\\`*_{}\[\]()#+\-.!|~<>])/g, (_, char) => keep(escapeHtml(char)));
      value = value.replace(/!\[([^\]]*)\]\(\s*<?((?:[^()\s<>]|\([^()\s]*\))+)>?(?:\s+"([^"]*)")?\s*\)/g, (_, alt, url, title) => {
        const src = safeSrc(url);
        if (!src) return keep(escapeHtml(alt));
        return keep(`<img src="${escapeHtml(src)}" alt="${escapeHtml(alt)}"${title ? ` title="${escapeHtml(title)}"` : ""} loading="lazy" />`);
      });
      value = value.replace(/<(https?:\/\/[^\s>]+)>/g, (_, url) => keep(`<a href="${escapeHtml(url)}" target="_blank" rel="noreferrer">${escapeHtml(url)}</a>`));
      value = value.replace(/\[((?:[^\[\]]|\[[^\]]*\])+)\]\(\s*<?((?:[^()\s<>]|\([^()\s]*\))+)>?(?:\s+"([^"]*)")?\s*\)/g, (_, label, url) => {
        const link = safeHref(url);
        const body = inline(label);
        if (!link) return keep(body);
        if (link.anchor !== undefined) return keep(`<a href="#" data-codex-plus-md-anchor="${escapeHtml(link.anchor)}">${body}</a>`);
        return keep(`<a href="${escapeHtml(link.href)}" target="_blank" rel="noreferrer">${body}</a>`);
      });
      value = value.replace(/https?:\/\/[^\s<>()\u0000]+[^\s<>()\u0000.,;:!?'"]/g, (url) => keep(`<a href="${escapeHtml(url)}" target="_blank" rel="noreferrer">${escapeHtml(url)}</a>`));
      value = escapeHtml(value)
        .replace(/(\*\*|__)(?=\S)([\s\S]*?\S)\1/g, "<strong>$2</strong>")
        .replace(/~~(?=\S)([\s\S]*?\S)~~/g, "<del>$1</del>")
        .replace(/\*(?=[^\s*])([\s\S]*?[^\s*])\*/g, "<em>$1</em>")
        .replace(/(^|[^\p{L}\p{N}_])_(?=\S)([\s\S]*?\S)_(?![\p{L}\p{N}_])/gu, "$1<em>$2</em>")
        .replace(/(?: {2,}|\\)\n/g, "<br />")
        .replace(/\n/g, " ");
      let guard = 0;
      while (value.includes("\u0000") && guard++ < 50) value = value.replace(/\u0000(\d+)\u0000/g, (_, index) => stash[Number(index)] ?? "");
      return value;
    };

    const fenceRe = /^ {0,3}(`{3,}|~{3,})\s*([^`\s]*)/;
    const headingRe = /^ {0,3}(#{1,6})(?:\s+(.*?))?\s*#*\s*$/;
    const hrRe = /^ {0,3}([-*_])(?:\s*\1){2,}\s*$/;
    const listRe = /^( *)([-*+]|\d{1,9}[.)])(?:\s+(.*)|$)/;
    const quoteRe = /^ {0,3}>/;
    const tableSepRe = /^\s*\|?\s*:?-{1,}:?\s*(\|\s*:?-{1,}:?\s*)*\|?\s*$/;
    const leading = (line) => line.match(/^ */)[0].length;
    const isBlockStart = (line) => fenceRe.test(line) || headingRe.test(line) || hrRe.test(line) || listRe.test(line) || quoteRe.test(line);
    const splitRow = (line) => line.trim().replace(/^\|/, "").replace(/(?<!\\)\|$/, "").split(/(?<!\\)\|/).map((cell) => cell.trim());

    const blocks = (lines) => {
      const out = [];
      let i = 0;
      while (i < lines.length) {
        const line = lines[i];
        if (!line.trim()) { i++; continue; }
        const fence = line.match(fenceRe);
        if (fence) {
          const marker = fence[1];
          const body = [];
          i++;
          while (i < lines.length && !new RegExp(`^ {0,3}${marker[0] === "`" ? "`" : "~"}{${marker.length},}\\s*$`).test(lines[i])) body.push(lines[i++]);
          i++;
          const lang = fence[2] ? ` data-lang="${escapeHtml(fence[2])}"` : "";
          out.push(`<pre><code${lang}>${escapeHtml(body.join("\n"))}</code></pre>`);
          continue;
        }
        if (leading(line) >= 4) {
          const body = [];
          while (i < lines.length && (leading(lines[i]) >= 4 || !lines[i].trim())) body.push(lines[i++].slice(4));
          while (body.length && !body[body.length - 1].trim()) body.pop();
          out.push(`<pre><code>${escapeHtml(body.join("\n"))}</code></pre>`);
          continue;
        }
        const heading = line.match(headingRe);
        if (heading) {
          const level = heading[1].length;
          const html = inline(heading[2] || "");
          out.push(`<h${level} id="codex-plus-md-${escapeHtml(slug(html))}">${html}</h${level}>`);
          i++;
          continue;
        }
        if (hrRe.test(line)) { out.push("<hr />"); i++; continue; }
        if (quoteRe.test(line)) {
          const body = [];
          while (i < lines.length && lines[i].trim() && (quoteRe.test(lines[i]) || !isBlockStart(lines[i]))) {
            body.push(lines[i].replace(/^ {0,3}> ?/, ""));
            i++;
          }
          out.push(`<blockquote>${blocks(body)}</blockquote>`);
          continue;
        }
        const list = line.match(listRe);
        if (list) {
          const indent = list[1].length;
          const ordered = /\d/.test(list[2]);
          const start = ordered ? parseInt(list[2], 10) : 1;
          const items = [];
          let loose = false;
          while (i < lines.length) {
            if (!lines[i].trim()) {
              let next = i;
              while (next < lines.length && !lines[next].trim()) next++;
              const sibling = lines[next]?.match(listRe);
              if (!sibling || sibling[1].length !== indent || /\d/.test(sibling[2]) !== ordered) break;
              loose = true;
              i = next;
            }
            const item = lines[i].match(listRe);
            if (!item || item[1].length !== indent || /\d/.test(item[2]) !== ordered) break;
            const contentIndent = item[1].length + item[2].length + 1;
            const body = [item[3] || ""];
            i++;
            while (i < lines.length) {
              const current = lines[i];
              if (!current.trim()) {
                let next = i;
                while (next < lines.length && !lines[next].trim()) next++;
                if (next < lines.length && leading(lines[next]) > indent && !(listRe.test(lines[next]) && leading(lines[next]) === indent)) {
                  body.push("");
                  loose = true;
                  i++;
                  continue;
                }
                break;
              }
              if (leading(current) > indent) { body.push(current.slice(Math.min(leading(current), contentIndent))); i++; continue; }
              if (!isBlockStart(current) && !(i + 1 < lines.length && tableSepRe.test(lines[i + 1]) && current.includes("|"))) { body.push(current.trim()); i++; continue; }
              break;
            }
            items.push(body);
          }
          const html = items.map((body) => {
            let first = body[0];
            let check = "";
            const task = first.match(/^\[([ xX])\]\s+/);
            if (task) {
              check = `<input type="checkbox" disabled${task[1] === " " ? "" : " checked"} /> `;
              first = first.slice(task[0].length);
            }
            let content = blocks([first, ...body.slice(1)]);
            if (!loose) content = content.replace(/<p>([\s\S]*?)<\/p>/g, "$1");
            return `<li${task ? ' class="codex-plus-markdown-task"' : ""}>${check}${content}</li>`;
          }).join("");
          out.push(ordered ? `<ol${start !== 1 ? ` start="${start}"` : ""}>${html}</ol>` : `<ul>${html}</ul>`);
          continue;
        }
        if (line.includes("|") && i + 1 < lines.length && tableSepRe.test(lines[i + 1]) && lines[i + 1].includes("-")) {
          const head = splitRow(line);
          const aligns = splitRow(lines[i + 1]).map((cell) => (cell.startsWith(":") && cell.endsWith(":") ? "center" : cell.endsWith(":") ? "right" : cell.startsWith(":") ? "left" : ""));
          i += 2;
          const rows = [];
          while (i < lines.length && lines[i].trim() && lines[i].includes("|")) rows.push(splitRow(lines[i++]));
          const cell = (tag, value, index) => `<${tag}${aligns[index] ? ` style="text-align:${aligns[index]}"` : ""}>${inline(value || "")}</${tag}>`;
          out.push(`<div class="codex-plus-markdown-table"><table><thead><tr>${head.map((value, index) => cell("th", value, index)).join("")}</tr></thead><tbody>${rows.map((row) => `<tr>${head.map((_, index) => cell("td", row[index], index)).join("")}</tr>`).join("")}</tbody></table></div>`);
          continue;
        }
        const paragraph = [];
        while (i < lines.length && lines[i].trim()) {
          if (paragraph.length && /^ {0,3}(=+|-+)\s*$/.test(lines[i])) break;
          if (paragraph.length && (isBlockStart(lines[i]) || (lines[i].includes("|") && i + 1 < lines.length && tableSepRe.test(lines[i + 1]) && lines[i + 1].includes("-")))) break;
          paragraph.push(paragraph.length ? lines[i].replace(/^ +/, "") : lines[i].trim());
          i++;
        }
        const setext = i < lines.length ? lines[i].match(/^ {0,3}(=+|-+)\s*$/) : null;
        if (setext && paragraph.length) {
          const level = setext[1][0] === "=" ? 1 : 2;
          const html = inline(paragraph.join("\n"));
          out.push(`<h${level} id="codex-plus-md-${escapeHtml(slug(html))}">${html}</h${level}>`);
          i++;
          continue;
        }
        out.push(`<p>${inline(paragraph.join("\n"))}</p>`);
      }
      return out.join("");
    };
    return blocks(text.split("\n"));
  }
