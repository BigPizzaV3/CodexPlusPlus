// Windows-only, opt-in acceptance harness. Never sends a prompt or installs dependencies.
// Usage: node verify-session-history-recovery.cjs managerExe appDir fixtureRoot outputDir
// fixtureRoot must contain the already isolated home/ and sqlite-home/ snapshots.
// history-recovery-targets.json supplies [{id,label}] for the two fixture conversations.
const assert = require("node:assert/strict");
const fs = require("node:fs/promises");
const { readFileSync } = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const net = require("node:net");
const http = require("node:http");
const { spawn, execFileSync } = require("node:child_process");
if (process.argv[2] === "--self-test") {
  selfTest().catch(error => { console.error(error); process.exitCode = 1; });
  return;
}
const { chromium } = require("playwright");

assert.equal(process.platform, "win32");
assert.equal(process.argv.length, 6, "Expected managerExe appDir fixtureRoot outputDir");
const [managerExe, appDir, fixtureRoot, outputDir] = process.argv.slice(2).map(p => path.resolve(p));
const token = `history-recovery-${Date.now()}-${process.pid}`;
const temporary = path.join(os.tmpdir(), token);
const targets = JSON.parse(readFileSync(path.join(fixtureRoot, "history-recovery-targets.json"), "utf8"));
assert.equal(targets.length, 2);
for (const target of targets) {
  assert.match(target.id, /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/);
  assert.match(target.label, /^[a-z][a-z0-9-]*$/);
}
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
const comparableText = value => value.normalize("NFKC").replace(/[\s\p{P}\p{S}]/gu, "");
const ps = (script, env = process.env) => execFileSync("powershell.exe",
  ["-NoProfile", "-NonInteractive", "-Command", script],
  { env, encoding: "utf8", windowsHide: true, maxBuffer: 16e6, timeout: 30000 });
const inventory = () => JSON.parse(ps(
  "ConvertTo-Json -Compress -InputObject @(Get-CimInstance Win32_Process | " +
  "Select-Object ProcessId,ParentProcessId,Name,ExecutablePath,CommandLine," +
  "@{Name='CreationDate';Expression={$_.CreationDate.ToUniversalTime().ToString('o')}})"));
const before = inventory();
const protectedProcesses = before.filter(p => /^(codex-plus-plus|codex|ChatGPT|cc-switch)\.exe$/i.test(p.Name));
const same = (a, b) => a && b && a.ProcessId === b.ProcessId
  && a.ExecutablePath === b.ExecutablePath && a.CreationDate === b.CreationDate;
const identity = p => ({ ProcessId: p.ProcessId, ParentProcessId: p.ParentProcessId,
  Name: p.Name, ExecutablePath: p.ExecutablePath, CreationDate: p.CreationDate });
const inside = (parent, child) => {
  const relative = path.relative(parent, child);
  return relative !== "" && !relative.startsWith("..") && !path.isAbsolute(relative);
};
const owned = new Map(), ports = new Set(), desktops = [];
const state = path.join(temporary, "manager");
let manager, managerBrowser, managerPage, upstream;
let interrupted = false, cleaning = false;
for (const signal of ["SIGINT", "SIGTERM"]) process.once(signal, () => {
  interrupted = true;
  process.exitCode = 1;
});
const report = { token, fixtureRoot, temporary, checks: {}, desktops: [], cleanupErrors: [],
  requests: [], cleanup: false };
