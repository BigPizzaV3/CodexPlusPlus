// Windows acceptance test. Uses real isolated Codex windows and synthetic local suppliers.
// NODE_PATH must resolve Playwright; no dependencies are installed by this script.
const assert = require("node:assert/strict");
const fs = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");
const net = require("node:net");
const http = require("node:http");
const { spawn, execFileSync } = require("node:child_process");
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
// Windows FILETIME 超过 JS 安全整数范围；编辑清单时必须保留原始数字。
function editableJson(contents) {
  return JSON.parse(contents, (_key, value, context) =>
    typeof value === "number" && !Number.isSafeInteger(value) ? JSON.rawJSON(context.source) : value);
}
let interrupted = "", cleaning = false;
const scenarioController = new AbortController();
for (const signal of ["SIGINT", "SIGTERM"]) process.on(signal, () => {
  interrupted = signal;
  scenarioController.abort(new Error(`acceptance interrupted: ${signal}`));
  process.exitCode = 1;
});
if (process.argv[2] === "--self-test") {
  selfTest().catch(error => { console.error(error); process.exitCode = 1; });
  return;
}
const { chromium } = require("playwright");
const { installDialogHandling } = require(path.join(
  process.env.CODEX_ISOLATED_TESTING_SKILL
    || path.join(process.env.USERPROFILE, ".codex", "skills", "codex-isolated-testing"),
  "scripts", "playwright-ui.cjs"));

assert.equal(process.platform, "win32");
assert.equal(process.argv.length, 5, "Usage: node verify-desktop-continuity.cjs <manager.exe> <Codex app dir> <output dir>");
const [executable, appDir, outputDir] = process.argv.slice(2).map(value => path.resolve(value));
const token = `desktop-continuity-${Date.now()}-${process.pid}`;
const temporary = path.join(os.tmpdir(), token);
async function bounded(promise, ms = 10000, reason = "operation timed out") {
  let timer, interruptTimer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      const check = () => {
        if (interrupted && !cleaning) reject(new Error(`acceptance interrupted: ${interrupted}`));
      };
      check();
      timer = setTimeout(() => reject(new Error(reason)), ms);
      interruptTimer = setInterval(check, 50);
    })]);
  } finally { clearTimeout(timer); clearInterval(interruptTimer); }
}
const powershell = (script, env = process.env) => execFileSync("powershell.exe",
  ["-NoProfile", "-NonInteractive", "-Command", script],
  { env, encoding: "utf8", windowsHide: true, maxBuffer: 12e6, timeout: 30000 });
const inventory = () => JSON.parse(powershell(
  "ConvertTo-Json -Compress -InputObject @(Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name,ExecutablePath,@{Name='CreationDate';Expression={$_.CreationDate.ToUniversalTime().ToString('o')}},CommandLine)"));
const identity = process => ({
  ProcessId: process.ProcessId, ParentProcessId: process.ParentProcessId,
  Name: process.Name, ExecutablePath: process.ExecutablePath, CreationDate: process.CreationDate,
});
const sameProcess = (a, b) => a && b && a.ProcessId === b.ProcessId
  && a.CreationDate === b.CreationDate && a.ExecutablePath === b.ExecutablePath;
const before = inventory();
const protectedProcesses = before.filter(process => /^(codex-plus-plus|codex|ChatGPT|cc-switch)\.exe$/i.test(process.Name));
const originalRenderers = protectedProcesses.filter(process =>
  /^ChatGPT\.exe$/i.test(process.Name) && /--type=renderer(?:\s|$)/.test(process.CommandLine || ""));
const children = [], contexts = [], desktops = [], owned = new Map(), servers = new Set(), ports = new Set();
const report = { temporary, executable, appDir, checks: {}, events: [], managers: [], desktops: [],
  protectedProcesses: protectedProcesses.map(identity), cleanup: false,
  protectionPolicy: "Preserve original manager/Codex/CLI identities and files; record natural renderer replacement separately." };
const protectedFiles = new Map();
const pageDialogs = new WeakMap();
let pageDialogsSequence = 0;
let mode = "complete", attempts = 0, held = [];

