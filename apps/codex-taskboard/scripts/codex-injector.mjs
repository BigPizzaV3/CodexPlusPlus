#!/usr/bin/env node

import { spawn, spawnSync } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

import { resolvePort } from "../server/app.mjs";
import {
  parseTaskboardAutomationHostRequest,
  reconcileTaskboardAutomation,
} from "../shared/taskboard-automation.mjs";
import {
  findResidentInjectorPids,
  handleHostBindingPayload,
  reconcileInjectionRuntime,
  restartResidentInjector,
} from "./codex-injector-runtime.mjs";
import { readCodexQuotaStatus } from "./codex-rate-limits.mjs";

const injectorPath = fileURLToPath(import.meta.url);
const projectRoot = path.resolve(path.dirname(injectorPath), "..");
const defaultCodexDebuggingPort = 9229;
const injectionPath = path.join(projectRoot, "inject", "codex-taskboard.user.js");
const automationPoliciesPath = path.join(projectRoot, ".data", "codex-automation-policies.json");
const taskboardOrigin = `http://127.0.0.1:${resolvePort()}`;
const taskboardHealthUrl = `${taskboardOrigin}/health`;
const taskboardPageUrl = `${taskboardOrigin}/?host=codex`;
const cdpHosts = [...new Set([process.env.CODEX_CDP_HOST, "localhost", "127.0.0.1"].filter(Boolean))];
const hostBindingName = "__codexTaskboardHostV1";
const hostHeartbeatName = "__codexTaskboardHostHeartbeatV1";
const hostStartupTokenName = "__codexTaskboardHostStartupTokenV1";
const injectionSourceHashName = "__CODEX_TASKBOARD_SOURCE_HASH__";
const injectionScriptIdentifierName = "__CODEX_TASKBOARD_SCRIPT_IDENTIFIER__";
const codexAutomationMethods = new Set([
  "list-automations",
  "automation-create",
  "automation-update",
]);
let codexAutomationRequestSequence = 0;
const quotaPolicyTimers = new Map();
const quotaPolicyRecords = new Map();
const quotaPolicyQueues = new Map();
let quotaPoliciesLoadPromise = null;
let quotaPoliciesWritePromise = Promise.resolve();
let quotaPoliciesRestored = false;

function parseArgs(argv) {
  const options = {
    port: defaultCodexDebuggingPort,
    portExplicit: false,
    launch: false,
    watch: false,
    open: false,
    refresh: false,
    refreshIfRunning: false,
    attachExisting: false,
    startupToken: null,
    daemon: false,
    screenshot: null,
    appPath: "/Applications/ChatGPT.app",
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--launch") options.launch = true;
    else if (arg === "--watch") options.watch = true;
    else if (arg === "--open") options.open = true;
    else if (arg === "--refresh") options.refresh = true;
    else if (arg === "--refresh-if-running") options.refreshIfRunning = true;
    else if (arg === "--attach-existing") options.attachExisting = true;
    else if (arg === "--startup-token") {
      options.startupToken = argv[++index];
      if (!/^[a-z0-9-]{1,100}$/i.test(options.startupToken || "")) {
        throw new Error("--startup-token must be an identifier");
      }
    }
    else if (arg === "--daemon") options.daemon = true;
    else if (arg === "--port") {
      options.port = Number(argv[++index]);
      options.portExplicit = true;
    }
    else if (arg === "--screenshot") options.screenshot = path.resolve(argv[++index]);
    else if (arg === "--app-path") options.appPath = path.resolve(argv[++index]);
    else throw new Error(`Unknown option: ${arg}`);
  }

  if (!Number.isInteger(options.port) || options.port < 1 || options.port > 65535) {
    throw new Error("--port must be an integer between 1 and 65535");
  }
  return options;
}

async function fetchJson(url) {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
  return response.json();
}

async function isReachable(url) {
  try {
    const response = await fetch(url, { signal: AbortSignal.timeout(1_500) });
    return response.ok;
  } catch {
    return false;
  }
}

async function waitUntilReachable(url, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await isReachable(url)) return;
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  throw new Error(`Timed out waiting for ${url}`);
}

function cdpHttpUrl(host, port, pathname = "") {
  return `http://${host}:${port}${pathname}`;
}

async function resolveCdpBaseUrl(port) {
  for (const host of cdpHosts) {
    const url = cdpHttpUrl(host, port, "/json/version");
    if (await isReachable(url)) return cdpHttpUrl(host, port);
  }
  return null;
}

async function waitUntilCdpReachable(port, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const baseUrl = await resolveCdpBaseUrl(port);
    if (baseUrl) return baseUrl;
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  throw new Error(`Timed out waiting for Codex CDP on ${cdpHosts.map((host) => `${host}:${port}`).join(" or ")}`);
}

async function fetchCdpJson(port, pathname) {
  const baseUrl = await resolveCdpBaseUrl(port);
  if (!baseUrl) {
    throw new Error(`Codex CDP is not listening on ${cdpHosts.map((host) => `${host}:${port}`).join(" or ")}`);
  }
  return fetchJson(`${baseUrl}${pathname}`);
}

function startTaskboard({ detached }) {
  return spawn(process.execPath, [path.join(projectRoot, "server", "index.mjs")], {
    cwd: projectRoot,
    detached,
    stdio: detached ? "ignore" : "inherit",
  });
}

