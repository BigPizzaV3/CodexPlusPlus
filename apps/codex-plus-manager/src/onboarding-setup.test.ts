import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
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
function harness(open = true) {
  const hooks: any[] = [];
  const cleanups = new Map<number, () => void>();
  const effects: Array<() => void> = [];
  const timeouts = new Map<number, () => void>();
  const intervals = new Map<number, () => void>();
  const requests: Array<{ command: string; args?: any; result: ReturnType<typeof deferred<any>> }> = [];
  const launches: Array<ReturnType<typeof deferred<boolean>>> = [];
  const dismissed: boolean[] = [];
  const navigation: Array<{ target: string; agent?: string }> = [];
  const opened: boolean[] = [];
  const added: number[] = [];
  const tours: number[] = [];
  let escape: { active: boolean; close: () => void } = { active: false, close: () => {} };
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
    // 进度卡投到侧栏、向导投到 body：这里只记下投到了哪里。
    "react-dom": { createPortal: (child: unknown, target: unknown) => ({ type: "Portal", props: { target, children: child } }) },
    "@tauri-apps/api/core": { invoke: (command: string, args?: any) => {
      const result = deferred<any>();
      requests.push({ command, args, result });
      return result.promise;
    } },
    "lucide-react": Object.fromEntries(["ArrowLeft", "ArrowRight", "Check", "CircleAlert", "Compass", "Loader2", "Play", "Plus", "Puzzle", "RefreshCw", "X", "Zap"].map((name) => [name, name])),
    "./hub-shared": { AgentGlyph: "AgentGlyph" },
    "./app-shared": { APP_LOGO_URL: "logo.png" },
    "./onboarding-setup-model": setupModel,
    "./use-escape-layer": { useEscapeLayer: (active: boolean, close: () => void) => { escape = { active, close }; } },
    "./use-modal-focus": { useModalFocus: () => {} },
    "./i18n": { t: (text: string) => text, tf: (text: string, args: unknown[]) => text.replace(/\{(\d+)\}/g, (_, index) => String(args[Number(index)])) },
    "./OnboardingTour.css": {},
    "./OnboardingSetup.css": {},
  };
  const exports: Record<string, any> = {};
  const body = { name: "body" };
  vm.runInNewContext(source, {
    exports,
    document: { visibilityState: "visible", body },
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
  const slot = { name: "sidebar-slot" };
  const props = {
    open,
    onOpenChange: (next: boolean) => { opened.push(next); props.open = next; },
    cardSlot: slot as object | null,
    relaySettings: relaySettings() as setupModel.SetupRelaySettings | null,
    codexInstalled: true as boolean | null, codexRunning: false, launchPending: false, codexEnhancementsEnabled: true,
    onDismiss: (completed: boolean) => { dismissed.push(completed); },
    onTour: () => { tours.push(1); },
    onNavigate: (target: string, agent?: string) => { navigation.push({ target, agent }); },
    onAddProvider: () => { added.push(1); },
    onLaunch: () => { const request = deferred<boolean>(); launches.push(request); return request.promise; },
  };
  return {
    requests, launches, dismissed, navigation, opened, added, tours, props, slot, body,
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
    escape: () => escape,
    unmount: () => { for (const cleanup of cleanups.values()) cleanup(); cleanups.clear(); },
    timerCount: () => timeouts.size + intervals.size,
    intervalCount: () => intervals.size,
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
function byClass(tree: Element, name: string): Element | undefined {
  return children(tree).find((element) => typeof element.props?.className === "string" && element.props.className.split(" ").includes(name));
}
/** 向导那张卡片；关着时没有。 */
const wizard = (tree: Element) => byClass(tree, "ob-card");
/** 侧栏的进度卡。 */
const progressCard = (tree: Element) => byClass(tree, "onboarding-card");
function button(tree: Element | undefined, label: string): Element {
  const found = children(tree).find((element) => element.type === "button" && text(element) === label);
  assert.ok(found, label);
  return found;
}
const hasButton = (tree: Element | undefined, label: string) => children(tree).some((element) => element.type === "button" && text(element) === label);
/** 一行状态（连接、测试、Codex）的色调。 */
function tones(tree: Element | undefined): string[] {
  return children(tree).filter((element) => element.props?.className === "ob-status").map((element) => element.props["data-tone"]);
}
async function load(f: ReturnType<typeof harness>, status: unknown = configured) {
  f.render();
  assert.equal(f.requests[0].command, "relay_status");
  f.requests[0].result.resolve(status);
  await flush();
  return f.render();
}
/** 点测试按钮，返回这次 test_relay_profile 请求。 */
function startTest(f: ReturnType<typeof harness>, label = "测试模型请求") {
  const before = f.requests.length;
  button(wizard(f.render()), label).props.onClick();
  const request = f.requests[before];
  assert.equal(request?.command, "test_relay_profile");
  return request;
}

test("第一次打开停在「连接模型」；读取失败可重试，失败或缺失数据不算已连上", async () => {
  for (const response of ["throw", { status: "failed" }, { status: "ok", message: "" }]) {
    const f = harness();
    let tree = f.render();
    assert.equal(wizard(tree)?.props["data-step"], "connect");
    assert.match(text(wizard(tree)), /正在读取配置/);
    assert.equal(f.requests.length, 1, "挂上和打开各触发一次读取，但同一时间只发一个请求");
    if (response === "throw") f.requests[0].result.reject(new Error("fixture unavailable"));
    else f.requests[0].result.resolve(response);
    await flush();
    tree = f.render();
    assert.match(text(wizard(tree)), /配置读取失败/);
    assert.deepEqual(tones(wizard(tree)), ["error"]);
    assert.equal(hasButton(wizard(tree), "测试模型请求"), false, "没连上时不给测试");
    assert.equal(hasButton(wizard(tree), "先跳过"), true, "没连上也能继续，只是主按钮换成跳过");
    assert.match(text(progressCard(tree)), /下一步：连接模型0\/2/);
    button(wizard(tree), "重试").props.onClick();
    assert.equal(f.requests.length, 2);
    f.requests[1].result.resolve(configured);
    await flush();
    tree = f.render();
    assert.doesNotMatch(text(wizard(tree)), /配置读取失败/);
    assert.match(text(wizard(tree)), /当前供应商：Relay · 纯 API/);
    assert.deepEqual(tones(wizard(tree)), ["ok", "idle"]);
    assert.equal(button(wizard(tree), "测试模型请求").props.disabled, false);
    assert.equal(hasButton(wizard(tree), "下一步"), true);
    assert.match(text(progressCard(tree)), /下一步：启动 Codex1\/2/);
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
  assert.match(text(wizard(tree)), /配置读取超时/);
  button(wizard(tree), "重试").props.onClick();
  f.poll();
  assert.equal(f.requests.length, 2);
  f.requests[1].result.resolve(configured);
  await flush();
  f.render();
  f.requests[0].result.resolve({ ...configured, configured: false });
  await flush();
  tree = f.render();
  assert.equal(hasButton(wizard(tree), "测试模型请求"), true, "旧的未配置状态不能覆盖重试读到的状态");
  f.unmount();
  assert.equal(f.timerCount(), 0);
});

test("没登录 ChatGPT 时不能测试；去添加供应商或「连接与供应商」时向导先收起，登录后轮询即可就绪", async () => {
  const f = harness();
  f.props.relaySettings = relaySettings(profile({ id: "default", relayMode: "official", baseUrl: "", apiKey: "" }));
  let tree = await load(f, { ...configured, configured: false });
  assert.match(text(wizard(tree)), /Codex 还没登录 ChatGPT/);
  assert.deepEqual(tones(wizard(tree)), ["todo"]);
  assert.equal(hasButton(wizard(tree), "测试模型请求"), false);
  button(wizard(tree), "添加供应商").props.onClick();
  assert.deepEqual(f.added, [1]);
  assert.deepEqual(f.opened, [false], "去别的页面前先收起向导");
  tree = f.render();
  assert.equal(wizard(tree), undefined);
  assert.ok(progressCard(tree), "进度卡留在侧栏");
  // 从进度卡回来：还没连上，回到「连接模型」。
  button(progressCard(tree), "开始使用下一步：连接模型0/2").props.onClick();
  assert.deepEqual(f.opened, [false, true]);
  tree = f.render();
  assert.equal(wizard(tree)?.props["data-step"], "connect");
  button(wizard(tree), "连接与供应商").props.onClick();
  assert.deepEqual(f.navigation, [{ target: "relay", agent: "codex" }]);
  assert.deepEqual(f.opened, [false, true, false]);
  // 在 Codex 里登录之后，进度卡知道已经连上；点它直接到第二步。
  f.props.open = true;
  f.render();
  const request = f.requests.at(-1)!;
  request.result.resolve({ ...configured, authenticated: true, configured: false });
  await flush();
  tree = f.render();
  assert.match(text(wizard(tree)), /当前供应商：Relay · 官方登录/);
  f.props.open = false;
  tree = f.render();
  button(progressCard(tree), "开始使用下一步：启动 Codex1/2").props.onClick();
  tree = f.render();
  assert.equal(wizard(tree)?.props["data-step"], "start");
  f.unmount();
});

test("测试是可选的：通过、失败都显示结果，失败不拦着继续；再测会立即撤销上一次结果", async () => {
  const f = harness();
  await load(f);
  let request = startTest(f);
  assert.equal(request.args.profile, f.props.relaySettings!.relayProfiles[0], "测的是当前这家供应商");
  let tree = f.render();
  assert.equal(button(wizard(tree), "测试模型请求").props.disabled, true, "测试进行中不能重复点");
  request.result.resolve({ status: "failed", message: "401 Unauthorized" });
  await flush();
  tree = f.render();
  assert.deepEqual(tones(wizard(tree)), ["ok", "error"]);
  assert.match(text(wizard(tree)), /测试没有通过401 Unauthorized/);
  assert.equal(hasButton(wizard(tree), "下一步"), true, "测试失败不拦着进下一步");
  request = startTest(f, "重新测试");
  tree = f.render();
  assert.doesNotMatch(text(wizard(tree)), /401 Unauthorized/, "重新测试时上一次的失败立即撤掉");
  request.result.resolve({ status: "ok", message: "pong" });
  await flush();
  tree = f.render();
  assert.deepEqual(tones(wizard(tree)), ["ok", "ok"]);
  assert.match(text(wizard(tree)), /测试通过当前供应商的模型请求测试通过。/);
  // 成功后再测：先撤销，抛错不能沿用旧成功。
  request = startTest(f, "重新测试");
  assert.deepEqual(tones(wizard(f.render())), ["ok", "idle"]);
  request.result.reject(new Error("network down"));
  await flush();
  tree = f.render();
  assert.deepEqual(tones(wizard(tree)), ["ok", "error"]);
  assert.match(text(wizard(tree)), /测试失败：Error: network down/);
  // 第二步的回顾只在测试通过时才说「测试通过」。
  button(wizard(tree), "下一步").props.onClick();
  tree = f.render();
  assert.equal(wizard(tree)?.props["data-step"], "start");
  assert.doesNotMatch(text(wizard(tree)), /测试通过/);
  f.unmount();
});

test("供应商变化撤销测试结果，进行中的旧测试不作用于新配置，改回也不复活", async () => {
  const f = harness();
  await load(f);
  const first = startTest(f);
  first.result.resolve({ status: "ok", message: "pong" });
  await flush();
  assert.deepEqual(tones(wizard(f.render())), ["ok", "ok"]);
  const original = f.props.relaySettings;
  f.props.relaySettings = relaySettings(profile({ id: "other", name: "Other", apiKey: "sk-other" }));
  let tree = f.render();
  assert.match(text(wizard(tree)), /当前供应商：Other/);
  assert.deepEqual(tones(wizard(tree)), ["ok", "idle"], "换了供应商，旧的通过不算数");
  const stale = startTest(f);
  f.props.relaySettings = original;
  tree = f.render();
  assert.deepEqual(tones(wizard(tree)), ["ok", "idle"], "改回原配置也不复活旧结果");
  assert.equal(button(wizard(tree), "测试模型请求").props.disabled, false, "配置一变，进行中的测试作废，可以重新测");
  stale.result.resolve({ status: "ok", message: "pong" });
  await flush();
  assert.deepEqual(tones(wizard(f.render())), ["ok", "idle"], "给另一份配置发的测试结果不作用于当前配置");
  f.unmount();
});

test("聚合供应商不做单条请求测试", async () => {
  const f = harness();
  f.props.relaySettings = relaySettings(profile({ relayMode: "aggregate" }));
  const tree = await load(f);
  assert.match(text(wizard(tree)), /当前供应商：Relay · 聚合供应商/);
  assert.equal(hasButton(wizard(tree), "测试模型请求"), false);
  assert.match(text(wizard(tree)), /请在「连接与供应商」里逐个测试成员/);
  assert.equal(hasButton(wizard(tree), "下一步"), true);
  f.unmount();
});

test("向导关着时只在挂上时读一次配置，打开后才每 5 秒重读", async () => {
  const f = harness(false);
  let tree = f.render();
  assert.equal(wizard(tree), undefined);
  assert.equal(f.requests.length, 1, "进度卡要知道连上没有：挂上时读一次");
  assert.equal(f.intervalCount(), 0, "向导关着不轮询");
  f.requests[0].result.resolve(configured);
  await flush();
  tree = f.render();
  assert.match(text(progressCard(tree)), /下一步：启动 Codex1\/2/);
  button(progressCard(tree), "开始使用下一步：启动 Codex1/2").props.onClick();
  tree = f.render();
  assert.equal(f.intervalCount(), 1, "打开后开始轮询");
  assert.equal(f.requests.length, 2, "打开时马上重读一次");
  f.requests[1].result.resolve(configured);
  await flush();
  f.poll();
  assert.equal(f.requests.length, 3);
  f.props.open = false;
  f.render();
  assert.equal(f.intervalCount(), 0, "收起后停止轮询");
  f.unmount();
  assert.equal(f.timerCount(), 0);
});

test("进度卡投到侧栏的卡位，向导投到 body；没有卡位（设置模式）时只有向导", async () => {
  const f = harness();
  let tree = f.render();
  const portals = children(tree).filter((element) => element.type === "Portal");
  assert.deepEqual(portals.map((portal) => portal.props.target), [f.slot, f.body]);
  assert.ok(progressCard(portals[0]));
  assert.ok(wizard(portals[1]));
  f.props.cardSlot = null;
  tree = f.render();
  assert.equal(progressCard(tree), undefined);
  assert.ok(wizard(tree), "切到设置模式向导不能跟着消失");
  // 关闭按钮是「不再显示」：进度卡收走，不算走完。
  f.props.cardSlot = f.slot;
  tree = f.render();
  children(progressCard(tree)).find((element) => element.props?.["aria-label"] === "不再显示启动引导")!.props.onClick();
  assert.deepEqual(f.dismissed, [false]);
  f.unmount();
});

test("稍后设置、关闭按钮和 Esc 都只是收起向导，不算关掉引导", async () => {
  const f = harness();
  let tree = await load(f);
  assert.equal(f.escape().active, true);
  button(wizard(tree), "稍后设置").props.onClick();
  assert.deepEqual(f.opened, [false]);
  assert.deepEqual(f.dismissed, []);
  tree = f.render();
  assert.equal(f.escape().active, false, "收起后不再占着 Esc");
  f.props.open = true;
  tree = f.render();
  children(wizard(tree)).find((element) => element.props?.["aria-label"] === "稍后设置")!.props.onClick();
  f.props.open = true;
  f.render();
  f.escape().close();
  assert.deepEqual(f.opened, [false, false, false]);
  assert.deepEqual(f.dismissed, []);
  f.unmount();
});

test("第二步：启动成功才算走完，失败留在原处；重复点击只启动一次，启动期间关不掉", async () => {
  const f = harness();
  let tree = await load(f);
  button(wizard(tree), "下一步").props.onClick();
  tree = f.render();
  assert.match(text(wizard(tree)), /都准备好了/);
  assert.deepEqual(tones(wizard(tree)), ["ok", "ok"]);
  assert.match(text(wizard(tree)), /已安装，增强已开启。/);
  assert.equal(hasButton(wizard(tree), "增强设置"), false, "一切就绪时不给多余的入口");
  const start = button(wizard(tree), "启动 Codex");
  start.props.onClick();
  start.props.onClick();
  assert.equal(f.launches.length, 1);
  f.escape().close();
  assert.deepEqual(f.opened, [], "启动过程中 Esc 不收起向导");
  f.launches[0].resolve(false);
  await flush();
  assert.deepEqual(f.dismissed, [], "启动没成功不算走完");
  button(wizard(f.render()), "启动 Codex").props.onClick();
  assert.equal(f.launches.length, 2, "失败后可以重试");
  f.launches[1].resolve(true);
  await flush();
  assert.deepEqual(f.dismissed, [true]);
  f.unmount();
});

test("第二步：可以不启动直接完成；Codex 正在运行时主按钮是重启；从这里能去导览和别的页面", async () => {
  const f = harness();
  f.props.codexRunning = true;
  let tree = await load(f);
  button(wizard(tree), "下一步").props.onClick();
  tree = f.render();
  assert.equal(hasButton(wizard(tree), "重启 Codex"), true);
  button(wizard(tree), "界面导览").props.onClick();
  assert.deepEqual(f.tours, [1]);
  button(wizard(tree), "万物皆拓展").props.onClick();
  button(wizard(tree), "Claude 增强").props.onClick();
  assert.deepEqual(f.navigation, [{ target: "userScripts", agent: undefined }, { target: "claude", agent: "claude" }]);
  assert.deepEqual(f.opened, [false, false], "去别的页面前先收起向导");
  f.props.open = true;
  button(wizard(f.render()), "上一步").props.onClick();
  assert.equal(wizard(f.render())?.props["data-step"], "connect");
  button(wizard(f.render()), "下一步").props.onClick();
  button(wizard(f.render()), "完成，暂不启动").props.onClick();
  assert.deepEqual(f.dismissed, [true]);
  assert.equal(f.launches.length, 0);
  f.unmount();
});

test("第二步：没连上模型、没装 Codex、增强总开关关着时各自说明并给出入口", async () => {
  const f = harness();
  f.props.relaySettings = relaySettings(profile({ id: "default", relayMode: "official", baseUrl: "", apiKey: "" }));
  f.props.codexInstalled = false;
  let tree = await load(f, { ...configured, configured: false });
  button(wizard(tree), "先跳过").props.onClick();
  tree = f.render();
  assert.match(text(wizard(tree)), /可以开始了/);
  assert.deepEqual(tones(wizard(tree)), ["todo", "todo"]);
  assert.match(text(wizard(tree)), /还没有连上模型，可以稍后在「连接与供应商」里完成。/);
  assert.match(text(wizard(tree)), /没有找到 Codex 桌面版。/);
  assert.equal(hasButton(wizard(tree), "启动 Codex"), false, "没装 Codex 就没有启动按钮");
  button(wizard(tree), "安装设置").props.onClick();
  assert.deepEqual(f.navigation, [{ target: "agentSettings", agent: "codex" }]);
  assert.deepEqual(f.opened, [false]);
  f.props.open = true;
  button(wizard(f.render()), "去完成").props.onClick();
  assert.equal(wizard(f.render())?.props["data-step"], "connect");
  // 装了 Codex 但增强总开关关着：能启动，同时提示去打开。
  f.props.codexInstalled = true;
  f.props.codexEnhancementsEnabled = false;
  button(wizard(f.render()), "先跳过").props.onClick();
  tree = f.render();
  assert.match(text(wizard(tree)), /增强总开关是关着的/);
  assert.equal(hasButton(wizard(tree), "启动 Codex"), true);
  button(wizard(tree), "增强设置").props.onClick();
  assert.deepEqual(f.navigation.at(-1), { target: "enhance", agent: "codex" });
  // 还在检查安装状态时不下结论。
  f.props.codexInstalled = null;
  f.props.open = true;
  tree = f.render();
  assert.match(text(wizard(tree)), /正在检查…/);
  button(wizard(tree), "完成").props.onClick();
  assert.deepEqual(f.dismissed, [true]);
  f.unmount();
});
