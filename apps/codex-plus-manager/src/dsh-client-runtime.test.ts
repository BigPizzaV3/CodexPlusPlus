import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { readFile } from "node:fs/promises";

// dsh 前端插件运行时（96-dsh-client.js）：用假的 React 与拓展 ctx 跑 codexPlusRunDshClient，
// 只验证服务桥接、依赖判定、ready 与配置传递，不渲染真组件。
async function runtime() {
  const source = await readFile(new URL("../../../assets/inject/renderer-inject/96-dsh-client.js", import.meta.url), "utf8");
  // 极简的 React：createElement 只记结构，renderTree 把函数组件 / 类组件递归展开成纯数据，测试读结果。
  const Fragment = Symbol("Fragment");
  const React = {
    Fragment,
    Component: class { props: any; constructor(props: any) { this.props = props; } },
    createElement: (type: unknown, props: Record<string, unknown> | null, ...children: unknown[]) => ({ type, props: { ...(props ?? {}), children } }),
    useSyncExternalStore: (_subscribe: unknown, get: () => unknown) => get(),
  };
  const libs = { react: React, reactDomClient: { createRoot: () => ({ render: (element: unknown) => renderTree(element), unmount() {} }) }, reactDom: {}, jsxRuntime: {} };
  const themeCalls: string[] = [];
  // 假 document：记下键盘监听，测试用 key() 派发按键事件。
  const documentListeners = new Map<string, Set<(event: any) => void>>();
  const fakeDocument = {
    documentElement: { classList: { contains: () => false } },
    addEventListener: (type: string, listener: (event: any) => void) => {
      if (!documentListeners.has(type)) documentListeners.set(type, new Set());
      documentListeners.get(type)!.add(listener);
    },
    removeEventListener: (type: string, listener: (event: any) => void) => documentListeners.get(type)?.delete(listener),
  };
  const key = (type: string, code: string, init: Record<string, unknown> = {}) => {
    const event = { type, code, key: code.replace(/^Key/, "").toLowerCase(), repeat: false, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, target: null, prevented: false, composedPath: () => [], preventDefault() { this.prevented = true; }, ...init };
    for (const listener of [...(documentListeners.get(type) ?? [])]) listener(event);
    return event;
  };
  type Runtime = {
    run: (ctx: unknown, modules: unknown[], options?: Record<string, unknown>) => Promise<void>;
    shortcutKey: (binding: unknown) => string;
    slotMap: Record<string, { kind: string; slot?: string }>;
    slotEntries: (name: string) => Array<{ order: number }>;
  };
  const run: Runtime = new Function(
    "codexAppAssetCandidateUrls", "codexPlusServices", "codexPlusCurrentSessionId", "codexPlusIsMacPlatform", "codexPlusThemesService", "document",
    `${source}\ncodexPlusReactModulesPromise = Promise.resolve(arguments[6]);\nreturn { run: codexPlusRunDshClient, shortcutKey: codexPlusDshShortcutKey, slotMap: codexPlusDshSlotMap, slotEntries: codexPlusDshEntriesOf };`,
  )(
    () => [],
    new Map(),
    () => "s-1",
    true,
    () => ({ current: null, apply: (id: string) => themeCalls.push(id) }),
    fakeDocument,
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
  return { ...run, ctx, calls, failures, themeCalls, key };
}

const modules = (plugin: unknown) => [{ id: "demo", factory: () => plugin }];

/** 把 createElement 记下的结构展开：函数组件调用、类组件 render、Fragment 摊平，最后只剩字符串与宿主元素。 */
function renderTree(node: any): any {
  if (node === null || node === undefined || typeof node === "string" || typeof node === "number") return node;
  if (Array.isArray(node)) return node.map(renderTree).flat().filter((item) => item !== null && item !== undefined);
  const { type, props } = node;
  if (typeof type === "function") {
    if (type.prototype?.render) {
      const instance = new type(props);
      instance.state = type.getDerivedStateFromError ? { failed: false } : {};
      return renderTree(instance.render());
    }
    return renderTree(type(props));
  }
  if (typeof type === "symbol") return renderTree(props.children);
  return { type, children: renderTree(props.children) };
}

describe("dsh 前端插件运行时", () => {
  it("可选依赖缺了照样加载，必需依赖缺了不加载；配置交给 apply，全部 apply 后发 ready", async () => {
    const k = await runtime();
    const seen: string[] = [];
    await k.run(k.ctx, [
      ...modules({ inject: { store: { required: false }, slots: true }, apply(ctx: any, config: any) {
        seen.push(`apply:${config.greeting}`);
        ctx.on("ready", () => seen.push("ready"));
        ctx.inject(["store"], () => seen.push("store 段不该激活"));
      } }),
      ...modules({ inject: ["store"], apply() { seen.push("必需依赖缺了不该加载"); } }),
    ], { packageName: "demo", config: { greeting: "hey" } });
    assert.deepEqual(seen, ["apply:hey", "ready"]);
    assert.equal(k.failures.length, 1, k.failures.join("\n"));
    assert.match(k.failures[0], /依赖 store/);
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

  it("ctx.remote 按宿主半边的 typert 描述调方法（命名参数）；stream 方法经 rpc.open 分批拉取并送上行", async () => {
    const k = await runtime();
    const backendCalls: Array<[string, any]> = [];
    const pulls: any[] = [{ items: [">a"], done: false }, { items: [">b"], done: true }];
    (k.ctx as any).backend = {
      async call(method: string, payload: any) {
        backendCalls.push([method, payload]);
        if (method === "rpc" && payload.endpoint === "$codexpp/typert-descriptors") {
          return { ok: true, value: [
            { namespace: "notes", method: "add", parameters: [{ name: "text", wire: "text" }] },
            { namespace: "notes", method: "watch", mode: "stream", uplink: true, parameters: [{ name: "prefix", wire: "prefix" }] },
          ] };
        }
        if (method === "rpc" && payload.endpoint === "notes/add") return { ok: true, value: `added ${payload.payload.args.text}` };
        if (method === "rpc") return { ok: false, error: { code: "gateway/unknown-endpoint", message: "nope", details: {} } };
        if (method === "rpcOpen") return { ok: true, id: "s1" };
        if (method === "rpcPull") return pulls.shift() ?? { items: [], done: true };
        return { ok: true };
      },
    };
    const seen: unknown[] = [];
    await k.run(k.ctx, modules({ inject: ["remote", "remote.notes"], async apply(ctx: any) {
      seen.push(await ctx.remote.notes.add("hi"));
      try {
        await ctx.remote.other.thing(1, 2);
      } catch (error: any) {
        seen.push(error.code);
      }
      const handle = ctx.remote.notes.watch(">");
      handle.send("a");
      handle.end();
      for await (const item of handle) seen.push(item);
    } }), { packageName: "demo" });
    assert.deepEqual(seen, ["added hi", "gateway/unknown-endpoint", ">a", ">b"], k.failures.join("\n"));
    const other = backendCalls.find(([method, payload]) => method === "rpc" && payload.endpoint === "other/thing");
    assert.deepEqual(other?.[1].payload, { positional: [1, 2] }, "没有描述的 namespace 按位置传参");
    const opened = backendCalls.find(([method]) => method === "rpcOpen");
    assert.deepEqual(opened?.[1], { channel: "/api", endpoint: "notes/watch", payload: { args: { prefix: ">" } } });
    await new Promise((resolve) => setTimeout(resolve, 0));
    assert.ok(backendCalls.some(([method, payload]) => method === "rpcPush" && payload.items[0] === "a"), "上行送上去了");
    assert.ok(!backendCalls.some(([method]) => method === "rpcClose"), "正常结束时宿主已清掉这个流，不必再关");
    assert.deepEqual(k.failures, []);
  });

  it("双键组合（secondCode）由运行时自己的键盘监听匹配；固定序列经 observeFixedInput 交给插件；目录与键帽同 dsh", async () => {
    const k = await runtime();
    const ran: string[] = [];
    let shortcuts: any;
    await k.run(k.ctx, modules({ inject: ["shortcuts"], apply(ctx: any) {
      shortcuts = ctx.shortcuts;
      ctx.shortcuts.register({
        id: "demo.chord",
        label: () => "Chord",
        defaults: { "web:macos": { code: "KeyK", secondCode: "KeyJ", modifiers: [] } },
        regions: ["page"],
        modals: [],
        resolve: () => ({ status: "handled", run: () => ran.push("chord") }),
      });
      ctx.shortcuts.registerFixed({ id: "demo.top", label: () => "Top", keys: ["g", "g"], bindings: [{ code: "KeyG", modifiers: [] }], group: "application" });
      // 插件自己认 `g g`：两次 G 之间没有别的键、没有被 reset 打断。
      let count = 0;
      ctx.shortcuts.observeFixedInput((input: any) => {
        if (input.type === "reset" || input.gesture.code !== "KeyG") {
          count = 0;
          return;
        }
        count += 1;
        if (count === 2) {
          input.consume();
          ran.push("gg");
          count = 0;
        }
      });
    } }), { packageName: "demo" });
    assert.equal(k.calls.filter(([kind]) => kind === "shortcut").length, 0, "双键组合不走 Codex++ 的单键快捷键");
    // 先按住 J 再按 K（顺序无关）。
    k.key("keydown", "KeyJ");
    const chord = k.key("keydown", "KeyK");
    k.key("keyup", "KeyK");
    k.key("keyup", "KeyJ");
    assert.equal(chord.prevented, true);
    // 只按 K 不触发。
    k.key("keydown", "KeyK");
    k.key("keyup", "KeyK");
    // g g：中间被点一下（reset）就作废，重来一次才算。
    k.key("keydown", "KeyG"); k.key("keyup", "KeyG");
    k.key("pointerdown", "");
    k.key("keydown", "KeyG"); k.key("keyup", "KeyG");
    const second = k.key("keydown", "KeyG"); k.key("keyup", "KeyG");
    assert.equal(second.prevented, true);
    assert.deepEqual(ran, ["chord", "gg"]);
    assert.deepEqual(shortcuts.catalog.getSnapshot().map((row: any) => [row.id, row.keys.join("")]), [["demo.chord", "JK"]]);
    assert.deepEqual(shortcuts.fixedCatalog.getSnapshot().map((row: any) => row.id), ["demo.top"]);
    assert.deepEqual(shortcuts.describeBinding({ code: "KeyJ", secondCode: "KeyK", modifiers: ["primary"] }).keys, ["⌘", "J", "K"]);
    assert.equal(shortcuts.stopSequenceMs, 500);
    assert.deepEqual(k.failures, []);
  });

  it("新映射的插槽落到 Codex++ 的位置；双键组合不交给 Codex++ 的单键快捷键", async () => {
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

  it("对不上 Codex++ 位置的插槽作为子插槽：父组件 renderSlot / renderSlotChain 渲染，factory 经 renderFactorySlot", async () => {
    const k = await runtime();
    let parentProps: any;
    await k.run(k.ctx, modules({ inject: ["slots"], apply(ctx: any) {
      ctx.slots.register({ name: "conversation.approval.detail" }, (props: any) => { parentProps = props; return null; });
      ctx.slots.register({ name: "demo.child", order: 2 }, (props: any) => `second:${props.label}`);
      ctx.slots.register({ name: "demo.child", order: 1 }, (props: any) => `first:${props.label}`);
      ctx.slots.register({ name: "demo.keyed", key: "bash" }, () => "bash-view");
      ctx.slots.register({ name: "demo.keyed", key: "web" }, () => "web-view");
      ctx.slots.register({ name: "demo.chain", select: (owner: any) => (owner.kind === "quota" ? { left: 3 } : null) }, (props: any) => `quota:${props.matched.left}`);
      ctx.slots.registerFactory({ name: "demo.card", scope: "root", slots: { footer: {} } }, (props: any) => {
        const Footer = props.useFactorySlot("footer", () => "default-footer");
        return [`card:${props.title}`, Footer({})];
      });
    } }), { packageName: "demo" });
    k.calls.find(([kind]) => kind === "slot")![1].render({ container: {} });
    assert.ok(parentProps, "映射到 approval 的父组件拿到了 props");
    assert.deepEqual(renderTree(parentProps.renderSlot("demo.child", { label: "x" })), ["first:x", "second:x"], "按 order 渲染全部条目");
    assert.deepEqual(renderTree(parentProps.renderSlot("demo.keyed", {}, { entryKey: "web" })), ["web-view"], "keyed 按 entryKey 分派");
    assert.equal(renderTree(parentProps.renderSlot("demo.none", {}, { fallback: "empty" })), "empty");
    assert.deepEqual(renderTree(parentProps.renderSlotChain("demo.chain", { kind: "quota" })), ["quota:3"], "chain 取第一个 select 命中的");
    assert.equal(renderTree(parentProps.renderSlotChain("demo.chain", { kind: "other" }, { fallback: null })), null);
    assert.deepEqual(renderTree(parentProps.renderFactorySlot("demo.card", { title: "T" })), ["card:T", "default-footer"]);
    assert.deepEqual(renderTree(parentProps.renderFactorySlot("demo.card", { title: "T" }, { slots: { footer: () => "custom-footer" } })), ["card:T", "custom-footer"]);
    assert.deepEqual(k.slotEntries("demo.child").map((entry: any) => entry.order), [1, 2]);
    assert.deepEqual(k.failures, [], "子插槽有人渲染就不算失败");
  });
});