function createTaskboardSupervisor({ detached }) {
  let child = null;
  let ensureInFlight = null;
  let retryAfter = 0;
  let stopping = false;

  async function ensure({ force = false } = {}) {
    if (await isReachable(taskboardHealthUrl)) {
      return { status: "ok", restarted: false };
    }
    if (ensureInFlight) return ensureInFlight;
    if (!force && Date.now() < retryAfter) {
      throw new Error("Taskboard restart is waiting before its next attempt");
    }

    ensureInFlight = (async () => {
      if (child?.exitCode === null && !child.killed) {
        try {
          await waitUntilReachable(taskboardHealthUrl, 3_000);
          return { status: "ok", restarted: false };
        } catch (_) {}
      }

      const started = startTaskboard({ detached });
      child = started;
      if (detached) started.unref();
      started.once("error", (error) => {
        if (!stopping) console.error(`Taskboard process error: ${error.message}`);
      });
      started.once("exit", (code, signal) => {
        if (child === started) child = null;
        if (!stopping && !detached && code !== 0) {
          console.error(`Taskboard exited (${signal || code}); it will be restarted automatically.`);
        }
      });

      try {
        await waitUntilReachable(taskboardHealthUrl, 10_000);
        retryAfter = 0;
        return { status: "ok", restarted: true };
      } catch (error) {
        retryAfter = Date.now() + 2_000;
        throw error;
      }
    })();

    try {
      return await ensureInFlight;
    } finally {
      ensureInFlight = null;
    }
  }

  function stop() {
    stopping = true;
    if (child?.exitCode === null && !child.killed) child.kill("SIGTERM");
  }

  return { ensure, stop };
}

function codexIsRunning() {
  return spawnSync("/usr/bin/pgrep", ["-x", "ChatGPT"], { stdio: "ignore" }).status === 0;
}

function launchCodex(appPath, port) {
  return spawn(
    "/usr/bin/open",
    [
      "-W",
      "-a",
      appPath,
      "--args",
      `--remote-debugging-port=${port}`,
      `--remote-allow-origins=http://localhost:${port},http://127.0.0.1:${port}`,
    ],
    { stdio: "ignore" },
  );
}

function windowsProcessRows() {
  const script = "Get-CimInstance Win32_Process | ForEach-Object { if ($_.CommandLine) { '{0} {1}' -f $_.ProcessId, $_.CommandLine } }";
  const result = spawnSync("powershell.exe", [
    "-NoProfile",
    "-ExecutionPolicy",
    "Bypass",
    "-Command",
    script,
  ], {
    encoding: "utf8",
    maxBuffer: 4 * 1024 * 1024,
  });
  return result.status === 0 ? result.stdout : "";
}

function unixProcessRows() {
  const result = spawnSync("/bin/ps", ["-axo", "pid=,command="], {
    encoding: "utf8",
    maxBuffer: 4 * 1024 * 1024,
  });
  return result.status === 0 ? result.stdout : "";
}

function processRows() {
  return process.platform === "win32" ? windowsProcessRows() : unixProcessRows();
}

class CdpConnection {
  constructor(url) {
    this.socket = new WebSocket(url);
    this.sequence = 0;
    this.pending = new Map();
    this.eventWaiters = new Map();
    this.eventHandlers = new Map();
    this.closed = false;
  }

  async open() {
    await new Promise((resolve, reject) => {
      this.socket.addEventListener("open", resolve, { once: true });
      this.socket.addEventListener("error", () => reject(new Error("CDP WebSocket connection failed")), {
        once: true,
      });
    });
    this.socket.addEventListener("message", (event) => {
      const message = JSON.parse(String(event.data));
      if (!message.id) {
        const waiters = this.eventWaiters.get(message.method) || [];
        this.eventWaiters.delete(message.method);
        waiters.forEach((waiter) => waiter.resolve(message.params));
        const handlers = this.eventHandlers.get(message.method) || [];
        handlers.forEach((handler) => {
          try {
            Promise.resolve(handler(message.params)).catch((error) => {
              console.error(`CDP ${message.method} handler failed: ${error.message}`);
            });
          } catch (error) {
            console.error(`CDP ${message.method} handler failed: ${error.message}`);
          }
        });
        return;
      }
      const pending = this.pending.get(message.id);
      if (!pending) return;
      this.pending.delete(message.id);
      if (message.error) pending.reject(new Error(message.error.message));
      else pending.resolve(message.result);
    });
    this.socket.addEventListener("close", () => {
      this.closed = true;
      const error = new Error("CDP WebSocket closed");
      this.pending.forEach((pending) => pending.reject(error));
      this.pending.clear();
      this.eventWaiters.forEach((waiters) => waiters.forEach((waiter) => waiter.reject(error)));
      this.eventWaiters.clear();
      this.eventHandlers.clear();
    });
  }

  send(method, params = {}) {
    const id = ++this.sequence;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }

  waitFor(method, timeoutMs) {
    return new Promise((resolve, reject) => {
      const waiters = this.eventWaiters.get(method) || [];
      const timeout = setTimeout(() => {
        this.eventWaiters.set(
          method,
          (this.eventWaiters.get(method) || []).filter((waiter) => waiter.resolve !== wrappedResolve),
        );
        reject(new Error(`Timed out waiting for CDP event ${method}`));
      }, timeoutMs);
      const wrappedResolve = (value) => {
        clearTimeout(timeout);
        resolve(value);
      };
      waiters.push({ resolve: wrappedResolve, reject });
      this.eventWaiters.set(method, waiters);
    });
  }

  on(method, handler) {
    const handlers = this.eventHandlers.get(method) || [];
    handlers.push(handler);
    this.eventHandlers.set(method, handlers);
    return () => {
      this.eventHandlers.set(
        method,
        (this.eventHandlers.get(method) || []).filter((candidate) => candidate !== handler),
      );
    };
  }

  close() {
    this.socket.close();
  }
}

async function codexTargets(port) {
  const targets = await fetchCdpJson(port, "/json/list");
  return targets.filter(
    (target) =>
      target.type === "page" &&
      target.webSocketDebuggerUrl &&
      !target.url?.includes("initialRoute=%2Fglobal-dictation") &&
      (target.url?.startsWith("app://") || target.title === "Codex"),
  );
}

function codexDebuggingPorts(preferredPort) {
  const ports = new Set([preferredPort]);

  for (const line of processRows().split("\n")) {
    const command = line.trim().replace(/^\d+\s+/, "");
    if (!/(?:ChatGPT|Codex)(?:\.exe|\.app)?/i.test(command)) continue;
    const match = command.match(/--remote-debugging-port=(\d+)/);
    if (match) ports.add(Number(match[1]));
  }
  return [...ports];
}

