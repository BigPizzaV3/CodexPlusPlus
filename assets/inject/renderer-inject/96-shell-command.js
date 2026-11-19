  /**
   * 内置拓展「! 命令」。
   *
   * 和 Codex CLI 一样：在输入框里输入 `!<命令>` 回车，就在当前会话的工作目录里跑这条 shell 命令，
   * 命令与输出作为 `<user_shell_command>` 进入会话上下文，模型下一轮能看到。底层是 app-server 的
   * `thread/shellCommand`（实测 0.169：立即返回 {}，结果以 source=userShell 的 commandExecution 条目推送）。
   *
   * 几条实测约束：
   * - 命令**不经沙箱**，以用户身份完整权限运行（协议注释原文 runs unsandboxed with full access），
   *   所以第一次使用要确认；确认框也给「当作普通消息发送」的出口，防止误伤以 ! 开头的普通消息。
   * - 原生只把它画成一行「已运行命令」，看不到命令和输出，所以这里在输入框上方给一张结果卡。
   * - 只支持本机会话：远端主机的会话不在本机 app-server 上。新对话还没有会话 id 时不能用。
   */
  const codexPlusShellCommandPattern = /^!\s*(\S[\s\S]*)$/;
  const codexPlusShellCommandConfirmedKey = "codexPlusShellCommandConfirmed";
  const codexPlusShellCommandMaxOutput = 20000;
  /** 每个会话最近一次 ! 命令的状态：{ command, status, itemId, output, exitCode, durationMs, startedAt }。 */
  const codexPlusShellCommandRuns = new Map();

  /** 输入框文字 → 要跑的命令；不是 ! 命令时为空串。 */
  function codexPlusShellCommandParse(text) {
    const match = codexPlusShellCommandPattern.exec(String(text || "").trim());
    return match ? match[1].trim() : "";
  }

  /** 输出只留末尾一段：长日志的结论通常在最后。 */
  function codexPlusShellCommandTrimOutput(output) {
    const text = String(output || "");
    if (text.length <= codexPlusShellCommandMaxOutput) return { text, trimmed: false };
    return { text: text.slice(text.length - codexPlusShellCommandMaxOutput), trimmed: true };
  }

  /**
   * 把 app-server 推来的条目对到等待中的命令上：只认 source=userShell 的 commandExecution。
   * 已记下条目 id 时按 id 对；还没有时按命令文字对——item.command 是包过一层 shell 的
   * （实测 `/bin/zsh -lc 'echo hi'`），原样命令在 commandActions[].command 里。返回更新后的状态或 null。
   */
  function codexPlusShellCommandMatches(item, command) {
    if (Array.isArray(item.commandActions) && item.commandActions.some((action) => String(action?.command || "").trim() === command)) return true;
    const wrapped = String(item.command || "").trim();
    return wrapped === command || wrapped.endsWith(` -lc ${JSON.stringify(command)}`) || wrapped.endsWith(` -lc '${command}'`) || wrapped.endsWith(` -c '${command}'`);
  }

  function codexPlusShellCommandApplyItem(runs, threadId, item, phase) {
    if (!item || item.type !== "commandExecution" || item.source !== "userShell") return null;
    const run = runs.get(threadId);
    if (!run) return null;
    const sameItem = run.itemId && run.itemId === item.id;
    const sameCommand = !run.itemId && run.status === "running" && codexPlusShellCommandMatches(item, run.command);
    if (!sameItem && !sameCommand) return null;
    const next = { ...run, itemId: item.id || run.itemId };
    if (phase === "completed") {
      const { text, trimmed } = codexPlusShellCommandTrimOutput(item.aggregatedOutput);
      next.status = item.status === "completed" ? "completed" : item.status || "failed";
      next.output = text;
      next.trimmed = trimmed;
      next.exitCode = typeof item.exitCode === "number" ? item.exitCode : null;
      next.durationMs = typeof item.durationMs === "number" ? item.durationMs : Date.now() - run.startedAt;
    }
    runs.set(threadId, next);
    return next;
  }

  function codexPlusShellCommandConfirmed() {
    try {
      return localStorage.getItem(codexPlusShellCommandConfirmedKey) === "1";
    } catch {
      return false;
    }
  }

  function codexPlusShellCommandRemember() {
    try {
      localStorage.setItem(codexPlusShellCommandConfirmedKey, "1");
    } catch {
      // 存不了就每次都问。
    }
  }

  function codexPlusShellCommandStatusText(run) {
    if (run.status === "running") return "运行中…";
    const seconds = typeof run.durationMs === "number" ? `${(run.durationMs / 1000).toFixed(run.durationMs < 10000 ? 2 : 0)} 秒` : "";
    if (run.status === "error") return `没能运行：${run.error || "未知错误"}`;
    const code = run.exitCode == null ? "无退出码" : `退出码 ${run.exitCode}`;
    return [code, seconds].filter(Boolean).join(" · ");
  }

  function codexPlusShellCommandCard(run, onClose) {
    const card = document.createElement("div");
    card.className = "codex-plus-shell-card";
    card.setAttribute("data-codex-plus-ext", "shell-command");
    card.dataset.status = run.status;
    const head = document.createElement("div");
    head.className = "codex-plus-shell-card-head";
    const command = document.createElement("code");
    command.className = "codex-plus-shell-card-command";
    command.textContent = `$ ${run.command}`;
    const status = document.createElement("span");
    status.className = "codex-plus-shell-card-status";
    status.textContent = codexPlusShellCommandStatusText(run);
    const copy = document.createElement("button");
    copy.type = "button";
    copy.textContent = "复制输出";
    copy.disabled = run.status === "running" || !run.output;
    copy.addEventListener("click", () => {
      void navigator.clipboard?.writeText(run.output || "").then(() => showToast("已复制输出"), () => showToast("复制失败", { type: "error" }));
    });
    const close = document.createElement("button");
    close.type = "button";
    close.textContent = "关闭";
    close.setAttribute("aria-label", "关闭命令结果");
    close.addEventListener("click", onClose);
    head.append(command, status, copy, close);
    card.append(head);
    if (run.status !== "running") {
      const output = document.createElement("pre");
      output.className = "codex-plus-shell-card-output";
      output.textContent = run.output ? `${run.trimmed ? "…（只显示最后一段）\n" : ""}${run.output}` : "（没有输出）";
      card.append(output);
      const note = document.createElement("div");
      note.className = "codex-plus-shell-card-note";
      note.textContent = "命令和输出已加入会话上下文，下一条消息模型就能看到。";
      card.append(note);
    }
    return card;
  }

  function codexPlusShellCommandEnsureStyles() {
    const id = "codex-plus-shell-command-style";
    if (document.getElementById(id)) return;
    const style = document.createElement("style");
    style.id = id;
    style.setAttribute("data-codex-plus-ext", "shell-command");
    style.textContent = `
      .codex-plus-shell-card{display:flex;flex-direction:column;gap:6px;padding:8px 10px;border-radius:12px;
        border:1px solid color-mix(in srgb,currentColor 14%,transparent);background:var(--color-token-bg-secondary,color-mix(in srgb,currentColor 5%,transparent));
        font-size:12px;line-height:1.45}
      .codex-plus-shell-card[data-status="failed"],.codex-plus-shell-card[data-status="error"]{border-color:color-mix(in srgb,#e5534b 45%,transparent)}
      .codex-plus-shell-card-head{display:flex;align-items:center;gap:8px;min-width:0}
      .codex-plus-shell-card-command{flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font:12px/1.4 ui-monospace,SFMono-Regular,Menlo,monospace}
      .codex-plus-shell-card-status{flex:none;color:var(--color-token-text-secondary,currentColor);opacity:.85}
      .codex-plus-shell-card-head button{flex:none;height:22px;padding:0 8px;border-radius:6px;cursor:pointer;font:inherit;color:inherit;
        border:1px solid color-mix(in srgb,currentColor 18%,transparent);background:transparent}
      .codex-plus-shell-card-head button:disabled{opacity:.45;cursor:default}
      .codex-plus-shell-card-output{margin:0;max-height:220px;overflow:auto;white-space:pre-wrap;word-break:break-all;
        font:12px/1.45 ui-monospace,SFMono-Regular,Menlo,monospace;padding:6px 8px;border-radius:8px;background:color-mix(in srgb,currentColor 6%,transparent)}
      .codex-plus-shell-card-note{color:var(--color-token-text-secondary,currentColor);opacity:.75;font-size:11px}`;
    document.head.append(style);
  }

  async function codexPlusShellCommandConfirm(ctx, command) {
    if (codexPlusShellCommandConfirmed()) return "run";
    const answer = await ctx.ask({
      title: "在本机运行这条命令？",
      message: `$ ${command}\n\n以 ! 开头的消息会在当前会话的工作目录里直接运行：不经沙箱，以你的身份拥有完整权限。命令和输出会加入会话上下文。`,
      options: [
        { value: "always", label: "运行，以后不再询问" },
        { value: "run", label: "运行" },
        { value: "message", label: "当作普通消息发送" },
      ],
    });
    if (answer?.value === "always") {
      codexPlusShellCommandRemember();
      return "run";
    }
    return answer?.value || "cancel";
  }

  function codexPlusShellCommandExtension(ctx) {
    codexPlusShellCommandEnsureStyles();
    const slot = ctx.registerSlot({
      slot: "composer.above",
      order: 40,
      render: ({ container }) => {
        const threadId = codexPlusCurrentSessionId();
        const run = threadId ? codexPlusShellCommandRuns.get(threadId) : null;
        if (!run) return;
        container.appendChild(codexPlusShellCommandCard(run, () => {
          codexPlusShellCommandRuns.delete(threadId);
          slot.refresh();
        }));
      },
    });

    // 每条命令单独订阅本机 app-server 的条目通知，命令结束（或超时、拓展卸载）就退订。
    // 不依赖拓展事件桥：它要靠资源文件名找通知分发器，新版 Codex 的资源名是纯哈希，可能装不上。
    const subscriptions = new Set();
    const release = (entry) => {
      if (!subscriptions.delete(entry)) return;
      clearTimeout(entry.timer);
      try {
        entry.handle?.[Symbol.dispose]?.();
      } catch {}
      try {
        entry.subscription?.[Symbol.dispose]?.();
      } catch {}
    };
    ctx.on("dispose", () => [...subscriptions].forEach(release));
    const watch = async (client, threadId) => {
      const entry = { subscription: null, handle: null, timer: 0 };
      subscriptions.add(entry);
      // thread/shellCommand 默认超时 1 小时，多等一分钟后放弃。
      entry.timer = setTimeout(() => release(entry), 61 * 60 * 1000);
      entry.subscription = client.subscribe({
        type: "notification",
        key: { hostId: "local" },
        methods: ["item/started", "item/completed"],
        listener: ({ params }) => {
          if (params?.threadId !== threadId) return;
          const phase = params?.item?.status === "inProgress" ? "started" : "completed";
          const run = codexPlusShellCommandApplyItem(codexPlusShellCommandRuns, threadId, params?.item, phase);
          if (!run) return;
          slot.refresh();
          if (run.status !== "running") release(entry);
        },
      });
      entry.handle = await entry.subscription;
      return entry;
    };

    const run = async (editor, command) => {
      const ref = typeof currentSessionRef === "function" ? currentSessionRef() : {};
      const threadId = codexPlusCurrentSessionId();
      if (!threadId) {
        showToast("先发一条普通消息开始会话，再用 ! 运行命令", { type: "warn" });
        return;
      }
      if (ref?.host_id && ref.host_id !== "local") {
        showToast("! 命令只支持本机会话", { type: "warn" });
        return;
      }
      const choice = await codexPlusShellCommandConfirm(ctx, command);
      if (choice === "message") {
        // 原样交给 Codex 发出：点发送按钮，找不到就留在输入框里。
        const button = typeof codexPlusDictationSendButton === "function" ? codexPlusDictationSendButton(editor) : null;
        if (button) button.click();
        return;
      }
      if (choice !== "run") return;
      codexPlusClearComposer(editor);
      codexPlusShellCommandRuns.set(threadId, { command, status: "running", itemId: "", output: "", exitCode: null, startedAt: Date.now() });
      slot.refresh();
      try {
        const client = await codexPlusAppServerClient("local");
        await watch(client, threadId);
        await client.sendRequest("thread/shellCommand", { threadId, command });
      } catch (error) {
        codexPlusShellCommandRuns.set(threadId, {
          ...codexPlusShellCommandRuns.get(threadId),
          status: "error",
          error: String(error?.message || error),
        });
        slot.refresh();
      }
    };

    ctx.listen(document, "keydown", (event) => {
      if (event.key !== "Enter" || event.isComposing || event.shiftKey || event.altKey || event.ctrlKey || event.metaKey) return;
      const editor = codexPlusComposerEditorFrom(event.target);
      if (!editor) return;
      const command = codexPlusShellCommandParse(editor.innerText);
      if (!command) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      void run(editor, command);
    }, { capture: true });
  }
