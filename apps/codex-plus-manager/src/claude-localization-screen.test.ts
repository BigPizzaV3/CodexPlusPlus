import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import test from "node:test";
import ts from "typescript";

// 独立运行真实页面函数和事件处理器，替换 hooks/IPC；不启动本机 Claude 或探测 PATH。
const app = ts.createSourceFile("App.tsx", readFileSync(new URL("./App.tsx", import.meta.url), "utf8"), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const component = app.statements.find((node): node is ts.FunctionDeclaration => ts.isFunctionDeclaration(node) && node.name?.text === "ClaudeLocalizationScreen");
assert.ok(component);
const transpile = (text: string) => ts.transpileModule(text, {
  compilerOptions: { jsx: ts.JsxEmit.React, target: ts.ScriptTarget.ES2022 },
}).outputText;
const source = transpile(component.getText(app));

// 页面用到的两个纯格式函数也直接取 App.tsx 里的真实实现（不复制一份，改了实现这边会跟着变）。
function functionSource(file: ts.SourceFile, name: string) {
  const found = file.statements.find((node): node is ts.FunctionDeclaration => ts.isFunctionDeclaration(node) && node.name?.text === name);
  assert.ok(found, name);
  // 去掉导出的函数上的 `export`，这里只把它们放进 vm 里当普通函数用。
  return found.getText(file).replace(/^export /, "");
}
// 上下文窗口格式化复用网关的 formatTokens（B/M/K），这里连它和 trimNumber 一起拿真实实现。
const gatewayModel = ts.createSourceFile(
  "gateway-model.ts",
  readFileSync(new URL("./gateway-model.ts", import.meta.url), "utf8"),
  ts.ScriptTarget.Latest,
  true,
  ts.ScriptKind.TS,
);
const formatHelpers = transpile([
  functionSource(gatewayModel, "trimNumber"),
  functionSource(gatewayModel, "formatTokens"),
  functionSource(app, "formatContextWindow"),
].join("\n"));

type Element = { type: string; props: Record<string, any> };
type Memo = { status: unknown; autoDetected: boolean };
const localReady = {
  status: "ok", supported: true, sourceApp: "/Applications/Claude.app", version: "1.2.3",
  claudeRunning: false, running: false, connected: false, port: null,
  caReady: true, caTrusted: true, nodeAvailable: true, dictationReady: true,
  config: { enabled: false, model: "claude-test", localization: true, voiceInput: true, computerUse: false, backgroundMode: false },
  models: ["claude-test", "gpt-test"], warning: null, needsRestart: false,
};
/** memo 对应模块级的 claudeLocalizationMemo：传同一个对象模拟「Codex++ 运行期间再次进入页面」。 */
function harness(
  memo: Memo = { status: null, autoDetected: false },
  cuStatus: unknown = {},
  options: {
    localMemo?: Memo;
    openAdvanced?: boolean;
    mode?: "local" | "patch";
    onLocalStatus?: (status: unknown) => void;
  } = {},
) {
  const props = {
    ...(options.mode ? { mode: options.mode } : {}),
    ...(options.onLocalStatus ? { onLocalStatus: options.onLocalStatus } : {}),
  };
  const renderArgs = Object.keys(props).length > 0 ? props : undefined;
  const hooks: any[] = [];
  /** 页头槽位：createPortal 渲染进来的元素记在这里，用来断言「启动增强版」在右上角。 */
  const topbarSlot = { portal: null as unknown };
  const cleanups = new Map<number, () => void>();
  const timers = new Map<number, () => void>();
  let timerId = 0;
  const calls: { command: string; args: unknown[]; resolve: (value: unknown) => void; reject: (cause: unknown) => void }[] = [];
  let cursor = 0;
  let changes = 0;
  const effectHook = (effect: () => void | (() => void), deps?: unknown[]) => {
    const index = cursor++;
    const previous = hooks[index];
    if (previous && deps && previous.deps && deps.length === previous.deps.length
      && deps.every((value, i) => Object.is(value, previous.deps[i]))) return;
    cleanups.get(index)?.();
    cleanups.delete(index);
    hooks[index] = { deps: deps?.slice() };
    const cleanup = effect();
    if (typeof cleanup === "function") cleanups.set(index, cleanup);
  };
  const context = vm.createContext({
    React: { Fragment: "Fragment", createElement: (type: string, props: object, ...children: unknown[]) => ({ type, props: { ...props, children } }) },
    ...Object.fromEntries(["Panel", "CardHead", "CardContent", "Button", "Input", "Info", "RefreshCw", "ToggleVisual", "PackageOpen", "Play", "Plug", "Rocket", "ShieldCheck", "Menu", "MenuItem", "Square", "ChevronRight", "AppSelect"].map((name) => [name, name])),
    // 页面骨架组件（cx-layout）按名字渲染成占位元素。
    AgentCx: new Proxy({}, { get: (_target, name) => String(name) }),
    t: (text: string) => text,
    // 只有带参数的词条会走 tf；沙箱里按中文模板原样替换 {n}。
    tf: (key: string, args: Array<string | number>) => key.replace(/\{(\d+)\}/g, (match, index: string) => {
      const value = args[Number(index)];
      return value === undefined || value === null ? match : String(value);
    }),
    // 跳转按钮在没传 onOpenPatch 时退化成写 hash；测试里给个壳，避免直接碰真的 window。
    window: { location: { hash: "" } },
    CLAUDE_DEFAULT_CONTEXT_WINDOW: 200_000,
    setTimeout: (callback: () => void) => { const id = ++timerId; timers.set(id, callback); return id; },
    clearTimeout: (id: number) => timers.delete(id),
    clampNumber: (value: number, min: number, max: number) => Math.min(max, Math.max(min, Math.round(value))),
    claudeLocalizationMemo: memo,
    claudeLocalMemo: options.localMemo ?? { status: localReady, autoDetected: true },
    announceBackendSettingsChanged: () => {},
    listen: () => Promise.resolve(() => {}),
    document: { querySelector: (selector: string) => (selector === "[data-topbar-actions]" ? topbarSlot : null) },
    createPortal: (child: unknown, target: typeof topbarSlot) => { target.portal = child; return null; },
    stringifyError: String,
    // Computer Use 接入状态不计入设置操作；同时记录 Bash 保存的参数和失败结果。
    invoke: (command: string, ...args: unknown[]) => command === "computer_use_status"
      ? Promise.resolve(cuStatus)
      : new Promise((resolve, reject) => calls.push({ command, args, resolve, reject })),
    useState: (initial: unknown) => {
      const index = cursor++;
      if (!(index in hooks)) hooks[index] = initial;
      return [hooks[index], (value: unknown) => {
        changes++;
        hooks[index] = typeof value === "function" ? (value as (current: unknown) => unknown)(hooks[index]) : value;
      }];
    },
    useRef: (initial: unknown) => {
      const index = cursor++;
      if (!(index in hooks)) hooks[index] = { current: initial };
      return hooks[index];
    },
    useEffect: effectHook,
    useLayoutEffect: effectHook,
  });
  vm.runInContext(`${source}\n${formatHelpers}\nglobalThis.render = ClaudeLocalizationScreen;`, context);
  let initiallyRendered = false;
  return {
    calls,
    render: () => {
      cursor = 0;
      let page = context.render(renderArgs) as Element;
      if (!initiallyRendered && options.openAdvanced !== false) {
        initiallyRendered = true;
        // 补丁页的副本块是常开结构，没有 details；完整功能模式里才需要手动展开。
        findElements(page, "details")[0]?.props.onToggle({ currentTarget: { open: true } });
        cursor = 0;
        page = context.render(renderArgs) as Element;
        cursor = 0;
        page = context.render(renderArgs) as Element;
      }
      return page;
    },
    /** 最近一次渲染时 portal 进页头的元素。 */
    topbar: () => topbarSlot.portal as Element | null,
    unmount: () => cleanups.forEach((cleanup) => cleanup()),
    timerCount: () => timers.size,
    runTimer: () => { const first = timers.entries().next().value; assert.ok(first); timers.delete(first[0]); first[1](); },
    changes: () => changes,
  };
}
function text(element: any): string {
  if (element == null || typeof element === "boolean") return "";
  if (Array.isArray(element)) return element.map(text).join("");
  if (typeof element !== "object") return String(element);
  // 骨架组件（CxRow / CxBanner …）的标题、说明放在 props 里，按显示顺序一起算进页面文字。
  const slots = String(element.type).startsWith("Cx") ? [element.props.icon, element.props.title, element.props.badges, element.props.detail] : [];
  return [...slots, element.props.children].map(text).join("");
}
function buttons(element: any): Element[] {
  if (element == null || typeof element !== "object") return [];
  if (Array.isArray(element)) return element.flatMap(buttons);
  // 页头的「启动增强版」是 Button 组件，页内操作是骨架的原生 button（图标按钮的文字在 sr-only 里）。
  // 检查、重新检测收进了三点菜单（MenuItem）。
  return [...(element.type === "Button" || element.type === "button" || element.type === "MenuItem" ? [element] : []), ...buttons(element.props.children)];
}
function inputs(element: any): Element[] {
  if (element == null || typeof element !== "object") return [];
  if (Array.isArray(element)) return element.flatMap(inputs);
  return [
    ...(element.type === "input" || element.type === "Input" ? [element] : []),
    ...inputs(element.props.children),
  ];
}
function findElements(element: any, type: string): Element[] {
  if (element == null || typeof element !== "object") return [];
  if (Array.isArray(element)) return element.flatMap((item) => findElements(item, type));
  return [...(element.type === type ? [element] : []), ...findElements(element.props.children, type)];
}
function enhancedLaunch(screen: ReturnType<typeof harness>) {
  return buttons(screen.render()).find((button) => button.props.title === "启动补丁版" || button.props.title === "制作副本并启动补丁版")!;
}
const tick = () => new Promise((resolve) => setImmediate(resolve));
const ready = {
  status: "ok", supported: true, prepared: true, needsRebuild: false,
  running: false, nodeAvailable: true, sourceVersion: "1.2.3", copyVersion: "1.2.3",
  catalogEntries: 1234, localization: true, voiceInput: true, extensions: false, computerUse: false,
  bashTimeout: { enabled: false, defaultEnabled: true, defaultSeconds: 120, maxEnabled: true, maxSeconds: 1800 },
};

test("首次进入自动检测一次，再次进入沿用上次结果；手动检测不并发", async () => {
  const memo: Memo = { status: null, autoDetected: false };
  const screen = harness(memo, {}, { mode: "patch" });
  let page = screen.render();
  assert.equal(screen.calls.length, 1, "首次进入应自动检测");
  assert.equal(screen.calls[0].command, "claude_localization_status");
  assert.match(text(page), /正在读取 Claude 安装状态/);
  // 自动检测还没回来时手动点，不能再发一次。
  buttons(screen.render()).find((button) => text(button) === "检测中…")!.props.onClick();
  assert.equal(screen.calls.length, 1);
  screen.calls[0].resolve({ ...ready, supported: false, prepared: false });
  await tick();
  page = screen.render();
  assert.match(text(page), /未检测到 Claude，请先安装官方应用/);
  assert.equal(buttons(page).find((button) => text(button) === "制作补丁版")!.props.disabled, true);
  screen.unmount();

  // Codex++ 没退出，再次进入：直接显示上次结果，不再自动探测。
  const again = harness(memo, {}, { mode: "patch" });
  page = again.render();
  assert.equal(again.calls.length, 0, "同一次运行内只自动检测一次");
  assert.match(text(page), /未检测到 Claude，请先安装官方应用/);
  buttons(page).find((button) => text(button) === "检测 Claude")!.props.onClick();
  assert.equal(again.calls.length, 1, "手动刷新仍然可用");
});

test("检测中切走页面后，迟到的检测结果不能写回已卸载页面", async () => {
  const memo: Memo = { status: null, autoDetected: false };
  const screen = harness(memo, {}, { mode: "patch" });
  screen.render();
  assert.equal(screen.calls.length, 1);
  screen.unmount();
  const before = screen.changes();
  screen.calls[0].resolve(ready);
  await tick();
  assert.equal(screen.changes(), before);
  // 结果仍记进 memo：回到页面时能看到这次检测，而不是又一片「未检测」。
  assert.match(text(harness(memo, {}, { mode: "patch" }).render()), /1\.2\.3/);
});

test("检查副本失败或检测超时保留最后有效状态，同时显示可重试错误", async () => {
  const screen = harness(undefined, {}, { mode: "patch" });
  screen.render();
  screen.calls[0].resolve(ready);
  await tick();
  buttons(screen.render()).find((button) => text(button) === "检查补丁版")!.props.onClick();
  screen.calls[1].resolve({ status: "failed", message: "副本检查失败", data: {} });
  await tick();
  assert.match(text(screen.render()), /1\.2\.3/);
  assert.match(text(screen.render()), /副本检查失败/);
  buttons(screen.render()).find((button) => text(button) === "检测 Claude")!.props.onClick();
  screen.calls[2].resolve({ status: "failed", message: "Claude 检测超时，检测进程已结束" });
  await tick();
  const page = screen.render();
  assert.match(text(page), /1\.2\.3/);
  assert.match(text(page), /检测超时/);
  assert.equal(buttons(page).find((button) => text(button) === "检测 Claude")!.props.disabled, false);
});

test("原版启动放在页头，默认不探测或制作增强副本", async () => {
  const localMemo: Memo = { status: null, autoDetected: false };
  const screen = harness(undefined, {}, { localMemo, openAdvanced: false });
  // 槽位在 layout effect 里取到，React 会在绘制前同步重渲染；harness 手动再渲染一次。
  screen.render();
  let page = screen.render();
  const launch = () => buttons(screen.topbar()).find((button) => text(button) === "启动 Claude");
  assert.ok(launch(), "启动按钮必须 portal 进 [data-topbar-actions]");
  assert.equal(buttons(page).some((button) => text(button) === "启动 Claude"), false, "页面主体里不应再有原版启动按钮");
  assert.equal(launch()!.props.disabled, true, "检测结果回来前不能启动");
  assert.equal(screen.calls[0].command, "claude_local_status");
  screen.calls[0].resolve(localReady);
  await tick();
  page = screen.render();
  assert.equal(launch()!.props.disabled, false);
  launch()!.props.onClick();
  assert.equal(screen.calls[1].command, "claude_local_action");
  assert.equal(JSON.stringify(screen.calls[1].args[0]), JSON.stringify({ action: "launch" }));
  assert.equal(screen.calls.some((call) => call.command.startsWith("claude_localization")), false);
  screen.render();
  assert.ok(buttons(screen.topbar()).some((button) => text(button) === "启动中…"));
});

test("补丁版电脑控制与 Claude Desktop 接入的 codex-plus-cu 同时开时，提示二选一", async () => {
  const conflict = { targets: { claudeDesktop: { state: "registered" } } };
  const screen = harness(undefined, conflict, { mode: "patch" });
  screen.render();
  screen.calls[0].resolve({ ...ready, computerUse: true, computerUseSupported: true });
  await tick();
  const warning = (page: Element) => JSON.stringify(page).includes("is-warning");
  assert.ok(warning(screen.render()), "两套都开时应显示冲突提醒");

  const quiet = harness(undefined, { targets: { claudeDesktop: { state: "missing" } } }, { mode: "patch" });
  quiet.render();
  quiet.calls[0].resolve({ ...ready, computerUse: true, computerUseSupported: true });
  await tick();
  const page = quiet.render();
  assert.equal(warning(page), false, "未接入时不应显示冲突提醒");
  // 二选一的说明收进 Computer Use 那一行的悬停提示，不再常驻占一行。
  assert.ok(JSON.stringify(page).includes("二选一"), "仍保留一句二选一的说明");
  assert.equal(text(page).includes("两套独立实现"), false);
});

test("Claude 增强页不再重复提供万物皆拓展开关，保留其他增强项", () => {
  for (const extensions of [false, true]) {
    const screen = harness({ status: { ...ready, extensions }, autoDetected: true }, {}, { mode: "patch" });
    const page = screen.render();
    assert.equal(inputs(page).some((input) => input.props["aria-label"] === "万物皆拓展"), false);
    assert.doesNotMatch(text(page), /万物皆拓展/);
    for (const feature of ["界面汉化", "语音输入", "Computer Use", "解锁 Bash 时间限制"]) {
      assert.ok(text(page).includes(feature), feature);
    }
    assert.equal(screen.calls.length, 0);
  }
});

test("Bash 时间限制可分别修改默认值和最大值", async () => {
  const screen = harness(undefined, {}, { mode: "patch" });
  screen.render();
  screen.calls[0].resolve(ready);
  await tick();
  let page = screen.render();
  assert.match(text(page), /解锁 Bash 时间限制/);
  const toggle = inputs(page).find((input) => input.props["aria-label"] === "解锁 Bash 时间限制");
  assert.ok(toggle);
  toggle.props.onChange({ currentTarget: { checked: true } });
  assert.equal(screen.calls[1].command, "claude_localization_set_bash_timeout");
  screen.calls[1].resolve({
    ...ready,
    bashTimeout: { ...ready.bashTimeout, enabled: true },
  });
  await tick();
  page = screen.render();
  assert.match(text(page), /修改默认等待时间/);
  assert.match(text(page), /修改最大等待时间/);
  const defaultValue = inputs(page).find((input) => input.props["aria-label"] === "默认等待时间（秒）");
  assert.equal(defaultValue!.props.value, 120);
  defaultValue!.props.onChange({ currentTarget: { value: "300" } });
  page = screen.render();
  inputs(page).find((input) => input.props["aria-label"] === "默认等待时间（秒）")!.props.onBlur();
  assert.equal(screen.calls[2].command, "claude_localization_set_bash_timeout");
  assert.deepEqual(
    (screen.calls[2].args[0] as { timeout: { defaultSeconds: number } }).timeout.defaultSeconds,
    300,
  );
});

test("Cowork 适配警告不覆盖启动成功状态，其他增强保持可用", async () => {
  const screen = harness({ status: { ...ready, bashTimeout: { ...ready.bashTimeout, enabled: true } }, autoDetected: true }, {}, { mode: "patch" });
  screen.render();
  screen.render();
  enhancedLaunch(screen).props.onClick();
  screen.calls[0].resolve({ ...ready, running: true, bashTimeout: { ...ready.bashTimeout, enabled: true }, bashTimeoutWarning: "不支持的 Cowork Bash 参数校验结构" });
  await tick();
  const page = screen.render();
  assert.ok(text(page).includes("不支持的 Cowork Bash 参数校验结构"));
  assert.match(text(page), /不支持的 Cowork Bash 参数校验结构/);
  assert.equal(enhancedLaunch(screen).props.disabled, false);
  assert.equal(inputs(page).find((input) => String(input.props.onChange).includes("toggleLocalization"))!.props.disabled, false);
  assert.equal(inputs(page).find((input) => String(input.props.onChange).includes("toggleVoiceInput"))!.props.disabled, false);
});

test("Cowork 已准备但未确认加载时显示中性提示，检测后保持可见且不阻止启动", async () => {
  const pendingStatus = {
    ...ready,
    bashTimeout: { ...ready.bashTimeout, enabled: true },
    bashTimeoutWarning: "已准备，暂未确认。将在补丁版启动后自动确认。",
  };
  const screen = harness({ status: pendingStatus, autoDetected: true }, {}, { mode: "patch" });
  screen.render();
  let page = screen.render();
  assert.match(text(page), /Bash 时限补丁： 已准备，暂未确认。将在补丁版启动后自动确认。/);
  assert.doesNotMatch(text(page), /Cowork Bash 时限补丁未生效/);
  assert.equal(enhancedLaunch(screen).props.disabled, false);
  buttons(page).find((button) => text(button) === "检测 Claude")!.props.onClick();
  assert.equal(screen.calls[0].command, "claude_localization_status");
  screen.calls[0].resolve(pendingStatus);
  await tick();
  page = screen.render();
  assert.match(text(page), /Bash 时限补丁： 已准备，暂未确认。将在补丁版启动后自动确认。/);
  assert.doesNotMatch(text(page), /Cowork Bash 时限补丁未生效/);
  assert.equal(enhancedLaunch(screen).props.disabled, false);
});

test("启动后自动确认并撤下顶部黄字提示，每次启动重新确认且不重置输入草稿", async () => {
  const pendingStatus = { ...ready, running: true, bashTimeout: { ...ready.bashTimeout, enabled: true },
    bashTimeoutPending: true, bashTimeoutWarning: "已准备，暂未确认。将在补丁版启动后自动确认。" };
  const screen = harness({ status: ready, autoDetected: true }, {}, { mode: "patch" });
  screen.render(); screen.render();
  enhancedLaunch(screen).props.onClick();
  screen.calls[0].resolve(pendingStatus);
  await tick();
  let page = screen.render();
  assert.match(text(page), /已准备，暂未确认/);
  assert.ok(text(page).indexOf("已准备，暂未确认") < text(page).indexOf("重新制作补丁版"));
  const advanced = findElements(page, "section")[0];
  assert.ok(advanced, "补丁页应有常开的副本块");
  assert.ok(JSON.stringify(advanced).indexOf("claude-l10n-warning") < JSON.stringify(advanced).indexOf("claude-l10n-actions"));
  assert.equal(screen.timerCount(), 1);
  inputs(page).find((input) => input.props["aria-label"] === "默认等待时间（秒）")!.props.onChange({ currentTarget: { value: "456" } });
  screen.runTimer();
  assert.equal(screen.calls[1].command, "claude_localization_bash_timeout_status");
  screen.calls[1].resolve({ status: "ok", warning: null, pending: false });
  await tick();
  page = screen.render();
  assert.doesNotMatch(text(page), /已准备，暂未确认/);
  assert.equal(inputs(page).find((input) => input.props["aria-label"] === "默认等待时间（秒）")!.props.value, "456");
  assert.equal(screen.timerCount(), 0);
  enhancedLaunch(screen).props.onClick();
  screen.calls[2].resolve(pendingStatus);
  await tick();
  page = screen.render();
  assert.match(text(page), /已准备，暂未确认/);
  assert.equal(screen.timerCount(), 1);
});

test("重新启动时，上一次尚在返回的确认不能撤下本次启动提示", async () => {
  const pendingStatus = { ...ready, running: true, bashTimeout: { ...ready.bashTimeout, enabled: true },
    bashTimeoutPending: true, bashTimeoutWarning: "已准备，暂未确认。将在补丁版启动后自动确认。" };
  const screen = harness({ status: pendingStatus, autoDetected: true }, {}, { mode: "patch" });
  screen.render(); screen.render();
  screen.runTimer();
  enhancedLaunch(screen).props.onClick();
  assert.match(text(screen.render()), /已准备，暂未确认/, "启动过程中保留提示，不能提前撤下");
  screen.calls[1].resolve(pendingStatus);
  await tick(); screen.render();
  screen.calls[0].resolve({ status: "ok", warning: null, pending: false });
  await tick();
  assert.match(text(screen.render()), /已准备，暂未确认/);
  assert.equal(screen.timerCount(), 1);
});

test("确认读取失败会重试，离开页面后取消轮询且迟到结果不写状态", async () => {
  const memo = { status: { ...ready, running: true, bashTimeout: { ...ready.bashTimeout, enabled: true },
    bashTimeoutPending: true, bashTimeoutWarning: "已准备，暂未确认。将在补丁版启动后自动确认。" }, autoDetected: true };
  const screen = harness(memo, {}, { mode: "patch" });
  screen.render(); screen.render();
  screen.runTimer();
  screen.calls[0].reject(new Error("temporary read error"));
  await tick(); screen.render();
  assert.equal(screen.timerCount(), 1);
  screen.runTimer();
  screen.unmount();
  const changes = screen.changes();
  screen.calls[1].resolve({ status: "ok", warning: null, pending: false });
  await tick();
  assert.equal(screen.changes(), changes);
  assert.equal(screen.timerCount(), 0);
  assert.equal(memo.status.bashTimeoutPending, true);
});

test("Bash 总开关保存失败后恢复已保存值，并允许重试", async () => {
  const screen = harness({ status: ready, autoDetected: true }, {}, { mode: "patch" });
  const toggle = () => inputs(screen.render()).find((input) => input.props["aria-label"] === "解锁 Bash 时间限制")!;
  toggle().props.onChange({ currentTarget: { checked: true } });
  screen.calls[0].resolve({ status: "failed", message: "写入失败", data: {} });
  await tick();
  assert.equal(toggle().props.checked, false);
  assert.match(text(screen.render()), /写入失败/);
  assert.equal(toggle().props.disabled, false);
  toggle().props.onChange({ currentTarget: { checked: true } });
  assert.equal(screen.calls.length, 2);
  screen.calls[1].resolve({ ...ready, bashTimeout: { ...ready.bashTimeout, enabled: true } });
  await tick();
  assert.equal(toggle().props.checked, true);
});

test("Bash 数字保存抛出异常时恢复已保存值，而非编辑草稿", async () => {
  const saved = { ...ready, bashTimeout: { ...ready.bashTimeout, enabled: true } };
  const screen = harness({ status: saved, autoDetected: true }, {}, { mode: "patch" });
  const input = () => inputs(screen.render()).find((input) => input.props["aria-label"] === "默认等待时间（秒）")!;
  input().props.onChange({ currentTarget: { value: "300" } });
  input().props.onBlur();
  screen.calls[0].reject(new Error("连接失败"));
  await tick();
  assert.equal(input().props.value, 120);
  assert.match(text(screen.render()), /连接失败/);
});

test("Bash 数字失焦后点击开关串行保存，不丢点击、不被旧响应覆盖", async () => {
  const saved = { ...ready, bashTimeout: { ...ready.bashTimeout, enabled: true } };
  const screen = harness({ status: saved, autoDetected: true }, {}, { mode: "patch" });
  const input = (name: string) => inputs(screen.render()).find((item) => item.props["aria-label"] === name)!;
  input("默认等待时间（秒）").props.onChange({ currentTarget: { value: "300" } });
  input("默认等待时间（秒）").props.onBlur();
  assert.equal(input("修改最大等待时间").props.disabled, false);
  input("修改最大等待时间").props.onChange({ currentTarget: { checked: false } });
  assert.equal(screen.calls.length, 1, "前一次保存完成前不并发写入");
  screen.calls[0].resolve({ ...saved, bashTimeout: { ...saved.bashTimeout, defaultSeconds: 300 } });
  await tick();
  assert.equal(screen.calls.length, 2);
  assert.equal(input("修改最大等待时间").props.checked, false, "旧响应不能覆盖后续点击");
  const timeout = (screen.calls[1].args[0] as { timeout: typeof saved.bashTimeout }).timeout;
  assert.equal(timeout.defaultSeconds, 300);
  assert.equal(timeout.maxEnabled, false);
  screen.calls[1].resolve({ ...saved, bashTimeout: timeout });
  await tick();
  assert.equal(input("默认等待时间（秒）").props.value, 300);
  assert.equal(input("修改最大等待时间").props.checked, false);
  assert.equal(input("默认等待时间（秒）").props.disabled, false);
});

test("Bash 排队保存失败回到最后一次成功值，离开页面后不写入组件状态", async () => {
  const saved = { ...ready, bashTimeout: { ...ready.bashTimeout, enabled: true } };
  const memo: Memo = { status: saved, autoDetected: true };
  const screen = harness(memo, {}, { mode: "patch" });
  const input = (name: string) => inputs(screen.render()).find((item) => item.props["aria-label"] === name)!;
  input("默认等待时间（秒）").props.onChange({ currentTarget: { value: "300" } });
  input("默认等待时间（秒）").props.onBlur();
  input("修改最大等待时间").props.onChange({ currentTarget: { checked: false } });
  screen.calls[0].resolve({ ...saved, bashTimeout: { ...saved.bashTimeout, defaultSeconds: 300 } });
  await tick();
  screen.calls[1].resolve({ status: "failed", message: "第二次保存失败" });
  await tick();
  assert.equal(input("默认等待时间（秒）").props.value, 300);
  assert.equal(input("修改最大等待时间").props.checked, true);
  input("修改最大等待时间").props.onChange({ currentTarget: { checked: false } });
  screen.unmount();
  const before = screen.changes();
  screen.calls[2].resolve({ ...saved, bashTimeout: { ...saved.bashTimeout, defaultSeconds: 300, maxEnabled: false } });
  await tick();
  assert.equal(screen.changes(), before);
  assert.equal(inputs(harness(memo, {}, { mode: "patch" }).render()).find((item) => item.props["aria-label"] === "修改最大等待时间")!.props.checked, false);
});

test("Bash 秒数允许清空后重输，失焦才显示规范值并保存数字", async () => {
  const saved = { ...ready, bashTimeout: { ...ready.bashTimeout, enabled: true } };
  const screen = harness({ status: saved, autoDetected: true }, {}, { mode: "patch" });
  const input = (name: string) => inputs(screen.render()).find((item) => item.props["aria-label"] === name)!;
  for (const [name, field, value] of [
    ["默认等待时间（秒）", "defaultSeconds", "1"],
    ["最大等待时间（秒）", "maxSeconds", "3"],
  ] as const) {
    input(name).props.onChange({ currentTarget: { value: "" } });
    assert.equal(input(name).props.value, "", "清空时不能立即补 1，否则新数字会被拼接");
    input(name).props.onChange({ currentTarget: { value } });
    assert.equal(input(name).props.value, value);
    input(name).props.onBlur();
    const call = screen.calls.at(-1)!;
    const timeout = (call.args[0] as { timeout: typeof saved.bashTimeout }).timeout;
    assert.equal(timeout[field], Number(value));
    assert.equal(typeof timeout[field], "number");
    call.resolve({ ...saved, bashTimeout: timeout });
    await tick();
    assert.equal(input(name).props.value, Number(value));
  }
});

test("仅手动启动可选补丁版时先制作副本；制作失败就不启动", async () => {
  const unprepared = { ...ready, prepared: false, copyVersion: "" };
  const screen = harness({ status: unprepared, autoDetected: true }, {}, { mode: "patch" });
  screen.render(); screen.render();
  const launch = () => enhancedLaunch(screen);
  assert.equal(launch().props.disabled, false, "没制作也能直接点启动");
  launch().props.onClick();
  assert.equal(JSON.stringify(screen.calls[0].args[0]), JSON.stringify({ action: "prepare" }));
  screen.render();
  assert.equal(text(launch()), "制作中…");
  screen.calls[0].resolve(ready);
  await tick();
  assert.equal(JSON.stringify(screen.calls[1].args[0]), JSON.stringify({ action: "launch" }), "制作成功后自动启动");

  const failing = harness({ status: unprepared, autoDetected: true }, {}, { mode: "patch" });
  failing.render(); failing.render();
  enhancedLaunch(failing).props.onClick();
  failing.calls[0].resolve({ status: "failed", message: "制作失败", data: {} });
  await tick();
  assert.equal(failing.calls.length, 1, "制作失败不能继续启动");
  assert.match(text(failing.render()), /制作失败/);
});

test("启动模式下拉独立保存 3p 与完整功能配置，两条原版启动均不制作副本", async () => {
  const screen = harness(undefined, {}, { openAdvanced: false });
  const mode = () => findElements(screen.render(), "AppSelect").find((item) => item.props.ariaLabel === "启动模式")!;
  assert.equal(mode().props.value, "3p");
  assert.equal(inputs(screen.render()).some((item) => item.props["aria-label"] === "本地原生听写"), false);
  mode().props.onChange("full");
  assert.equal(screen.calls[0].command, "claude_local_configure");
  const fullConfig = (screen.calls[0].args[0] as { config: typeof localReady.config }).config;
  assert.deepEqual(structuredClone(fullConfig), { ...localReady.config, enabled: true });
  screen.calls[0].resolve({ ...localReady, config: fullConfig });
  await tick();
  assert.equal(mode().props.value, "full");
  assert.ok(inputs(screen.render()).some((item) => item.props["aria-label"] === "本地原生听写"));
  screen.topbar()!.props.onClick();
  assert.equal(screen.calls[1].command, "claude_local_action");
  assert.equal(JSON.stringify(screen.calls[1].args[0]), JSON.stringify({ action: "launch" }));
  screen.calls[1].resolve({ ...localReady, config: fullConfig });
  await tick();
  mode().props.onChange("3p");
  const threePConfig = (screen.calls[2].args[0] as { config: typeof localReady.config }).config;
  assert.equal(threePConfig.enabled, false);
  screen.calls[2].resolve({ ...localReady, config: threePConfig });
  await tick(); screen.render();
  screen.topbar()!.props.onClick();
  assert.equal(screen.calls[3].command, "claude_local_action");
  assert.equal(screen.calls.some((call) => call.command.startsWith("claude_localization")), false);
});

test("原版启动与模式保存失败保留配置，不自动回退增强副本", async () => {
  const memo: Memo = { status: localReady, autoDetected: true };
  const screen = harness(undefined, {}, { localMemo: memo, openAdvanced: false });
  screen.render(); screen.render();
  screen.topbar()!.props.onClick();
  screen.calls[0].resolve({ status: "failed", message: "本地服务启动失败", data: {} });
  await tick();
  assert.match(text(screen.render()), /本地服务启动失败/);
  assert.equal(screen.topbar()!.props.disabled, false);
  const mode = findElements(screen.render(), "AppSelect").find((item) => item.props.ariaLabel === "启动模式")!;
  mode.props.onChange("full");
  screen.calls[1].reject(new Error("配置写入失败"));
  await tick();
  assert.match(text(screen.render()), /配置写入失败/);
  assert.equal(findElements(screen.render(), "AppSelect").find((item) => item.props.ariaLabel === "启动模式")!.props.value, "3p");
  assert.equal(memo.status, localReady);
  assert.equal(screen.calls.some((call) => call.command.startsWith("claude_localization")), false);
});

test("用户名与套餐：默认 Ultra，改名与换套餐各发一次 claude_local_configure", async () => {
  const screen = harness(undefined, {}, { localMemo: { status: localReady, autoDetected: true }, openAdvanced: false });
  const selects = findElements(screen.render(), "AppSelect");
  const plan = selects.find((item) => item.props.ariaLabel === "套餐")!;
  assert.equal(plan.props.value, "ultra");
  plan.props.onChange("free");
  await tick();
  assert.equal(screen.calls[0].command, "claude_local_configure");
  assert.equal((screen.calls[0].args[0] as { config: { plan: string } }).config.plan, "free");
  // 上一次保存未完成时 configureLocal 会被丢弃：先让它落定再改用户名。
  screen.calls[0].resolve({ ...localReady, config: { ...localReady.config, plan: "free" } });
  await tick();
  const name = findElements(screen.render(), "Input").find((item) => item.props["aria-label"] === "用户名")!;
  name.props.onBlur({ currentTarget: { value: "张三" } });
  await tick();
  assert.equal(screen.calls[1].command, "claude_local_configure");
  assert.equal((screen.calls[1].args[0] as { config: { displayName: string } }).config.displayName, "张三");
  screen.calls[1].resolve({ ...localReady, config: { ...localReady.config, plan: "free", displayName: "张三" } });
  await tick();
  const email = findElements(screen.render(), "Input").find((item) => item.props["aria-label"] === "邮箱")!;
  assert.equal(email.props.placeholder, "local@claude.local");
  email.props.onBlur({ currentTarget: { value: "me@example.com" } });
  await tick();
  assert.equal(screen.calls[2].command, "claude_local_configure");
  assert.equal((screen.calls[2].args[0] as { config: { email: string } }).config.email, "me@example.com");
});

test("会话同步：开关默认关闭，切换走 configure，立即同步走 action", async () => {
  const screen = harness(undefined, {}, { localMemo: { status: localReady, autoDetected: true }, openAdvanced: false });
  const sync = findElements(screen.render(), "input").find((item) => item.props["aria-label"] === "会话同步")!;
  assert.equal(sync.props.checked, false);
  sync.props.onChange({ currentTarget: { checked: true } });
  await tick();
  assert.equal(screen.calls[0].command, "claude_local_configure");
  assert.equal((screen.calls[0].args[0] as { config: { sessionSync: boolean } }).config.sessionSync, true);
  screen.calls[0].resolve({ ...localReady, config: { ...localReady.config, sessionSync: true } });
  await tick();
  const syncNow = buttons(screen.render()).find((item) => item.props.title === "立即同步")!;
  syncNow.props.onClick();
  await tick();
  assert.equal(screen.calls[1].command, "claude_local_action");
  assert.equal((screen.calls[1].args[0] as { action: string }).action, "sync_sessions");
});

test("默认模型不在这一页选：只留一句说明，不放下拉也不放跳转", () => {
  const screen = harness(undefined, {}, {
    localMemo: { status: { ...localReady, config: { ...localReady.config, enabled: true } }, autoDetected: true },
    openAdvanced: false,
  });
  const page = screen.render();
  assert.equal(findElements(page, "AppSelect").some((item) => item.props.ariaLabel === "Claude 默认模型"), false, "不再有默认模型下拉");
  assert.equal(findElements(page, "AppSelect").some((item) => item.props.ariaLabel === "启动模式"), true, "启动模式那一行还在");
  assert.match(text(page), /模型在「Agent 接入」里的 Claude Desktop 行选择，改完需要重新启动本地服务。/);
  assert.equal(buttons(page).some((button) => text(button).includes("去「Agent 接入」")), false, "不给跳转按钮");
  // 模型字段还留在配置类型里（后端旧值兜底），页面只是不再让人在这里改。
  assert.equal(screen.calls.length, 0, "页面不写配置");
});

test("完整功能模式的四项能力分别保存，不修改补丁版开关或代点系统权限", async () => {
  const saved = { ...localReady, config: { ...localReady.config, enabled: true } };
  const screen = harness(undefined, {}, { localMemo: { status: saved, autoDetected: true }, openAdvanced: false });
  let config = saved.config;
  for (const [index, label, key] of [
    [0, "本地界面汉化", "localization"],
    [1, "本地原生听写", "voiceInput"],
    [2, "本地电脑控制", "computerUse"],
    [3, "本地后台模式", "backgroundMode"],
  ] as const) {
    const toggle = inputs(screen.render()).find((item) => item.props["aria-label"] === label)!;
    toggle.props.onChange({ currentTarget: { checked: !config[key] } });
    const next = (screen.calls[index].args[0] as { config: typeof saved.config }).config;
    assert.deepEqual(structuredClone(next), { ...config, [key]: !config[key] });
    assert.equal(screen.calls[index].command, "claude_local_configure");
    config = next;
    screen.calls[index].resolve({ ...saved, config });
    await tick();
  }
  assert.equal(screen.calls.length, 4);
  assert.equal(screen.calls.some((call) => /permission|computer_use|claude_localization/.test(call.command)), false);
});

test("原版显示证书准备与信任操作，本地服务在线不冒充 Claude 已连接", async () => {
  const saved = {
    ...localReady, running: true, claudeRunning: true, connected: false,
    caReady: false, caTrusted: false, dictationReady: false,
    config: { ...localReady.config, enabled: true },
    needsRestart: true, warning: "请确认客户端版本",
  };
  const screen = harness(undefined, {}, { localMemo: { status: saved, autoDetected: true }, openAdvanced: false });
  let page = screen.render();
  assert.match(text(page), /原版运行，未连接本地服务/);
  assert.doesNotMatch(text(page), /本地模式已连接/);
  assert.match(text(page), /完全退出 Claude 后重新启动/);
  assert.match(text(page), /请确认客户端版本/);
  assert.equal(buttons(page).find((button) => text(button) === "信任证书")!.props.disabled, true);
  buttons(page).find((button) => text(button) === "准备证书")!.props.onClick();
  assert.equal(JSON.stringify(screen.calls[0].args[0]), JSON.stringify({ action: "prepare" }));
  screen.calls[0].resolve({ ...saved, caReady: true });
  await tick();
  page = screen.render();
  buttons(page).find((button) => text(button) === "信任证书")!.props.onClick();
  assert.equal(JSON.stringify(screen.calls[1].args[0]), JSON.stringify({ action: "trust" }));
  screen.calls[1].resolve({ ...saved, caReady: true, caTrusted: true, connected: true, needsRestart: false, warning: null });
  await tick();
  page = screen.render();
  assert.match(text(page), /本地模式已连接/);
  // 已信任时同一个位置换成「取消信任证书」，走 claude_local_action("untrust")。
  assert.equal(buttons(page).some((button) => text(button) === "信任证书"), false, "已信任时不再重复给信任按钮");
  buttons(page).find((button) => text(button) === "取消信任证书")!.props.onClick();
  assert.equal(JSON.stringify(screen.calls[2].args[0]), JSON.stringify({ action: "untrust" }));
  screen.calls[2].resolve({ ...saved, caReady: true, caTrusted: false, connected: false, needsRestart: false, warning: null });
  await tick();
  page = screen.render();
  assert.match(text(page), /未信任/);
  assert.ok(buttons(page).find((button) => text(button) === "信任证书"));
});

test("原版检测跨挂载只自动一次，迟到检测不更新已卸载页面", async () => {
  const memo: Memo = { status: null, autoDetected: false };
  const screen = harness(undefined, {}, { localMemo: memo, openAdvanced: false });
  screen.render();
  assert.equal(screen.calls.length, 1);
  buttons(screen.render()).find((button) => text(button) === "检测原版 Claude")!.props.onClick();
  assert.equal(screen.calls.length, 1, "检测进行中不并发");
  screen.unmount();
  const changes = screen.changes();
  screen.calls[0].resolve(localReady);
  await tick();
  assert.equal(screen.changes(), changes);
  const again = harness(undefined, {}, { localMemo: memo, openAdvanced: false });
  assert.match(text(again.render()), /原版版本 1.2.3/);
  assert.equal(again.calls.length, 0);
});

test("原版轮询的旧结果不能覆盖新配置，卸载后清理轮询", async () => {
  const saved = { ...localReady, running: true, claudeRunning: true, config: { ...localReady.config, enabled: true } };
  const screen = harness(undefined, {}, { localMemo: { status: saved, autoDetected: true }, openAdvanced: false });
  screen.render(); screen.render();
  assert.equal(screen.timerCount(), 1);
  screen.runTimer();
  assert.equal(screen.calls[0].command, "claude_local_status");
  const mode = findElements(screen.render(), "AppSelect").find((item) => item.props.ariaLabel === "启动模式")!;
  mode.props.onChange("3p");
  screen.render();
  const config = (screen.calls[1].args[0] as { config: typeof saved.config }).config;
  screen.calls[1].resolve({ ...saved, config });
  await tick(); screen.render();
  screen.calls[0].resolve(saved);
  await tick();
  assert.equal(findElements(screen.render(), "AppSelect").find((item) => item.props.ariaLabel === "启动模式")!.props.value, "3p");
  assert.equal(screen.timerCount(), 1);
  screen.runTimer();
  screen.unmount();
  const changes = screen.changes();
  screen.calls[2].resolve(saved);
  await tick();
  assert.equal(screen.changes(), changes);
  assert.equal(screen.timerCount(), 0);
});

test("上下文窗口按供应商声明显示，未声明时说明 Claude 按 200k 兜底", () => {
  const declared = harness(undefined, {}, {
    localMemo: { status: { ...localReady, contextWindow: 1_100_000 }, autoDetected: true },
    openAdvanced: false,
  });
  assert.match(text(declared.render()), /上下文窗口 1\.1M（供应商声明）/);

  const undeclared = harness(undefined, {}, {
    localMemo: { status: { ...localReady, contextWindow: null }, autoDetected: true },
    openAdvanced: false,
  });
  assert.match(text(undeclared.render()), /上下文窗口 未声明（Claude 按 200k）/);
});


test("补丁 Desktop 页：常开的副本块，无重复说明卡、没有本地状态与页头启动", () => {
  const screen = harness({ status: ready, autoDetected: true }, {}, { mode: "patch" });
  const page = screen.render();
  assert.equal(JSON.stringify(page).includes("claude-l10n-patch-intro"), false, "补丁页不再有与标题重复的说明卡");
  assert.doesNotMatch(text(page), /给改过代码的 Desktop 副本用/);
  assert.doesNotMatch(text(page), /关闭 asar 完整性校验/);
  assert.ok(findElements(page, "section").length > 0, "副本块是常开的 section");
  assert.equal(findElements(page, "details").length, 0, "补丁页不再用折叠块");
  assert.match(text(page), /补丁版 Claude/);
  assert.equal(screen.topbar(), null, "补丁页不往页头塞原版启动按钮");
  assert.doesNotMatch(text(page), /增强/, "补丁页整体不再出现「增强」字眼");
  assert.doesNotMatch(text(page), /Claude Desktop 卡|本地证书|运行环境/);
  assert.doesNotMatch(text(page), /账号、套餐与用量显示来自本地合成数据/);
});

test("Desktop 增强页不再提副本：没有副本块，也没有指向补丁页的提示或按钮", () => {
  const screen = harness(undefined, {}, { localMemo: { status: localReady, autoDetected: true }, openAdvanced: false });
  const page = screen.render();
  assert.doesNotMatch(text(page), /搬到了「补丁 Desktop」页/);
  assert.doesNotMatch(text(page), /打开「补丁 Desktop」/);
  assert.equal(text(page).includes("补丁版 Claude"), false, "副本块不再出现在完整功能模式");
  assert.equal(findElements(page, "details").length, 0, "完整功能模式里连折叠块也没有了");
  assert.equal(buttons(page).some((button) => text(button).includes("补丁 Desktop")), false, "不放跳转按钮");
});

test("上下文窗口格式化：1M / 1.1M / 200k，非正数与缺省不出东西", () => {
  const formatWindow = vm.runInContext(formatHelpers + "\nformatContextWindow", vm.createContext({}));
  assert.equal(formatWindow(1_000_000), "1M");
  assert.equal(formatWindow(1_100_000), "1.1M");
  assert.equal(formatWindow(200_000), "200k");
  assert.equal(formatWindow(16_000), "16k");
  assert.equal(formatWindow(null), "");
  assert.equal(formatWindow(0), "");
  assert.equal(formatWindow(-1), "");
});

test("页面里启动 / 停止后把本地状态交给 App，侧栏的播放键不用轮询也能跟着变", async () => {
  const seen: Array<{ running?: boolean; status?: string }> = [];
  const screen = harness(undefined, {}, {
    localMemo: { status: localReady, autoDetected: true },
    openAdvanced: false,
    onLocalStatus: (status) => seen.push(status as { running?: boolean; status?: string }),
  });
  screen.render();
  assert.equal(seen.length, 0, "只是渲染不回调（状态没变）");
  buttons(screen.render()).find((button) => text(button) === "检测原版 Claude")!.props.onClick();
  assert.equal(screen.calls[0].command, "claude_local_status");
  screen.calls[0].resolve({ ...localReady, running: true });
  await tick();
  assert.equal(seen.length, 1, "读到新状态就交给 App");
  assert.equal(seen[0].running, true);
  assert.equal(seen[0].status, "ok");

  // 失败结果不回调：App 那边保留上一份有效状态。
  buttons(screen.render()).find((button) => text(button) === "停止本地服务")!.props.onClick();
  screen.calls[1].resolve({ status: "failed", message: "停止失败" });
  await tick();
  assert.equal(seen.length, 1, "失败不覆盖 App 里的状态");
});