function processCwd(pid) {
  if (process.platform === "win32") return null;
  const result = spawnSync("/usr/sbin/lsof", [
    "-a",
    "-p",
    String(pid),
    "-d",
    "cwd",
    "-Fn",
  ], {
    encoding: "utf8",
    maxBuffer: 64 * 1024,
  });
  if (result.status !== 0) return null;
  const cwd = result.stdout.split("\n").find((line) => line.startsWith("n"))?.slice(1);
  return cwd ? path.resolve(cwd) : null;
}

function residentInjectorPids(port) {
  return findResidentInjectorPids({
    processList: processRows(),
    currentPid: process.pid,
    injectorPath,
    projectRoot,
    port,
    defaultPort: defaultCodexDebuggingPort,
    cwdForPid: processCwd,
  });
}

function startResidentInjector(
  port,
  shouldOpen,
  attachExisting = false,
  startupToken = null,
) {
  const [existingPid] = residentInjectorPids(port);
  if (existingPid) return { pid: existingPid, started: false };
  const args = [injectorPath, "--watch", "--port", String(port)];
  if (shouldOpen) args.push("--open");
  if (attachExisting) args.push("--attach-existing");
  if (startupToken) args.push("--startup-token", startupToken);
  const child = spawn(process.execPath, args, {
    cwd: projectRoot,
    detached: true,
    stdio: "ignore",
  });
  child.unref();
  return { pid: child.pid, started: true };
}

async function stopResidentInjector(pid) {
  process.kill(pid, "SIGTERM");
  const deadline = Date.now() + 5_000;
  while (Date.now() < deadline) {
    try {
      process.kill(pid, 0);
      await new Promise((resolve) => setTimeout(resolve, 50));
    } catch {
      return;
    }
  }
  throw new Error(`Timed out stopping resident Taskboard injector ${pid}`);
}

async function waitForResidentInjectorReady(port, pid, startupToken, expectedSourceHash) {
  const deadline = Date.now() + 15_000;
  while (Date.now() < deadline) {
    try {
      process.kill(pid, 0);
      const targets = await codexTargets(port);
      for (const target of targets) {
        const cdp = new CdpConnection(target.webSocketDebuggerUrl);
        await cdp.open();
        try {
          const readiness = await cdp.send("Runtime.evaluate", {
            expression: `({
              token: window[${JSON.stringify(hostStartupTokenName)}],
              taskboardEntryMounted: Boolean(document.getElementById("codex-taskboard-entry")),
              sourceHash: window.__codexTaskboardInjection__?.sourceHash || null
            })`,
            returnByValue: true,
          });
          if (
            readiness.result.value?.token === startupToken
            && readiness.result.value.taskboardEntryMounted
            && readiness.result.value.sourceHash === expectedSourceHash
          ) return;
        } finally {
          cdp.close();
        }
      }
    } catch {}
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`Timed out waiting for resident Taskboard injector ${pid}`);
}

async function restartResidentInjectorForRefresh(port) {
  const { sourceHash } = await currentInjectionSource();
  return restartResidentInjector(port, {
    findResidents: residentInjectorPids,
    stopResident: stopResidentInjector,
    createStartupToken: randomUUID,
    startResident: (targetPort, startupToken) => (
      startResidentInjector(targetPort, false, true, startupToken)
    ),
    waitUntilReady: (targetPort, pid, startupToken) => (
      waitForResidentInjectorReady(targetPort, pid, startupToken, sourceHash)
    ),
  });
}

async function refreshTaskboardFrames(port) {
  const targets = await codexTargets(port);
  const results = [];

  for (const target of targets) {
    const cdp = new CdpConnection(target.webSocketDebuggerUrl);
    await cdp.open();
    try {
      await cdp.send("Runtime.enable");
      const evaluation = await cdp.send("Runtime.evaluate", {
        expression: `(() => {
          const taskboard = window.__codexTaskboardInjection__;
          if (typeof taskboard?.reloadFrame === "function") {
            return { refreshed: taskboard.reloadFrame(), via: "injection" };
          }
          const frame = document.getElementById("codex-taskboard-frame");
          if (!frame) return { refreshed: false, via: "not-mounted" };
          const url = new URL(frame.getAttribute("src") || frame.src);
          url.searchParams.set("__codex_taskboard_refresh", Date.now().toString(36));
          frame.setAttribute("src", url.href);
          return { refreshed: true, via: "fallback", frameUrl: url.href };
        })()`,
        returnByValue: true,
      });
      if (evaluation.exceptionDetails) {
        throw new Error(
          evaluation.exceptionDetails.exception?.description || "Taskboard frame refresh failed",
        );
      }
      results.push({
        targetId: target.id,
        title: target.title,
        url: target.url,
        ...evaluation.result.value,
      });
    } finally {
      cdp.close();
    }
  }

  return results;
}

function frameTreeContains(frameTree, expectedUrl) {
  if (frameTree.frame?.url === expectedUrl) return true;
  return frameTree.childFrames?.some((child) => frameTreeContains(child, expectedUrl)) || false;
}

