import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import { keepUserScriptRuntime } from "./extension-manager-pages.ts";

// 本地拓展列表的运行状态（已加载 / 加载失败）只有两个命令的返回值里有：refresh_user_script_inventory 与 reload_user_scripts。
// 其余命令返回的清单只扫了磁盘。这里把 App 里真实的那几段代码取出来执行，守住「不带状态的返回值不冲掉运行状态、
// 带状态的刷新结果不被合并挡住」。

const text = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
const app = ts.createSourceFile("App.tsx", text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);

/** App 组件里某个 `const name = …` 的源码。 */
function declaration(name: string): string {
  let result: ts.VariableDeclaration | undefined;
  const visit = (node: ts.Node) => {
    if (!result && ts.isVariableDeclaration(node) && node.name.getText(app) === name) result = node;
    ts.forEachChild(node, visit);
  };
  visit(app);
  assert.ok(result, name);
  return `const ${result.getText(app)};`;
}

const transpile = (source: string) => ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;

type Script = { key: string; enabled: boolean; status: string; error: string; manifestError: string };
type Result = { status: string; message: string; settings: { revision: string }; user_scripts: { enabled: boolean; skip_next_start?: boolean; scripts: Script[] } };

const script = (key: string, status: string, error = ""): Script => ({ key, enabled: status !== "disabled", status, error, manifestError: "" });
const result = (scripts: Script[], extra: Partial<Result["user_scripts"]> = {}): Result => ({
  status: "ok", message: "", settings: { revision: "1" }, user_scripts: { enabled: true, scripts, ...extra },
});

/** 把 App 里的 setSettings（以及指定的几个动作）放进 vm：settings 状态、后端返回值都由测试控制。 */
function harness(actionNames: string[] = []) {
  const state: { settings: Result | null } = { settings: null };
  const responses = new Map<string, unknown>();
  const requests: string[] = [];
  const context = vm.createContext({
    useCallback: (callback: unknown) => callback,
    keepUserScriptRuntime,
    setSettingsWithRuntime: (next: Result | null | ((current: Result | null) => Result | null)) => {
      state.settings = typeof next === "function" ? next(state.settings) : next;
    },
    run: async (operation: () => Promise<unknown>) => operation(),
    call: async (command: string) => {
      requests.push(command);
      if (!responses.has(command)) throw new Error(`没有准备 ${command} 的返回值`);
      return responses.get(command);
    },
    t: (value: string) => value,
    isSuccessStatus: (status: string) => status === "ok",
    setScriptMarket: () => {},
    syncMarketInstalledState: (current: unknown) => current,
    showResultNotice: () => {},
  });
  vm.runInContext(transpile([
    declaration("setSettings"),
    ...actionNames.map(declaration),
    `Object.assign(globalThis, { ${["setSettings", ...actionNames].join(", ")} });`,
  ].join("\n")), context);
  return { state, responses, requests, actions: context as unknown as Record<string, (...args: unknown[]) => Promise<unknown>> };
}

const statuses = (settings: Result | null) => Object.fromEntries((settings?.user_scripts.scripts ?? []).map((item) => [item.key, item.status]));

