import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import * as focusModel from "./agent-focus-model.ts";
import * as gatewayModel from "./gateway-model.ts";
import * as setupModel from "./onboarding-setup-model.ts";

const source = ts.transpileModule(readFileSync(new URL("./OnboardingSetup.tsx", import.meta.url), "utf8"), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX, target: ts.ScriptTarget.ES2022 },
}).outputText;

type Element = { type: string; props: Record<string, any> };
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (cause: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const flush = () => new Promise<void>((resolve) => setImmediate(resolve));

function fixture(): gatewayModel.GatewayPayload & { status: string; message: string } {
  return {
    status: "ok", message: "",
    gateway: {
      version: 1, keyPolicy: { mode: "sticky" }, restoreOnExit: false,
      bindings: {
        codex: { ...gatewayModel.blankBinding(), enabled: true, model: "chat" },
        claude: { ...gatewayModel.blankBinding(), enabled: true, model: "chat" },
      },
    },
    port: 57321, helperRunning: true,
    pool: [{ id: "test", name: "Test", baseUrl: "https://test.invalid", protocol: "responses", official: false, models: ["chat", "other"], keyCount: 0 }],
    groups: [],
    agents: [
      { agent: "codex", name: "Codex", enabled: true, baseUrl: "http://127.0.0.1:57321", defaultModel: "chat", models: ["chat"] },
      { agent: "claude", name: "Claude Code", enabled: true, baseUrl: "http://127.0.0.1:57321", defaultModel: "chat", models: ["chat"] },
    ],
    health: { keys: [], providers: [] },
    codexProviders: [], claudeProviders: [], grokProfiles: [],
  };
}

/** 保留真实组件与状态模型，只替换 React 调度、桌面命令和子弹窗边界。 */
function harness(focus: focusModel.AgentFocus = "codex") {
  const hooks: any[] = [];
  const cleanups = new Map<number, () => void>();
  const effects: Array<() => void> = [];
  const timeouts = new Map<number, () => void>();
  const intervals = new Map<number, () => void>();
  const subscribers = new Set<(payload: gatewayModel.GatewayPayload) => void>();
  const requests: Array<{ command: string; result: ReturnType<typeof deferred<any>> }> = [];
  const checks: Array<ReturnType<typeof deferred<boolean>>> = [];
  const launches: Array<ReturnType<typeof deferred<boolean>>> = [];
  const notices: Array<{ title: string; message: string; status?: string }> = [];
  const dismissed: boolean[] = [];
  const navigation: Array<{ target: string; agent?: string }> = [];
  let cursor = 0;
  let changed = false;
  let timerId = 0;
  let generation = 0;
  const sameDeps = (previous: unknown[] | undefined, deps: unknown[]) =>
    previous?.length === deps.length && previous.every((value, index) => Object.is(value, deps[index]));
  const react = {
    useState: (initialValue: unknown) => {
      const index = cursor++;
      if (!(index in hooks)) hooks[index] = typeof initialValue === "function" ? (initialValue as () => unknown)() : initialValue;
      return [hooks[index], (value: unknown) => {
        const next = typeof value === "function" ? (value as (current: unknown) => unknown)(hooks[index]) : value;
        if (!Object.is(next, hooks[index])) { hooks[index] = next; changed = true; }
      }];
    },
    useRef: (initialValue: unknown) => {
      const index = cursor++;
      if (!(index in hooks)) hooks[index] = { current: initialValue };
      return hooks[index];
    },
    useCallback: (callback: (...args: any[]) => any, deps: unknown[]) => {
      const index = cursor++;
      if (!sameDeps(hooks[index]?.deps, deps)) hooks[index] = { deps: deps.slice(), callback };
      return hooks[index].callback;
    },
    useEffect: (effect: () => void | (() => void), deps: unknown[]) => {
      const index = cursor++;
      if (sameDeps(hooks[index]?.deps, deps)) return;
      hooks[index] = { deps: deps.slice() };
      effects.push(() => {
        cleanups.get(index)?.();
        cleanups.delete(index);
        const cleanup = effect();
        if (typeof cleanup === "function") cleanups.set(index, cleanup);
      });
    },
  };
  const publish = (payload: gatewayModel.GatewayPayload) => {
    generation += 1;
    for (const subscriber of subscribers) subscriber(payload);
  };
  const jsx = (type: string, props: Record<string, any>) => ({ type, props });
  const modules: Record<string, unknown> = {
    react,
    "react/jsx-runtime": { jsx, jsxs: jsx, Fragment: "Fragment" },
    "@tauri-apps/api/core": { invoke: (command: string) => {
      const result = deferred<any>();
      requests.push({ command, result });
      return result.promise;
    } },
    "lucide-react": Object.fromEntries(["ArrowRight", "Check", "ChevronDown", "ChevronUp", "CircleHelp", "Loader2", "Play", "Plus", "RefreshCw", "Settings2", "X", "Zap"].map((name) => [name, name])),
    "@/components/ui/button": { Button: "Button" },
    "./AppSelect": { AppSelect: "AppSelect" },
    "./HubOverview": { AgentGlyph: "AgentGlyph" },
    "./GatewayAgents": { AgentConnectDialog: "AgentConnectDialog" },
    "./GatewayProviders": { ProviderAdder: "ProviderAdder" },
    "./ProviderTestDialog": { ProviderTestDialog: "ProviderTestDialog" },
    "./gateway-model": gatewayModel,
    "./gateway-store": {
      gatewayGeneration: () => generation,
      publishGatewayPayload: publish,
      subscribeGatewayPayload: (subscriber: (payload: gatewayModel.GatewayPayload) => void) => {
        subscribers.add(subscriber);
        return () => subscribers.delete(subscriber);
      },
    },
    "./gateway-ui": { makeRun: () => () => assert.fail("当前测试不保存供应商或绑定") },
    "./agent-focus-model": focusModel,
    "./onboarding-setup-model": setupModel,
    "./i18n": { t: (text: string) => text, tf: (text: string, args: unknown[]) => text.replace(/\{(\d+)\}/g, (_, index) => String(args[Number(index)])) },
    "./OnboardingSetup.css": {},
  };
  const exports: Record<string, any> = {};
  vm.runInNewContext(source, {
    exports,
    document: { visibilityState: "visible" },
    window: {
      setTimeout: (callback: () => void) => { const id = ++timerId; timeouts.set(id, callback); return id; },
      clearTimeout: (id: number) => timeouts.delete(id),
      setInterval: (callback: () => void) => { const id = ++timerId; intervals.set(id, callback); return id; },
      clearInterval: (id: number) => intervals.delete(id),
    },
    require: (name: string) => {
      assert.ok(name in modules, `unexpected import ${name}`);
      return modules[name];
    },
  });
  const props = {
    focus, codexInstalled: true as boolean | null, codexRunning: false, launchPending: false,
    codexEnhancementsEnabled: true,
    notify: async (title: string, message: string, status?: string) => { notices.push({ title, message, status }); },
    onChangeMode: () => {},
    onDismiss: (completed: boolean) => { dismissed.push(completed); },
    onTour: () => {},
    onNavigate: (target: string, agent?: string) => { navigation.push({ target, agent }); },
    onCheckCodex: () => { const request = deferred<boolean>(); checks.push(request); return request.promise; },
    onLaunch: () => { const request = deferred<boolean>(); launches.push(request); return request.promise; },
  };
  return {
    requests, checks, launches, notices, dismissed, navigation, props, publish,
    render: () => {
      for (let attempt = 0; attempt < 20; attempt++) {
        cursor = 0;
        changed = false;
        const tree = exports.OnboardingSetup(props) as Element;
        for (const effect of effects.splice(0)) effect();
        if (!changed) return tree;
      }
      throw new Error("组件未在 20 次渲染内稳定");
    },
    poll: () => { for (const run of intervals.values()) run(); },
    expireLoad: () => {
      const item = timeouts.entries().next().value;
      assert.ok(item, "读取配置应有超时");
      timeouts.delete(item[0]);
      item[1]();
    },
    unmount: () => { for (const cleanup of cleanups.values()) cleanup(); cleanups.clear(); },
    timerCount: () => timeouts.size + intervals.size,
  };
}

function children(element: any): Element[] {
  if (element == null || typeof element !== "object") return [];
  if (Array.isArray(element)) return element.flatMap(children);
  return [element, ...children(element.props?.children)];
}
function text(element: any): string {
  if (element == null || typeof element === "boolean") return "";
  if (Array.isArray(element)) return element.map(text).join("");
  return typeof element === "object" ? text(element.props?.children) : String(element);
}
function button(tree: Element, label: string): Element {
  const found = children(tree).find((element) => ["button", "Button"].includes(element.type) && text(element) === label);
  assert.ok(found, label);
  return found;
}
function select(tree: Element, label: string): Element {
  const found = children(tree).find((element) => element.type === "AppSelect" && element.props.ariaLabel === label);
  assert.ok(found, label);
  return found;
}
function probeDialog(tree: Element): Element {
  const found = children(tree).find((element) => element.type === "ProviderTestDialog");
  assert.ok(found, "应打开具体供应商测试弹窗");
  return found;
}
async function load(f: ReturnType<typeof harness>) {
  f.render();
  assert.equal(f.requests[0].command, "load_gateway");
  f.requests[0].result.resolve(fixture());
  await flush();
  return f.render();
}
function openProbe(f: ReturnType<typeof harness>) {
  button(f.render(), "测试模型请求").props.onClick();
  return probeDialog(f.render());
}

test("入门配置读取失败可重试，失败或缺失数据不算已配置", async () => {
  for (const response of ["throw", { status: "failed" }, { status: "ok", pool: [], agents: [] }]) {
    const f = harness();
    assert.equal(button(f.render(), "添加供应商").props.disabled, true);
    if (response === "throw") f.requests[0].result.reject(new Error("fixture unavailable"));
    else f.requests[0].result.resolve(response);
    await flush();
    let tree = f.render();
    assert.match(text(tree), /配置读取失败/);
    assert.equal(button(tree, "测试模型请求").props.disabled, true);
    button(tree, "重试").props.onClick();
    assert.equal(f.requests.length, 2);
    f.requests[1].result.resolve(fixture());
    await flush();
    tree = f.render();
    assert.doesNotMatch(text(tree), /配置读取失败/);
    assert.equal(button(tree, "测试模型请求").props.disabled, false);
    assert.equal(button(tree, "启动 Codex 并完成").props.disabled, true, "仅加载配置不等于测试成功");
    f.unmount();
    assert.equal(f.timerCount(), 0);
  }
});

test("超慢读取期间轮询保持单飞，超时允许重试并忽略旧请求晚到", async () => {
  const f = harness();
  f.render();
  for (let count = 0; count < 4; count++) f.poll();
  assert.equal(f.requests.length, 1);
  f.expireLoad();
  let tree = f.render();
  assert.match(text(tree), /配置读取超时/);
  button(tree, "重试").props.onClick();
  f.poll();
  assert.equal(f.requests.length, 2);
  f.requests[1].result.resolve(fixture());
  await flush();
  f.render();
  f.requests[0].result.resolve({ ...fixture(), pool: [] });
  await flush();
  tree = f.render();
  assert.equal(button(tree, "测试模型请求").props.disabled, false, "旧空池不能覆盖已重试读到的配置");
  f.unmount();
  assert.equal(f.timerCount(), 0);
});

test("供应商探测明确失败不允许完成，只有成功且启动确认成功才完成", async () => {
  const f = harness();
  await load(f);
  const dialog = openProbe(f);
  assert.equal(dialog.props.initialModel, "chat");
  assert.equal(dialog.props.lockModel, true);
  dialog.props.onResult({ ok: false });
  let tree = f.render();
  assert.equal(button(tree, "启动 Codex 并完成").props.disabled, true);
  button(tree, "启动 Codex 并完成").props.onClick();
  assert.equal(f.launches.length, 0);
  probeDialog(tree).props.onResult({ ok: true });
  tree = f.render();
  assert.equal(button(tree, "启动 Codex 并完成").props.disabled, false);
  probeDialog(tree).props.onClose();
  button(f.render(), "启动 Codex 并完成").props.onClick();
  assert.deepEqual(f.dismissed, []);
  f.launches[0].resolve(false);
  await flush();
  assert.deepEqual(f.dismissed, [], "启动失败保留清单");
  button(f.render(), "启动 Codex 并完成").props.onClick();
  f.launches[1].resolve(true);
  await flush();
  assert.deepEqual(f.dismissed, [true]);
  f.unmount();
});

test("成功后在测试弹窗再测会立即撤销验证，取消或失败不能沿用旧成功", async () => {
  const f = harness();
  await load(f);
  openProbe(f).props.onResult({ ok: true });
  let tree = f.render();
  assert.equal(button(tree, "启动 Codex 并完成").props.disabled, false);
  probeDialog(tree).props.onStart();
  tree = f.render();
  assert.equal(button(tree, "启动 Codex 并完成").props.disabled, true);
  probeDialog(tree).props.onClose();
  assert.equal(button(f.render(), "启动 Codex 并完成").props.disabled, true, "取消新测试也不能恢复旧成功");
  openProbe(f).props.onResult({ ok: true });
  probeDialog(f.render()).props.onClose();
  button(f.render(), "重新测试模型请求").props.onClick();
  tree = f.render();
  assert.equal(button(tree, "启动 Codex 并完成").props.disabled, true);
  probeDialog(tree).props.onResult({ ok: false });
  assert.equal(button(f.render(), "启动 Codex 并完成").props.disabled, true);
  f.unmount();
});

test("切换 Agent 撤销已有验证，正在测试的旧 Agent 结果不作用于新 Agent", async () => {
  const f = harness("all");
  await load(f);
  const oldDialog = openProbe(f);
  oldDialog.props.onResult({ ok: true });
  let tree = f.render();
  assert.equal(button(tree, "启动 Codex 并完成").props.disabled, false);
  select(tree, "本次配置的 Agent").props.onChange("claude");
  tree = f.render();
  assert.equal(button(tree, "完成配置").props.disabled, true);
  assert.equal(children(tree).some((element) => element.type === "ProviderTestDialog"), false);
  oldDialog.props.onResult({ ok: true });
  assert.equal(button(f.render(), "完成配置").props.disabled, true, "旧弹窗晚到不能验证当前 Agent");
  select(f.render(), "本次配置的 Agent").props.onChange("codex");
  assert.equal(button(f.render(), "启动 Codex 并完成").props.disabled, true, "切回也需要重新验证");
  f.unmount();
});

test("关键配置变化关闭旧测试并撤销验证，改回原配置不会复活旧结果", async () => {
  const f = harness();
  await load(f);
  openProbe(f).props.onResult({ ok: true });
  assert.equal(button(f.render(), "启动 Codex 并完成").props.disabled, false);
  const next = fixture();
  next.pool[0].baseUrl = "https://changed.invalid";
  f.publish(next);
  let tree = f.render();
  assert.equal(button(tree, "启动 Codex 并完成").props.disabled, true);
  assert.equal(children(tree).some((element) => element.type === "ProviderTestDialog"), false);
  f.publish(fixture());
  tree = f.render();
  assert.equal(button(tree, "启动 Codex 并完成").props.disabled, true);
  f.unmount();
});

test("Codex 检查失败或抛错不完成，重复点击只执行一次且可以重试", async () => {
  for (const failure of ["false", "throw"]) {
    const f = harness("enhance");
    await load(f);
    const check = button(f.render(), "检查安装与配置");
    check.props.onClick();
    check.props.onClick();
    assert.equal(f.checks.length, 1);
    assert.equal(button(f.render(), "检查安装与配置").props.disabled, true);
    if (failure === "false") f.checks[0].resolve(false);
    else f.checks[0].reject(new Error("fixture check failed"));
    await flush();
    let tree = f.render();
    assert.equal(button(tree, "启动 Codex 并完成").props.disabled, true);
    assert.equal(button(tree, "检查安装与配置").props.disabled, false);
    assert.equal(f.notices.length, 1);
    assert.deepEqual(f.dismissed, []);
    button(tree, "检查安装与配置").props.onClick();
    f.checks[1].resolve(true);
    await flush();
    tree = f.render();
    assert.equal(button(tree, "启动 Codex 并完成").props.disabled, false);
    f.unmount();
  }
});

test("检查返回成功仍需最新应用状态就绪，旧 Agent 的检查结果不能跨选择生效", async () => {
  const f = harness("enhance");
  await load(f);
  button(f.render(), "检查安装与配置").props.onClick();
  f.props.codexInstalled = false;
  f.checks[0].resolve(true);
  await flush();
  assert.equal(button(f.render(), "启动 Codex 并完成").props.disabled, true);
  f.props.codexInstalled = true;
  button(f.render(), "检查安装与配置").props.onClick();
  select(f.render(), "本次配置的 Agent").props.onChange("claude");
  f.checks[1].resolve(true);
  await flush();
  assert.equal(button(f.render(), "完成配置").props.disabled, true);
  select(f.render(), "本次配置的 Agent").props.onChange("codex");
  assert.equal(button(f.render(), "启动 Codex 并完成").props.disabled, true);
  f.unmount();
});

test("Codex 检查采用等待期间父层刷新的增强配置，开启后可完成、关闭后不可完成", async () => {
  for (const enabled of [true, false]) {
    const f = harness("enhance");
    f.props.codexEnhancementsEnabled = !enabled;
    await load(f);
    button(f.render(), "检查安装与配置").props.onClick();
    assert.equal(f.checks.length, 1);
    f.props.codexEnhancementsEnabled = enabled;
    assert.equal(button(f.render(), "启动 Codex 并完成").props.disabled, true, "父层更新还不能替代进行中的检查结果");
    f.checks[0].resolve(true);
    await flush();
    const tree = f.render();
    assert.equal(button(tree, "启动 Codex 并完成").props.disabled, !enabled);
    if (enabled) {
      assert.match(text(tree), /已验证，可以开始/);
      button(tree, "启动 Codex 并完成").props.onClick();
      f.launches[0].resolve(true);
      await flush();
      assert.deepEqual(f.dismissed, [true]);
    } else {
      button(tree, "启动 Codex 并完成").props.onClick();
      assert.equal(f.launches.length, 0);
      assert.deepEqual(f.dismissed, []);
    }
    f.unmount();
  }
});
