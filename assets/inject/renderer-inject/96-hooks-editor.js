  /**
   * 内置拓展「Hooks 编辑器」。
   *
   * Codex 原生设置页（编码 → Hooks）只能列出 hook、标记信任；hook 本身要手写 hooks.json。
   * 这里补上：按模板新建、编辑、删除、单条启停、喂示例事件试运行。
   *
   * 几条实测得来的约束（Codex 26.1111 / app-server 0.169）：
   * - 文件：用户层 `<CODEX_HOME>/hooks.json`，项目层 `<项目>/.codex/hooks.json`（项目在 config.toml 里受信任才加载）。
   *   格式 `{"hooks":{"<事件>":[{"matcher"?,"hooks":[handler…]}]}}`，这里按原始 JSON 改，未知字段原样保留。
   * - 每条 hook 的 key 是 `<规范化文件路径>:<snake 事件名>:<分组序号>:<handler 序号>`，开关与信任记录在
   *   config.toml 的 `[hooks.state."<key>"]` 里，**按位置记**：插入、删除、挪动都会让记录对到别的 hook 上。
   *   所以保存时按每条 handler 的原位置算出搬迁，一次 `config/batchWrite` 把记录挪到新 key。
   * - 信任哈希只看内容（事件、matcher、handler 字段），和位置无关；内容改了状态变「已改动」，要重新信任才运行。
   * - app-server 缓存 hook 集合，改了文件要发一次 `config/batchWrite`（`reloadUserConfig: true`）才重读；
   *   已打开的会话下一轮生效。
   * - prompt / agent 类型的 handler 当前版本会跳过（仍占序号），mcp_tool 只读展示。
   */
  const codexPlusHooksEditorEvents = [
    { name: "SessionStart", label: "会话开始", hint: "stdout 会作为上下文加入对话" },
    { name: "SessionEnd", label: "会话结束", hint: "" },
    { name: "UserPromptSubmit", label: "提交提问", hint: "stdout 会作为上下文加入对话" },
    { name: "PreToolUse", label: "工具调用前", hint: "matcher 匹配工具名（如 ^Bash$）；退出码 2 或 JSON deny 可拦截" },
    { name: "PermissionRequest", label: "请求权限", hint: "matcher 匹配工具名" },
    { name: "PostToolUse", label: "工具调用后", hint: "matcher 匹配工具名" },
    { name: "PreModelRequest", label: "模型请求前", hint: "" },
    { name: "PostModelRequest", label: "模型请求后", hint: "" },
    { name: "PreCompact", label: "压缩前", hint: "" },
    { name: "PostCompact", label: "压缩后", hint: "" },
    { name: "SubagentStart", label: "子 agent 开始", hint: "" },
    { name: "SubagentStop", label: "子 agent 结束", hint: "" },
    { name: "Stop", label: "回合结束", hint: "stdout 必须是 JSON 或留空；退出码 2 会让 agent 继续，容易死循环" },
    { name: "Interrupt", label: "中断", hint: "" },
  ];
  const codexPlusHooksEditorEventNames = new Set(codexPlusHooksEditorEvents.map((event) => event.name));
  const codexPlusHooksEditorTrustLabels = {
    trusted: "已信任",
    untrusted: "未信任，不会运行",
    modified: "内容已改，待重新信任",
    managed: "受管理",
  };
  const codexPlusHooksEditorRootsKey = "codexPlusHooksEditorRoots";

  function codexPlusHooksEditorEventLabel(name) {
    return codexPlusHooksEditorEvents.find((event) => event.name === name)?.label || name;
  }

  /** `PreToolUse` → `pre_tool_use`，与 hook key 里的事件段一致。 */
  function codexPlusHooksEditorSnake(event) {
    return String(event || "").replace(/([a-z0-9])([A-Z])/g, "$1_$2").toLowerCase();
  }

  function codexPlusHooksEditorKey(prefix, position) {
    const [event, group, handler] = String(position).split(":");
    return `${prefix}:${codexPlusHooksEditorSnake(event)}:${group}:${handler}`;
  }

  function codexPlusHooksEditorPlatform() {
    const agent = typeof navigator === "object" ? String(navigator.userAgent || navigator.platform || "") : "";
    if (/Windows/i.test(agent)) return "windows";
    if (/Mac/i.test(agent)) return "mac";
    return "linux";
  }

  /** 解析文件文本。空文本当作空文档；不是对象、hooks 不是对象时返回错误，交给原文编辑。 */
  function codexPlusHooksEditorParse(text) {
    if (!String(text || "").trim()) return { doc: { hooks: {} }, error: "" };
    let doc;
    try {
      doc = JSON.parse(text);
    } catch (error) {
      return { doc: null, error: `不是合法的 JSON：${error?.message || error}` };
    }
    if (!doc || typeof doc !== "object" || Array.isArray(doc)) return { doc: null, error: "顶层必须是 JSON 对象" };
    if (doc.hooks === undefined) doc.hooks = {};
    if (!doc.hooks || typeof doc.hooks !== "object" || Array.isArray(doc.hooks)) {
      return { doc: null, error: "hooks 必须是对象（事件名 → 分组数组）" };
    }
    for (const [event, groups] of Object.entries(doc.hooks)) {
      if (!Array.isArray(groups)) return { doc: null, error: `hooks.${event} 必须是数组` };
    }
    return { doc, error: "" };
  }

  /** 按文件里的顺序列出每个 handler 及其位置 `事件:分组:序号`。分组不是对象或没有 hooks 数组的跳过。 */
  function codexPlusHooksEditorRows(doc) {
    const rows = [];
    for (const [event, groups] of Object.entries(doc?.hooks || {})) {
      if (!Array.isArray(groups)) continue;
      groups.forEach((group, groupIndex) => {
        if (!group || typeof group !== "object" || !Array.isArray(group.hooks)) return;
        group.hooks.forEach((handler, handlerIndex) => {
          if (!handler || typeof handler !== "object") return;
          rows.push({
            event,
            groupIndex,
            handlerIndex,
            position: `${event}:${groupIndex}:${handlerIndex}`,
            group,
            handler,
            editable: handler.type === "command" || handler.type === undefined,
          });
        });
      });
    }
    return rows;
  }

  /** 记下每个 handler 对象读进来时的位置；之后增删挪动都按对象身份追踪。 */
  function codexPlusHooksEditorOrigins(doc) {
    const origins = new Map();
    for (const row of codexPlusHooksEditorRows(doc)) origins.set(row.handler, row.position);
    return origins;
  }

  /**
   * 保存时 `[hooks.state]` 的搬迁：返回 config/batchWrite 的 edits。
   * - 挪了位置的 handler：旧 key 的记录搬到新 key，旧 key 清掉（除非被别的 handler 搬进来）；
   * - 删掉的 handler：清掉旧 key；
   * - 新建的 handler：清掉落点上残留的旧记录，避免继承别人的开关与信任。
   * 与现有值相同的项不写。
   */
  function codexPlusHooksEditorMigrationEdits({ doc, origins, prefix, state }) {
    const current = state && typeof state === "object" ? state : {};
    const plan = new Map();
    const moves = [];
    const survivors = new Set();
    const fresh = [];
    for (const row of codexPlusHooksEditorRows(doc)) {
      const origin = origins.get(row.handler);
      if (!origin) {
        fresh.push(row.position);
        continue;
      }
      survivors.add(origin);
      if (origin !== row.position) moves.push([origin, row.position]);
    }
    for (const origin of origins.values()) {
      if (!survivors.has(origin)) plan.set(codexPlusHooksEditorKey(prefix, origin), null);
    }
    for (const [from] of moves) plan.set(codexPlusHooksEditorKey(prefix, from), null);
    for (const position of fresh) plan.set(codexPlusHooksEditorKey(prefix, position), null);
    for (const [from, to] of moves) {
      const value = current[codexPlusHooksEditorKey(prefix, from)];
      plan.set(codexPlusHooksEditorKey(prefix, to), value === undefined ? null : value);
    }
    const edits = [];
    for (const [key, value] of plan) {
      const existing = current[key];
      if (value === null && existing === undefined) continue;
      if (value !== null && JSON.stringify(existing) === JSON.stringify(value)) continue;
      edits.push({ keyPath: `hooks.state.${JSON.stringify(key)}`, value, mergeStrategy: "replace" });
    }
    return edits;
  }

  /** 新建 hook：追加成新分组，放在事件末尾，不挪动已有 hook 的位置。 */
  function codexPlusHooksEditorAdd(doc, { event, matcher, handler }) {
    if (!codexPlusHooksEditorEventNames.has(event)) throw new Error(`未知的事件：${event}`);
    doc.hooks = doc.hooks || {};
    const groups = Array.isArray(doc.hooks[event]) ? doc.hooks[event] : (doc.hooks[event] = []);
    const group = { hooks: [handler] };
    if (matcher) group.matcher = matcher;
    groups.push(group);
    return handler;
  }

  /** 删除一条 handler；分组空了删分组，事件空了删事件。 */
  function codexPlusHooksEditorRemove(doc, handler) {
    for (const [event, groups] of Object.entries(doc.hooks || {})) {
      if (!Array.isArray(groups)) continue;
      for (let index = 0; index < groups.length; index += 1) {
        const list = groups[index]?.hooks;
        if (!Array.isArray(list)) continue;
        const at = list.indexOf(handler);
        if (at < 0) continue;
        list.splice(at, 1);
        if (!list.length) groups.splice(index, 1);
        if (!groups.length) delete doc.hooks[event];
        return true;
      }
    }
    return false;
  }

  /** 把表单值写进 handler（就地改，保留对象身份与未知字段）；空值删掉字段。 */
  function codexPlusHooksEditorApplyFields(handler, fields) {
    if (handler.type === undefined) handler.type = "command";
    handler.command = String(fields.command || "");
    const optional = {
      commandWindows: String(fields.commandWindows || "").trim() ? String(fields.commandWindows) : undefined,
      statusMessage: String(fields.statusMessage || "").trim() || undefined,
      timeout: Number(fields.timeout) > 0 ? Math.floor(Number(fields.timeout)) : undefined,
      async: fields.async === true ? true : undefined,
    };
    for (const [key, value] of Object.entries(optional)) {
      if (value === undefined) delete handler[key];
      else handler[key] = value;
    }
    return handler;
  }

  /**
   * 编辑：事件与 matcher 没变（或分组里只有它一个）时就地改；否则从原分组挪出来、追加到目标事件末尾。
   * handler 对象不换，保存时开关记录跟着搬。
   */
  function codexPlusHooksEditorUpdate(doc, row, { event, matcher, fields }) {
    const nextMatcher = String(matcher || "");
    const sameEvent = row.event === event;
    const sameMatcher = String(row.group.matcher || "") === nextMatcher;
    codexPlusHooksEditorApplyFields(row.handler, fields);
    if (sameEvent && (sameMatcher || row.group.hooks.length === 1)) {
      if (nextMatcher) row.group.matcher = nextMatcher;
      else delete row.group.matcher;
      return row.handler;
    }
    codexPlusHooksEditorRemove(doc, row.handler);
    return codexPlusHooksEditorAdd(doc, { event, matcher: nextMatcher, handler: row.handler });
  }

  /**
   * 某个文件的 hook key 前缀。hooks/list 里有这个文件的 hook 时直接用它的 sourcePath；
   * 没有时（新文件、项目未受信任）按 Codex 的规则推：用户层规范化，项目层保持传入目录原样。
   */
  function codexPlusHooksEditorKeyPrefix(file, listedHooks) {
    if (!file) return "";
    const candidates = new Set([file.path, file.canonicalPath].filter(Boolean));
    for (const hook of listedHooks || []) {
      if (candidates.has(hook?.sourcePath)) return hook.sourcePath;
    }
    return file.scope === "user" ? file.canonicalPath || file.path : file.path || file.canonicalPath || "";
  }

  function codexPlusHooksEditorSerialize(doc) {
    return `${JSON.stringify(doc, null, 2)}\n`;
  }

  /** 模板：按平台给命令。都只写在用户确认后才保存。 */
  function codexPlusHooksEditorTemplates(platform = codexPlusHooksEditorPlatform()) {
    const notify = platform === "mac"
      ? `osascript -e 'display notification "这一轮跑完了" with title "Codex"'`
      : platform === "windows"
        ? `powershell -NoProfile -Command "[console]::beep(880,300)"`
        : `notify-send Codex "这一轮跑完了"`;
    return [
      {
        id: "stop-notify",
        title: "回合结束时提醒",
        event: "Stop",
        matcher: "",
        command: notify,
        timeout: 10,
        note: "Stop hook 的输出必须是 JSON 或留空；不要用退出码 2，否则 agent 会一直继续。",
      },
      {
        id: "block-dangerous",
        title: "拦截危险命令",
        event: "PreToolUse",
        matcher: "^Bash$",
        command: platform === "windows"
          ? `findstr /R /C:"rm -rf /" /C:"push --force" >nul && (echo 已拦截危险命令 1>&2 & exit /b 2) || exit /b 0`
          : `if grep -Eq 'rm -rf /|git push (-f|--force)'; then echo "已拦截危险命令" >&2; exit 2; fi`,
        timeout: 10,
        note: "退出码 2 拦截这次工具调用，stderr 会作为原因告诉模型。",
      },
      {
        id: "session-context",
        title: "会话开始时注入分支信息",
        event: "SessionStart",
        matcher: "",
        command: platform === "windows"
          ? `git branch --show-current`
          : `echo "当前 git 分支：$(git branch --show-current 2>/dev/null || echo 无)"`,
        timeout: 10,
        note: "stdout 会作为上下文加入新会话。",
      },
      {
        id: "prompt-log",
        title: "记录每次提问",
        event: "UserPromptSubmit",
        matcher: "",
        command: platform === "windows"
          ? `more >> "%CODEX_HOME%\\prompt-log.jsonl"`
          : `{ cat; echo; } >> "$CODEX_HOME/prompt-log.jsonl"`,
        timeout: 10,
        note: "把事件 JSON 追加到 CODEX_HOME 下的 prompt-log.jsonl，不输出任何内容。",
      },
      {
        id: "tool-log",
        title: "记录工具调用结果",
        event: "PostToolUse",
        matcher: "",
        command: platform === "windows"
          ? `more >> "%CODEX_HOME%\\tool-log.jsonl"`
          : `{ cat; echo; } >> "$CODEX_HOME/tool-log.jsonl"`,
        timeout: 10,
        note: "matcher 留空匹配所有工具。",
      },
    ];
  }

  /** 试运行用的示例事件（stdin）。字段按实测的 Codex 输入，内容是假的，可以改。 */
  function codexPlusHooksEditorSampleInput(event, { cwd = "", matcher = "" } = {}) {
    const input = {
      session_id: "00000000-0000-0000-0000-000000000000",
      turn_id: "test-turn",
      transcript_path: null,
      cwd,
      hook_event_name: event,
      model: "gpt-5.1-codex",
      permission_mode: "default",
    };
    const toolName = /^\^?([A-Za-z_]+)\$?$/.test(matcher) ? matcher.replace(/^\^|\$$/g, "") : "Bash";
    if (event === "SessionStart") input.source = "startup";
    if (event === "UserPromptSubmit") input.prompt = "帮我看看这个仓库";
    if (event === "PreToolUse" || event === "PermissionRequest" || event === "PostToolUse") {
      input.tool_name = toolName;
      input.tool_input = { command: event === "PostToolUse" ? "ls" : "rm -rf /tmp/example" };
      input.tool_use_id = "call_test";
    }
    if (event === "PostToolUse") input.tool_response = "README.md\nsrc\n";
    if (event === "Stop" || event === "SubagentStop") {
      input.stop_hook_active = false;
      input.last_assistant_message = "改好了。";
    }
    return JSON.stringify(input, null, 2);
  }

  /** 按 Codex 的规则解读试运行结果：返回 `{ tone, lines }`。 */
  function codexPlusHooksEditorInterpret(event, result) {
    const lines = [];
    let tone = "ok";
    if (result?.timedOut) {
      return { tone: "error", lines: [`超过 ${result.timeoutSec} 秒没有结束，已终止。Codex 里会按 hook 的 timeout 处理。`] };
    }
    const code = result?.exitCode;
    const stdout = String(result?.stdout || "").trim();
    const stderr = String(result?.stderr || "").trim();
    let json = null;
    if (stdout.startsWith("{")) {
      try {
        json = JSON.parse(stdout);
      } catch {
        json = null;
      }
    }
    const blocking = event === "PreToolUse" || event === "PermissionRequest";
    if (code === 2) {
      if (event === "Stop" || event === "SubagentStop") {
        tone = "warn";
        lines.push("退出码 2：Codex 会让 agent 继续这一轮，hook 每次都返回 2 就会死循环。");
      } else if (blocking) {
        tone = "warn";
        lines.push(`退出码 2：会拦截这次工具调用，模型看到的原因：${stderr || "（stderr 为空）"}`);
      } else {
        tone = "warn";
        lines.push(`退出码 2：阻止这一步，原因取 stderr：${stderr || "（空）"}`);
      }
    } else if (code !== 0) {
      tone = "error";
      lines.push(`退出码 ${code ?? "未知"}：Codex 会把它记为 hook 出错，不拦截。`);
    }
    const decision = json?.hookSpecificOutput?.permissionDecision;
    if (decision) {
      const reason = json.hookSpecificOutput.permissionDecisionReason;
      if (decision === "deny" && !String(reason || "").trim()) {
        tone = "error";
        lines.push("JSON 给了 deny，但没有 permissionDecisionReason：Codex 要求 deny 必须带原因。");
      } else {
        tone = decision === "deny" ? "warn" : tone;
        lines.push(`JSON 决定：${decision}${reason ? `，原因：${reason}` : ""}`);
      }
    }
    if ((event === "Stop" || event === "SubagentStop") && stdout && !json) {
      tone = "error";
      lines.push("Stop hook 的 stdout 必须是 JSON 或留空，否则 Codex 报 invalid stop hook JSON output。");
    }
    if ((event === "SessionStart" || event === "UserPromptSubmit") && stdout && !json && code === 0) {
      lines.push("stdout 会作为上下文加入对话。");
    }
    if (!lines.length) lines.push(code === 0 ? "通过：退出码 0。" : "已结束。");
    return { tone, lines };
  }

  async function codexPlusHooksEditorBackend(path, payload) {
    if (typeof postJson !== "function") throw new Error("Codex++ 后端还没就绪");
    const result = await postJson(path, payload || {});
    if (!result || result.status !== "ok") throw new Error(result?.message || "Codex++ 后端没有响应");
    return result;
  }

  async function codexPlusHooksEditorConfigWrite(client, edits) {
    return client.sendRequest("config/batchWrite", {
      edits,
      filePath: null,
      expectedVersion: null,
      reloadUserConfig: true,
    });
  }

  /** hooks/list 的结果按 key 合并（多个 cwd 会重复列出用户层）。 */
  async function codexPlusHooksEditorListHooks(client, cwds) {
    const result = await client.sendRequest("hooks/list", { cwds });
    const hooks = new Map();
    const warnings = [];
    for (const entry of Array.isArray(result?.data) ? result.data : []) {
      for (const hook of Array.isArray(entry?.hooks) ? entry.hooks : []) {
        if (hook?.key && !hooks.has(hook.key)) hooks.set(hook.key, hook);
      }
      for (const text of [...(entry?.warnings || []), ...(entry?.errors || [])]) {
        const message = typeof text === "string" ? text : text?.message || JSON.stringify(text);
        if (message && !warnings.includes(message)) warnings.push(message);
      }
    }
    return { hooks, warnings };
  }

  async function codexPlusHooksEditorReadState(client) {
    const result = await client.sendRequest("config/read", {});
    const state = result?.config?.hooks?.state;
    return state && typeof state === "object" ? state : {};
  }

  function codexPlusHooksEditorSavedRoots() {
    try {
      const value = JSON.parse(localStorage.getItem(codexPlusHooksEditorRootsKey) || "[]");
      return Array.isArray(value) ? value.filter((item) => typeof item === "string") : [];
    } catch {
      return [];
    }
  }

  function codexPlusHooksEditorSaveRoots(roots) {
    try {
      localStorage.setItem(codexPlusHooksEditorRootsKey, JSON.stringify(roots));
    } catch {
      // 存不了就只在本次打开有效。
    }
  }

  // 改下面任何 CSS 都要把它 +1：已注入过的页面靠它判断旧 <style> 是否过期。
  const codexPlusHooksEditorStyleVersion = "2";

  // 配色、圆角、开关复用 Codex++ 设置页（10-style.js :root）的变量，与左栏其它分组一致。
  function codexPlusHooksEditorStyles() {
    return `
      .codex-plus-hooks-editor {
        display: flex; flex-direction: column; gap: 12px;
        /* 标题与外边距由 Codex++ 设置页的分组外壳给（registerSettingsSection）。 */
        box-sizing: border-box; width: 100%;
        padding: 0;
        color: var(--codex-plus-text);
        font-size: 13px; line-height: 18px;
      }
      .codex-plus-hooks-editor * { box-sizing: border-box; }
      .codex-plus-hooks-editor h3 { margin: 12px 0 0; font-size: 13px; font-weight: 500; line-height: calc(1.25 / .875); }
      .codex-plus-hooks-editor p { margin: 0; }
      .codex-plus-hooks-editor code, .codex-plus-hooks-editor .che-mono {
        font: 12px/1.45 ui-monospace, SFMono-Regular, Menlo, monospace;
      }
      .codex-plus-hooks-editor .che-sub { color: var(--codex-plus-text-secondary); }
      .codex-plus-hooks-editor .che-toolbar { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
      .codex-plus-hooks-editor input[type="text"], .codex-plus-hooks-editor input[type="number"],
      .codex-plus-hooks-editor select, .codex-plus-hooks-editor textarea {
        min-height: 28px; padding: 4px 10px; border-radius: var(--codex-plus-control-radius);
        border: 1px solid var(--codex-plus-control-border);
        background: var(--codex-plus-control-bg); color: inherit; font: inherit; outline: none;
      }
      .codex-plus-hooks-editor input:focus, .codex-plus-hooks-editor select:focus,
      .codex-plus-hooks-editor textarea:focus { border-color: var(--codex-plus-focus); }
      .codex-plus-hooks-editor select { appearance: auto !important; }
      .codex-plus-hooks-editor input[type="checkbox"] { appearance: auto !important; margin: 0; }
      .codex-plus-hooks-editor textarea { width: 100%; resize: vertical; font: 12px/1.45 ui-monospace, SFMono-Regular, Menlo, monospace; }
      .codex-plus-hooks-editor button {
        height: 28px; padding: 0 12px; border-radius: var(--codex-plus-control-radius); cursor: pointer;
        border: 0;
        background: var(--codex-plus-button-bg); color: inherit; font: inherit;
      }
      .codex-plus-hooks-editor button:hover:not(:disabled) { background: var(--codex-plus-button-hover-bg); }
      .codex-plus-hooks-editor button:disabled { cursor: default; opacity: .55; }
      /* 主按钮跟 Codex 原生一样用前景色反白。 */
      .codex-plus-hooks-editor button.che-primary,
      .codex-plus-hooks-editor button.che-primary:hover:not(:disabled) { background: var(--codex-plus-text); color: var(--codex-plus-bg-primary); }
      .codex-plus-hooks-editor button.che-primary:hover:not(:disabled) { opacity: .85; }
      .codex-plus-hooks-editor button.che-danger { color: var(--codex-plus-danger); }
      .codex-plus-hooks-editor .che-file { flex: 1 1 320px; min-width: 220px; }
      .codex-plus-hooks-editor .che-root-input { flex: 1 1 260px; }
      .codex-plus-hooks-editor .che-status { min-height: 16px; color: var(--codex-plus-text-secondary); font-size: 12px; line-height: 16px; white-space: pre-wrap; }
      .codex-plus-hooks-editor .che-status:empty { display: none; }
      .codex-plus-hooks-editor [data-tone="error"] { color: var(--codex-plus-danger); }
      .codex-plus-hooks-editor [data-tone="ok"] { color: var(--codex-plus-success); }
      .codex-plus-hooks-editor [data-tone="warn"] { color: var(--codex-plus-warning); }
      .codex-plus-hooks-editor .che-note { color: var(--codex-plus-text-tertiary); font-size: 12px; line-height: 16px; }
      .codex-plus-hooks-editor .che-note[data-tone="warn"] { color: var(--codex-plus-warning); }
      /* 与设置页的 .codex-plus-settings-card 一致：一张卡片，行之间分隔线。 */
      .codex-plus-hooks-editor .che-list {
        display: flex; flex-direction: column;
        padding: 0 16px;
        border: 1px solid var(--codex-plus-settings-card-border);
        border-radius: var(--codex-plus-card-radius);
        background: var(--codex-plus-settings-card-bg);
      }
      .codex-plus-hooks-editor .che-list:empty { display: none; }
      .codex-plus-hooks-editor .che-section { display: flex; flex-direction: column; gap: 8px; }
      .codex-plus-hooks-editor .che-list > * + * { border-top: 1px solid var(--codex-plus-settings-card-border); }
      .codex-plus-hooks-editor .che-row { display: flex; flex-direction: column; gap: 8px; padding: 12px 0; }
      /* 未保存的行：整行淡淡的警示底色，铺满卡片宽度。 */
      .codex-plus-hooks-editor .che-row[data-dirty="true"] {
        margin: 0 -16px; padding: 12px 16px;
        background: color-mix(in srgb, var(--codex-plus-warning) 7%, transparent);
      }
      .codex-plus-hooks-editor .che-head { display: flex; gap: 12px; align-items: flex-start; }
      .codex-plus-hooks-editor .che-main { flex: 1 1 auto; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
      .codex-plus-hooks-editor .che-title { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; font-weight: 500; line-height: calc(1.25 / .875); }
      .codex-plus-hooks-editor .che-command { white-space: pre-wrap; word-break: break-all; color: var(--codex-plus-text-secondary); }
      .codex-plus-hooks-editor .che-meta { font-size: 12px; line-height: 16px; color: var(--codex-plus-text-tertiary); }
      .codex-plus-hooks-editor .che-actions { display: flex; gap: 6px; flex: 0 0 auto; align-items: center; }
      /* 标签沿用设置页 .codex-plus-feature-tag 的描边小标签。 */
      .codex-plus-hooks-editor .che-badge {
        display: inline-flex; align-items: center; padding: 0 5px;
        border: 1px solid var(--codex-plus-border); border-radius: 4px;
        font-size: 10.5px; font-weight: 500; line-height: 16px; color: var(--codex-plus-text-secondary);
      }
      .codex-plus-hooks-editor .che-badge[data-tone="ok"] { border-color: color-mix(in srgb, var(--codex-plus-success) 40%, transparent); }
      .codex-plus-hooks-editor .che-badge[data-tone="warn"] { border-color: color-mix(in srgb, var(--codex-plus-warning) 40%, transparent); }
      .codex-plus-hooks-editor .che-badge[data-tone="error"] { border-color: color-mix(in srgb, var(--codex-plus-danger) 40%, transparent); }
      /* 开关与设置页 .codex-plus-toggle 同尺寸同变量。 */
      .codex-plus-hooks-editor .che-switch {
        position: relative; flex: 0 0 32px; width: 32px; height: 20px; padding: 0; border: 0; border-radius: 999px;
        background: var(--codex-plus-switch-off); transition: background .15s ease;
      }
      .codex-plus-hooks-editor .che-switch:hover:not(:disabled) { background: var(--codex-plus-switch-off); }
      .codex-plus-hooks-editor .che-switch::after {
        content: ""; position: absolute; top: 2px; left: 2px; width: 16px; height: 16px; border-radius: 50%;
        background: var(--codex-plus-switch-thumb); box-shadow: var(--switch-thumb-shadow, 0 1px 2px rgba(0,0,0,.2)); transition: transform .15s ease;
      }
      .codex-plus-hooks-editor .che-switch[aria-checked="true"],
      .codex-plus-hooks-editor .che-switch[aria-checked="true"]:hover:not(:disabled) { background: var(--codex-plus-switch-on); }
      .codex-plus-hooks-editor .che-switch[aria-checked="true"]::after { transform: translateX(12px); background: var(--switch-thumb-color-checked, var(--codex-plus-switch-thumb)); }
      .codex-plus-hooks-editor .che-form {
        display: grid; grid-template-columns: 110px 1fr; gap: 8px 10px; align-items: center;
        padding: 12px 16px; border-radius: var(--codex-plus-card-radius);
        border: 1px solid var(--codex-plus-settings-card-border);
        background: var(--codex-plus-settings-card-bg);
      }
      .codex-plus-hooks-editor .che-row .che-form { padding: 12px 0 0; border: 0; border-radius: 0; background: transparent; }
      .codex-plus-hooks-editor .che-form > label { color: var(--codex-plus-text-secondary); }
      .codex-plus-hooks-editor .che-form .che-wide { grid-column: 1 / -1; }
      .codex-plus-hooks-editor .che-check { display: inline-flex; gap: 6px; align-items: center; }
      .codex-plus-hooks-editor .che-test { display: flex; flex-direction: column; gap: 6px; padding-top: 4px; }
      .codex-plus-hooks-editor .che-test .che-toolbar input { flex: 1 1 auto; }
      .codex-plus-hooks-editor .che-output {
        margin: 0; padding: 8px 10px; border-radius: var(--codex-plus-control-radius); max-height: 220px; overflow: auto; white-space: pre-wrap;
        background: var(--codex-plus-button-bg); font: 12px/1.45 ui-monospace, SFMono-Regular, Menlo, monospace;
      }
      .codex-plus-hooks-editor .che-empty { padding: 14px 0; color: var(--codex-plus-text-secondary); }
    `;
  }

  function codexPlusHooksEditorEnsureStyles() {
    const id = "codex-plus-hooks-editor-style";
    const existing = document.getElementById(id);
    if (existing?.dataset.codexPlusStyleVersion === codexPlusHooksEditorStyleVersion) return;
    existing?.remove();
    const style = document.createElement("style");
    style.id = id;
    style.dataset.codexPlusStyleVersion = codexPlusHooksEditorStyleVersion;
    style.textContent = codexPlusHooksEditorStyles();
    document.head.appendChild(style);
  }

  function codexPlusHooksEditorEl(tag, props = {}, ...children) {
    const node = document.createElement(tag);
    for (const [key, value] of Object.entries(props)) {
      if (value === undefined || value === null || value === false) continue;
      if (key === "className") node.className = value;
      else if (key === "text") node.textContent = value;
      else if (key === "dataset") Object.assign(node.dataset, value);
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

  function codexPlusHooksEditorRender({ container, ctx }) {
    if (!container) return;
    codexPlusHooksEditorEnsureStyles();
    const el = codexPlusHooksEditorEl;
    const state = {
      files: [],
      fileIndex: 0,
      configDefinesHooks: false,
      file: null,
      doc: null,
      parseError: "",
      rawText: "",
      origins: new Map(),
      touched: new Set(),
      dirty: false,
      listed: new Map(),
      warnings: [],
      hookState: {},
      editing: null,
      testing: null,
      busy: false,
      trustOnSave: true,
      extraRoots: codexPlusHooksEditorSavedRoots(),
    };

    const status = el("div", { className: "che-status", role: "status" });
    const fileSelect = el("select", { className: "che-file", "aria-label": "选择 hooks 文件" });
    const rootInput = el("input", { type: "text", className: "che-root-input", placeholder: "添加项目目录（绝对路径）" });
    const body = el("div", { className: "che-body" });
    const root = el(
      "div",
      { className: "codex-plus-hooks-editor" },
      el("p", {
        className: "che-sub",
        text: "在 Codex 的回合事件上挂命令：会话开始、工具调用前后、回合结束等。改的是 hooks.json，开关和信任与原生「Hooks」页共用；保存后新会话立即生效，已打开的会话下一轮生效。",
      }),
      el(
        "div",
        { className: "che-toolbar" },
        fileSelect,
        el("button", { type: "button", text: "重新读取", onclick: () => void load() }),
      ),
      el(
        "div",
        { className: "che-toolbar" },
        rootInput,
        el("button", { type: "button", text: "添加项目", onclick: () => void addRoot() }),
      ),
      status,
      body,
      el("div", {
        className: "che-note",
        text: "hook 命令在沙箱外以你的身份运行（$SHELL -c），只放你看得懂的命令。项目层 hooks.json 只有在 Codex 里信任了该项目才会加载。",
      }),
    );
    container.replaceChildren(root);

    const setStatus = (message, tone = "") => {
      if (!status.isConnected) return;
      status.textContent = message || "";
      status.dataset.tone = tone;
    };

    // hook key 的路径段：用户层是规范化后的路径，项目层是按传给 hooks/list 的目录拼出来的原样路径
    // （macOS 上 /tmp 不会被展开成 /private/tmp）。以 hooks/list 实际返回的 sourcePath 为准。
    const prefixFor = (file) => codexPlusHooksEditorKeyPrefix(file, state.listed.values());
    const keyPrefix = () => prefixFor(state.file);
    const listedFor = (row) => {
      const origin = state.origins.get(row.handler);
      return origin ? state.listed.get(codexPlusHooksEditorKey(keyPrefix(), origin)) || null : null;
    };

    const confirmDiscard = async () => {
      if (!state.dirty) return true;
      const answer = await ctx.ask({
        title: "放弃未保存的修改？",
        message: `${state.file?.path || "当前文件"} 有未保存的修改。`,
        options: [{ value: "discard", label: "放弃修改" }],
      });
      return answer?.value === "discard";
    };

    const openFile = (index) => {
      state.fileIndex = index;
      state.file = state.files[index] || null;
      state.editing = null;
      state.testing = null;
      state.touched = new Set();
      state.dirty = false;
      state.rawText = state.file?.text || "";
      const parsed = state.file && !state.file.error ? codexPlusHooksEditorParse(state.file.text) : { doc: null, error: state.file?.error || "" };
      state.doc = parsed.doc;
      state.parseError = parsed.error;
      state.origins = state.doc ? codexPlusHooksEditorOrigins(state.doc) : new Map();
    };

    const markDirty = (handler) => {
      state.dirty = true;
      if (handler) state.touched.add(handler);
    };

    const refreshListed = async (client) => {
      const cwds = [
        ...state.files.filter((file) => file.scope === "project" && file.rootExists !== false).map((file) => file.root),
      ];
      // hooks/list 至少要一个目录；没有项目时用 CODEX_HOME 本身（它下面没有 .codex/hooks.json，不会多出项目层）。
      if (!cwds.length && state.codexHome) cwds.push(state.codexHome);
      const [{ hooks, warnings }, hookState] = await Promise.all([
        codexPlusHooksEditorListHooks(client, cwds),
        codexPlusHooksEditorReadState(client).catch(() => ({})),
      ]);
      state.listed = hooks;
      state.warnings = warnings;
      state.hookState = hookState;
    };

    const load = async ({ keepIndex = true } = {}) => {
      if (!(await confirmDiscard())) return;
      state.busy = true;
      setStatus("正在读取…");
      try {
        const result = await codexPlusHooksEditorBackend("/hooks/files", { roots: state.extraRoots });
        const previous = state.file?.path;
        state.files = Array.isArray(result.files) ? result.files : [];
        state.codexHome = result.codexHome || "";
        state.configDefinesHooks = result.configDefinesHooks === true;
        const index = keepIndex ? Math.max(0, state.files.findIndex((file) => file.path === previous)) : 0;
        openFile(index);
        const client = await codexPlusAppServerClient("local");
        await refreshListed(client);
        const skipped = Array.isArray(result.skipped) ? result.skipped : [];
        setStatus(skipped.length ? `跳过的目录：${skipped.map((item) => `${item.root}（${item.message}）`).join("；")}` : "", skipped.length ? "warn" : "");
      } catch (error) {
        setStatus(`读取失败：${error?.message || error}`, "error");
      } finally {
        state.busy = false;
        draw();
      }
    };

    const addRoot = async () => {
      const value = rootInput.value.trim();
      if (!value) return;
      if (!state.extraRoots.includes(value)) state.extraRoots.push(value);
      codexPlusHooksEditorSaveRoots(state.extraRoots);
      rootInput.value = "";
      await load();
      const index = state.files.findIndex((file) => file.root === value);
      if (index >= 0) {
        openFile(index);
        draw();
      }
    };

    const save = async () => {
      if (!state.file || state.busy) return;
      state.busy = true;
      draw();
      setStatus("正在保存…");
      try {
        let text;
        let edits = [];
        const client = await codexPlusAppServerClient("local");
        if (state.doc) {
          text = codexPlusHooksEditorSerialize(state.doc);
          const hookState = await codexPlusHooksEditorReadState(client);
          edits = codexPlusHooksEditorMigrationEdits({ doc: state.doc, origins: state.origins, prefix: keyPrefix(), state: hookState });
        } else {
          text = state.rawText;
        }
        const written = await codexPlusHooksEditorBackend("/hooks/file/write", {
          scope: state.file.scope,
          root: state.file.root,
          text,
          expectedVersion: state.file.version || "",
        });
        // 先把开关记录搬到新位置，再让 app-server 重读 hooks（edits 为空也要发，靠它触发重读）。
        await codexPlusHooksEditorConfigWrite(client, edits);
        const touchedPositions = state.doc
          ? codexPlusHooksEditorRows(state.doc).filter((row) => state.touched.has(row.handler)).map((row) => row.position)
          : [];
        state.file = { ...state.file, text, version: written.version, exists: true };
        state.files[state.fileIndex] = state.file;
        openFile(state.fileIndex);
        await refreshListed(client);
        let trusted = 0;
        if (state.trustOnSave && touchedPositions.length) {
          const trustEdits = [];
          for (const position of touchedPositions) {
            const hook = state.listed.get(codexPlusHooksEditorKey(keyPrefix(), position));
            if (hook?.currentHash && hook.trustStatus !== "trusted" && hook.trustStatus !== "managed") {
              trustEdits.push({
                keyPath: `hooks.state.${JSON.stringify(hook.key)}.trusted_hash`,
                value: hook.currentHash,
                mergeStrategy: "upsert",
              });
            }
          }
          if (trustEdits.length) {
            await codexPlusHooksEditorConfigWrite(client, trustEdits);
            trusted = trustEdits.length;
            await refreshListed(client);
          }
        }
        const loaded = state.file.scope === "user" || state.file.trusted;
        setStatus(
          `已保存 ${written.path}${edits.length ? `，搬迁了 ${edits.length} 条开关记录` : ""}${trusted ? `，信任了 ${trusted} 条 hook` : ""}。`
            + (loaded ? "新会话立即生效，已打开的会话下一轮生效。" : "这个项目还没被 Codex 信任，hooks 不会加载。"),
          loaded ? "ok" : "warn",
        );
      } catch (error) {
        setStatus(`保存失败：${error?.message || error}`, "error");
      } finally {
        state.busy = false;
        draw();
      }
    };

    const writeHookState = async (edits, success) => {
      state.busy = true;
      draw();
      try {
        const client = await codexPlusAppServerClient("local");
        const result = await codexPlusHooksEditorConfigWrite(client, edits);
        await refreshListed(client);
        if (result?.status === "okOverridden") setStatus(`${success}，但有更高优先级的配置覆盖了它。`, "warn");
        else setStatus(success, "ok");
      } catch (error) {
        setStatus(`写入失败：${error?.message || error}`, "error");
      } finally {
        state.busy = false;
        draw();
      }
    };

    const toggleEnabled = (hook) => writeHookState(
      [{ keyPath: `hooks.state.${JSON.stringify(hook.key)}.enabled`, value: hook.enabled === false, mergeStrategy: "upsert" }],
      hook.enabled === false ? "已启用" : "已停用",
    );

    const trustHook = (hook) => writeHookState(
      [{ keyPath: `hooks.state.${JSON.stringify(hook.key)}.trusted_hash`, value: hook.currentHash, mergeStrategy: "upsert" }],
      "已信任",
    );

    const statusBadges = (hook) => {
      if (!hook) {
        return [el("span", { className: "che-badge", text: state.file?.scope === "project" && !state.file?.trusted ? "未加载（项目未受信任）" : "未加载" })];
      }
      const trust = hook.trustStatus;
      const badges = [
        el("span", {
          className: "che-badge",
          dataset: { tone: trust === "trusted" || trust === "managed" ? "ok" : "warn" },
          text: codexPlusHooksEditorTrustLabels[trust] || trust || "未知",
        }),
      ];
      if (hook.enabled === false) badges.push(el("span", { className: "che-badge", dataset: { tone: "error" }, text: "已停用" }));
      return badges;
    };

    const enableSwitch = (hook, disabledReason) => el("button", {
      type: "button",
      className: "che-switch",
      role: "switch",
      "aria-checked": hook && hook.enabled !== false ? "true" : "false",
      "aria-label": "启用这条 hook",
      title: disabledReason || "",
      disabled: !hook || state.busy || Boolean(disabledReason),
      onclick: () => hook && void toggleEnabled(hook),
    });

    const hookForm = ({ initial, onSubmit, onCancel, submitLabel }) => {
      const templates = codexPlusHooksEditorTemplates();
      const eventSelect = el("select", { "aria-label": "事件" }, codexPlusHooksEditorEvents.map((event) =>
        el("option", { value: event.name, text: `${event.label}（${event.name}）`, selected: event.name === initial.event })));
      const matcher = el("input", { type: "text", value: initial.matcher || "", placeholder: "留空匹配全部；工具事件填工具名正则，如 ^Bash$" });
      const command = el("textarea", { rows: 3, "aria-label": "命令", placeholder: "shell 命令，事件 JSON 从 stdin 读" });
      command.value = initial.command || "";
      const commandWindows = el("input", { type: "text", value: initial.commandWindows || "", placeholder: "可选：Windows 上改用这条命令" });
      const timeout = el("input", { type: "number", min: "1", value: initial.timeout ? String(initial.timeout) : "", placeholder: "默认 600" });
      const statusMessage = el("input", { type: "text", value: initial.statusMessage || "", placeholder: "可选：运行时界面上显示的文字" });
      const asyncBox = el("input", { type: "checkbox", checked: initial.async === true });
      const hint = el("div", { className: "che-meta che-wide" });
      const updateHint = () => {
        hint.textContent = codexPlusHooksEditorEvents.find((event) => event.name === eventSelect.value)?.hint || "";
      };
      eventSelect.addEventListener("change", updateHint);
      updateHint();
      const templateSelect = el(
        "select",
        { "aria-label": "模板" },
        el("option", { value: "", text: "从模板开始…" }),
        templates.map((template) => el("option", { value: template.id, text: template.title })),
      );
      templateSelect.addEventListener("change", () => {
        const template = templates.find((item) => item.id === templateSelect.value);
        if (!template) return;
        eventSelect.value = template.event;
        matcher.value = template.matcher;
        command.value = template.command;
        timeout.value = template.timeout ? String(template.timeout) : "";
        hint.textContent = template.note;
      });
      const error = el("div", { className: "che-meta che-wide", dataset: { tone: "error" } });
      const submit = () => {
        if (!command.value.trim()) {
          error.textContent = "命令不能为空";
          return;
        }
        if (matcher.value.trim()) {
          try {
            new RegExp(matcher.value.trim());
          } catch (regexError) {
            error.textContent = `matcher 不是合法的正则：${regexError?.message || regexError}`;
            return;
          }
        }
        onSubmit({
          event: eventSelect.value,
          matcher: matcher.value.trim(),
          fields: {
            command: command.value,
            commandWindows: commandWindows.value,
            timeout: timeout.value,
            statusMessage: statusMessage.value,
            async: asyncBox.checked,
          },
        });
      };
      return el(
        "div",
        { className: "che-form" },
        initial.isNew ? el("label", { text: "模板" }) : null,
        initial.isNew ? templateSelect : null,
        el("label", { text: "事件" }), eventSelect,
        hint,
        el("label", { text: "matcher" }), matcher,
        el("label", { text: "命令" }), command,
        el("label", { text: "Windows 命令" }), commandWindows,
        el("label", { text: "超时（秒）" }), timeout,
        el("label", { text: "状态文字" }), statusMessage,
        el("label", { text: "" }), el("label", { className: "che-check" }, asyncBox, "async（当前版本仍会同步执行）"),
        error,
        el(
          "div",
          { className: "che-toolbar che-wide" },
          el("button", { type: "button", className: "che-primary", text: submitLabel, onclick: submit }),
          el("button", { type: "button", text: "取消", onclick: onCancel }),
        ),
      );
    };

    const testPanel = (row) => {
      const cwd = state.file?.scope === "project" ? state.file.root : state.codexHome || "";
      const cwdInput = el("input", { type: "text", value: cwd, placeholder: "工作目录" });
      const stdin = el("textarea", { rows: 7, "aria-label": "示例事件 JSON（stdin）" });
      stdin.value = codexPlusHooksEditorSampleInput(row.event, { cwd, matcher: String(row.group.matcher || "") });
      const output = el("div", { className: "che-test-result" });
      const run = el("button", { type: "button", className: "che-primary", text: "运行" });
      run.addEventListener("click", async () => {
        run.disabled = true;
        output.replaceChildren(el("div", { className: "che-meta", text: "运行中…" }));
        try {
          const result = await codexPlusHooksEditorBackend("/hooks/test-run", {
            command: row.handler.command,
            commandWindows: row.handler.commandWindows,
            cwd: cwdInput.value.trim(),
            stdin: stdin.value,
            timeoutSec: Math.min(Number(row.handler.timeout) || 30, 120),
          });
          const verdict = codexPlusHooksEditorInterpret(row.event, result);
          output.replaceChildren(
            el("div", { className: "che-meta", text: `${result.shell} · 退出码 ${result.exitCode ?? "无"} · ${result.durationMs} ms${result.truncated ? " · 输出已截断" : ""}` }),
            ...verdict.lines.map((line) => el("div", { dataset: { tone: verdict.tone }, text: line })),
            el("div", { className: "che-meta", text: "stdout" }),
            el("pre", { className: "che-output", text: result.stdout || "（空）" }),
            el("div", { className: "che-meta", text: "stderr" }),
            el("pre", { className: "che-output", text: result.stderr || "（空）" }),
          );
        } catch (error) {
          output.replaceChildren(el("div", { dataset: { tone: "error" }, text: `运行失败：${error?.message || error}` }));
        } finally {
          run.disabled = false;
        }
      });
      return el(
        "div",
        { className: "che-test" },
        el("div", { className: "che-meta", text: "用示例事件在本机跑一次命令（真的会执行），不经过 Codex，不影响会话。stdin 可以改。" }),
        el("div", { className: "che-toolbar" }, el("span", { className: "che-meta", text: "工作目录" }), cwdInput),
        stdin,
        el("div", { className: "che-toolbar" }, run, el("button", { type: "button", text: "收起", onclick: () => { state.testing = null; draw(); } })),
        output,
      );
    };

    const removeRow = async (row) => {
      const answer = await ctx.ask({
        title: "删除这条 hook？",
        message: `${codexPlusHooksEditorEventLabel(row.event)}：${String(row.handler.command || row.handler.type || "").slice(0, 200)}\n删除后点「保存」才写入文件。`,
        options: [{ value: "delete", label: "删除" }],
      });
      if (answer?.value !== "delete") return;
      codexPlusHooksEditorRemove(state.doc, row.handler);
      state.touched.delete(row.handler);
      markDirty(null);
      draw();
    };

    const rowView = (row) => {
      const hook = listedFor(row);
      const dirtyRow = state.touched.has(row.handler) || !state.origins.has(row.handler);
      const kind = row.handler.type || "command";
      const title = el(
        "div",
        { className: "che-title" },
        el("span", { text: codexPlusHooksEditorEventLabel(row.event) }),
        el("span", { className: "che-badge", text: row.event }),
        row.group.matcher ? el("span", { className: "che-badge che-mono", text: `matcher ${row.group.matcher}` }) : null,
        kind !== "command" ? el("span", { className: "che-badge", text: kind }) : null,
        dirtyRow ? el("span", { className: "che-badge", dataset: { tone: "warn" }, text: "未保存" }) : statusBadges(hook),
      );
      const commandText = kind === "command"
        ? String(row.handler.command || "")
        : kind === "mcp_tool"
          ? `${row.handler.server || "?"} / ${row.handler.tool || "?"}`
          : JSON.stringify(row.handler);
      const meta = [
        row.handler.timeout ? `超时 ${row.handler.timeout} 秒` : "",
        row.handler.async ? "async" : "",
        row.handler.statusMessage ? `状态文字：${row.handler.statusMessage}` : "",
        row.handler.commandWindows ? `Windows：${row.handler.commandWindows}` : "",
        kind === "prompt" || kind === "agent" ? "当前版本不支持这种类型，Codex 会跳过" : "",
      ].filter(Boolean).join(" · ");
      const actions = el(
        "div",
        { className: "che-actions" },
        hook && (hook.trustStatus === "untrusted" || hook.trustStatus === "modified") && !state.dirty
          ? el("button", { type: "button", text: "信任", disabled: state.busy, onclick: () => void trustHook(hook) })
          : null,
        row.editable ? el("button", { type: "button", text: "试运行", disabled: state.busy, onclick: () => { state.testing = row.handler; state.editing = null; draw(); } }) : null,
        row.editable ? el("button", { type: "button", text: "编辑", disabled: state.busy, onclick: () => { state.editing = row.handler; state.testing = null; draw(); } }) : null,
        el("button", { type: "button", className: "che-danger", text: "删除", disabled: state.busy, onclick: () => void removeRow(row) }),
        enableSwitch(hook, state.dirty ? "先保存或放弃修改，再切换开关" : ""),
      );
      const view = el(
        "div",
        { className: "che-row", dataset: { dirty: dirtyRow ? "true" : "false", event: row.event } },
        el("div", { className: "che-head" }, el("div", { className: "che-main" }, title, el("div", { className: "che-command che-mono", text: commandText }), meta ? el("div", { className: "che-meta", text: meta }) : null), actions),
      );
      if (state.editing === row.handler) {
        view.append(hookForm({
          initial: {
            event: row.event,
            matcher: row.group.matcher || "",
            command: row.handler.command,
            commandWindows: row.handler.commandWindows,
            timeout: row.handler.timeout,
            statusMessage: row.handler.statusMessage,
            async: row.handler.async === true,
          },
          submitLabel: "完成",
          onCancel: () => { state.editing = null; draw(); },
          onSubmit: (values) => {
            if (values.event === row.event && values.matcher !== String(row.group.matcher || "") && row.group.hooks.length > 1) {
              setStatus("这条 hook 和同组的其他 hook 共用 matcher，改了 matcher 会把它拆成单独一组。", "warn");
            }
            codexPlusHooksEditorUpdate(state.doc, row, values);
            markDirty(row.handler);
            state.editing = null;
            draw();
          },
        }));
      }
      if (state.testing === row.handler) view.append(testPanel(row));
      return view;
    };

    const otherSources = () => {
      const prefix = keyPrefix();
      const editablePrefixes = new Set(state.files.flatMap((file) => [file.path, file.canonicalPath, prefixFor(file)]));
      const rows = [...state.listed.values()].filter((hook) => {
        const source = String(hook.sourcePath || "");
        return !editablePrefixes.has(source) && source !== prefix;
      });
      if (!rows.length) return null;
      // 标题和说明在卡片外，卡片里只放行，和设置页其它分区一样。
      return el(
        "div",
        { className: "che-section" },
        el("h3", { text: "其他来源（只读，可启停）" }),
        el("p", { className: "che-sub", text: "config.toml、插件或托管配置里定义的 hook，这里不能改内容。" }),
        el("div", { className: "che-list" }, rows.map((hook) => el(
          "div",
          { className: "che-row" },
          el(
            "div",
            { className: "che-head" },
            el(
              "div",
              { className: "che-main" },
              el(
                "div",
                { className: "che-title" },
                el("span", { text: codexPlusHooksEditorEventLabel(hook.eventName ? hook.eventName[0].toUpperCase() + hook.eventName.slice(1) : "") }),
                el("span", { className: "che-badge", text: hook.source || "unknown" }),
                hook.matcher ? el("span", { className: "che-badge che-mono", text: `matcher ${hook.matcher}` }) : null,
                statusBadges(hook),
              ),
              el("div", { className: "che-command che-mono", text: hook.command || hook.handlerType || "" }),
              el("div", { className: "che-meta", text: hook.sourcePath || "" }),
            ),
            el(
              "div",
              { className: "che-actions" },
              (hook.trustStatus === "untrusted" || hook.trustStatus === "modified")
                ? el("button", { type: "button", text: "信任", disabled: state.busy, onclick: () => void trustHook(hook) })
                : null,
              enableSwitch(hook, hook.isManaged ? "受管理的 hook 不能在这里切换" : ""),
            ),
          ),
        ))),
      );
    };

    const draw = () => {
      if (!body.isConnected) return;
      fileSelect.replaceChildren(...state.files.map((file, index) => el("option", {
        value: String(index),
        selected: index === state.fileIndex,
        text: file.scope === "user"
          ? `用户层 · ${file.path}${file.exists ? "" : "（未创建）"}`
          : `项目 · ${file.root}${file.trusted ? "" : "（未受信任）"}${file.exists ? "" : "（未创建）"}`,
      })));
      fileSelect.disabled = state.busy;
      const parts = [];
      if (state.configDefinesHooks && state.file?.scope === "user") {
        parts.push(el("div", { className: "che-note", dataset: { tone: "warn" }, text: "config.toml 里也定义了 hooks。同一层两种写法并存时 Codex 会提示只用一种，建议把它们挪进 hooks.json。" }));
      }
      if (state.file?.scope === "project" && !state.file.trusted) {
        parts.push(el("div", { className: "che-note", dataset: { tone: "warn" }, text: "这个项目没有在 Codex 里被信任，项目层 hooks.json 不会加载。在 Codex 里打开该项目并选择信任后才生效。" }));
      }
      for (const warning of state.warnings) parts.push(el("div", { className: "che-note", dataset: { tone: "warn" }, text: warning }));

      if (!state.file) {
        parts.push(el("div", { className: "che-empty", text: state.busy ? "正在读取…" : "没有可编辑的 hooks 文件。" }));
      } else if (!state.doc) {
        // 读不了或不是合法结构：给原文编辑，保存时后端再校验。
        parts.push(el("div", { className: "che-note", dataset: { tone: "error" }, text: `无法按结构编辑：${state.parseError || state.file.error}` }));
        if (!state.file.error) {
          const raw = el("textarea", { rows: 16, "aria-label": "hooks.json 原文" });
          raw.value = state.rawText;
          raw.addEventListener("input", () => {
            state.rawText = raw.value;
            state.dirty = true;
            saveBar.hidden = false;
          });
          parts.push(raw);
        }
      } else {
        const rows = codexPlusHooksEditorRows(state.doc);
        parts.push(el(
          "div",
          { className: "che-toolbar" },
          el("button", {
            type: "button",
            text: "新建 hook",
            disabled: state.busy || state.editing === "new",
            onclick: () => { state.editing = "new"; state.testing = null; draw(); },
          }),
          el("span", { className: "che-meta", text: `${rows.length} 条` }),
        ));
        if (state.editing === "new") {
          parts.push(hookForm({
            initial: { isNew: true, event: "Stop", matcher: "", command: "" },
            submitLabel: "添加",
            onCancel: () => { state.editing = null; draw(); },
            onSubmit: ({ event, matcher, fields }) => {
              const handler = codexPlusHooksEditorApplyFields({ type: "command" }, fields);
              codexPlusHooksEditorAdd(state.doc, { event, matcher, handler });
              markDirty(handler);
              state.editing = null;
              draw();
            },
          }));
        }
        parts.push(rows.length
          ? el("div", { className: "che-list" }, rows.map(rowView))
          : el("div", { className: "che-empty", text: "这个文件里还没有 hook。点「新建 hook」从模板开始。" }));
      }

      const trustBox = el("input", { type: "checkbox", checked: state.trustOnSave, onchange: (event) => { state.trustOnSave = event.target.checked; } });
      const saveBar = el(
        "div",
        { className: "che-toolbar", hidden: !state.dirty },
        el("button", { type: "button", className: "che-primary", text: "保存", disabled: state.busy, onclick: () => void save() }),
        el("button", { type: "button", text: "放弃修改", disabled: state.busy, onclick: () => { openFile(state.fileIndex); draw(); } }),
        state.doc ? el("label", { className: "che-check" }, trustBox, "保存后信任我新建或改过的 hook") : null,
      );
      parts.push(saveBar);
      const others = otherSources();
      if (others) parts.push(others);
      body.replaceChildren(...parts);
    };

    fileSelect.addEventListener("change", async () => {
      const next = Number(fileSelect.value);
      if (next === state.fileIndex) return;
      if (!(await confirmDiscard())) {
        fileSelect.value = String(state.fileIndex);
        return;
      }
      openFile(next);
      draw();
    });

    draw();
    void load({ keepIndex: false });
  }

  function codexPlusHooksEditorExtension(ctx) {
    ctx.registerSettingsSection({
      id: "hooks-editor",
      title: "Hooks 编辑器",
      order: 80,
      render: ({ container }) => codexPlusHooksEditorRender({ container, ctx }),
    });
  }
