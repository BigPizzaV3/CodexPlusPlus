import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { readFile } from "node:fs/promises";

// dsh 前端插件运行时（96-dsh-client.js）：用假的 React 与拓展 ctx 跑 codexPlusRunDshClient，
// 只验证服务桥接、依赖判定、ready 与配置传递，不渲染真组件。
async function runtime() {
  const source = await readFile(new URL("../../../assets/inject/renderer-inject/96-dsh-client.js", import.meta.url), "utf8");
  const React = {
    Component: class {},
    createElement: () => null,
    useSyncExternalStore: (_subscribe: unknown, get: () => unknown) => get(),
  };
  const libs = { react: React, reactDomClient: { createRoot: () => ({ render() {}, unmount() {} }) }, reactDom: {}, jsxRuntime: {} };
  const themeCalls: string[] = [];
  type Runtime = {
    run: (ctx: unknown, modules: unknown[], options?: Record<string, unknown>) => Promise<void>;
    shortcutKey: (binding: unknown) => string;
    slotMap: Record<string, { kind: string; slot?: string }>;
  };
  const run: Runtime = new Function(
    "codexAppAssetCandidateUrls", "codexPlusServices", "codexPlusCurrentSessionId", "codexPlusIsMacPlatform", "codexPlusThemesService", "document",
    `${source}\ncodexPlusReactModulesPromise = Promise.resolve(arguments[6]);\nreturn { run: codexPlusRunDshClient, shortcutKey: codexPlusDshShortcutKey, slotMap: codexPlusDshSlotMap };`,
  )(
    () => [],
    new Map(),
    () => "s-1",
    true,
    () => ({ current: null, apply: (id: string) => themeCalls.push(id) }),
    { documentElement: { classList: { contains: () => false } } },
    libs,
  );
  const calls: Array<[string, any]> = [];
  const failures: string[] = [];
  const disposers: Array<() => void> = [];
  const ctx = {
    id: "dsh-demo",
    disposed: false,
    locale: { current: "zh-CN", register: () => () => {}, bind: () => (key: string) => key, resolve: (text: string) => text },
    fail: (error: Error) => failures.push(error.message),
    effect: (fn: () => () => void) => { const dispose = fn(); disposers.push(dispose); return dispose; },
    on: () => () => {},
    emit() {},
    setTimeout: () => () => {},
    setInterval: () => () => {},
    ask: async (question: any) => ({ value: question.options[1].value, text: null }),
    registerCommand: (definition: any) => { calls.push(["command", definition]); return () => {}; },
    registerShortcut: (definition: any) => { calls.push(["shortcut", definition]); return () => {}; },
    registerTheme: (definition: any) => { calls.push(["theme", definition]); return () => {}; },
    registerSlot: (definition: any) => { calls.push(["slot", definition]); return () => {}; },
    registerSidePanel: (definition: any) => { calls.push(["sidePanel", definition]); return () => {}; },
    registerToolView: (definition: any) => { calls.push(["toolView", definition]); return () => {}; },
  };
  return { ...run, ctx, calls, failures, themeCalls };
}

const modules = (plugin: unknown) => [{ id: "demo", factory: () => plugin }];