test("默认的 setSettings 沿用上一份清单的运行状态，其余内容用新的", () => {
  const h = harness();
  const first = result([script("user:a", "not_loaded")]);
  h.actions.setSettings(first);
  assert.equal(h.state.settings, first, "没有上一份时原样放进去");
  // 带运行状态的那一份由 setSettingsWithRuntime 放进来（见下一个用例），这里直接摆好。
  h.state.settings = result([script("user:a", "loaded"), script("user:b", "failed", "TypeError: boom")]);
  const saved = { ...result([script("user:a", "not_loaded"), script("user:b", "not_loaded")]), settings: { revision: "2" } };
  h.actions.setSettings(saved);
  assert.deepEqual(statuses(h.state.settings), { "user:a": "loaded", "user:b": "failed" });
  assert.equal(h.state.settings?.user_scripts.scripts[1].error, "TypeError: boom");
  assert.equal(h.state.settings?.settings.revision, "2", "设置本身用新的");
  assert.equal(saved.user_scripts.scripts[0].status, "not_loaded", "不改后端返回的对象");
  // 函数式更新（刷新市场、安装市场拓展用的写法）同样经过合并。
  h.actions.setSettings((current: Result | null) => (current ? { ...current, user_scripts: result([script("user:a", "not_loaded"), script("user:c", "not_loaded")]).user_scripts } : current));
  assert.deepEqual(statuses(h.state.settings), { "user:a": "loaded", "user:c": "not_loaded" });
  // 只改设置、清单还是同一个对象时不重新合并。
  const before = h.state.settings!;
  h.actions.setSettings((current: Result | null) => (current ? { ...current, settings: { revision: "3" } } : current));
  assert.equal(h.state.settings?.user_scripts, before.user_scripts);
  h.actions.setSettings(null);
  assert.equal(h.state.settings, null);
});

test("切「下次启动跳过拓展」、保存拓展配置且原地生效：列表里的已加载不变成未加载", async () => {
  const h = harness(["setUserScriptsSkipNextStart", "setExtensionConfig"]);
  h.state.settings = result([script("user:a", "loaded"), script("user:b", "loaded"), script("user:off", "disabled")]);
  const bare = [script("user:a", "not_loaded"), script("user:b", "not_loaded"), script("user:off", "disabled")];
  h.responses.set("set_user_scripts_skip_next_start", result(bare, { skip_next_start: true }));
  await h.actions.setUserScriptsSkipNextStart(true);
  assert.deepEqual(statuses(h.state.settings), { "user:a": "loaded", "user:b": "loaded", "user:off": "disabled" });
  assert.equal(h.state.settings?.user_scripts.skip_next_start, true, "开关本身照新清单");
  h.responses.set("set_extension_config", result(bare));
  h.responses.set("update_extension_config_live", "applied");
  assert.equal(await h.actions.setExtensionConfig("user:a", { color: "red" }, { merge: true }), true);
  assert.deepEqual(h.requests, ["set_user_scripts_skip_next_start", "set_extension_config", "update_extension_config_live"], "原地生效时不热重载");
  assert.deepEqual(statuses(h.state.settings), { "user:a": "loaded", "user:b": "loaded", "user:off": "disabled" });
});

test("带运行状态的刷新与热重载整份替换，不被合并挡住", async () => {
  const h = harness(["refreshUserScriptInventory", "reloadUserScripts"]);
  h.state.settings = result([script("user:a", "loaded"), script("user:b", "failed", "TypeError: boom")]);
  // Codex 已经退出：刷新得到的「未加载」是实打实的状态，必须显示出来。
  const stopped = result([script("user:a", "not_loaded"), script("user:b", "not_loaded")]);
  h.responses.set("refresh_user_script_inventory", stopped);
  assert.equal(await h.actions.refreshUserScriptInventory(), stopped);
  assert.equal(h.state.settings, stopped);
  h.state.settings = result([script("user:a", "loaded"), script("user:b", "failed", "TypeError: boom")]);
  // 热重载后出错的拓展恢复了、另一个这次没加载上。
  const reloaded = result([script("user:a", "not_loaded"), script("user:b", "loaded")]);
  h.responses.set("reload_user_scripts", reloaded);
  assert.equal(await h.actions.reloadUserScripts(true), "ok");
  assert.equal(h.state.settings, reloaded);
  // 进「万物皆拓展」页时两个请求都回来后再放一次带状态的清单，也是整份替换。
  assert.match(text, /if \(inventory\) \{\n\s*setSettingsWithRuntime\(inventory\);/);
  // 整份替换只用在这三处：别的命令的返回值都不带运行状态，走默认的 setSettings。
  assert.equal(text.match(/setSettingsWithRuntime\((?:result|inventory)\);/g)?.length, 3);
});
