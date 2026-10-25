import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, readFileSync, rmdirSync, unlinkSync, writeFileSync } from "node:fs";
import { createRequire, Module } from "node:module";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import vm from "node:vm";

const require = createRequire(import.meta.url);
const { beginTimeoutOverrides } = require("../../../assets/claude-localization/bash-timeout.js");
const hook = readFileSync(new URL("../../../assets/claude-localization/hook.js", import.meta.url), "utf8");
const settings = { enabled: true, defaultEnabled: true, defaultSeconds: 120, maxEnabled: true, maxSeconds: 1800 };

test("未知 Cowork 源码只返回可选适配失败，不留下全局改写或模块加载钩子", async () => {
  const root = mkdtempSync(path.join(os.tmpdir(), "cpx-bash-isolation-"));
  const archive = path.join(root, "app.asar");
  const vite = path.join(archive, ".vite");
  const build = path.join(vite, "build");
  mkdirSync(build, { recursive: true });
  const file = path.join(build, "workspace.js");
  writeFileSync(file, "// Run a shell command in the session's isolated Linux workspace.\nunsupported();");
  const extensions = (Module as unknown as { _extensions: Record<string, unknown> })._extensions;
  const original = extensions[".js"];
  try {
    const result = await beginTimeoutOverrides(settings, root);
    assert.equal(result.prepared, false);
    assert.match(result.error, /参数校验结构/);
    assert.equal(extensions[".js"], original);
    assert.equal((globalThis as any).__codexPlusBashTimeout, undefined);
    assert.equal((globalThis as any).__claudeCnBashTimeoutResult, result);
    assert.equal(await (globalThis as any).__claudeCnBashTimeoutReady, result);
  } finally {
    delete (globalThis as any).__claudeCnBashTimeoutResult;
    delete (globalThis as any).__claudeCnBashTimeoutReady;
    unlinkSync(file);
    for (const directory of [build, vite, archive, root]) rmdirSync(directory);
  }
});

for (const failure of ["unsupported", "missing-helper", "prestarted-rejection"]) {
  test(`真实 hook 在 ${failure} 时仍注册汉化、语音与后续窗口监听`, async () => {
    const commands: { name: string; args: any }[] = [];
    const listeners: string[] = [];
    let permissionHandler = false;
    const session = {
      setPermissionRequestHandler: () => { permissionHandler = true; },
      setPermissionCheckHandler: () => {},
    };
    const wc = {
      session, isDestroyed: () => false, getURL: () => "app://localhost/index.html",
      on: (name: string) => listeners.push(name),
      debugger: {
        isAttached: () => true,
        on: (name: string) => listeners.push(name),
        sendCommand: (name: string, args: any) => { commands.push({ name, args }); return Promise.resolve({}); },
      },
    };
    const electron = {
      app: { on: (name: string) => listeners.push(name), whenReady: () => Promise.resolve(), getVersion: () => "fixture" },
      session: { defaultSession: session }, webContents: { getAllWebContents: () => [wc] },
    };
    const fakeFs = {
      existsSync: () => false, appendFileSync: () => {},
      readFileSync: (file: string) => file.endsWith("page-shim.js") ? "var LOCALE = 'zh-Hans'; /* locale fixture */" : "/* voice fixture */",
    };
    const context = vm.createContext({
      __dirname: "/fixture", process: { platform: "win32", pid: 123, env: {} }, URL, Buffer,
      __claudeCnBashTimeout: settings, setTimeout: () => 0,
      require: (name: string) => {
        if (name === "electron") return electron;
        if (name === "node:fs") return fakeFs;
        if (name === "./bash-timeout.js") {
          if (failure === "missing-helper") throw new Error("helper missing");
          return { beginTimeoutOverrides: () => Promise.resolve({ enabled: true, prepared: false, error: "unsupported workspace" }) };
        }
        return require(name);
      },
    });
    if (failure === "prestarted-rejection") vm.runInContext("globalThis.__claudeCnBashTimeoutReady = Promise.reject(new Error('unknown workspace'));", context);
    vm.runInContext(hook, context);
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(context.__claudeCnHookInstalled, true);
    assert.ok(listeners.includes("web-contents-created"));
    assert.ok(listeners.includes("did-start-navigation"));
    assert.ok(commands.some((command) => command.name === "Runtime.addBinding"));
    assert.ok(commands.some((command) => command.name === "Page.addScriptToEvaluateOnNewDocument" && command.args.source.includes("locale fixture") && command.args.source.includes("voice fixture")));
    assert.equal(permissionHandler, true);
    assert.equal(context.__claudeCnInjectionReady, undefined, "可选适配不能引入聚合启动信号");
  });
}
