import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";

const source = readFileSync(new URL("../../../assets/inject/renderer-inject/32-fork-readiness-fallback.js", import.meta.url), "utf8");
// 真实注入时整份脚本包在一个 IIFE 里，重新注入会得到新的作用域。
const inject = `(() => {\n${source}\nreturn installCodexForkReadinessFallback();\n})()`;

// 仿照 Codex 26.1027 的原生实现：getForkCapabilities 把 this.settings.read 绑定后交给 readPlacement。
function nativeModule() {
  class AppServerManager {
    hostId = "local";
    disposed = false;
    #secret = "private";
    runtime: any;
    settings: any;
    constructor(runtime: any, settings: any) {
      this.runtime = runtime;
      this.settings = settings;
    }
    secret() { return this.#secret; }
    getForkCapabilities() {
      const readSetting = this.settings.read.bind(this.settings);
      const isActive = () => !this.disposed;
      return {
        hostId: this.hostId,
        isActive,
        readPlacement() {
          const sections = readSetting("sidebarCustomSectionsEnabled");
          const appServer = readSetting("useAppServerCustomSections");
          if (sections === undefined || appServer === undefined) return { status: "not-ready", reason: "execution-config-loading" };
          return { status: "ready", placement: { sections, appServer } };
        },
      };
    }
  }
  return { rYn: AppServerManager, other: { getForkCapabilities() {} } };
}

class Settings {
  #gates: Record<string, boolean | undefined>;
  constructor(gates: Record<string, boolean | undefined>) { this.#gates = gates; }
  read(key: string) { return this.#gates[key]; }
}

async function install(enhancementsEnabled = true) {
  const module = nativeModule();
  const diagnostics: unknown[][] = [];
  const window: any = { setTimeout: () => 0 };
  const context = vm.createContext({
    window,
    codexPlusBackendSettings: { enhancementsEnabled },
    loadOptionalCodexAppModule: async (name: string) => (name === "app-initial-" ? module : null),
    sendCodexPlusDiagnostic: (...args: unknown[]) => diagnostics.push(args),
  });
  const patched = await vm.runInContext(inject, context);
  return { Manager: module.rYn, patched, diagnostics, context };
}

test("无账号且 Statsig 门控读不到时，Fork 放置按缺省 false 继续，不再停在 not-ready", async () => {
  const { Manager, patched, diagnostics } = await install();
  assert.equal(patched, true);
  const manager = new Manager({ accountId: () => null }, new Settings({ sidebarCustomSectionsEnabled: false, useAppServerCustomSections: undefined }));
  const capabilities = manager.getForkCapabilities();
  assert.deepEqual(JSON.parse(JSON.stringify(capabilities.readPlacement())), { status: "ready", placement: { sections: false, appServer: false } });
  // 其他字段照常从原实例读取，实例本身不被改动。
  assert.equal(capabilities.hostId, "local");
  assert.equal(capabilities.isActive(), true);
  manager.disposed = true;
  assert.equal(capabilities.isActive(), false);
  assert.equal(manager.secret(), "private");
  assert.equal(diagnostics.length, 1);
});

test("登录账号时保持原生行为：门控没就绪就照常返回 not-ready", async () => {
  const { Manager } = await install();
  const manager = new Manager({ accountId: () => "account-1" }, new Settings({ sidebarCustomSectionsEnabled: true, useAppServerCustomSections: undefined }));
  assert.equal(manager.getForkCapabilities().readPlacement().status, "not-ready");
});

test("门控有值时原样使用，增强总开关关闭时完全不介入", async () => {
  const ready = await install();
  const manager = new ready.Manager({ accountId: () => null }, new Settings({ sidebarCustomSectionsEnabled: true, useAppServerCustomSections: true }));
  assert.deepEqual(JSON.parse(JSON.stringify(manager.getForkCapabilities().readPlacement())), { status: "ready", placement: { sections: true, appServer: true } });
  assert.equal(ready.diagnostics.length, 0);

  const off = await install(false);
  const plain = new off.Manager({ accountId: () => null }, new Settings({ sidebarCustomSectionsEnabled: false, useAppServerCustomSections: undefined }));
  assert.equal(plain.getForkCapabilities().readPlacement().status, "not-ready");
});

test("重复注入不会叠加包装", async () => {
  const { Manager, context } = await install();
  const first: any = Manager.prototype.getForkCapabilities;
  vm.runInContext(`window.__codexPlusForkReadinessState.patched = false;`, context);
  await vm.runInContext(inject, context);
  assert.equal(Manager.prototype.getForkCapabilities, first);
  assert.equal(typeof first.__codexPlusOriginal, "function");
  assert.equal(first.__codexPlusOriginal.__codexPlusForkReadiness, undefined);
});
