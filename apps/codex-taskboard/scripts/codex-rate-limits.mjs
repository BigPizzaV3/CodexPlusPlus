import { spawn } from "node:child_process";
import readline from "node:readline";

const REQUEST_TIMEOUT_MS = 8_000;

export async function readCodexQuotaStatus(model) {
  const checkedAt = Date.now();
  try {
    const session = startAppServer();
    try {
      await session.request("initialize", {
        clientInfo: { name: "codex-taskboard", version: "0.1.0" },
      });
      session.notify("initialized", {});
      const account = await session.request("account/read", { refreshToken: false });
      if (account?.account?.type === "apiKey") {
        return { state: "unavailable", reason: "api-key", checkedAt };
      }
      if (account?.account?.type !== "chatgpt") {
        return { state: "unknown", checkedAt };
      }
      const result = await session.request("account/rateLimits/read", {});
      return evaluateRateLimits(result, model, checkedAt);
    } finally {
      session.close();
    }
  } catch {
    return { state: "unknown", checkedAt };
  }
}

function startAppServer() {
  const child = spawn("codex", ["app-server", "--stdio"], {
    stdio: ["pipe", "pipe", "ignore"],
  });
  const pending = new Map();
  let sequence = 0;
  let closed = false;
  const lines = readline.createInterface({ input: child.stdout });

  lines.on("line", (line) => {
    let message;
    try {
      message = JSON.parse(line);
    } catch {
      return;
    }
    if (!Number.isInteger(message?.id)) return;
    const request = pending.get(message.id);
    if (!request) return;
    pending.delete(message.id);
    clearTimeout(request.timeout);
    if (message.error) request.reject(new Error("Codex App Server request failed"));
    else request.resolve(message.result);
  });

  const failPending = () => {
    if (closed) return;
    closed = true;
    for (const request of pending.values()) {
      clearTimeout(request.timeout);
      request.reject(new Error("Codex App Server stopped"));
    }
    pending.clear();
  };
  child.once("error", failPending);
  child.once("exit", failPending);

  function write(message) {
    child.stdin.write(`${JSON.stringify(message)}\n`);
  }

  return {
    request(method, params) {
      const id = ++sequence;
      return new Promise((resolve, reject) => {
        const timeout = setTimeout(() => {
          pending.delete(id);
          reject(new Error("Codex App Server request timed out"));
        }, REQUEST_TIMEOUT_MS);
        pending.set(id, { resolve, reject, timeout });
        write({ id, method, params });
      });
    },
    notify(method, params) {
      write({ method, params });
    },
    close() {
      closed = true;
      lines.close();
      child.stdin.end();
      child.kill("SIGTERM");
      for (const request of pending.values()) {
        clearTimeout(request.timeout);
        request.reject(new Error("Codex App Server stopped"));
      }
      pending.clear();
    },
  };
}

function evaluateRateLimits(result, model, checkedAt) {
  const snapshots = result?.rateLimitsByLimitId;
  const entries = snapshots && typeof snapshots === "object" && !Array.isArray(snapshots)
    ? Object.entries(snapshots).filter(([, snapshot]) => (
    snapshot && typeof snapshot === "object"
    ))
    : [];
  const normalizedModel = normalizeName(model);
  const snapshot = entries.find(([, value]) => (
    normalizeName(value.limitName) === normalizedModel
  ))?.[1]
    ?? entries.find(([limitId]) => limitId === "codex")?.[1]
    ?? (
      result?.rateLimits
      && typeof result.rateLimits === "object"
      && !Array.isArray(result.rateLimits)
        ? result.rateLimits
        : null
    );
  if (!snapshot) return { state: "unknown", checkedAt };

  const windows = [snapshot.primary, snapshot.secondary].filter(Boolean);
  const creditsAvailable = snapshot.credits?.unlimited === true
    || snapshot.credits?.hasCredits === true;
  const exhaustedWindows = windows.filter((window) => (
    Number(window.usedPercent) >= 100
  ));
  const individuallyExhausted = Number(snapshot.individualLimit?.remainingPercent) <= 0;
  const blocked = Boolean(snapshot.rateLimitReachedType)
    || snapshot.spendControlReached === true
    || individuallyExhausted
    || (exhaustedWindows.length > 0 && !creditsAvailable);

  if (!blocked) return { state: "available", checkedAt };

  const resetCandidates = [
    ...exhaustedWindows.map((window) => Number(window.resetsAt)),
    Number(snapshot.individualLimit?.resetsAt),
  ];
  if (snapshot.rateLimitReachedType || snapshot.spendControlReached === true) {
    resetCandidates.push(...windows.map((window) => Number(window.resetsAt)));
  }
  const resetsAt = Math.max(...resetCandidates.filter(Number.isFinite));
  return {
    state: "blocked",
    checkedAt,
    ...(Number.isFinite(resetsAt) ? { resetsAt } : {}),
  };
}

function normalizeName(value) {
  return typeof value === "string"
    ? value.toLowerCase().replace(/[^a-z0-9]+/g, "")
    : "";
}