const save = () => bounded(fs.writeFile(path.join(outputDir, `${token}.json`), JSON.stringify(report, null, 2)));
async function until(check, reason, timeout = 60000) {
  const deadline = Date.now() + timeout;
  do {
    if (interrupted && !cleaning) throw Error("Acceptance interrupted; cleaning owned processes");
    if (await bounded(Promise.resolve().then(check), Math.max(1, deadline - Date.now()))) return;
    await bounded(pause(Math.min(350, Math.max(1, deadline - Date.now()))));
  } while (Date.now() < deadline);
  throw Error(reason);
}
async function bounded(promise, ms = 10000) {
  let timer, interruptTimer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      if (interrupted && !cleaning) {
        reject(Error("Acceptance interrupted"));
        return;
      }
      timer = setTimeout(() => reject(Error("Operation timed out")), ms);
      interruptTimer = setInterval(() => {
        if (interrupted && !cleaning) reject(Error("Acceptance interrupted"));
      }, 250);
    })]);
  } finally { clearTimeout(timer); clearInterval(interruptTimer); }
}
async function rejectLinks(root) {
  for (const entry of await bounded(fs.readdir(root, { withFileTypes: true }))) {
    const filename = path.join(root, entry.name);
    assert.ok(!entry.isSymbolicLink(), "Fixture links must be flattened before acceptance");
    if (entry.isDirectory()) await rejectLinks(filename);
  }
}
async function freePort() {
  const server = net.createServer();
  let port;
  try {
    await bounded(new Promise((resolve, reject) => {
      server.once("error", reject);
      server.listen(0, "127.0.0.1", resolve);
    }));
    port = server.address().port;
  } finally {
    await bounded(new Promise(resolve => server.close(resolve)));
  }
  // Avoid reusing a previously selected port, although binding still has a TOCTOU window.
  if (ports.has(port)) return freePort();
  ports.add(port);
  return port;
}
function captureOwned() {
  const processes = inventory();
  const ids = new Set(processes.filter(p => same(p, owned.get(p.ProcessId))).map(p => p.ProcessId));
  for (const p of processes) {
    if (manager && p.ProcessId === manager.pid && p.ParentProcessId === process.pid
      && p.ExecutablePath?.toLowerCase() === managerExe.toLowerCase()) ids.add(p.ProcessId);
    if (/^(ChatGPT|codex|codex-plus-plus|msedgewebview2|cmd)\.exe$/i.test(p.Name)
      && p.CommandLine?.toLowerCase().includes(temporary.toLowerCase())
      && !before.some(old => same(old, p))) ids.add(p.ProcessId);
  }
  for (let changed = true; changed;) {
    changed = false;
    for (const p of processes) if (ids.has(p.ParentProcessId) && !ids.has(p.ProcessId)
      && !before.some(old => same(old, p))) {
      ids.add(p.ProcessId);
      changed = true;
    }
  }
  for (const p of processes) if (ids.has(p.ProcessId)) {
    assert.ok(!before.some(old => same(old, p)), "Refuse ownership of a pre-existing process");
    owned.set(p.ProcessId, identity(p));
  }
  return processes;
}
function killOwned(pid) {
  const actual = inventory().find(p => p.ProcessId === pid);
  if (!actual) return;
  assert.ok(same(actual, owned.get(pid)), "Refuse termination after PID identity changed");
  assert.ok(!before.some(old => same(old, actual)));
  ps("$ErrorActionPreference='Stop'; $target=[Diagnostics.Process]::GetProcessById([int]$env:HISTORY_PID); " +
    "try { $null=$target.SafeHandle; $birth=[DateTimeOffset]::Parse($env:HISTORY_BIRTH).UtcDateTime; " +
    "if ($target.Path -eq $env:HISTORY_EXE -and " +
    "[Math]::Abs(($target.StartTime.ToUniversalTime()-$birth).Ticks) -lt 10) { " +
    "$target.Kill(); $null=$target.WaitForExit(3000) } else { throw 'Identity changed' } " +
    "} finally { $target.Dispose() }",
  { ...process.env, HISTORY_PID: String(pid), HISTORY_EXE: actual.ExecutablePath,
    HISTORY_BIRTH: actual.CreationDate });
}
const invoke = (command, args = {}, timeout = 30000) => bounded(managerPage.evaluate(
  ({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args), { command, args }), timeout);

async function connectBrowser(port) {
  const connecting = chromium.connectOverCDP(`http://127.0.0.1:${port}`, { timeout: 10000 });
  // A connection can settle after interruption; close that late transport as well.
  let abandoned = false;
  connecting.then(browser => {
    if (abandoned) bounded(browser.close()).catch(error => report.cleanupErrors.push(String(error)));
  }, () => {});
  try {
    const browser = await bounded(connecting);
    for (const context of browser.contexts()) context.setDefaultTimeout(10000);
    return browser;
  } catch (error) {
    abandoned = true;
    throw error;
  }
}

// Match CODEX_SQLITE_HOME precedence without ever falling back to the real user home.
const sqlitePython = String.raw`
import json, pathlib, sqlite3, sys
def normalized(value):
 value=str(value)
 if value.lower().startswith("\\\\?\\unc\\"): value="\\\\"+value[8:]
 elif value.startswith("\\\\?\\"): value=value[4:]
 return value.replace("\\","/").rstrip("/")
def databases(home,sqlite):
 result=[]
 # An explicit CODEX_SQLITE_HOME excludes the legacy home database tree.
 for root in (sqlite,sqlite/"sqlite"):
  if not root.is_dir(): continue
  for file in sorted(root.iterdir()):
   if file.is_file() and file.suffix.lower() in (".sqlite",".db") and file not in result: result.append(file)
 return result
def connect_read(file):
 return sqlite3.connect(file.resolve().as_uri()+"?mode=ro",uri=True)
def has_table(db,name):
 return db.execute("select 1 from sqlite_master where type='table' and name=?",(name,)).fetchone() is not None
`;
// Only the copied databases are opened writable. JSON argv avoids shell interpolation.
const preparePython = sqlitePython + String.raw`
a=json.loads(sys.argv[1])
old=pathlib.Path(normalized(a["fixtureHome"])).resolve()
home=pathlib.Path(normalized(a["home"])).resolve()
sqlite=pathlib.Path(normalized(a["sqlite"])).resolve()
oldtext=normalized(old).lower()
rewritten=0
for file in databases(home,sqlite):
 db=sqlite3.connect(str(file))
 try:
  for table in ("threads","local_thread_catalog"):
   if not has_table(db,table): continue
   columns={r[1] for r in db.execute('pragma table_info("'+table+'")')}
   if "rollout_path" not in columns: continue
   for rowid,value in db.execute('select rowid,rollout_path from "'+table+'"').fetchall():
    if not isinstance(value,str): continue
    clean=normalized(value)
    if clean.lower().startswith(oldtext+"/"):
     db.execute('update "'+table+'" set rollout_path=? where rowid=?',
      (str(home.joinpath(*clean[len(oldtext)+1:].split("/"))),rowid))
     rewritten+=1
  db.commit()
 finally: db.close()
print(json.dumps({"rolloutPathsRewritten":rewritten}))
`;
const referencesPython = sqlitePython + String.raw`
a=json.loads(sys.argv[1]); home=pathlib.Path(a["home"]); sqlite=pathlib.Path(a["sqlite"])
def texts(v):
 if isinstance(v,dict):
  for k,x in v.items():
   if k in ("text","output_text") and isinstance(x,str) and len(x.strip())>=30: yield " ".join(x.split())[:90]
   elif isinstance(x,(dict,list)): yield from texts(x)
 elif isinstance(v,list):
  for x in v: yield from texts(x)
history_path=sqlite/"thread_history_1.sqlite"
assert history_path.is_file(), "Missing history DB under explicit isolated CODEX_SQLITE_HOME"
history=connect_read(history_path)
result=[]
for target in a["targets"]:
 snippets=[]
 for row in history.execute("select item_json from thread_items where thread_id=? order by rollout_ordinal desc limit 40",(target["id"],)):
  try: snippets.extend(texts(json.loads(row[0])))
  except (ValueError,TypeError): pass
 project=""; cwd=""; title=""
 for file in databases(home,sqlite):
  db=connect_read(file)
  try:
   if not has_table(db,"threads"): continue
   columns={r[1] for r in db.execute('pragma table_info("threads")')}
   title_column="coalesce(nullif(name,''),title)" if "name" in columns else "title"
   row=db.execute("select cwd,"+title_column+" from threads where id=?",(target["id"],)).fetchone()
   if row and row[0]:
    candidate=str(row[0])
    assert not cwd or normalized(cwd).lower()==normalized(candidate).lower(), "Conflicting canonical project roots"
    cwd=candidate
    project=normalized(cwd).split("/")[-1]
    title=str(row[1] or "")
  finally: db.close()
 result.append(dict(target,snippets=list(dict.fromkeys(snippets))[:30],project=project,cwd=cwd,title=title))
print(json.dumps(result,ensure_ascii=True))
`;
const catalogPython = sqlitePython + String.raw`
a=json.loads(sys.argv[1]); home=pathlib.Path(a["home"]); sqlite=pathlib.Path(a["sqlite"])
files=databases(home,sqlite)
ids=set(a.get("ids",[]))
if a["mode"]=="seed":
 projects=set()
 for file in files:
  db=connect_read(file)
  try:
   if not has_table(db,"threads"): continue
   for target in a["targets"]:
    row=db.execute("select cwd from threads where id=?",(target["id"],)).fetchone()
    if row: projects.add(normalized(row[0]).lower())
  finally: db.close()
 assert len(projects)==2, "Expected the two distinct target projects in canonical threads"
 for file in files:
  db=connect_read(file)
  try:
   if not has_table(db,"threads"): continue
   columns={r[1] for r in db.execute('pragma table_info("threads")')}
   if "thread_source" not in columns: continue
   for id,cwd in db.execute("select id,cwd from threads where thread_source='user'"):
    if normalized(cwd).lower() in projects: ids.add(id)
  finally: db.close()
 assert len(ids)==26, "Expected exactly 26 target project user threads; got "+str(len(ids))
 assert {t["id"] for t in a["targets"]} <= ids
found=set(); changed=0; catalog_files=[]
for file in files:
 db=connect_read(file) if a["mode"]=="verify" else sqlite3.connect(str(file))
 try:
  if not has_table(db,"local_thread_catalog"): continue
  catalog_files.append(str(file))
  for id in sorted(ids):
   rows=db.execute("select thread_id from local_thread_catalog where thread_id=?",(id,)).fetchall()
   if rows: found.add(id)
   if a["mode"]=="seed":
    changed+=db.execute("delete from local_thread_catalog where thread_id=?",(id,)).rowcount
  if a["mode"]=="seed": db.commit()
 finally: db.close()
assert catalog_files, "No isolated catalog database"
if a["mode"]=="seed":
 assert found==ids, "Fixture must contain all 26 target catalog entries before seeding"
 assert changed==26, "Expected exactly 26 catalog rows in the isolated fixture; got "+str(changed)
else: assert found==ids, "Automatic catalog recovery did not restore all 26 target entries"
print(json.dumps({"ids":sorted(ids),"catalogRowsRemoved":changed,"present":len(found),"catalogFiles":catalog_files}))
`;
const python = (code, args) => execFileSync(process.env.HISTORY_PYTHON || "python",
  ["-X", "utf8", "-c", code, JSON.stringify(args)],
  { encoding: "utf8", windowsHide: true, maxBuffer: 4e6, timeout: 60000 });

async function openDesktop(desktop) {
  const launchedAt = Date.now();
  const result = await invoke("launch_codex_multi_instance", { request: {
    instanceId: desktop.instanceId, profileId: "fixture", appPath: appDir, debugPort: desktop.debug,
  } });
  assert.notEqual(result.status, "failed", result.message);
  await until(async () => {
    captureOwned();
    const receipt = await fs.readFile(path.join(desktop.root, "launch-result.json"), "utf8")
      .then(JSON.parse).catch(() => null);
    if (!receipt) return false;
    assert.notEqual(receipt.status, "failed", receipt.message);
    desktop.receipt = receipt;
    return receipt.identity;
  }, "Isolated desktop failed to start", 150000);
  const id = desktop.receipt.identity;
  desktop.id = id;
  assert.equal(id.instanceId, desktop.instanceId);
  assert.equal(id.debugPort, desktop.debug);
  const data = path.resolve(id.userDataDir.replace(/^\\\\\?\\/, ""));
  assert.ok(inside(temporary, data), "Electron state escaped this test tree");
  const actual = captureOwned().find(p => p.ProcessId === id.processId);
  assert.ok(same(actual, owned.get(id.processId)), "Unowned desktop");
  ports.add(id.helperPort);
  const listed = await invoke("list_codex_multi_instances");
  const manifest = listed.instances.find(item => item.instanceId === desktop.instanceId);
  assert.ok(manifest?.running, "Desktop manifest is not running");
  assert.equal(path.resolve(manifest.codexHome), desktop.home);
  assert.equal(path.resolve(manifest.sqliteHome), desktop.sqlite);
  desktop.browser = await connectBrowser(id.debugPort);
  await until(async () => {
    desktop.page = desktop.browser.contexts().flatMap(c => c.pages()).find(p => p.url().startsWith("app://"));
    return desktop.page && await desktop.page.locator("body").innerText().then(t => t.length > 100).catch(() => false);
  }, "Desktop page is blank");
  await until(async () => {
    const automatic = await fs.readFile(path.join(desktop.home, "session-index-repair/report.json"), "utf8")
      .then(JSON.parse).catch(() => null);
    if (!automatic || automatic.checkedAtMs < launchedAt) return false;
    assert.ok(automatic.compressedFiles > 0, "Automatic repair did not cover compressed sources");
    desktop.automatic = automatic;
    return true;
  }, "No fresh automatic compressed-source report", 240000);
  const catalog = JSON.parse(python(catalogPython, {
    mode: "verify", home: desktop.home, sqlite: desktop.sqlite, ids: desktop.seed.ids,
  }));
  report.desktops.push({ identity: id, automatic: desktop.automatic, home: desktop.home,
    sqlite: desktop.sqlite, preparation: desktop.preparation, seed: desktop.seed, catalog });
  report.checks[`${desktop.instanceId}.automaticCompressed`] = true;
  report.checks[`${desktop.instanceId}.automaticCatalog26Restored`] = true;
  await save();
}
async function verifyConversation(desktop, target) {
  assert.ok(target.snippets.length, `No reference body for ${target.id}`);
  assert.ok(target.project, `No fixture project for ${target.id}`);
  assert.ok(target.title, `No fixture conversation title for ${target.id}`);
  const continueModel = desktop.page.getByRole("button", { name: "继续使用当前模型", exact: true });
  if (await bounded(continueModel.isVisible())) await bounded(continueModel.click());
  await bounded(desktop.page.screenshot({
    path: path.join(outputDir, `${token}-${desktop.instanceId}-${target.label}-before.png`),
  }));
  let link = desktop.page.locator(
    `[data-app-action-sidebar-thread-id="local:${target.id}"]:visible,a[href*="${target.id}"]:visible`,
  ).first();
  let titled = desktop.page.getByText(target.title, { exact: true }).first();
  for (let attempt = 0; attempt < 8 && !await bounded(link.isVisible()) && !await bounded(titled.isVisible()); attempt++) {
    const expand = desktop.page.getByText("展开显示", { exact: true }).first();
    if (!await bounded(expand.isVisible())) break;
    await bounded(expand.click());
    await bounded(pause(500));
  }
  const evidence = { instanceId: desktop.instanceId, threadId: target.id, project: target.project };
  if (await bounded(link.isVisible()) || await bounded(titled.isVisible())) {
    evidence.sidebarVisible = true;
    evidence.navigation = await bounded(link.isVisible()) ? "visible-thread-link" : "visible-conversation-title";
    await bounded(desktop.page.screenshot({
      path: path.join(outputDir, `${token}-${desktop.instanceId}-${target.label}-list.png`),
    }));
    await bounded((await bounded(link.isVisible()) ? link : titled).click());
  }
  else {
    evidence.sidebarVisible = false;
    evidence.navigation = "not-discovered";
    await bounded(fs.writeFile(path.join(outputDir, `${token}-${desktop.instanceId}-navigation.txt`),
      await bounded(desktop.page.locator("body").innerText())));
    throw Error(`Conversation title is not discoverable in the isolated sidebar: ${target.label}`);
  }
  await until(async () => {
    const text = (await desktop.page.locator("body").innerText()).replace(/\s+/g, " ");
    const rendered = comparableText(text);
    const activeIdentity = await desktop.page.locator(
      `[data-above-composer-conversation-id="${target.id}"]`,
    ).count() > 0;
    return activeIdentity && text.toLowerCase().includes(target.project.toLowerCase())
      && target.snippets.some(snippet => {
        const expected = comparableText(snippet);
        return expected.length >= 25 && rendered.includes(expected);
      });
  }, `Desktop did not render project and existing message: ${target.label}`, 90000);
  await bounded(desktop.page.screenshot({ path: path.join(outputDir, `${token}-${desktop.instanceId}-${target.label}.png`) }));
  report.navigation ??= [];
  report.navigation.push(evidence);
  report.sidebarCoverage ??= {};
  report.sidebarCoverage[`${desktop.instanceId}.${target.label}`] =
    evidence.sidebarVisible ? "verified-visible" : "not-verified";
  report.checks[`${desktop.instanceId}.${target.label}.rendered`] = true;
  await save();
}
async function cleanup() {
  cleaning = true;
  const safely = async action => { try { await action(); } catch (e) { report.cleanupErrors.push(String(e)); } };
  await safely(async () => {
    const listeners = JSON.parse(ps("ConvertTo-Json -Compress -InputObject @(Get-NetTCPConnection -State Listen " +
      "-ErrorAction SilentlyContinue | Select-Object LocalPort,OwningProcess)"));
    captureOwned();
    for (const listener of listeners) if (owned.has(listener.OwningProcess)) ports.add(listener.LocalPort);
  });
  for (const desktop of desktops) await safely(async () => {
    if (!desktop.browser || !desktop.id) return;
    assert.ok(same(inventory().find(p => p.ProcessId === desktop.id.processId), owned.get(desktop.id.processId)));
    const version = await fetch(`http://127.0.0.1:${desktop.id.debugPort}/json/version`,
      { signal: AbortSignal.timeout(3000) }).then(r => r.json());
    assert.ok(version.webSocketDebuggerUrl.endsWith(`/devtools/browser/${desktop.id.browserId}`));
    const session = await bounded(desktop.browser.newBrowserCDPSession());
    // Electron may close the socket before acknowledging Browser.close.
    await bounded(session.send("Browser.close"), 5000).catch(() => {});
  });
  if (managerPage && manager && manager.exitCode === null) await safely(async () => {
    await invoke("manager_exit_app", {}, 5000).catch(() => {});
    await until(() => manager.exitCode !== null, "Manager normal exit timed out", 15000)
      .catch(error => { report.managerExitFallback = String(error); });
  });
  for (const desktop of desktops) await safely(() => bounded(desktop.browser?.close() || Promise.resolve()));
  await safely(() => bounded(managerBrowser?.close() || Promise.resolve()));
  captureOwned();
  for (const p of inventory().reverse()) if (same(p, owned.get(p.ProcessId))) {
    await safely(async () => killOwned(p.ProcessId));
  }
  if (upstream) await safely(async () => {
    upstream.closeAllConnections?.();
    await bounded(new Promise(resolve => upstream.close(resolve)));
  });
  await safely(() => until(() => {
    const remaining = captureOwned().filter(p => same(p, owned.get(p.ProcessId)));
    for (const p of remaining.reverse()) killOwned(p.ProcessId);
    return remaining.length === 0;
  }, "Owned test descendants remain", 15000));
  const current = inventory();
  report.checks.originalProcessesPreserved = protectedProcesses.every(old => current.some(p => same(old, p)));
  report.checks.ownedProcessesExited = !current.some(p => same(p, owned.get(p.ProcessId)));
  const listeners = JSON.parse(ps("ConvertTo-Json -Compress -InputObject @(Get-NetTCPConnection -State Listen " +
    "-ErrorAction SilentlyContinue | Select-Object LocalPort,OwningProcess)"));
  report.remainingListeners = listeners.filter(p => ports.has(p.LocalPort));
  report.checks.portsReleased = report.remainingListeners.length === 0;
  report.checks.noModelCalls = report.requests.every(r => r.method === "GET");
  report.owned = [...owned.values()];
  if (report.checks.ownedProcessesExited && report.checks.portsReleased) await safely(async () => {
    const destination = path.join(outputDir, `${token}-archive`);
    assert.equal(path.dirname(temporary), os.tmpdir());
    assert.ok(inside(outputDir, destination));
    // Move only this run's verified directory; never recursively delete fixture or user data.
    ps("$ErrorActionPreference='Stop'; Move-Item -LiteralPath $env:HISTORY_SOURCE -Destination $env:HISTORY_DEST",
      { ...process.env, HISTORY_SOURCE: temporary, HISTORY_DEST: destination });
    report.archive = destination;
  });
  report.cleanup = report.checks.originalProcessesPreserved && report.checks.ownedProcessesExited
    && report.checks.portsReleased && report.cleanupErrors.length === 0;
}

async function selfTest() {
  const vm = require("node:vm");
  const source = readFileSync(__filename, "utf8");
  const helpers = source.slice(source.indexOf("async function until("), source.indexOf("async function rejectLinks("));
  const context = vm.createContext({
    setTimeout, clearTimeout, setInterval, clearInterval,
    pause: ms => new Promise(resolve => setTimeout(resolve, ms)),
    interrupted: false, cleaning: false,
  });
  vm.runInContext(helpers, context);
  const { EventEmitter } = require("node:events");
  context.process = new EventEmitter();
  vm.runInContext(source.slice(source.indexOf('for (const signal of ["SIGINT"'),
    source.indexOf("const report =")), context);
  const never = () => new Promise(() => {});
  const watchdog = async promise => {
    let timer;
    try {
      return await Promise.race([promise, new Promise((_, reject) => {
        timer = setTimeout(() => reject(Error("self-test watchdog: wait did not finish")), 1000);
      })]);
    } finally { clearTimeout(timer); }
  };
  await assert.rejects(watchdog(context.until(never, "deadline", 20)), /timed out|deadline/i);
  let finalized = false;
  const interrupted = (async () => {
    try { await context.until(never, "signal", 10000); }
    finally { finalized = true; }
  })();
  setTimeout(() => context.process.emit("SIGINT"), 20);
  await assert.rejects(watchdog(interrupted), /interrupted/i);
  assert.ok(finalized, "interruption must reach finally");
  context.cleaning = true;
  await assert.rejects(watchdog(context.bounded(never(), 20)), /timed out/i);
  assert.equal(await context.bounded(Promise.resolve("cleanup"), 20), "cleanup");
  context.managerPage = { evaluate: () => never() };
  let completeConnection;
  let lateClosed = false;
  context.chromium = { connectOverCDP: (_url, options) => {
    assert.ok(options.timeout > 0);
    return new Promise(resolve => { completeConnection = resolve; });
  } };
  context.report = { cleanupErrors: [] };
  vm.runInContext(source.slice(source.indexOf("const invoke ="),
    source.indexOf("// Match CODEX_SQLITE_HOME")) + "\nglobalThis.testInvoke = invoke;", context);
  await assert.rejects(watchdog(context.testInvoke("synthetic", {}, 20)), /timed out/i);
  context.cleaning = false;
  context.interrupted = false;
  const ipc = (async () => {
    try { await context.testInvoke("synthetic"); }
    finally { finalized = true; }
  })();
  finalized = false;
  setTimeout(() => context.process.emit("SIGTERM"), 20);
  await assert.rejects(watchdog(ipc), /interrupted/i);
  assert.ok(finalized);
  context.interrupted = false;
  const connecting = context.connectBrowser(12345);
  setTimeout(() => { context.interrupted = true; }, 20);
  await assert.rejects(watchdog(connecting), /interrupted/i);
  context.cleaning = true;
  completeConnection({ close: async () => { lateClosed = true; } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.ok(lateClosed, "late CDP connection must close after cancellation");
  // Inspect both synchronous launchers without creating any process.
  const calls = [];
  const execContext = vm.createContext({
    process: { env: {} }, JSON,
    execFileSync: (...args) => calls.push(args),
  });
  vm.runInContext(source.slice(source.indexOf("const ps ="), source.indexOf("const inventory ="))
    + source.slice(source.indexOf("const python ="), source.indexOf("async function openDesktop("))
    + '\nps("synthetic"); python("synthetic", {});', execContext);
  assert.equal(calls.length, 2);
  assert.ok(calls.every(([, , options]) => options.timeout > 0 && Number.isFinite(options.timeout)));
  console.log("Synthetic deadline, interruption/finally, and cleanup tests passed");
}

(async () => {
  try {
    await bounded(fs.mkdir(outputDir, { recursive: true }));
    assert.ok(!inside(fixtureRoot, outputDir) && outputDir !== fixtureRoot, "Output must be outside source fixture");
    assert.notEqual(await bounded(fs.realpath(path.join(fixtureRoot, "home"))),
      path.resolve(process.env.USERPROFILE, ".codex"), "Production CODEX_HOME is not a fixture");
    await bounded(fs.access(path.join(fixtureRoot, "sqlite-home")));
    await rejectLinks(fixtureRoot);
    await bounded(fs.access(managerExe));
    await bounded(fs.access(appDir));
    await bounded(fs.mkdir(state, { recursive: true }));
    upstream = http.createServer((request, response) => {
      report.requests.push({ method: request.method, path: request.url });
      // Any model submission is a harness failure, not a synthetic successful completion.
      response.writeHead(request.method === "GET" ? 200 : 503, { "content-type": "application/json" });
      response.end(JSON.stringify(request.method === "GET"
        ? { object: "list", data: [{ id: "fixture-model", object: "model" }] }
        : { error: { message: "History-only acceptance: model calls forbidden" } }));
    });
    await bounded(new Promise((resolve, reject) => {
      upstream.once("error", reject);
      upstream.listen(0, "127.0.0.1", resolve);
    }));
    ports.add(upstream.address().port);
    const base = `http://127.0.0.1:${upstream.address().port}/v1`;
    for (const suffix of ["a", "b"]) {
      const instanceId = `${token}-${suffix}`, root = path.join(state, "codex-multi", instanceId);
      const desktop = { instanceId, root, home: path.join(root, "codex-home"),
        sqlite: path.join(root, "sqlite-home"), debug: await freePort() };
      desktops.push(desktop); // Register before any process creation or fallible launch.
      await bounded(fs.cp(path.join(fixtureRoot, "home"), desktop.home,
        { recursive: true, errorOnExist: true, force: false }), 120000);
      await bounded(fs.cp(path.join(fixtureRoot, "sqlite-home"), desktop.sqlite,
        { recursive: true, errorOnExist: true, force: false }), 120000);
      desktop.preparation = JSON.parse(python(preparePython, {
        fixtureHome: path.join(fixtureRoot, "home"), home: desktop.home, sqlite: desktop.sqlite,
      }));
      desktop.references = JSON.parse(python(referencesPython, { home: desktop.home, sqlite: desktop.sqlite, targets }));
      const roots = [...new Set(desktop.references.map(target => target.cwd))];
      assert.equal(roots.length, 2, "Expected two canonical target workspace roots");
      assert.ok(roots.every(root => typeof root === "string" && path.isAbsolute(root)),
        "Canonical workspace roots must be absolute");
      // Construct only the native project roots and display labels; never inherit production state.
      await bounded(fs.writeFile(path.join(desktop.home, ".codex-global-state.json"), JSON.stringify({
        "electron-saved-workspace-roots": roots,
        "active-workspace-roots": roots,
        "electron-workspace-root-labels": Object.fromEntries(
          desktop.references.map(target => [target.cwd, target.project])),
      })));
      desktop.seed = JSON.parse(python(catalogPython, {
        mode: "seed", home: desktop.home, sqlite: desktop.sqlite, targets,
      }));
      report.preparations ??= [];
      report.preparations.push({ instanceId, ...desktop.preparation, ...desktop.seed });
      await save();
    }
    const profile = { id: "fixture", name: "History Fixture", relayMode: "pureApi", protocol: "responses",
      model: "fixture-model", modelList: "fixture-model", baseUrl: base, upstreamBaseUrl: base,
      apiKey: "synthetic-history-only", authContents: '{"OPENAI_API_KEY":"synthetic-history-only"}',
      configContents: `model="fixture-model"\nmodel_provider="custom"\n[model_providers.custom]\nname="Fixture"\nwire_api="responses"\nbase_url="${base}"\nrequires_openai_auth=false\n` };
    await bounded(fs.writeFile(path.join(state, "settings.json"), JSON.stringify({
      localConfigProviderImported: true, providerSyncEnabled: false, sessionIndexAutoRepairEnabled: true,
      codexAppAutoTakeover: false, enhancementsEnabled: false, codexAppStandaloneQuotaUnlock: false,
      codexAppPath: appDir, relayProfilesEnabled: true, activeRelayId: "fixture", relayProfiles: [profile],
      relayCommonConfigContents: "analytics.enabled=false\n[features]\nplugins=false\napps=false\nremote_plugin=false\n",
    })));
    const debug = await freePort(), managerGuard = await freePort(), launcherGuard = await freePort();
    const proxy = await freePort();
    const env = { ...process.env, CODEX_PLUS_STATE_DIR: state, CODEX_HOME: desktops[0].home,
      CODEX_SQLITE_HOME: desktops[0].sqlite, CODEX_PLUS_MANAGER_GUARD_PORT: String(managerGuard),
      CODEX_PLUS_LAUNCHER_GUARD_PORT: String(launcherGuard), CODEX_PLUS_PROTOCOL_PROXY_PORT: String(proxy),
      WEBVIEW2_USER_DATA_FOLDER: path.join(state, "webview"),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${debug}`,
      CODEX_ELECTRON_USER_DATA_PATH: path.join(state, "unused-electron"),
      CODEX_PLUS_HELPER_BIND: "127.0.0.1", OTEL_SDK_DISABLED: "true" };
    for (const key of ["CODEX_PLUS_GUARD_PORT", "CODEX_PLUS_MULTI_INSTANCE", "CODEX_PLUS_INSTANCE_ID",
      "CODEX_PLUS_INSTANCE_LABEL", "OPENAI_API_KEY", "OPENAI_BASE_URL"]) delete env[key];
    manager = spawn(managerExe, [], { env, windowsHide: true, stdio: "ignore" });
    let managerSpawnError;
    manager.once("error", error => { managerSpawnError = error; });
    await until(async () => {
      if (managerSpawnError) throw managerSpawnError;
      assert.equal(manager.exitCode, null, "Manager exited");
      captureOwned();
      return fetch(`http://127.0.0.1:${debug}/json`, { signal: AbortSignal.timeout(1000) })
        .then(r => r.ok).catch(() => false);
    }, "Manager WebView unavailable");
    managerBrowser = await connectBrowser(debug);
    managerPage = managerBrowser.contexts().flatMap(c => c.pages())[0];
    await bounded(managerPage.waitForFunction(() => window.__TAURI_INTERNALS__?.invoke,
      undefined, { timeout: 10000 }));
    for (const desktop of desktops) await openDesktop(desktop);
    for (const desktop of desktops) for (const target of desktop.references) await verifyConversation(desktop, target);
    report.manual = [];
    for (let attempt = 0; attempt < 2; attempt++) {
      const result = await invoke("repair_session_index", {}, 240000);
      report.manual.push(result);
      assert.equal(result.status, "ok", result.message);
      assert.equal(result.catalogRecovery?.status, "synced", result.message);
      assert.ok(result.compressedFiles > 0);
      assert.equal(result.repairedItems, 0, "Repeated repair should not insert messages again");
      assert.equal(result.catalogRecovery.sqlite_catalog_rows_inserted, 0, "Catalog is not idempotent");
    }
    report.checks.manualSameHomeIdempotent = true;
    assert.ok(!interrupted, "Acceptance interrupted");
    report.checks.noModelCalls = report.requests.every(r => r.method === "GET");
    assert.ok(report.checks.noModelCalls, "Unexpected model submission");
  } catch (error) {
    report.error = String(error.stack || error);
    process.exitCode = 1;
    for (const desktop of desktops) if (desktop.page) {
      await bounded(desktop.page.screenshot({
        path: path.join(outputDir, `${token}-${desktop.instanceId}-failure.png`),
      })).catch(() => {});
    }
  } finally {
    try { await cleanup(); } catch (error) { report.cleanupErrors.push(String(error)); }
    await save();
    if (!report.cleanup || report.error || Object.values(report.checks).some(value => !value)) process.exitCode = 1;
    console.log(JSON.stringify({ report: path.join(outputDir, `${token}.json`),
      checks: report.checks, cleanup: report.cleanup, error: report.error }, null, 2));
  }
})();