async function save() {
  await bounded(fs.mkdir(outputDir, { recursive: true }));
  await bounded(fs.writeFile(path.join(outputDir, "desktop-verification.json"), JSON.stringify(report, null, 2)));
}
async function pass(name, detail = {}) {
  report.checks[name] = true;
  report.events.push({ name, time: new Date().toISOString(), ...detail });
  console.log(`PASS ${name}`);
  await save();
}
async function until(check, message, timeout = 45000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (interrupted && !cleaning) throw new Error(`acceptance interrupted: ${interrupted}`);
    if (await bounded(Promise.resolve().then(check), Math.max(1, deadline - Date.now()), message)) return;
    await bounded(pause(Math.min(150, Math.max(1, deadline - Date.now()))));
  }
  throw new Error(message);
}
async function listen(server, port = 0) {
  servers.add(server);
  await bounded(new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, "127.0.0.1", resolve);
  }));
  ports.add(server.address().port);
  return server.address().port;
}
async function close(server) {
  if (!server?.listening) return;
  server.closeAllConnections?.();
  await bounded(new Promise(resolve => server.close(resolve)), 3000);
  servers.delete(server);
}
async function freePort() {
  const server = net.createServer();
  try {
    const port = await listen(server);
    return port;
  } finally { await close(server); }
}
async function freeDesktopDebugPort() {
  for (let attempt = 0; attempt < 50; attempt++) {
    const debug = net.createServer(), helper = net.createServer();
    try {
      const port = await listen(debug);
      if (port >= 65535) continue;
      await listen(helper, port + 1);
      return port;
    } catch (error) {
      if (!["EADDRINUSE", "EACCES"].includes(error.code)) throw error;
    } finally {
      await close(helper);
      await close(debug);
    }
  }
  throw new Error("No available isolated desktop debug/helper port pair");
}
function captureOwned() {
  const processes = inventory();
  for (const child of children) if (child.exitCode === null && !owned.has(child.pid)) {
    const candidate = processes.find(process => process.ProcessId === child.pid);
    if (candidate?.ParentProcessId === process.pid && candidate.ExecutablePath === child.testExecutable) {
      owned.set(candidate.ProcessId, identity(candidate));
    }
  }
  const ids = new Set(processes.filter(process => sameProcess(process, owned.get(process.ProcessId)))
    .map(process => process.ProcessId));
  for (const process of processes) {
    if (/^(ChatGPT|codex|codex-plus-plus|msedgewebview2|cmd)\.exe$/i.test(process.Name)
      && process.CommandLine?.includes(temporary) && !before.some(prior => sameProcess(prior, process))) {
      ids.add(process.ProcessId);
    }
  }
  for (let changed = true; changed;) {
    changed = false;
    for (const process of processes) {
      if (ids.has(process.ParentProcessId) && !ids.has(process.ProcessId)
        && !before.some(prior => sameProcess(prior, process))) {
        ids.add(process.ProcessId);
        changed = true;
      }
    }
  }
  for (const process of processes) if (ids.has(process.ProcessId)) {
    assert.ok(!protectedProcesses.some(prior => sameProcess(prior, process)), "test must never own a protected process");
    owned.set(process.ProcessId, identity(process));
  }
  report.owned = [...owned.values()];
}
function killOwned(pid) {
  const actual = inventory().find(process => process.ProcessId === pid);
  if (!actual) return;
  assert.ok(sameProcess(actual, owned.get(pid)), `refused to terminate unverified PID ${pid}`);
  assert.ok(!protectedProcesses.some(process => sameProcess(process, actual)));
  powershell(
    "$ErrorActionPreference = 'Stop'; $target = [Diagnostics.Process]::GetProcessById([int]$env:DESKTOP_TEST_PID); " +
    "try { $null = $target.SafeHandle; $birth = [DateTimeOffset]::Parse($env:DESKTOP_TEST_BIRTH).UtcDateTime; " +
    "if ($target.Path -eq $env:DESKTOP_TEST_EXE -and " +
    "[Math]::Abs(($target.StartTime.ToUniversalTime() - $birth).Ticks) -lt 10) { " +
    "$target.Kill(); $null = $target.WaitForExit(2000) } else { throw 'Process identity changed' } } " +
    "finally { $target.Dispose() }",
    { ...process.env, DESKTOP_TEST_PID: String(pid), DESKTOP_TEST_EXE: actual.ExecutablePath,
      DESKTOP_TEST_BIRTH: actual.CreationDate });
}
async function protectOriginalFiles() {
  for (const file of [
    path.join(process.env.USERPROFILE, ".codex", "config.toml"),
    path.join(process.env.USERPROFILE, ".codex-session-delete", "settings.json"),
    ...(process.env.CODEX_PLUS_STATE_DIR ? [path.join(process.env.CODEX_PLUS_STATE_DIR, "settings.json")] : []),
  ]) protectedFiles.set(file, await fs.stat(file).then(stat => ({
    size: stat.size, mtimeMs: stat.mtimeMs, ino: stat.ino,
  })).catch(error => {
    if (error.code !== "ENOENT") throw error;
    return null;
  }));
}
async function originalsIntact() {
  const current = inventory();
  const missing = protectedProcesses.filter(original => !current.some(process => sameProcess(original, process)));
  report.originalRendererChanges = missing.filter(original =>
    originalRenderers.some(renderer => sameProcess(renderer, original))).map(identity);
  const fixedChanges = missing.filter(original =>
    !originalRenderers.some(renderer => sameProcess(renderer, original)));
  if (fixedChanges.length) report.originalProcessChanges = fixedChanges.map(identity);
  assert.equal(fixedChanges.length, 0, `original application process identities changed: ${fixedChanges.map(p => `${p.Name}:${p.ProcessId}`).join(", ")}`);
  for (const [file, original] of protectedFiles) {
    const current = await fs.stat(file).then(stat => ({
      size: stat.size, mtimeMs: stat.mtimeMs, ino: stat.ino,
    })).catch(error => { if (error.code === "ENOENT") return null; throw error; });
    const unchanged = JSON.stringify(original) === JSON.stringify(current);
    if (!unchanged) report.originalFileChanges = [...new Set([...(report.originalFileChanges || []), file])];
    assert.ok(unchanged, `original configuration changed: ${file}`);
  }
}
async function supplier(owner = "original") {
  const server = http.createServer(async (request, response) => {
    try {
      if (request.method === "GET") {
        response.setHeader("content-type", "application/json");
        response.end(JSON.stringify({ object: "list", data: [{ id: "gpt-5.1-codex", object: "model", owned_by: "fixture" }] }));
        return;
      }
      const chunks = [];
      for await (const chunk of request) chunks.push(chunk);
      const body = JSON.parse(Buffer.concat(chunks).toString() || "{}");
      report.events.push({ name: "supplierRequest", time: new Date().toISOString(),
        owner, url: request.url, model: body.model, stream: body.stream });
      assert.ok(request.url.endsWith("/responses"), `Unexpected supplier path ${request.url}`);
      const answer = owner === "original" ? `DESKTOP_RECOVERED_${mode}` : `EXTERNAL_SUPPLIER_${mode}`;
      const responseId = `resp_${++attempts}`;
      const item = { id: `msg_${attempts}`, type: "message", role: "assistant", status: "completed",
        content: [{ type: "output_text", text: answer, annotations: [] }] };
      if (!body.stream) {
        response.setHeader("content-type", "application/json");
        response.end(JSON.stringify({ id: responseId, object: "response", status: "completed", output: [item] }));
        return;
      }
      response.setHeader("content-type", "text/event-stream");
      response.setHeader("cache-control", "no-cache");
      const send = (type, value) => response.write(`event: ${type}\ndata: ${JSON.stringify({ type, ...value })}\n\n`);
      send("response.created", { response: { id: responseId, object: "response", status: "in_progress", output: [] } });
      send("response.output_item.added", { output_index: 0, item: { ...item, status: "in_progress", content: [] } });
      send("response.content_part.added", { output_index: 0, item_id: item.id, content_index: 0,
        part: { type: "output_text", text: "", annotations: [] } });
      if (mode === "hold" && owner === "original") {
        send("response.output_text.delta", { item_id: item.id, output_index: 0, content_index: 0, delta: "PARTIAL" });
        held.push(response);
        return;
      }
      send("response.output_text.delta", { item_id: item.id, output_index: 0, content_index: 0, delta: answer });
      send("response.output_text.done", { item_id: item.id, output_index: 0, content_index: 0, text: answer });
      send("response.output_item.done", { output_index: 0, item });
      send("response.completed", { response: { id: responseId, object: "response", status: "completed",
        output: [item], usage: { input_tokens: 10, output_tokens: 5, total_tokens: 15 } } });
      response.end();
    } catch (error) {
      report.supplierError = String(error);
      response.destroy(error);
    }
  });
  return `http://127.0.0.1:${await listen(server)}`;
}
async function context(name, upstream) {
  const state = path.join(temporary, name), home = path.join(state, "home");
  await fs.mkdir(home, { recursive: true });
  await fs.mkdir(path.join(state, "appdata", "Codex++"), { recursive: true });
  const profile = {
    id: "fixture", name: "Desktop Fixture", relayMode: "pureApi", protocol: "responses",
    webSearchHistoryCompat: true, apiKey: "synthetic-desktop-test", model: "gpt-5.1-codex",
    modelList: "gpt-5.1-codex[128K]\ngpt-fixture-secondary[64K]", baseUrl: `${upstream}/v1`, upstreamBaseUrl: `${upstream}/v1`,
    configContents: `model = "gpt-5.1-codex"\nmodel_provider = "custom"\n[model_providers.custom]\nname = "Fixture"\nrequires_openai_auth = false\nwire_api = "responses"\nbase_url = "${upstream}/v1"\n`,
    authContents: '{"OPENAI_API_KEY":"synthetic-desktop-test"}',
  };
  await fs.writeFile(path.join(state, "settings.json"), JSON.stringify({
    localConfigProviderImported: true, providerSyncEnabled: false, codexAppAutoTakeover: false,
    enhancementsEnabled: true, codexAppStandaloneQuotaUnlock: false, codexAppPath: appDir,
    relayProfilesEnabled: true, activeRelayId: profile.id, relayProfiles: [profile],
    relayCommonConfigContents: "analytics.enabled=false\nsandbox_mode=\"danger-full-access\"\napproval_policy=\"never\"\n[windows]\nsandbox=\"unelevated\"\n[features]\nplugins=false\napps=false\nremote_plugin=false\n",
  }));
  const result = { state, home, sqlite: path.join(state, "sqlite"), debug: await freePort(),
    guard: await freePort(), launcherGuard: await freePort(), port: await freePort() };
  contexts.push(result);
  return result;
}
function environment(context) {
  const env = { ...process.env, CODEX_HOME: context.home, CODEX_SQLITE_HOME: context.sqlite,
    APPDATA: path.join(context.state, "appdata"),
    CODEX_PLUS_STATE_DIR: context.state, CODEX_PLUS_MANAGER_GUARD_PORT: String(context.guard),
    CODEX_PLUS_LAUNCHER_GUARD_PORT: String(context.launcherGuard), CODEX_PLUS_PROTOCOL_PROXY_PORT: String(context.port),
    WEBVIEW2_USER_DATA_FOLDER: path.join(context.state, "test-webview"),
    CODEX_ELECTRON_USER_DATA_PATH: path.join(context.state, "unused-electron"),
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${context.debug}`,
    CODEX_PLUS_HELPER_BIND: "127.0.0.1", OTEL_SDK_DISABLED: "true" };
  for (const name of ["CODEX_PLUS_GUARD_PORT", "CODEX_PLUS_MULTI_INSTANCE", "CODEX_PLUS_INSTANCE_ID",
    "CODEX_PLUS_INSTANCE_LABEL", "OPENAI_API_KEY", "OPENAI_BASE_URL", "CODEX_API_KEY",
    "ANTHROPIC_API_KEY", "ANTHROPIC_BASE_URL", "AZURE_OPENAI_API_KEY", "AZURE_OPENAI_ENDPOINT",
    "OPENAI_ORG_ID", "OPENAI_PROJECT_ID"]) delete env[name];
  return env;
}
async function startManager(context) {
  await fs.mkdir(path.join(context.state, "appdata", "Codex++"), { recursive: true });
  const started = Date.now();
  const binary = context.executable || executable;
  context.current = spawn(binary, [], { env: environment(context), windowsHide: true,
    stdio: ["ignore", "pipe", "pipe"] });
  context.current.testExecutable = binary;
  children.push(context.current);
  context.spawnError = null;
  context.current.once("error", error => { context.spawnError = error; });
  const launch = { state: context.state, executable: binary, pid: context.current.pid,
    started, helperPort: context.port, debugPort: context.debug, guardPort: context.guard, ready: false };
  for (const stream of ["stdout", "stderr"]) context.current[stream]?.on("data", data => {
    launch[stream] = ((launch[stream] || "") + data.toString()).slice(-12000);
  });
  report.managers.push(launch);
  await save();
  await until(async () => {
    if (context.spawnError) throw context.spawnError;
    assert.equal(context.current.exitCode, null, "manager exited before WebView became ready");
    captureOwned();
    try { return (await fetch(`http://127.0.0.1:${context.debug}/json`,
      { signal: AbortSignal.timeout(1500) })).ok; } catch { return false; }
  }, "manager WebView did not start");
  context.browser = await connectBrowser(context.debug);
  context.page = context.browser.contexts()[0].pages()[0];
  await bounded(context.page.waitForFunction(() => window.__TAURI_INTERNALS__?.invoke));
  captureOwned();
  launch.ready = true;
  await save();
}
async function connectBrowser(port) {
  const browser = await bounded(chromium.connectOverCDP(`http://127.0.0.1:${port}`, { timeout: 10000 }));
  for (const context of browser.contexts()) {
    context.setDefaultTimeout(10000);
    context.setDefaultNavigationTimeout(10000);
    context.on("page", page => { dialogsFor(page).catch(error => {
      report.dialogError = String(error);
    }); });
    for (const page of context.pages()) await dialogsFor(page);
  }
  return browser;
}
function dialogsFor(page) {
  if (!pageDialogs.has(page)) pageDialogs.set(page, installDialogHandling(page, {
    evidenceDir: path.join(outputDir, `dialogs-${pageDialogsSequence++}`),
    record: event => report.events.push({ name: "dialog", ...event }),
  }));
  return pageDialogs.get(page);
}
async function ensureClear(page) {
  assert.ok(!report.dialogError, report.dialogError);
  await bounded((await dialogsFor(page)).ensureClear(), 15000);
}
const invoke = (context, command, args = {}) => bounded(context.page.evaluate(
  ({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args), { command, args }), 30000);
async function exitWithFallback(requestExit, hasExited, forceExit, timeout = 5000) {
  await bounded(Promise.resolve().then(requestExit), timeout).catch(() => {});
  let forced = false;
  if (!hasExited()) {
    try { await until(hasExited, "manager did not exit normally", timeout); }
    catch (error) {
      if (interrupted && !cleaning) throw error;
      await forceExit();
      forced = true;
    }
  }
  await until(hasExited, "manager did not exit after forced cleanup", timeout);
  return forced;
}
async function stopManager(context, crash = false) {
  if (!context.current || context.current.exitCode !== null) return;
  captureOwned();
  if (crash) killOwned(context.current.pid);
  else {
    const forced = await exitWithFallback(
      () => context.hangExit ? new Promise(() => {}) : invoke(context, "manager_exit_app"),
      () => context.current.exitCode !== null,
      () => killOwned(context.current.pid));
    if (forced) {
      report.events.push({ name: "managerExitFallback", pid: context.current.pid });
      assert.ok(cleaning, "normal manager exit required forced termination");
    }
  }
  await until(() => context.current.exitCode !== null, "manager did not exit", 15000);
  if (context.browser) await bounded(context.browser.close(), 3000).catch(() => {});
  context.browser = context.page = null;
}
async function launchDesktop(context, instanceId = token) {
  const root = path.join(context.state, "codex-multi", instanceId);
  const home = path.join(root, "codex-home"), workspace = path.join(context.state, "workspace");
  await fs.mkdir(home, { recursive: true });
  await fs.mkdir(workspace, { recursive: true });
  await fs.writeFile(path.join(home, ".codex-global-state.json"), JSON.stringify({
    "electron-saved-workspace-roots": [workspace],
    "active-workspace-roots": [workspace],
    "electron-persisted-atom-state": { "electron:onboarding-projectless-completed": true,
      last_completed_onboarding: Math.floor(Date.now() / 1000) },
  }));
  const result = await invoke(context, "launch_codex_multi_instance", { request: {
    instanceId, profileId: "fixture", appPath: appDir, debugPort: await freeDesktopDebugPort(),
  } });
  assert.notEqual(result.status, "failed", result.message);
  const desktop = { context, root, instanceId };
  desktops.push(desktop);
  await until(async () => {
    captureOwned();
    const result = await fs.readFile(path.join(root, "launch-result.json"), "utf8")
      .then(JSON.parse).catch(() => null);
    if (!result) return false;
    desktop.result = result;
    assert.notEqual(result.status, "failed", result.message);
    return result.identity;
  }, "real desktop launch did not become ready", 135000);
  desktop.id = desktop.result.identity;
  ports.add(desktop.id.debugPort);
  ports.add(desktop.id.helperPort);
  const listed = await invoke(context, "list_codex_multi_instances");
  desktop.manifest = listed.instances.find(instance => instance.instanceId === instanceId);
  assert.ok(desktop.manifest?.running, JSON.stringify(listed));
  assertWithin(temporary, desktop.manifest.codexHome);
  assertWithin(temporary, desktop.manifest.sqliteHome);
  const data = path.resolve(desktop.id.userDataDir.replace(/^\\\\\?\\/, ""));
  desktop.actualData = data;
  const browserProcess = inventory().find(process => process.ProcessId === desktop.id.processId);
  assert.equal(desktop.id.instanceId, instanceId);
  assert.ok(browserProcess?.CommandLine.includes(data) && browserProcess.CommandLine.includes(
    `--remote-debugging-port=${desktop.id.debugPort}`), "launched browser must match its receipt");
  assert.ok(!protectedProcesses.some(original => sameProcess(original, browserProcess)));
  owned.set(browserProcess.ProcessId, identity(browserProcess));
  captureOwned();
  report.desktops.push({ ...desktop.id, codexHome: desktop.manifest.codexHome,
    sqliteHome: desktop.manifest.sqliteHome, workerPid: desktop.manifest.pid });
  await save();
  assert.ok(data.toLowerCase().startsWith(path.resolve(temporary).toLowerCase() + path.sep),
    "new Electron data must stay in the isolated state tree");
  desktop.browser = await connectBrowser(desktop.id.debugPort);
  const errors = [];
  for (const context of desktop.browser.contexts()) {
    context.on("page", page => page.on("pageerror", error => errors.push(String(error))));
    for (const page of context.pages()) page.on("pageerror", error => errors.push(String(error)));
  }
  await until(async () => {
    const candidates = [];
    for (const page of desktop.browser.contexts().flatMap(context => context.pages())) {
      const ui = await bounded(page.evaluate(() => ({
        title: document.title, text: document.body?.innerText?.slice(0, 1500) || "",
        elements: document.body?.querySelectorAll("*").length,
        composer: !!document.querySelector('[contenteditable="true"]'),
      })), 3000).catch(() => null);
      candidates.push({ url: page.url(), ...ui });
      if (ui?.composer && page.url().startsWith("app://")) desktop.page = page;
    }
    desktop.candidates = candidates;
    return desktop.page;
  }, "real desktop has no rendered application page", 60000).catch(error => {
    report.events.push({ name: "blankDesktop", candidates: desktop.candidates, errors });
    throw error;
  });
  await bounded(desktop.page.waitForLoadState("domcontentloaded"));
  await ensureClear(desktop.page);
  await bounded(desktop.page.screenshot({ path: path.join(outputDir, `desktop-${desktops.length}.png`) }));
  await ensureClear(desktop.page);
  await pass(`realDesktop${desktops.length}Ready`, { processId: desktop.id.processId, debugPort: desktop.id.debugPort });
  return desktop;
}
async function snapshot(desktop, label) {
  await ensureClear(desktop.page);
  const ui = await bounded(desktop.page.evaluate(() => ({
    url: location.href, text: document.body.innerText.slice(-12000),
    fields: [...document.querySelectorAll("input,textarea,[contenteditable=true]")].map(el => ({
      tag: el.tagName, role: el.getAttribute("role"), placeholder: el.getAttribute("placeholder"),
      aria: el.getAttribute("aria-label"), contenteditable: el.getAttribute("contenteditable"),
    })),
    buttons: [...document.querySelectorAll("button")].map(el => ({
      text: el.innerText, aria: el.getAttribute("aria-label"), title: el.getAttribute("title"),
      disabled: el.disabled,
    })).filter(button => button.text || button.aria || button.title),
  })), 3000);
  report.events.push({ name: label, ui });
  await bounded(desktop.page.screenshot({ path: path.join(outputDir, `${label}.png`) }), 3000);
  await ensureClear(desktop.page);
  await save();
}
async function sendPrompt(desktop, text) {
  await until(async () => !await desktop.page.getByText("正在完成 Windows 设置", { exact: true }).isVisible(),
    "isolated desktop sandbox setup did not finish", 90000);
  await until(async () => {
    const trigger = desktop.page.locator("[data-codex-intelligence-trigger]:visible").first();
    return await trigger.count() > 0 && await trigger.isEnabled()
      && !/loading model|加载模型/i.test(await trigger.innerText());
  }, "native model selector did not become ready before sending", 90000);
  await ensureClear(desktop.page);
  await snapshot(desktop, "before-prompt");
  const composer = desktop.page.locator('[contenteditable="true"]').first();
  await bounded(composer.waitFor({ state: "visible", timeout: 15000 }), 16000);
  await bounded(composer.fill(text));
  await bounded(composer.press("Enter"));
  await ensureClear(desktop.page);
}
async function assertDesktopAlive(desktop) {
  const browser = await bounded(fetch(`http://127.0.0.1:${desktop.id.debugPort}/json/version`,
    { signal: AbortSignal.timeout(2000) }).then(response => response.json()), 3000);
  assert.ok(browser.webSocketDebuggerUrl.endsWith(`/devtools/browser/${desktop.id.browserId}`));
  const processes = inventory();
  assert.ok(processes.some(process => sameProcess(process, owned.get(desktop.id.processId))));
}
async function closeDesktop(desktop) {
  if (!desktop.id) return;
  try {
    await assertDesktopAlive(desktop);
    const session = await bounded(desktop.browser.newBrowserCDPSession(), 3000);
    await bounded(session.send("Browser.close"), 5000).catch(() => {});
  } catch {}
  if (desktop.browser) await bounded(desktop.browser.close(), 3000).catch(() => {});
}

async function verifyLiveWorkerBlocks(context, desktop) {
  const manifestPath = path.join(desktop.root, "instance.json");
  const receiptPath = path.join(desktop.root, "launch-result.json");
  const manifest = await fs.readFile(manifestPath, "utf8");
  const receipt = await fs.readFile(receiptPath, "utf8");
  try {
    for (const legacy of [false, true]) {
      const starting = editableJson(manifest);
      starting.identity = null;
      if (legacy) delete starting.workerBirthId;
      await fs.writeFile(manifestPath, JSON.stringify(starting));
      await fs.writeFile(receiptPath, JSON.stringify({ ...editableJson(receipt), identity: null }));
      const blocked = await invoke(context, "launch_codex_multi_instance", { request: {
        instanceId: token, profileId: "fixture", appPath: appDir,
      } });
      assert.equal(blocked.status, "failed", "live worker must block a second generation");
      assert.match(blocked.message, /worker 尚未退出/);
      await assertDesktopAlive(desktop);
    }
  } finally {
    await fs.writeFile(receiptPath, receipt);
    await fs.writeFile(manifestPath, manifest);
  }
}

function assertWithin(root, target) {
  const relative = path.relative(path.resolve(root), path.resolve(target));
  assert.ok(relative && !relative.startsWith(`..${path.sep}`) && relative !== ".."
    && !path.isAbsolute(relative), "fixture path must be inside this test runtime");
}
// Only reads generated fixture root strings; reject unsupported TOML rather than guessing.
function fixtureRootString(config, key) {
  const root = config.split(/^\s*\[/m)[0];
  const match = root.match(new RegExp(`^\\s*${key}\\s*=\\s*("(?:[^"\\\\]|\\\\.)*"|'[^']*')\\s*$`, "m"));
  assert.ok(match, `fixture requires a quoted root ${key}`);
  return match[1].startsWith("'") ? match[1].slice(1, -1) : JSON.parse(match[1]);
}
async function providerSnapshot(state, home) {
  const settings = JSON.parse(await fs.readFile(path.join(state, "settings.json"), "utf8"));
  const config = await fs.readFile(path.join(home, "config.toml"), "utf8");
  const pointer = fixtureRootString(config, "model_catalog_json");
  const catalogPath = path.resolve(home, pointer);
  assertWithin(home, catalogPath);
  return {
    activeRelayId: settings.activeRelayId,
    profiles: settings.relayProfiles,
    defaultModel: fixtureRootString(config, "model"),
    catalog: JSON.parse(await fs.readFile(catalogPath, "utf8")),
  };
}
async function writeExternalProvider(root, home, upstream, phase, catalogTemplate) {
  assertWithin(root, home);
  const url = new URL(upstream);
  assert.equal(url.hostname, "127.0.0.1");
  assert.equal(url.protocol, "http:");
  const model = `gpt-fixture-external-${phase}`;
  const catalog = structuredClone(catalogTemplate);
  assert.ok(Array.isArray(catalog.models) && catalog.models.length > 0, "fixture needs a generated catalog");
  catalog.models = [{ ...catalog.models[0], slug: model, display_name: model }];
  const catalogPath = path.join(home, "external-ccswitch-catalog.json");
  await fs.writeFile(catalogPath, JSON.stringify(catalog));
  await fs.writeFile(path.join(home, "config.toml"),
    `model = "${model}"\nmodel_provider = "external_fixture"\nmodel_catalog_json = ${JSON.stringify(catalogPath)}\n` +
    'sandbox_mode = "danger-full-access"\napproval_policy = "never"\n' +
    '[model_providers.external_fixture]\nname = "Synthetic CC Switch"\nwire_api = "responses"\n' +
    `requires_openai_auth = false\nbase_url = "${url.origin}/v1"\n` +
    '[windows]\nsandbox = "unelevated"\n[features]\nplugins = false\napps = false\n');
  await fs.writeFile(path.join(home, "auth.json"), JSON.stringify({
    OPENAI_API_KEY: `synthetic-external-${phase}`,
  }));
}
function assertOriginalOwner(events, start) {
  const requests = events.slice(start).filter(event => event.name === "supplierRequest");
  assert.ok(requests.length > 0, "native recovery must make a real local synthetic request");
  assert.ok(requests.every(event => event.owner === "original"), "old endpoint changed supplier owner");
}

async function verifyRecoveredProvider(desktop, state, home, baseline, label, externalRewrite = true) {
  const settings = JSON.parse(await fs.readFile(path.join(state, "settings.json"), "utf8"));
  assert.equal(settings.activeRelayId, baseline.activeRelayId);
  assert.deepEqual(settings.relayProfiles, baseline.profiles, "recovery must not backfill external supplier data");
  await until(async () => {
    const catalog = await bounded(desktop.page.evaluate(() =>
      window.__codexSessionDeleteBridge("/codex-model-catalog", {})));
    return catalog.status === "ok" && catalog.model === baseline.defaultModel
      && catalog.default_model === baseline.defaultModel
      && JSON.stringify([...catalog.models].sort()) === JSON.stringify(baseline.catalog.models.map(model => model.slug).sort());
  }, "runtime catalog/default changed after external rewrite");
  // 多开默认关闭配置接管；恢复后台不能因此覆盖 CC Switch 的实时文件。
  if (externalRewrite && !settings.relayProfilesEnabled) {
    const disk = await providerSnapshot(state, home);
    assert.equal(disk.defaultModel, "gpt-fixture-external-after-crash");
  } else {
    assert.deepEqual(await providerSnapshot(state, home), baseline);
  }
  await ensureClear(desktop.page);
  const selector = '[data-model-picker-view]:not([data-model-picker-view="grid"]) [class*="_ModelList_"]';
  const paintedIndexes = nodes => {
    const painted = node => {
      for (let current = node; current; current = current.parentElement) {
        const style = getComputedStyle(current);
        if (style.display === "none" || style.visibility === "hidden" || Number(style.opacity) === 0) return false;
      }
      const rect = node.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return false;
      const hit = document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2);
      return hit && node.contains(hit);
    };
    return nodes.flatMap((node, index) => painted(node) ? [index] : []);
  };
  const listIndex = async () => (await desktop.page.locator(selector).evaluateAll(paintedIndexes))[0] ?? -1;
  await desktop.page.locator("[data-codex-intelligence-trigger]:visible").first().click();
  if (await listIndex() < 0) {
    const title = desktop.page.getByText(/^(?:5\.1 Codex|gpt-5\.1-codex)(?:\s*[›>])?$/);
    let indexes = [];
    await until(async () => (indexes = await title.evaluateAll(paintedIndexes)).length > 0,
      "recovered model title did not become painted");
    assert.equal(indexes.length, 1, "one painted recovery model title");
    await title.nth(indexes[0]).click();
  }
  let index = -1;
  await until(async () => (index = await listIndex()) >= 0, "recovered model list did not open");
  const list = desktop.page.locator(selector).nth(index);
  const rows = list.locator('[role="menuitemradio"]:visible:not(.codex-plus-model-default-row)');
  await until(async () => await rows.count() === 2, "recovered model list changed");
  const texts = await rows.allInnerTexts();
  assert.ok(texts.some(text => /(?:5\.1 Codex|gpt-5\.1-codex)/.test(text)));
  assert.ok(texts.some(text => /fixture[- ]secondary/i.test(text)));
  assert.ok(texts.every(text => !text.includes("external")));
  await ensureClear(desktop.page);
  await bounded(desktop.page.screenshot({ path: path.join(outputDir, `${label}-models.png`) }));
  await desktop.page.keyboard.press("Escape");
  await until(async () => await listIndex() < 0, "recovered menu did not close");
  await ensureClear(desktop.page);
  await pass(`${label}SavedProviderRuntimeCatalogAndVisibleModels`, { models: texts, configuredDefault: baseline.defaultModel });
}

async function runProviderScenario(desktop) {
  if (!process.env.DESKTOP_PROVIDER_SCENARIO) return;
  const scenarioPath = path.resolve(process.env.DESKTOP_PROVIDER_SCENARIO);
  const scenario = require(scenarioPath);
  assert.equal(typeof scenario.run, "function", "provider scenario must export run(context)");
  const companion = {
    state: path.join(desktop.root, "worker-home", ".codex-session-delete"),
    home: desktop.manifest.codexHome, sqlite: desktop.manifest.sqliteHome,
    debug: await freePort(), guard: await freePort(), launcherGuard: await freePort(), port: await freePort(),
  };
  contexts.push(companion);
  const evidenceDir = path.join(outputDir, "provider-scenario");
  await fs.mkdir(evidenceDir, { recursive: true });
  try {
    // 先停止本次 worker，保留它的窗口；恢复管理器成为唯一桥接/代理宿主。
    captureOwned();
    killOwned(desktop.manifest.pid);
    await until(() => !inventory().some(process => process.ProcessId === desktop.manifest.pid),
      "provider worker did not stop before manager takeover");
    await startManager(companion);
    await assertDesktopAlive(desktop);
    await ensureClear(desktop.page);
    await bounded(scenario.run({
      managerPage: companion.page, codexPage: desktop.page,
      invoke: (command, args) => {
        scenarioController.signal.throwIfAborted();
        return invoke(companion, command, args);
      },
      paths: { runtime: temporary, state: companion.state, home: companion.home,
        sqlite: companion.sqlite, appdata: path.join(companion.state, "appdata") },
      dialogs: { codex: await dialogsFor(desktop.page), manager: await dialogsFor(companion.page) },
      outputDir: evidenceDir, check: pass, signal: scenarioController.signal,
    }), 900000, "provider scenario timed out");
    await ensureClear(desktop.page);
    const restored = await providerSnapshot(companion.state, companion.home);
    assert.equal(restored.activeRelayId, "fixture", "scenario must restore original active fixture via real switch");
    assert.equal(restored.defaultModel, "gpt-5.1-codex");
    await pass("providerScenarioCompletedBeforeContinuityTests", { scenario: scenarioPath });
  } catch (error) {
    scenarioController.abort(error);
    throw error;
  } finally {
    await stopManager(companion);
    const stopped = await invoke(desktop.context, "stop_codex_multi_instance", {
      request: { instanceId: desktop.instanceId },
    });
    if (stopped.status === "failed") report.events.push({
      name: "providerDesktopStopFallbackRequired", message: stopped.message,
    });
    await closeDesktop(desktop);
  }
}

async function waitForWorkerHandoff(desktop, previousPid, binary) {
  await until(async () => {
    captureOwned();
    const current = JSON.parse(await fs.readFile(path.join(desktop.root, "instance.json"), "utf8"));
    const ready = await fs.readFile(path.join(desktop.root, "worker-ready.json"), "utf8")
      .then(JSON.parse).catch(() => null);
    if (!ready || ready.pid === previousPid || ready.pid !== current.pid) return false;
    const worker = inventory().find(candidate => candidate.ProcessId === ready.pid);
    if (!worker) return false;
    assert.equal(worker.ExecutablePath, binary, "replacement must execute the new manager binary");
    assert.deepEqual(current.identity, desktop.id, "handoff must preserve the exact Codex process and Browser identity");
    desktop.manifest = current;
    return true;
  }, "manager did not hand off the existing worker", 135000);
  await assertDesktopAlive(desktop);
}

async function verifyStatusIsolation(active, idle, label) {
  const status = desktop => bounded(desktop.page.evaluate(() =>
    window.__codexSessionDeleteBridge("/proxy-status", {})));
  await until(async () => {
    const [own, other] = await Promise.all([status(active), status(idle)]);
    assert.equal(own.status, "ok");
    assert.equal(other.status, "ok");
    assert.equal(other.requests.length, 0, "idle worker must not expose another worker's request");
    return own.requests.length > 0;
  }, "active worker did not expose its own request");
  await active.page.bringToFront();
  await ensureClear(active.page);
  await until(async () => {
    await active.page.evaluate(() => window.__codexPlusProxyRequestStatusRuntime.refresh());
    return active.page.locator('#codex-plus-proxy-request-status[data-hidden="false"]').isVisible();
  }, "request status bar was not painted");
  await snapshot(active, `${label}-active-status`);
  await idle.page.bringToFront();
  await ensureClear(idle.page);
  await idle.page.evaluate(() => window.__codexPlusProxyRequestStatusRuntime.refresh());
  assert.equal(await idle.page.locator('#codex-plus-proxy-request-status[data-hidden="false"]').isVisible(), false);
  await snapshot(idle, `${label}-idle-status`);
  await pass(`${label}WorkerStatusDoesNotCrossInstances`);
}

async function runWorkerHandoffScenario(first, manager) {
  const second = await launchDesktop(manager, `${token}-sibling`);
  const baseline = await Promise.all([first, second].map(desktop =>
    providerSnapshot(path.join(desktop.root, "worker-home", ".codex-session-delete"), desktop.manifest.codexHome)));
  await pass("twoMultiInstancesReadyBeforeUpgrade");
  for (const [index, label] of ["legacyUpgrade", "cooperativeRestart", "missingWorkerRecovery"].entries()) {
    const oldPids = [first.manifest.pid, second.manifest.pid];
    if (index === 2) {
      captureOwned();
      killOwned(oldPids[0]);
      await until(() => !inventory().some(candidate => candidate.ProcessId === oldPids[0]), "crashed worker remained alive");
    } else {
      mode = "hold";
      const heldBefore = held.length;
      await sendPrompt(first, "Return a short synthetic response without tools.");
      await until(() => held.length > heldBefore, "native request did not reach held local supplier");
      await verifyStatusIsolation(first, second, label);
    }
    await stopManager(manager);
    await assertDesktopAlive(first);
    await assertDesktopAlive(second);
    manager.executable = executable;
    const requestStart = report.events.length;
    mode = label;
    await startManager(manager);
    for (const [i, desktop] of [first, second].entries()) {
      await waitForWorkerHandoff(desktop, oldPids[i], executable);
      const settings = JSON.parse(await fs.readFile(
        path.join(desktop.root, "worker-home", ".codex-session-delete", "settings.json"), "utf8"));
      assert.equal(settings.activeRelayId, baseline[i].activeRelayId);
      assert.deepEqual(settings.relayProfiles, baseline[i].profiles);
      const options = JSON.parse(await fs.readFile(path.join(desktop.root, "worker-options.json"), "utf8"));
      assert.deepEqual(options.resumeIdentity, desktop.id, "replacement must use attach-only identity");
    }
    if (index === 2) await sendPrompt(first, "Return a short synthetic response after recovery without tools.");
    await first.page.getByText(`DESKTOP_RECOVERED_${label}`, { exact: true }).waitFor({ timeout: 90000 });
    assertOriginalOwner(report.events, requestStart);
    await pass(`${label}UsesNewWorkerWithoutRestartingCodex`);
    if (index === 1) {
      const requested = JSON.parse(await fs.readFile(path.join(first.root, "worker-handoff.json"), "utf8"));
      assert.equal(requested.pid, oldPids[0], "new workers must honor cooperative shutdown");
      await pass("cooperativeWorkerShutdownRequested");
    }
    await originalsIntact();
  }
  for (const [i, desktop] of [first, second].entries()) {
    await verifyRecoveredProvider(desktop,
      path.join(desktop.root, "worker-home", ".codex-session-delete"),
      desktop.manifest.codexHome, baseline[i], `handoff-${i}`, false);
  }
  await pass("handoffPreservesBothSavedSuppliersAndVisibleModelMenus");
}

(async () => {
  await fs.mkdir(outputDir, { recursive: true });
  try {
    await protectOriginalFiles();
    const upstream = await supplier();
    const externalUpstream = await supplier("external");
    const a = await context("manager-a", upstream), b = await context("manager-b", upstream);
    if (process.env.DESKTOP_BASELINE_MANAGER) a.executable = path.resolve(process.env.DESKTOP_BASELINE_MANAGER);
    await startManager(a);
    if (process.env.DESKTOP_PROVIDER_SCENARIO) {
      const providerDesktop = await launchDesktop(a, `${token}-provider`);
      await runProviderScenario(providerDesktop);
    }
    const first = await launchDesktop(a);
    await originalsIntact();
    if (process.env.DESKTOP_WORKER_HANDOFF_ONLY) {
      await runWorkerHandoffScenario(first, a);
      return;
    }
    if (process.env.DESKTOP_CONTINUITY_FAULT) {
      const fault = process.env.DESKTOP_CONTINUITY_FAULT;
      assert.ok(["cdp-hang", "interrupt"].includes(fault), "unknown fault injection");
      report.fault = fault;
      a.hangExit = true;
      if (fault === "interrupt") setTimeout(() => process.emit("SIGINT"), 200);
      await until(() => a.page.evaluate(() => new Promise(() => {})), "injected CDP timeout", 2000);
      assert.fail("injected hanging CDP call unexpectedly completed");
    }
    for (const request of [
      { instanceId: " main ", profileId: "fixture" },
      { profileId: "main", settings: { relayProfiles: [{ id: "main", name: "Fixture",
        relayMode: "pureApi", protocol: "responses" }] } },
    ]) {
      const rejected = await invoke(a, "launch_codex_multi_instance", { request });
      assert.equal(rejected.status, "failed");
      assert.match(rejected.message, /保留名称/);
      assert.equal(await fs.stat(path.join(a.state, "codex-multi", "main")).catch(() => null), null);
    }
    await pass("reservedMainRejectedBeforeLaunch");
    await startManager(b);
    const second = await launchDesktop(b);
    assert.notEqual(first.id.userDataDir, second.id.userDataDir);
    assert.notEqual(first.manifest.codexHome, second.manifest.codexHome);
    assert.notEqual(first.manifest.sqliteHome, second.manifest.sqliteHome);
    assert.notEqual(first.id.processId, second.id.processId);
    assert.notEqual(first.id.browserId, second.id.browserId);
    await pass("sameNameAcrossManagersIsIsolated");
    const repeated = await invoke(a, "launch_codex_multi_instance", { request: {
      instanceId: token, profileId: "fixture", appPath: appDir, debugPort: first.id.debugPort,
    } });
    assert.notEqual(repeated.status, "failed", repeated.message);
    assert.equal(repeated.pid, first.manifest.pid);
    await assertDesktopAlive(first);
    await pass("repeatedLaunchKeepsOriginalDesktop");
    await verifyLiveWorkerBlocks(a, first);
    await pass("liveWorkerBlocksOverwriteIncludingLegacyManifest");
    await sendPrompt(second, "Return a short synthetic response without tools.");
    await second.page.getByText("DESKTOP_RECOVERED_complete", { exact: true }).waitFor({ timeout: 30000 });
    await pass("secondRealDesktopCompletesSyntheticTurn");
    let workerPid = first.manifest.pid;
    await stopManager(a);
    await assertDesktopAlive(first);
    // 升级常会改变管理器安装路径；窗口保留，后台交给新路径的 worker。
    const updated = path.join(temporary, "updated-manager");
    await fs.mkdir(updated);
    for (const name of [path.basename(executable), "onnxruntime.dll",
      "onnxruntime_providers_shared.dll", "sherpa-onnx-c-api.dll", "sherpa-onnx-cxx-api.dll"]) {
      await bounded(fs.copyFile(path.join(path.dirname(executable), name), path.join(updated, name)));
    }
    a.executable = path.join(updated, path.basename(executable));
    await startManager(a);
    await waitForWorkerHandoff(first, workerPid, a.executable);
    workerPid = first.manifest.pid;
    const listed = await invoke(a, "list_codex_multi_instances");
    assert.ok(listed.instances.some(instance => instance.instanceId === token && instance.running));
    await pass("managerRestartRediscoversLiveDesktop");
    await verifyLiveWorkerBlocks(a, first);
    assert.equal(inventory().find(p => p.ProcessId === workerPid)?.ExecutablePath, a.executable);
    await pass("newManagerPathReplacesWorkerAndPreservesDesktop");
    await stopManager(a, true);
    await assertDesktopAlive(first);
    await assertDesktopAlive(second);
    await startManager(a);
    await waitForWorkerHandoff(first, workerPid, a.executable);
    workerPid = first.manifest.pid;
    const afterCrash = await invoke(a, "list_codex_multi_instances");
    assert.ok(afterCrash.instances.some(instance => instance.instanceId === token && instance.running));
    await pass("managerCrashRestartPreservesBothDesktops");
    await sendPrompt(first, "Return a short synthetic response without tools.");
    await first.page.getByText("DESKTOP_RECOVERED_complete", { exact: true }).waitFor({ timeout: 30000 });
    await pass("firstRealDesktopCompletesAfterManagerRestart");
    // Reproduce a lost proxy host while preserving its real Codex desktop and app-server.
    const resume = { state: path.join(first.root, "worker-home", ".codex-session-delete"),
      home: first.manifest.codexHome, sqlite: first.manifest.sqliteHome, debug: await freePort(),
      guard: await freePort(), launcherGuard: await freePort(), port: await freePort() };
    contexts.push(resume);
    const baseline = await providerSnapshot(resume.state, resume.home);
    assert.equal(baseline.activeRelayId, "fixture");
    assert.equal(baseline.defaultModel, "gpt-5.1-codex");
    mode = "hold";
    await sendPrompt(first, "Return another short synthetic response without tools.");
    await until(() => held.length > 0, "desktop did not start held streaming request");
    await writeExternalProvider(temporary, resume.home, externalUpstream, "before-crash", baseline.catalog);
    assert.deepEqual(JSON.parse(await fs.readFile(path.join(resume.state, "settings.json"), "utf8")).relayProfiles,
      baseline.profiles, "external config edit must leave saved supplier snapshots untouched");
    captureOwned();
    killOwned(workerPid);
    await writeExternalProvider(temporary, resume.home, externalUpstream, "after-crash", baseline.catalog);
    const recoveryEvents = report.events.length;
    mode = "after_crash";
    const blocker = http.createServer((_request, response) => response.writeHead(503).end());
    await listen(blocker, resume.port);
    await startManager(resume);
    await first.page.getByText("DESKTOP_RECOVERED_after_crash", { exact: true }).waitFor({ timeout: 90000 });
    await ensureClear(first.page);
    assertOriginalOwner(report.events, recoveryEvents);
    await verifyRecoveredProvider(first, resume.state, resume.home, baseline, "after-crash");
    await pass("externalCCSwitchBeforeAndAfterCrashPreservesSavedProviderCatalogAndOldEndpoint");
    await assertDesktopAlive(first);
    await pass("realDesktopRetriesAfterProxyHostCrash", { cachedPort: first.id.helperPort, newPort: resume.port });
    const resumeLog = await fs.readFile(path.join(resume.state, "codex-plus.log"), "utf8");
    assert.ok(resumeLog.includes("helper.proxy_primary_waiting"));
    await pass("realDesktopRecoversWhileNewPrimaryIsOccupied");
    await close(blocker);
    mode = "hold";
    const previousHeld = held.length;
    await sendPrompt(first, "Return a final short synthetic response without tools.");
    await until(() => held.length > previousHeld, "desktop did not start second held stream");
    await stopManager(resume);
    mode = "after_exit";
    resume.port = await freePort();
    const history = path.join(resume.state, "proxy-endpoints.json");
    await fs.access(path.join(resume.state, "proxy-endpoints.backup.json"));
    await fs.writeFile(history, '{}');
    await startManager(resume);
    await first.page.getByText("DESKTOP_RECOVERED_after_exit", { exact: true }).waitFor({ timeout: 90000 });
    await ensureClear(first.page);
    assertOriginalOwner(report.events, recoveryEvents);
    await verifyRecoveredProvider(first, resume.state, resume.home, baseline, "after-exit");
    await assertDesktopAlive(first);
    await pass("realDesktopRetriesAfterNormalProxyHostExit");
    const repaired = JSON.parse(await fs.readFile(history, "utf8"));
    assert.ok(repaired.homes.some(home => home.ports.includes(first.id.helperPort)));
    await until(async () => {
      const current = JSON.parse(await fs.readFile(history, "utf8"));
      return current.homes.some(home => home.ports.includes(resume.port));
    }, "new proxy port was not persisted after JSON corruption", 15000);
    await pass("realDesktopRecoversWithCorruptedPrimaryHistory");
    const manifestPath = path.join(second.root, "instance.json"), receiptPath = path.join(second.root, "launch-result.json");
    const savedManifest = await fs.readFile(manifestPath, "utf8"), savedReceipt = await fs.readFile(receiptPath, "utf8");
    try {
      const staleManifest = editableJson(savedManifest), staleReceipt = editableJson(savedReceipt);
      staleManifest.identity.browserId = staleReceipt.identity.browserId = "stale-test-browser";
      await fs.writeFile(receiptPath, JSON.stringify(staleReceipt));
      await fs.writeFile(manifestPath, JSON.stringify(staleManifest));
      const refused = await invoke(b, "stop_codex_multi_instance", { request: { instanceId: token } });
      assert.equal(refused.status, "failed", "stale receipt must never authorize a stop");
      await assertDesktopAlive(first);
      await assertDesktopAlive(second);
      await pass("staleIdentityRefusesToStopEitherDesktop");
    } finally {
      await fs.writeFile(receiptPath, savedReceipt);
      await fs.writeFile(manifestPath, savedManifest);
    }
    const stopped = await invoke(b, "stop_codex_multi_instance", { request: { instanceId: token } });
    assert.notEqual(stopped.status, "failed", stopped.message);
    await assertDesktopAlive(first);
    await originalsIntact();
    await pass("stoppingOneRealInstancePreservesOthers");
    // Simulate a reused PID using this harness, without terminating or modifying that process.
    const stale = editableJson(await fs.readFile(manifestPath, "utf8"));
    stale.pid = process.pid;
    stale.workerBirthId = 1;
    stale.identity = null;
    stale.launchResult = null;
    await fs.writeFile(manifestPath, JSON.stringify(stale));
    await fs.writeFile(receiptPath, JSON.stringify({ ...editableJson(savedReceipt), identity: null }));
    const relaunched = await launchDesktop(b);
    assert.notEqual(relaunched.id.processId, second.id.processId);
    await assertDesktopAlive(first);
    await originalsIntact();
    await pass("reusedWorkerPidDoesNotBlockRelaunch");
    await snapshot(first, "desktop-recovered");
  } catch (error) {
    report.error = error.stack || String(error);
    console.error(report.error);
    for (const desktop of desktops) if (desktop.page) {
      await snapshot(desktop, `failure-${desktops.indexOf(desktop)}`).catch(() => {});
    }
    for (const context of contexts) {
      const log = await bounded(fs.readFile(path.join(context.state, "codex-plus.log"), "utf8"), 3000).catch(() => "");
      report.events.push({ name: "diagnostic", state: context.state, text: log.slice(-20000) });
    }
    process.exitCode = 1;
  } finally {
    cleaning = true;
    report.interrupted = interrupted || null;
    for (const desktop of desktops) {
      const context = desktop.context;
      if (!context.page || context.current?.exitCode !== null) continue;
      try {
        const stopped = await invoke(context, "stop_codex_multi_instance", {
          request: { instanceId: desktop.instanceId },
        });
        if (stopped.status === "failed") report.events.push({
          name: "multiStopFallbackRequired", instanceId: desktop.instanceId, message: stopped.message,
        });
      } catch (error) {
        report.events.push({ name: "multiStopFallbackRequired", message: String(error) });
      }
    }
    for (const context of contexts) await stopManager(context).catch(error => {
      report.cleanupErrors = [...(report.cleanupErrors || []), String(error)];
    });
    // A crashed worker can leave its desktop alive: only then use its verified CDP identity.
    for (const desktop of desktops) await closeDesktop(desktop).catch(() => {});
    captureOwned();
    for (const process of inventory()) if (sameProcess(process, owned.get(process.ProcessId))) {
      try { killOwned(process.ProcessId); }
      catch (error) { report.cleanupErrors = [...(report.cleanupErrors || []), String(error)]; }
    }
    for (const response of held) response.destroy();
    for (const server of [...servers]) await close(server).catch(error => {
      report.cleanupErrors = [...(report.cleanupErrors || []), String(error)];
    });
    await until(() => !inventory().some(process => sameProcess(process, owned.get(process.ProcessId))),
      "owned test descendants remain alive", 15000).catch(error => {
        report.cleanupErrors = [...(report.cleanupErrors || []), String(error)];
      });
    report.checks.ownedProcessesExited = !inventory().some(process => sameProcess(process, owned.get(process.ProcessId)));
    report.checks.originalsPreserved = await originalsIntact().then(() => true, () => false);
    report.checks.portsReleased = true;
    report.unreleasedPorts = [];
    for (const port of [...ports]) {
      const server = net.createServer();
      try { await listen(server, port); }
      catch { report.unreleasedPorts.push(port); }
      finally { await close(server); }
    }
    // Windows may retain TIME_WAIT sockets after every owner has exited.
    if (report.unreleasedPorts.length) {
      const listeners = JSON.parse(powershell(
        "ConvertTo-Json -Compress -InputObject @(Get-NetTCPConnection -State Listen -ErrorAction SilentlyContinue | Select-Object LocalPort,OwningProcess)"));
      report.remainingListeners = listeners.filter(listener => report.unreleasedPorts.includes(listener.LocalPort));
      report.checks.portsReleased = report.remainingListeners.length === 0;
    }
    const finalInventory = inventory();
    // 原正式窗口可自然重建 renderer；必须确认其父进程仍是原身份，且不含测试目录。
    report.originalRendererDescendants = finalInventory.filter(candidate =>
      /^ChatGPT\.exe$/i.test(candidate.Name) && /--type=renderer(?:\s|$)/.test(candidate.CommandLine || "")
      && !candidate.CommandLine.includes(temporary)
      && protectedProcesses.some(parent => parent.ProcessId === candidate.ParentProcessId
        && parent.ExecutablePath === candidate.ExecutablePath
        && finalInventory.some(current => sameProcess(parent, current)))).map(identity);
    report.unattributed = finalInventory.filter(candidate =>
      /^(codex-plus-plus|codex|ChatGPT|msedgewebview2)\.exe$/i.test(candidate.Name)
      && !before.some(prior => sameProcess(prior, candidate))
      && !sameProcess(candidate, owned.get(candidate.ProcessId))
      && !report.originalRendererDescendants.some(renderer => sameProcess(renderer, candidate))).map(identity);
    report.checks.noUnattributedApplicationProcesses = report.unattributed.length === 0;
    if (report.checks.ownedProcessesExited && report.checks.portsReleased
      && report.checks.originalsPreserved && report.checks.noUnattributedApplicationProcesses
      && !report.cleanupErrors?.length) {
      assert.equal(path.dirname(temporary), os.tmpdir());
      assert.equal(path.basename(temporary), token);
      try {
        for (const desktop of desktops) if (desktop.actualData) assertWithin(temporary, desktop.actualData);
        const archive = path.join(outputDir, `archived-test-data-${token}`);
        assert.ok(!path.resolve(outputDir).toLowerCase().startsWith(temporary.toLowerCase() + path.sep));
        await fs.rename(temporary, archive);
        report.archive = archive;
        report.cleanup = true;
      } catch (error) { report.cleanupErrors = [...(report.cleanupErrors || []), String(error)]; }
    }
    report.owned = [...owned.values()];
    await save();
    console.log(JSON.stringify({ checks: report.checks, cleanup: report.cleanup, error: report.error }, null, 2));
    if (!report.cleanup || report.cleanupErrors?.length || Object.values(report.checks).some(value => !value)) process.exitCode = 1;
  }
})();

async function selfTest() {
  assert.equal(fixtureRootString('model = "fixture"\n[x]\nmodel = "other"\n', "model"), "fixture");
  assert.equal(fixtureRootString("model_catalog_json = 'C:\\fixture\\models.json'\n", "model_catalog_json"),
    "C:\\fixture\\models.json");
  assert.throws(() => fixtureRootString('[x]\nmodel = "other"\n', "model"), /quoted root/);
  assert.throws(() => assertWithin(os.tmpdir(), path.join(os.tmpdir(), "..", "outside")), /inside/);
  assertOriginalOwner([{ name: "supplierRequest", owner: "original" }], 0);
  assert.throws(() => assertOriginalOwner([], 0), /real local/);
  assert.throws(() => assertOriginalOwner([{ name: "supplierRequest", owner: "external" }], 0), /changed supplier/);
  const evidence = path.resolve("output", `desktop-continuity-self-test-${Date.now()}-${process.pid}`);
  const runtime = path.join(evidence, "runtime"), home = path.join(runtime, "home");
  await fs.mkdir(home, { recursive: true });
  const fixtureCatalog = { models: [{ slug: "original", display_name: "Original" }] };
  await writeExternalProvider(runtime, home, "http://127.0.0.1:12345", "before-crash", fixtureCatalog);
  assert.deepEqual(fixtureCatalog.models.map(model => model.slug), ["original"], "fixture must not mutate baseline");
  await fs.writeFile(path.join(runtime, "settings.json"), JSON.stringify({
    activeRelayId: "original", relayProfiles: [{ id: "original", model: "original" }],
  }));
  const captured = await providerSnapshot(runtime, home);
  assert.equal(captured.defaultModel, "gpt-fixture-external-before-crash");
  assert.equal(captured.catalog.models[0].slug, captured.defaultModel);
  await writeExternalProvider(runtime, home, "http://127.0.0.1:23456", "after-crash", fixtureCatalog);
  assert.equal((await providerSnapshot(runtime, home)).defaultModel, "gpt-fixture-external-after-crash");
  await assert.rejects(writeExternalProvider(runtime, home, "https://example.com", "bad", fixtureCatalog), /AssertionError/);
  await assert.rejects(writeExternalProvider(runtime, evidence, "http://127.0.0.1:12345", "bad", fixtureCatalog), /inside/);
  await fs.rename(runtime, path.join(evidence, "archived-test-data"));
  console.log(`PASS synthetic external edits, catalog/default snapshots, owner rejection, and archive: ${evidence}`);
  const receipt = '{"birth":134052217980000001,"generation":42}';
  assert.equal(JSON.stringify(editableJson(receipt)), receipt);
  const never = () => new Promise(() => {});
  const watchdog = promise => {
    let timer;
    return Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error("self-test watchdog expired")), 1000);
    })]).finally(() => clearTimeout(timer));
  };
  await assert.rejects(watchdog(until(never, "hung check", 20)), /hung check/);
  let exited = false, forced = 0;
  assert.equal(await exitWithFallback(never, () => exited, () => {
    forced++;
    exited = true;
  }, 20), true);
  assert.equal(forced, 1);
  assert.equal(await exitWithFallback(() => Promise.resolve(), () => true,
    () => assert.fail("already exited manager must not be forced"), 20), false);
  try {
    const timer = setTimeout(() => process.emit("SIGTERM"), 20);
    try { await assert.rejects(until(never, "hung CDP", 2000), /interrupted: SIGTERM/); }
    finally { clearTimeout(timer); }
    cleaning = true;
    await bounded(pause(20), 100);
    exited = false;
    assert.equal(await exitWithFallback(never, () => exited, () => { exited = true; }, 20), true);
  } finally {
    interrupted = "";
    cleaning = false;
    process.exitCode = 0;
  }
  console.log("PASS bounded checks, hung exit fallback, interruption, and cleanup after interruption");
}
