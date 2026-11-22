import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import type { AgentFocus } from "./agent-focus-model.ts";
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

function profile(overrides: Partial<setupModel.SetupRelayProfile> = {}): setupModel.SetupRelayProfile {
  return { id: "relay", name: "Relay", relayMode: "pureApi", officialMixApiKey: false, baseUrl: "https://relay.invalid/v1", apiKey: "sk-test", noAuth: false, ...overrides };
}
function relaySettings(active = profile()): setupModel.SetupRelaySettings {
  return { relayProfilesEnabled: true, activeRelayId: active.id, relayProfiles: [active] };
}
const configured = { status: "ok", message: "", authenticated: false, configured: true };

/** 保留真实组件与状态模型，只替换 React 调度和桌面命令。 */
function harness(focus: AgentFocus = "codex") {
  const hooks: any[] = [];
  const cleanups = new Map<number, () => void>();
  const effects: Array<() => void> = [];
  const timeouts = new Map<number, () => void>();
  const intervals = new Map<number, () => void>();
  const requests: Array<{ command: string; args?: any; result: ReturnType<typeof deferred<any>> }> = [];
  const checks: Array<ReturnType<typeof deferred<boolean>>> = [];
  const launches: Array<ReturnType<typeof deferred<boolean>>> = [];
  const notices: Array<{ title: string; message: string; status?: string }> = [];
  const dismissed: boolean[] = [];
  const navigation: Array<{ target: string; agent?: string }> = [];
  let cursor = 0;
  let changed = false;
  let timerId = 0;
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
  const jsx = (type: string, props: Record<string, any>) => ({ type, props });
  const modules: Record<string, unknown> = {
    react,
    "react/jsx-runtime": { jsx, jsxs: jsx, Fragment: "Fragment" },
    "@tauri-apps/api/core": { invoke: (command: string, args?: any) => {
      const result = deferred<any>();
      requests.push({ command, args, result });
      return result.promise;
    } },
    "lucide-react": Object.fromEntries(["ArrowRight", "Check", "ChevronDown", "ChevronUp", "CircleHelp", "Loader2", "Play", "Plus", "RefreshCw", "Settings2", "X", "Zap"].map((name) => [name, name])),
    "@/components/ui/button": { Button: "Button" },
    "./AppSelect": { AppSelect: "AppSelect" },
    "./HubOverview": { AgentGlyph: "AgentGlyph" },
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
  const added: number[] = [];
  const props = {
    focus, relaySettings: relaySettings() as setupModel.SetupRelaySettings | null, codexInstalled: true as boolean | null, codexRunning: false, launchPending: false,
    codexEnhancementsEnabled: true,
    notify: async (title: string, message: string, status?: string) => { notices.push({ title, message, status }); },
    onChangeMode: () => {},
    onDismiss: (completed: boolean) => { dismissed.push(completed); },
    onTour: () => {},
    onNavigate: (target: string, agent?: string) => { navigation.push({ target, agent }); },
    onAddProvider: () => { added.push(1); },
    onCheckCodex: () => { const request = deferred<boolean>(); checks.push(request); return request.promise; },
    onLaunch: () => { const request = deferred<boolean>(); launches.push(request); return request.promise; },
  };
  return {
    requests, checks, launches, notices, dismissed, navigation, added, props,
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
async function load(f: ReturnType<typeof harness>, status: unknown = configured) {
  f.render();
  assert.equal(f.requests[0].command, "relay_status");
  f.requests[0].result.resolve(status);
  await flush();
  return f.render();
}
/** 点「测试模型请求」，返回这次 test_relay_profile 请求。 */
function startTest(f: ReturnType<typeof harness>, label = "测试模型请求") {
  const before = f.requests.length;
  button(f.render(), label).props.onClick();
  const request = f.requests[before];
  assert.equal(request?.command, "test_relay_profile");
  return request;
}

test("入门配置读取失败可重试，失败或缺失数据不算已配置", async () => {
  for (const response of ["throw", { status: "failed" }, { status: "ok", message: "" }]) {
    const f = harness();
    assert.match(text(f.render()), /正在读取配置/);
    if (response === "throw") f.requests[0].result.reject(new Error("fixture unavailable"));
    else f.requests[0].result.resolve(response);
    await flush();
    let tree = f.render();
    assert.match(text(tree), /配置读取失败/);
    assert.equal(button(tree, "测试模型请求").props.disabled, true);
    button(tree, "重试").props.onClick();
    assert.equal(f.requests.length, 2);
    f.requests[1].result.resolve(configured);
    await flush();
    tree = f.render();
    assert.doesNotMatch(text(tree), /配置读取失败/);
    assert.match(text(tree), /当前供应商：Relay · 纯 API/);
    assert.equal(button(tree, "测试模型请求").props.disabled, false);
    assert.equal(button(tree, "启动 Codex 并完成").props.disabled, true, "仅读到配置不等于测试成功");
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
  f.requests[1].result.resolve(configured);
  await flush();
  f.render();
  f.requests[0].result.resolve({ ...configured, configured: false });
  await flush();
  tree = f.render();
  assert.equal(button(tree, "测试模型请求").props.disabled, false, "旧的未配置状态不能覆盖重试读到的状态");
  f.unmount();
  assert.equal(f.timerCount(), 0);
});

test("没登录 ChatGPT 时不能测试，入口通往添加供应商和连接与供应商", async () => {
  const f = harness("all");
  f.props.relaySettings = relaySettings(profile({ id: "default", relayMode: "official", baseUrl: "", apiKey: "" }));
  let tree = await load(f, { ...configured, configured: false });
  assert.equal(children(tree).some((element) => element.type === "AppSelect"), false, "非增强模式只配置 Codex");
  assert.match(text(tree), /Codex 还没登录 ChatGPT/);
  assert.equal(button(tree, "测试模型请求").props.disabled, true);
  button(tree, "添加供应商").props.onClick();
  button(tree, "连接与供应商").props.onClick();
  assert.deepEqual(f.added, [1]);
  assert.deepEqual(f.navigation, [{ target: "relay", agent: "codex" }]);
  f.poll();
  f.requests[1].result.resolve({ ...configured, authenticated: true, configured: false });
  await flush();
  tree = f.render();
  assert.match(text(tree), /当前供应商：Relay · 官方登录/, "在 Codex 里登录后轮询即可就绪");
  assert.equal(button(tree, "测试模型请求").props.disabled, false);
  f.props.relaySettings = { ...relaySettings(), relayProfilesEnabled: false };
  assert.match(text(f.render()), /供应商配置切换已关闭/);
  f.unmount();
});

test("测试明确失败不允许完成，只有成功且启动确认成功才完成", async () => {
  const f = harness();
  await load(f);
  const first = startTest(f);
  assert.equal(first.args.profile.id, "relay");
  assert.equal(button(f.render(), "测试模型请求").props.disabled, true, "测试中不能重复发起");
  first.result.resolve({ status: "failed", message: "HTTP 401" });
  await flush();
  let tree = f.render();
  assert.match(text(tree), /HTTP 401/);
  assert.equal(button(tree, "启动 Codex 并完成").props.disabled, true);
  button(tree, "启动 Codex 并完成").props.onClick();
  assert.equal(f.launches.length, 0);
  startTest(f).result.resolve({ status: "warning", message: "incomplete" });
  await flush();
  tree = f.render();
  assert.match(text(tree), /模型请求测试通过/, "协议有效但未完整结束也算通过");
  assert.equal(button(tree, "启动 Codex 并完成").props.disabled, false);
  button(tree, "启动 Codex 并完成").props.onClick();
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

test("成功后再测会立即撤销验证，失败或抛错不能沿用旧成功", async () => {
  const f = harness();
  await load(f);
  startTest(f).result.resolve({ status: "ok", message: "" });
  await flush();
  assert.equal(button(f.render(), "启动 Codex 并完成").props.disabled, false);
  const again = startTest(f, "重新测试模型请求");
  assert.equal(button(f.render(), "启动 Codex 并完成").props.disabled, true);
  again.result.reject("network down");
  await flush();
  const tree = f.render();
  assert.match(text(tree), /测试失败：network down/);
  assert.equal(button(tree, "启动 Codex 并完成").props.disabled, true);
  f.unmount();
});

test("供应商变化撤销验证，进行中的旧测试结果不作用于新配置，改回也不复活", async () => {
  const f = harness();
  await load(f);
  startTest(f).result.resolve({ status: "ok", message: "" });
  await flush();
  assert.equal(button(f.render(), "启动 Codex 并完成").props.disabled, false);
  const original = f.props.relaySettings;
  f.props.relaySettings = relaySettings(profile({ apiKey: "sk-changed" }));
  assert.equal(button(f.render(), "启动 Codex 并完成").props.disabled, true);
  const pending = startTest(f);
  f.props.relaySettings = original;
  f.render();
  pending.result.resolve({ status: "ok", message: "" });
  await flush();
  const tree = f.render();
  assert.equal(button(tree, "启动 Codex 并完成").props.disabled, true, "旧配置的测试晚到也不能验证当前配置");
  assert.equal(button(tree, "测试模型请求").props.disabled, false, "配置变化后可以重新测试");
  f.unmount();
});

test("聚合供应商不做单条请求测试，配置写入后即可完成", async () => {
  const f = harness();
  f.props.relaySettings = relaySettings(profile({ relayMode: "aggregate" }));
  const tree = await load(f);
  assert.equal(children(tree).some((element) => ["button", "Button"].includes(element.type) && text(element) === "测试模型请求"), false);
  assert.match(text(tree), /逐个测试成员/);
  assert.equal(button(tree, "启动 Codex 并完成").props.disabled, false);
  f.unmount();
});
test("Codex 检查失败或抛错不完成，重复点击只执行一次且可以重试", async () => {
  for (const failure of ["false", "throw"]) {
    const f = harness("enhance");
    f.render();
    assert.equal(f.requests.length, 0, "增强模式不读取 Codex 连接状态");
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
  f.render();
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
    f.render();
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

test("入门清单收起时只在挂上时读一次配置，展开后才每 5 秒重读", async () => {
  const f = harness();
  (f.props as Record<string, unknown>).defaultExpanded = false;
  await load(f);
  const before = f.requests.length;
  f.poll();
  assert.equal(f.requests.length, before, "收起时不轮询 relay_status");
  const toggle = children(f.render()).find((element) => element.type === "button" && element.props["aria-label"] === "展开入门清单");
  assert.ok(toggle);
  toggle.props.onClick();
  f.render();
  f.poll();
  assert.equal(f.requests.length, before + 1, "展开后轮询");
  assert.equal(f.requests.at(-1)?.command, "relay_status");
});
