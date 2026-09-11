(() => {
  const adapterVersion = "1.0.0";
  const requestTimeoutMs = 30000;
  const nameTimeoutMs = 8000;

  function requestError(method, error) {
    const message = String(error?.message || error || `Codex ${method} failed`);
    return new Error(`${method}: ${message}`);
  }

  function callCodex(method, params, timeoutMs = requestTimeoutMs) {
    const bridge = window.electronBridge;
    if (!bridge || typeof bridge.sendMessageFromView !== "function") {
      return Promise.reject(new Error("Codex renderer command bridge unavailable"));
    }
    const requestId = typeof crypto?.randomUUID === "function"
      ? crypto.randomUUID()
      : `xuan-mobile-${Date.now()}-${Math.random().toString(16).slice(2)}`;
    return new Promise((resolve, reject) => {
      let timeout = 0;
      const cleanup = () => {
        window.clearTimeout(timeout);
        window.removeEventListener("message", onMessage);
      };
      const onMessage = (event) => {
        const message = event?.data;
        if (!message || message.type !== "mcp-response" || message.hostId !== "local"
            || message.message?.id !== requestId) return;
        cleanup();
        if (message.message?.error) {
          reject(requestError(method, message.message.error));
          return;
        }
        resolve(message.message?.result ?? null);
      };
      window.addEventListener("message", onMessage);
      timeout = window.setTimeout(() => {
        cleanup();
        reject(new Error(`Codex ${method} timed out`));
      }, timeoutMs);
      Promise.resolve(bridge.sendMessageFromView({
        type: "mcp-request",
        request: { id: requestId, method, params },
        hostId: "local",
        priority: "critical",
        source: "xuan_plus_remote",
        timeoutMs,
        expiresAtMs: Date.now() + timeoutMs,
      })).catch((error) => {
        cleanup();
        reject(requestError(method, error));
      });
    });
  }

  function install() {
    const bridge = window.electronBridge;
    if (!bridge || typeof bridge.sendMessageFromView !== "function") return false;
    window.__xuanPlusRemoteCommandAdapterVersion = adapterVersion;
    window.__codexPlusMobileRemoteCommand = async (request) => {
      const commandType = String(request?.commandType || "");
      const threadId = String(request?.threadId || "");
      const turnId = String(request?.turnId || "");
      const clientRequestId = String(request?.clientRequestId || "");
      const text = String(request?.text || "");
      if (commandType === "create_task") {
        const started = await callCodex("thread/start", {
          cwd: String(request?.cwd || ""),
          model: String(request?.model || ""),
          modelProvider: String(request?.provider || ""),
          approvalPolicy: "on-request",
          sandbox: "workspace-write",
          persistExtendedHistory: true,
        });
        const createdThreadId = String(started?.thread?.id || started?.threadId || started?.id || "");
        if (!createdThreadId) throw new Error("thread/start returned no thread id");
        const turn = await callCodex("turn/start", {
          threadId: createdThreadId,
          clientUserMessageId: `xuan-mobile-${clientRequestId}`,
          input: [{ type: "text", text, text_elements: [] }],
        });
        const createdTurnId = String(turn?.turn?.id || turn?.turnId || turn?.id || "");
        if (!createdTurnId) throw new Error("turn/start returned no turn id");
        const name = String(request?.name || "").trim();
        if (name) {
          try {
            await callCodex("thread/name/set", { threadId: createdThreadId, name }, nameTimeoutMs);
          } catch {}
        }
        return { status: "completed", threadId: createdThreadId, turnId: createdTurnId };
      }
      if (commandType === "start_task" || commandType === "send_input") {
        await callCodex("thread/resume", { threadId, persistExtendedHistory: true });
        const turn = await callCodex("turn/start", {
          threadId,
          clientUserMessageId: `xuan-mobile-${clientRequestId}`,
          input: [{ type: "text", text, text_elements: [] }],
        });
        const acceptedTurnId = String(turn?.turn?.id || turn?.turnId || turn?.id || "");
        if (!acceptedTurnId) throw new Error("turn/start returned no turn id");
        return { status: "completed", threadId, turnId: acceptedTurnId };
      }
      if (commandType === "stop_task") {
        if (!turnId) throw new Error("stop_task requires turnId");
        await callCodex("turn/interrupt", { threadId, turnId });
        return { status: "completed", threadId, turnId };
      }
      return { status: "rejected", errorCode: "unsupported_operation" };
    };
    return true;
  }

  if (install()) return;
  let attempts = 0;
  const retry = window.setInterval(() => {
    attempts += 1;
    if (install() || attempts >= 60) window.clearInterval(retry);
  }, 500);
})();