async function waitForFrame(cdp, expectedUrl, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const [{ targetInfos }, { frameTree }] = await Promise.all([
      cdp.send("Target.getTargets"),
      cdp.send("Page.getFrameTree"),
    ]);
    if (
      targetInfos.some((target) => target.type === "iframe" && target.url === expectedUrl) ||
      frameTreeContains(frameTree, expectedUrl)
    ) {
      return true;
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  return false;
}

async function requestCodexAutomationViaCdp(cdp, executionContextId, method, params) {
  if (!codexAutomationMethods.has(method)) {
    throw new Error(`Unsupported Codex automation method: ${method}`);
  }
  const requestId = [
    "taskboard-automation",
    process.pid,
    Date.now().toString(36),
    (++codexAutomationRequestSequence).toString(36),
  ].join("-");
  const evaluation = await cdp.send("Runtime.evaluate", {
    expression: `(() => new Promise((resolve) => {
      const method = ${JSON.stringify(method)};
      const params = ${JSON.stringify(params)};
      const requestId = ${JSON.stringify(requestId)};
      const bridge = window.electronBridge;
      if (!bridge || typeof bridge.sendMessageFromView !== "function") {
        resolve({ ok: false, error: "当前 Codex 版本没有提供原生自动任务能力" });
        return;
      }
      let settled = false;
      const finish = (result) => {
        if (settled) return;
        settled = true;
        window.clearTimeout(timeout);
        window.removeEventListener("message", onMessage);
        resolve(result);
      };
      const onMessage = (event) => {
        const message = event.data;
        if (
          !message
          || typeof message !== "object"
          || message.type !== "fetch-response"
          || message.requestId !== requestId
        ) return;
        finish({
          ok: true,
          responseType: message.responseType,
          status: message.status,
          bodyJsonString: message.bodyJsonString,
        });
      };
      const timeout = window.setTimeout(
        () => finish({ ok: false, error: "Codex 自动任务接口没有响应" }),
        10_000,
      );
      window.addEventListener("message", onMessage);
      Promise.resolve(bridge.sendMessageFromView({
        type: "fetch",
        requestId,
        method: "POST",
        url: \`vscode://codex/${method}\`,
        body: JSON.stringify(params),
      })).catch((error) => {
        finish({
          ok: false,
          error: error instanceof Error ? error.message : String(error),
        });
      });
    }))()`,
    ...(Number.isInteger(executionContextId) ? { contextId: executionContextId } : {}),
    awaitPromise: true,
    returnByValue: true,
  });
  if (evaluation.exceptionDetails) {
    throw new Error(
      evaluation.exceptionDetails.exception?.description
      || "Codex automation request failed",
    );
  }
  const response = evaluation.result.value;
  if (!response?.ok) throw new Error(response?.error || "Codex automation request failed");
  if (!Number.isInteger(response.status) || response.status < 200 || response.status >= 300) {
    throw new Error(`Codex automation request returned HTTP ${response.status}`);
  }
  if (typeof response.bodyJsonString !== "string" || response.bodyJsonString.length === 0) {
    return {};
  }
  try {
    return JSON.parse(response.bodyJsonString);
  } catch {
    throw new Error("Codex automation request returned invalid JSON");
  }
}

async function applyTaskboardAutomationPolicy(request, rpc, stillCurrent = () => true) {
  const quota = request.quotaAware
    ? await readCodexQuotaStatus(request.model)
    : null;
  if (!stillCurrent()) return { quota, stale: true };
  const shouldRun = request.enabledByUser
    && (!request.quotaAware || quota?.state === "available");
  const result = await reconcileTaskboardAutomation(
    { ...request, operation: shouldRun ? "ensure-active" : "pause" },
    rpc,
  );
  if (result?.error === "not-found") {
    return { ...(quota ? { quota } : {}) };
  }
  return { ...result, ...(quota ? { quota } : {}) };
}

function storedAutomationPolicy(request) {
  return {
    taskboardProjectId: request.taskboardProjectId,
    codexProjectId: request.codexProjectId,
    projectName: request.projectName,
    workspacePath: request.workspacePath,
    skillPath: request.skillPath,
    ...(request.automationId ? { automationId: request.automationId } : {}),
    enabledByUser: request.enabledByUser,
    quotaAware: request.quotaAware,
    intervalMinutes: request.intervalMinutes,
    model: request.model,
    reasoningEffort: request.reasoningEffort,
  };
}

function restoredAutomationPolicy(value) {
  return parseTaskboardAutomationHostRequest({
    ...value,
    id: "restored-policy",
    action: "automation",
    requestId: "restored-policy",
    operation: "apply-policy",
  });
}

async function ensureQuotaPoliciesLoaded() {
  if (quotaPoliciesLoadPromise) return quotaPoliciesLoadPromise;
  quotaPoliciesLoadPromise = (async () => {
    let stored = {};
    try {
      stored = JSON.parse(await readFile(automationPoliciesPath, "utf8"));
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
    if (!stored || typeof stored !== "object" || Array.isArray(stored)) return;
    for (const value of Object.values(stored)) {
      const request = restoredAutomationPolicy(value);
      if (!request) continue;
      quotaPolicyRecords.set(request.taskboardProjectId, { version: 1, request });
    }
  })();
  return quotaPoliciesLoadPromise;
}

function persistQuotaPolicies() {
  const data = Object.fromEntries(
    [...quotaPolicyRecords.entries()].map(([projectId, record]) => [
      projectId,
      storedAutomationPolicy(record.request),
    ]),
  );
  quotaPoliciesWritePromise = quotaPoliciesWritePromise
    .catch(() => {})
    .then(async () => {
      await mkdir(path.dirname(automationPoliciesPath), { recursive: true });
      await writeFile(automationPoliciesPath, `${JSON.stringify(data, null, 2)}\n`, {
        mode: 0o600,
      });
    });
  return quotaPoliciesWritePromise;
}

function scheduleQuotaPolicyCheck(record, cdp, result) {
  const { request, version } = record;
  const key = request.taskboardProjectId;
  const previous = quotaPolicyTimers.get(key);
  if (previous) clearTimeout(previous);
  quotaPolicyTimers.delete(key);
  if (!request.enabledByUser || !request.quotaAware) return;

  const nextRunAt = Number(result.item?.nextRunAt);
  const nextRunDelay = Number.isFinite(nextRunAt) && nextRunAt > Date.now()
    ? Math.max(1_000, nextRunAt - Date.now() - 15_000)
    : 60_000;
  const resetDelay = result.quota?.state === "blocked"
    && Number.isFinite(result.quota.resetsAt)
    ? Math.max(1_000, result.quota.resetsAt * 1_000 - Date.now() + 1_000)
    : nextRunDelay;
  const timer = setTimeout(async () => {
    if (quotaPolicyRecords.get(key)?.version !== version) return;
    try {
      await enqueueCurrentQuotaPolicy(key, cdp);
    } catch (error) {
      console.error(`Taskboard quota policy check failed: ${error.message}`);
      const current = quotaPolicyRecords.get(key);
      if (current?.version === version) {
        scheduleQuotaPolicyCheck(current, cdp, { quota: { state: "unknown" } });
      }
    }
  }, Math.min(nextRunDelay, resetDelay));
  timer.unref();
  quotaPolicyTimers.set(key, timer);
}

function enqueueQuotaPolicyMutation(record, cdp, rpc) {
  const key = record.request.taskboardProjectId;
  const previous = quotaPolicyQueues.get(key) ?? Promise.resolve();
  const run = previous
    .catch(() => {})
    .then(async () => {
      const current = quotaPolicyRecords.get(key);
      if (!current || current.version !== record.version) return { stale: true };
      const result = await applyTaskboardAutomationPolicy(
        current.request,
        rpc,
        () => quotaPolicyRecords.get(key)?.version === current.version,
      );
      if (result.stale) return result;
      if (result.item?.id && quotaPolicyRecords.get(key)?.version === current.version) {
        current.request = { ...current.request, automationId: result.item.id };
        await persistQuotaPolicies();
      }
      scheduleQuotaPolicyCheck(current, cdp, result);
      return result;
    });
  const tracked = run.finally(() => {
    if (quotaPolicyQueues.get(key) === tracked) quotaPolicyQueues.delete(key);
  });
  quotaPolicyQueues.set(key, tracked);
  return tracked;
}

async function updateAndApplyQuotaPolicy(request, cdp, rpc) {
  await ensureQuotaPoliciesLoaded();
  const previous = quotaPolicyRecords.get(request.taskboardProjectId);
  const record = {
    version: (previous?.version ?? 0) + 1,
    request,
  };
  quotaPolicyRecords.set(request.taskboardProjectId, record);
  try {
    await persistQuotaPolicies();
    return await enqueueQuotaPolicyMutation(record, cdp, rpc);
  } catch (error) {
    if (quotaPolicyRecords.get(request.taskboardProjectId)?.version === record.version) {
      if (previous) quotaPolicyRecords.set(request.taskboardProjectId, previous);
      else quotaPolicyRecords.delete(request.taskboardProjectId);
      await persistQuotaPolicies();
    }
    throw error;
  }
}

async function readStoredAutomationPolicy(projectId) {
  await ensureQuotaPoliciesLoaded();
  const record = quotaPolicyRecords.get(projectId);
  return record ? storedAutomationPolicy(record.request) : null;
}

async function enqueueCurrentQuotaPolicy(projectId, cdp) {
  await ensureQuotaPoliciesLoaded();
  const record = quotaPolicyRecords.get(projectId);
  if (!record) return { stale: true };
  return enqueueQuotaPolicyMutation(
    record,
    cdp,
    (method, body) => requestCodexAutomationViaCdp(cdp, undefined, method, body),
  );
}

async function restoreQuotaPolicies(cdp) {
  if (quotaPoliciesRestored) return;
  quotaPoliciesRestored = true;
  await ensureQuotaPoliciesLoaded();
  for (const [projectId, record] of quotaPolicyRecords) {
    if (record.request.enabledByUser && record.request.quotaAware) {
      void enqueueCurrentQuotaPolicy(projectId, cdp).catch((error) => {
        console.error(`Taskboard quota policy restore failed: ${error.message}`);
      });
    }
  }
}

async function prefillTaskComposerViaCdp(cdp, executionContextId, request) {
  const {
    instruction,
    skillDisplayName,
    skillName,
    skillPath,
  } = request;
  const deadline = Date.now() + 8_000;
  while (Date.now() < deadline) {
    const prepared = await cdp.send("Runtime.evaluate", {
      expression: `(() => {
        const instruction = ${JSON.stringify(instruction)};
        const skillName = ${JSON.stringify(skillName)};
        const skillPath = ${JSON.stringify(skillPath)};
        const editor = Array.from(document.querySelectorAll(
          '[data-codex-composer="true"][contenteditable="true"]'
        )).find((candidate) => candidate.getClientRects().length > 0);
        if (!editor) return { ready: false };
        const mention = Array.from(editor.querySelectorAll("[skill-mention-name]"))
          .find((candidate) => (
            candidate.getAttribute("skill-mention-name") === skillName
            && candidate.getAttribute("skill-mention-path") === skillPath
          ));
        if (mention && (editor.textContent || "").includes(instruction)) {
          return { ready: true, matches: true };
        }
        editor.focus();
        const selection = window.getSelection();
        const range = document.createRange();
        range.selectNodeContents(editor);
        selection?.removeAllRanges();
        selection?.addRange(range);
        return { ready: true, matches: false };
      })()`,
      contextId: executionContextId,
      returnByValue: true,
    });
    if (!prepared.result.value?.ready) {
      await new Promise((resolve) => setTimeout(resolve, 80));
      continue;
    }
    if (prepared.result.value.matches) return { prefilled: true };

    await cdp.send("Input.insertText", { text: "$" });
    break;
  }

  let selectedSkill = false;
  while (Date.now() < deadline) {
    const selection = await cdp.send("Runtime.evaluate", {
      expression: `(() => {
        const displayName = ${JSON.stringify(skillDisplayName)};
        const overlay = Array.from(document.querySelectorAll(
          '[data-composer-overlay-floating-ui="true"]'
        )).find((candidate) => candidate.getClientRects().length > 0);
        if (!overlay) return { ready: false };
        const button = Array.from(overlay.querySelectorAll(
          'button[data-list-navigation-item="true"]'
        )).find((candidate) => Array.from(candidate.querySelectorAll("span"))
          .some((label) => (label.textContent || "").trim() === displayName));
        if (!button) return { ready: true, found: false };
        button.click();
        return { ready: true, found: true };
      })()`,
      contextId: executionContextId,
      returnByValue: true,
    });
    if (selection.result.value?.found) {
      selectedSkill = true;
      break;
    }
    await new Promise((resolve) => setTimeout(resolve, 80));
  }
  if (!selectedSkill) {
    throw new Error(`Timed out while selecting the ${skillDisplayName} Skill`);
  }

  let mentionReady = false;
  while (Date.now() < deadline) {
    const mention = await cdp.send("Runtime.evaluate", {
      expression: `(() => {
        const skillName = ${JSON.stringify(skillName)};
        const skillPath = ${JSON.stringify(skillPath)};
        const editor = Array.from(document.querySelectorAll(
          '[data-codex-composer="true"][contenteditable="true"]'
        )).find((candidate) => candidate.getClientRects().length > 0);
        if (!editor) return { ready: false };
        const selected = Array.from(editor.querySelectorAll("[skill-mention-name]"))
          .find((candidate) => candidate.getAttribute("skill-mention-name") === skillName);
        return {
          ready: Boolean(selected),
          pathMatches: selected?.getAttribute("skill-mention-path") === skillPath,
        };
      })()`,
      contextId: executionContextId,
      returnByValue: true,
    });
    if (mention.result.value?.ready) {
      if (!mention.result.value.pathMatches) {
        throw new Error(`Codex selected a different ${skillDisplayName} Skill`);
      }
      mentionReady = true;
      break;
    }
    await new Promise((resolve) => setTimeout(resolve, 80));
  }
  if (!mentionReady) {
    throw new Error(`Timed out while creating the ${skillDisplayName} Skill mention`);
  }

  await cdp.send("Input.insertText", { text: instruction });
  while (Date.now() < deadline) {
    const verified = await cdp.send("Runtime.evaluate", {
      expression: `(() => {
        const instruction = ${JSON.stringify(instruction)};
        const skillName = ${JSON.stringify(skillName)};
        const skillPath = ${JSON.stringify(skillPath)};
        const editor = Array.from(document.querySelectorAll(
          '[data-codex-composer="true"][contenteditable="true"]'
        )).find((candidate) => candidate.getClientRects().length > 0);
        const mention = editor && Array.from(editor.querySelectorAll("[skill-mention-name]"))
          .find((candidate) => (
            candidate.getAttribute("skill-mention-name") === skillName
            && candidate.getAttribute("skill-mention-path") === skillPath
          ));
        return Boolean(mention && (editor.textContent || "").includes(instruction));
      })()`,
      contextId: executionContextId,
      returnByValue: true,
    });
    if (verified.result.value === true) return { prefilled: true };
    await new Promise((resolve) => setTimeout(resolve, 80));
  }
  throw new Error("Timed out while writing the issue instruction into the Codex composer");
}

async function sendHostResponse(cdp, executionContextId, response) {
  await cdp.send("Runtime.evaluate", {
    expression: `window.__codexTaskboardInjection__?.hostResponse(${JSON.stringify(response)})`,
    contextId: executionContextId,
    returnByValue: true,
  });
}

async function installTaskboardHostBinding(cdp, supervisor) {
  cdp.on("Runtime.bindingCalled", async (params) => {
    if (params.name !== hostBindingName) return;
    await handleHostBindingPayload(params, {
      parseAutomationRequest: parseTaskboardAutomationHostRequest,
      ensure: () => supervisor.ensure({ force: true }),
      runAutomation: (request, executionContextId) => (
        (async () => {
          const rpc = (method, body) => requestCodexAutomationViaCdp(
            cdp,
            executionContextId,
            method,
            body,
          );
          const result = request.operation === "apply-policy"
            ? await updateAndApplyQuotaPolicy(request, cdp, rpc)
            : await reconcileTaskboardAutomation(request, rpc);
          if (request.operation === "list") {
            const policy = await readStoredAutomationPolicy(request.taskboardProjectId);
            return { ...result, ...(policy ? { policy } : {}) };
          }
          return result;
        })()
      ),
      prefill: (request, executionContextId) => (
        prefillTaskComposerViaCdp(cdp, executionContextId, request)
      ),
      sendResponse: (executionContextId, response) => (
        sendHostResponse(cdp, executionContextId, response)
      ),
    });
  });
  await cdp.send("Runtime.addBinding", { name: hostBindingName });
  await restoreQuotaPolicies(cdp);
}

async function publishHostHeartbeat(cdp, startupToken) {
  await cdp.send("Runtime.evaluate", {
    expression: `(() => {
      window[${JSON.stringify(hostHeartbeatName)}] = Date.now();
      window[${JSON.stringify(hostStartupTokenName)}] = ${JSON.stringify(startupToken)};
    })()`,
    returnByValue: true,
  });
}

async function readInjectionStatus(cdp) {
  const status = await cdp.send("Runtime.evaluate", {
    expression: `({
      version: window.__codexTaskboardInjection__?.version || null,
      sourceHash: window.__codexTaskboardInjection__?.sourceHash || null,
      scriptIdentifier: window[${JSON.stringify(injectionScriptIdentifierName)}] || null,
      entryMounted: Boolean(document.getElementById("codex-taskboard-entry")),
      pageMounted: Boolean(document.getElementById("codex-taskboard-page")),
      pageVisible: document.getElementById("codex-taskboard-page")?.hidden === false,
      frameUrl: document.getElementById("codex-taskboard-frame")?.src || null
    })`,
    returnByValue: true,
  });
  return status.result.value;
}

async function waitForInjectionStatus(cdp, shouldOpen, expectedSourceHash, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let status = await readInjectionStatus(cdp);
  while (
    Date.now() < deadline
    && (
      status.sourceHash !== expectedSourceHash
      || !status.entryMounted
      || (shouldOpen && (!status.pageVisible || !status.frameUrl))
    )
  ) {
    await new Promise((resolve) => setTimeout(resolve, 250));
    status = await readInjectionStatus(cdp);
  }
  return status;
}

async function evaluateInjectionSource(cdp, source) {
  const evaluation = await cdp.send("Runtime.evaluate", {
    expression: source,
    awaitPromise: true,
    returnByValue: true,
  });
  if (evaluation.exceptionDetails) {
    throw new Error(
      evaluation.exceptionDetails.exception?.description || "Taskboard injection failed",
    );
  }
}

async function publishInjectionScriptIdentifier(cdp, scriptIdentifier) {
  await cdp.send("Runtime.evaluate", {
    expression: `window[${JSON.stringify(injectionScriptIdentifierName)}] = ${JSON.stringify(scriptIdentifier)}`,
    returnByValue: true,
  });
}

async function registerInjectionSource(cdp, source) {
  const registration = await cdp.send("Page.addScriptToEvaluateOnNewDocument", {
    source: `${source}\n//# sourceURL=codex-taskboard.user.js`,
  });
  return registration.identifier;
}

async function injectTarget(
  target,
  source,
  sourceHash,
  shouldOpen,
  screenshotPath,
  keepAlive,
  supervisor,
  attachExisting,
  startupToken,
) {
  const cdp = new CdpConnection(target.webSocketDebuggerUrl);
  let retained = false;
  await cdp.open();
  try {
    await cdp.send("Page.enable");
    await cdp.send("Page.setBypassCSP", { enabled: true });
    await cdp.send("Runtime.enable");
    if (keepAlive) await installTaskboardHostBinding(cdp, supervisor);
    if (keepAlive && attachExisting) {
      const currentStatus = await readInjectionStatus(cdp);
      const reconciled = await reconcileInjectionRuntime({
        currentStatus,
        source,
        sourceHash,
        removeRegisteredSource: (identifier) => cdp.send(
          "Page.removeScriptToEvaluateOnNewDocument",
          { identifier },
        ),
        registerCurrentSource: (currentSource) => registerInjectionSource(cdp, currentSource),
        evaluateCurrentSource: (currentSource) => evaluateInjectionSource(cdp, currentSource),
        publishRegistration: (identifier) => publishInjectionScriptIdentifier(cdp, identifier),
        reopen: () => cdp.send("Runtime.evaluate", {
          expression: "window.__codexTaskboardInjection__?.open()",
          returnByValue: true,
        }),
      });
      cdp.on("Page.loadEventFired", () => (
        publishInjectionScriptIdentifier(cdp, reconciled.scriptIdentifier)
      ));
      await publishHostHeartbeat(cdp, startupToken);
      const status = await waitForInjectionStatus(
        cdp,
        reconciled.shouldRemainOpen,
        sourceHash,
        15_000,
      );
      const frameLoaded = status.frameUrl
        ? await waitForFrame(cdp, status.frameUrl, 15_000)
        : false;
      retained = true;
      return {
        result: { ...status, cspBypassed: true, frameLoaded },
        connection: cdp,
      };
    }
    const scriptIdentifier = await registerInjectionSource(cdp, source);
    cdp.on("Page.loadEventFired", () => (
      publishInjectionScriptIdentifier(cdp, scriptIdentifier)
    ));
    const reloaded = cdp.waitFor("Page.loadEventFired", 15_000);
    await cdp.send("Page.reload");
    await reloaded;
    await evaluateInjectionSource(cdp, source);
    await publishInjectionScriptIdentifier(cdp, scriptIdentifier);
    if (keepAlive) await publishHostHeartbeat(cdp, startupToken);
    if (shouldOpen) {
      await cdp.send("Runtime.evaluate", {
        expression: `(() => {
          const taskboard = window.__codexTaskboardInjection__;
          taskboard?.close();
          taskboard?.open();
        })()`,
      });
      await new Promise((resolve) => setTimeout(resolve, 500));
    }
    const status = await waitForInjectionStatus(cdp, shouldOpen, sourceHash, 15_000);
    const frameLoaded = status.frameUrl
      ? await waitForFrame(cdp, status.frameUrl, 15_000)
      : false;
    if (shouldOpen && !frameLoaded) {
      throw new Error("Taskboard iframe did not finish loading in the Codex renderer");
    }
    const result = {
      ...status,
      cspBypassed: true,
      frameLoaded,
    };
    if (screenshotPath) {
      const screenshot = await cdp.send("Page.captureScreenshot", { format: "png" });
      await writeFile(screenshotPath, Buffer.from(screenshot.data, "base64"));
      result.screenshot = screenshotPath;
    }
    retained = keepAlive;
    return { result, connection: retained ? cdp : null };
  } finally {
    if (!retained) cdp.close();
  }
}

async function injectAll(
  port,
  source,
  sourceHash,
  shouldOpen,
  screenshotPath,
  injectedTargets,
  keepAlive,
  supervisor,
  attachExisting,
  startupToken,
) {
  const targets = await codexTargets(port);
  if (targets.length === 0) throw new Error("No Codex renderer target found");

  const activeIds = new Set(targets.map((target) => target.id));
  for (const [id, connection] of injectedTargets) {
    if (!activeIds.has(id) || connection.closed) {
      connection.close();
      injectedTargets.delete(id);
    }
  }

  const results = [];
  for (const target of targets) {
    if (injectedTargets.has(target.id)) continue;
    const firstTarget = injectedTargets.size === 0 && results.length === 0;
    const { result, connection } = await injectTarget(
      target,
      source,
      sourceHash,
      shouldOpen && firstTarget,
      firstTarget ? screenshotPath : null,
      keepAlive,
      supervisor,
      attachExisting,
      startupToken,
    );
    if (connection) injectedTargets.set(target.id, connection);
    results.push({ targetId: target.id, title: target.title, url: target.url, ...result });
  }
  return results;
}

async function injectInitialWhenRendererReady(options) {
  const deadline = Date.now() + (options.keepAlive ? 30_000 : 0);
  while (true) {
    try {
      return await injectAll(
        options.port,
        options.source,
        options.sourceHash,
        options.shouldOpen,
        options.screenshotPath,
        options.injectedTargets,
        options.keepAlive,
        options.supervisor,
        options.attachExisting,
        options.startupToken,
      );
    } catch (error) {
      if (
        !options.keepAlive
        || error.message !== "No Codex renderer target found"
        || Date.now() >= deadline
      ) {
        throw error;
      }
      console.error(`Waiting for Codex renderer: ${error.message}`);
      await new Promise((resolve) => setTimeout(resolve, 500));
    }
  }
}

async function currentInjectionSource() {
  const userScript = await readFile(injectionPath, "utf8");
  const runtimeSource = `window.__CODEX_TASKBOARD_MANAGED_ORIGIN__ = ${JSON.stringify(taskboardOrigin)};
if (typeof window.__CODEX_TASKBOARD_URL__ !== "string" || !window.__CODEX_TASKBOARD_URL__.trim()) {
  window.__CODEX_TASKBOARD_URL__ = ${JSON.stringify(taskboardPageUrl)};
}
${userScript}`;
  const sourceHash = createHash("sha256").update(runtimeSource).digest("hex");
  return {
    sourceHash,
    source: `window[${JSON.stringify(injectionSourceHashName)}] = ${JSON.stringify(sourceHash)};
${runtimeSource}`,
  };
}

async function main() {
  const options = parseArgs(process.argv.slice(2));

  if (options.daemon) {
    let port = options.port;
    if (!options.portExplicit) {
      const candidates = codexDebuggingPorts(options.port);
      const activePort = await Promise.any(candidates.map(async (candidate) => {
        if (!(await resolveCdpBaseUrl(candidate))) {
          throw new Error("unreachable");
        }
        if ((await codexTargets(candidate)).length === 0) throw new Error("not Codex");
        return candidate;
      })).catch(() => null);
      if (!activePort) throw new Error("No debuggable Codex window found");
      port = activePort;
    }
    console.log(JSON.stringify({ launcher: startResidentInjector(port, options.open), port }, null, 2));
    return;
  }

  if (options.refresh || options.refreshIfRunning) {
    const ports = options.portExplicit
      ? [options.port]
      : codexDebuggingPorts(options.port);
    const refreshed = [];
    for (const port of ports) {
      if (!(await resolveCdpBaseUrl(port))) continue;
      if (options.refreshIfRunning) await restartResidentInjectorForRefresh(port);
      const results = await refreshTaskboardFrames(port);
      refreshed.push(...results.map((result) => ({ port, ...result })));
    }
    if (refreshed.length === 0) {
      if (options.refreshIfRunning) {
        console.log(JSON.stringify({ refreshed: [], skipped: "No debuggable Codex window is running" }));
        return;
      }
      throw new Error(`No debuggable Codex window found on ports: ${ports.join(", ")}`);
    }
    console.log(JSON.stringify({ refreshed }, null, 2));
    return;
  }

  let codexProcess = null;
  const supervisor = createTaskboardSupervisor({ detached: !options.watch });

  try {
    const cdpReachable = Boolean(await resolveCdpBaseUrl(options.port));
    if (!cdpReachable) {
      if (!options.launch) {
        throw new Error(`Codex CDP is not listening on ${cdpHosts.map((host) => `${host}:${options.port}`).join(" or ")}`);
      }
      if (codexIsRunning()) {
        throw new Error(
          "Codex is already running without this CDP port. Quit Codex completely, then run this command again.",
        );
      }
    }

    await supervisor.ensure({ force: true });

    if (!cdpReachable) {
      codexProcess = launchCodex(options.appPath, options.port);
      await waitUntilCdpReachable(options.port, 30_000);
    }

    const { source, sourceHash } = await currentInjectionSource();
    const injectedTargets = new Map();
    const firstResults = await injectInitialWhenRendererReady({
      port: options.port,
      source,
      sourceHash,
      shouldOpen: options.open,
      screenshotPath: options.screenshot,
      injectedTargets,
      keepAlive: options.watch,
      supervisor,
      attachExisting: options.attachExisting,
      startupToken: options.startupToken,
    });
    console.log(JSON.stringify({ injected: firstResults }, null, 2));

    if (!options.watch) {
      codexProcess?.unref();
      return;
    }

    const stop = () => {
      injectedTargets.forEach((connection) => connection.close());
      supervisor.stop();
      process.exit(0);
    };
    process.once("SIGINT", stop);
    process.once("SIGTERM", stop);

    while (true) {
      await new Promise((resolve) => setTimeout(resolve, 2_000));
      if (!(await resolveCdpBaseUrl(options.port))) break;
      try {
        await supervisor.ensure();
      } catch (error) {
        console.error(`Waiting for Taskboard service: ${error.message}`);
      }
      for (const connection of injectedTargets.values()) {
        try {
          await publishHostHeartbeat(connection, options.startupToken);
        } catch (_) {}
      }
      try {
        const results = await injectAll(
          options.port,
          source,
          sourceHash,
          false,
          null,
          injectedTargets,
          true,
          supervisor,
          options.attachExisting,
          options.startupToken,
        );
        if (results.length > 0) console.log(JSON.stringify({ injected: results }, null, 2));
      } catch (error) {
        if ((codexProcess && codexProcess.exitCode !== null) || !(await resolveCdpBaseUrl(options.port))) {
          break;
        }
        console.error(`Waiting for Codex renderer: ${error.message}`);
      }
    }
    supervisor.stop();
  } catch (error) {
    supervisor.stop();
    throw error;
  }
}

main().catch((error) => {
  console.error(error.message);
  process.exitCode = 1;
});