describe("dsh 前端插件运行时", () => {
  it("可选依赖缺了照样加载，必需依赖缺了不加载；配置交给 apply，全部 apply 后发 ready", async () => {
    const k = await runtime();
    const seen: string[] = [];
    await k.run(k.ctx, [
      ...modules({ inject: { remote: { required: false }, slots: true }, apply(ctx: any, config: any) {
        seen.push(`apply:${config.greeting}`);
        ctx.on("ready", () => seen.push("ready"));
        ctx.inject(["remote"], () => seen.push("remote 段不该激活"));
      } }),
      ...modules({ inject: ["remote"], apply() { seen.push("必需依赖缺了不该加载"); } }),
    ], { packageName: "demo", config: { greeting: "hey" } });
    assert.deepEqual(seen, ["apply:hey", "ready"]);
    assert.equal(k.failures.length, 1, k.failures.join("\n"));
    assert.match(k.failures[0], /依赖 remote/);
  });

  it("shortcuts / commandUi / theme 桥接到 Codex++ 的快捷键、斜杠命令与主题；inputTriggers 只记说明", async () => {
    const k = await runtime();
    const ran: string[] = [];
    await k.run(k.ctx, modules({ inject: ["shortcuts", "commandUi", "theme", "inputTriggers"], apply(ctx: any) {
      ctx.shortcuts.register({
        id: "demo.toggle",
        label: () => "Toggle",
        defaults: { "web:mac": { code: "KeyK", modifiers: ["primary", "shift"] } },
        resolve: () => ({ status: "handled", run: () => ran.push("shortcut") }),
      });
      ctx.commandUi.register({ name: "hello", available: () => true, ui: { kind: "action", run: ({ sessionId }: any) => ran.push(`action:${sessionId}`) } });
      ctx.commandUi.register({ name: "pick", available: () => true, ui: {
        kind: "popupSelect",
        options: async () => [{ id: "a", label: "A" }, { id: "b", label: "B" }],
        onSelect: (option: any) => ran.push(`picked:${option.id}`),
      } });
      ctx.theme.register({ id: "night", colorScheme: "dark", tokens: { "bg-primary": "#000" } });
      ctx.theme.setTheme("night");
      ctx.inputTriggers.register({ trigger: "@", name: "files" });
    } }), { packageName: "demo" });
    const shortcut = k.calls.find(([kind]) => kind === "shortcut")![1];
    assert.equal(shortcut.key, "Mod+Shift+K");
    shortcut.run({ event: { target: null } });
    const commands = k.calls.filter(([kind]) => kind === "command").map(([, definition]) => definition);
    assert.deepEqual(commands.map((command) => command.name), ["hello", "pick"]);
    await commands[0].run("", { toast() {} });
    await commands[1].run("", { toast() {} });
    assert.deepEqual(ran, ["shortcut", "action:s-1", "picked:b"]);
    assert.deepEqual(k.calls.find(([kind]) => kind === "theme")![1].variables, { "--bg-primary": "#000" });
    assert.deepEqual(k.themeCalls, ["night"]);
    assert.equal(k.failures.length, 1);
    assert.match(k.failures[0], /输入触发源 @files/);
  });

  it("新映射的插槽落到 Codex++ 的位置；双键序列的快捷键认不出", async () => {
    const k = await runtime();
    for (const [name, slot] of [
      ["conversation.approval.detail", "approval"],
      ["conversation.input.activity", "activity.after"],
      ["conversation.hero.workspace", "home.above"],
      ["conversation.session.header.lineage", "header.title"],
    ]) assert.equal(k.slotMap[name]?.slot, slot, name);
    await k.run(k.ctx, modules({ inject: ["slots"], apply(ctx: any) {
      ctx.slots.register({ name: "rightbar.session", label: "Panel" }, () => null);
      ctx.slots.register({ name: "tool.view.cordis", key: "cordis_inspect" }, () => null);
      ctx.slots.register({ name: "conversation.approval.detail" }, () => null);
    } }), { packageName: "demo" });
    assert.deepEqual(k.calls.map(([kind]) => kind), ["sidePanel", "toolView", "slot"]);
    assert.deepEqual(k.calls[1][1].tools, ["cordis_inspect"]);
    assert.equal(k.calls[2][1].slot, "approval");
    assert.equal(k.shortcutKey({ code: "KeyG", secondCode: "KeyH", modifiers: ["primary"] }), "");
    assert.equal(k.shortcutKey({ code: "Digit1", modifiers: ["alt"] }), "Alt+1");
    assert.deepEqual(k.failures, []);
  });
});
