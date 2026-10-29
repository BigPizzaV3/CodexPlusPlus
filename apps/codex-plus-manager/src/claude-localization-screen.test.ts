import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import test from "node:test";
import ts from "typescript";

// 独立运行真实页面函数和事件处理器，替换 hooks/IPC；不启动本机 Claude 或探测 PATH。
const app = ts.createSourceFile("App.tsx", readFileSync(new URL("./App.tsx", import.meta.url), "utf8"), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const component = app.statements.find((node): node is ts.FunctionDeclaration => ts.isFunctionDeclaration(node) && node.name?.text === "ClaudeLocalizationScreen");
assert.ok(component);
const source = ts.transpileModule(component.getText(app), {
  compilerOptions: { jsx: ts.JsxEmit.React, target: ts.ScriptTarget.ES2022 },
}).outputText;

type Element = { type: string; props: Record<string, any> };
type Memo = { status: unknown; autoDetected: boolean };
/** memo 对应模块级的 claudeLocalizationMemo：传同一个对象模拟「Codex++ 运行期间再次进入页面」。 */
function harness(memo: Memo = { status: null, autoDetected: false }, cuStatus: unknown = {}) {
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
    React: { createElement: (type: string, props: object, ...children: unknown[]) => ({ type, props: { ...props, children } }) },
    ...Object.fromEntries(["Panel", "CardHead", "CardContent", "Button", "Input", "Info", "RefreshCw", "ToggleVisual", "PackageOpen", "Play", "Rocket", "ShieldCheck"].map((name) => [name, name])),
    t: (text: string) => text,
    setTimeout: (callback: () => void) => { const id = ++timerId; timers.set(id, callback); return id; },
    clearTimeout: (id: number) => timers.delete(id),
    clampNumber: (value: number, min: number, max: number) => Math.min(max, Math.max(min, Math.round(value))),
    claudeLocalizationMemo: memo,
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
  vm.runInContext(`${source}\nglobalThis.render = ClaudeLocalizationScreen;`, context);
  return {
    calls,
    render: () => { cursor = 0; return context.render() as Element; },
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
  return typeof element === "object" ? text(element.props.children) : String(element);
}
function buttons(element: any): Element[] {
  if (element == null || typeof element !== "object") return [];
  if (Array.isArray(element)) return element.flatMap(buttons);
  return [...(element.type === "Button" ? [element] : []), ...buttons(element.props.children)];
}
function inputs(element: any): Element[] {
  if (element == null || typeof element !== "object") return [];
  if (Array.isArray(element)) return element.flatMap(inputs);
  return [
    ...(element.type === "input" || element.type === "Input" ? [element] : []),
    ...inputs(element.props.children),
  ];
}
const tick = () => new Promise((resolve) => setImmediate(resolve));
const ready = {
  status: "ok", supported: true, prepared: true, needsRebuild: false,
  running: false, nodeAvailable: true, sourceVersion: "1.2.3", copyVersion: "1.2.3",
  catalogEntries: 1234, localization: true, voiceInput: true, computerUse: false,
  bashTimeout: { enabled: false, defaultEnabled: true, defaultSeconds: 120, maxEnabled: true, maxSeconds: 1800 },
};

test("首次进入自动检测一次，再次进入沿用上次结果；手动检测不并发", async () => {
  const memo: Memo = { status: null, autoDetected: false };
  const screen = harness(memo);
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
  assert.equal(buttons(page).find((button) => text(button) === "制作增强版")!.props.disabled, true);
  screen.unmount();

  // Codex++ 没退出，再次进入：直接显示上次结果，不再自动探测。
  const again = harness(memo);
  page = again.render();
  assert.equal(again.calls.length, 0, "同一次运行内只自动检测一次");
  assert.match(text(page), /未检测到 Claude，请先安装官方应用/);
  buttons(page).find((button) => text(button) === "检测 Claude")!.props.onClick();
  assert.equal(again.calls.length, 1, "手动刷新仍然可用");
});

test("检测中切走页面后，迟到的检测结果不能写回已卸载页面", async () => {
  const memo: Memo = { status: null, autoDetected: false };
  const screen = harness(memo);
  screen.render();
  assert.equal(screen.calls.length, 1);
  screen.unmount();
  const before = screen.changes();
  screen.calls[0].resolve(ready);
  await tick();
  assert.equal(screen.changes(), before);
  // 结果仍记进 memo：回到页面时能看到这次检测，而不是又一片「未检测」。
  assert.match(text(harness(memo).render()), /1\.2\.3/);
});

test("检查副本失败或检测超时保留最后有效状态，同时显示可重试错误", async () => {
  const screen = harness();
  screen.render();
  screen.calls[0].resolve(ready);
  await tick();
  buttons(screen.render()).find((button) => text(button) === "检查增强版")!.props.onClick();
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

test("「启动增强版」放在页头右上角（与 Codex 的启动按钮同位），页面主体里不再重复", async () => {
  const screen = harness();
  // 槽位在 layout effect 里取到，React 会在绘制前同步重渲染；harness 手动再渲染一次。
  screen.render();
  let page = screen.render();
  const launch = () => buttons(screen.topbar()).find((button) => text(button) === "启动增强版");
  assert.ok(launch(), "启动按钮必须 portal 进 [data-topbar-actions]");
  assert.equal(buttons(page).some((button) => text(button) === "启动增强版"), false, "页面主体里不应再有启动按钮");
  assert.equal(launch()!.props.disabled, true, "检测结果回来前不能启动");
  screen.calls[0].resolve(ready);
  await tick();
  page = screen.render();
  assert.equal(launch()!.props.disabled, false);
  launch()!.props.onClick();
  assert.equal(screen.calls[1].command, "claude_localization_action");
  screen.render();
  assert.ok(buttons(screen.topbar()).some((button) => text(button) === "启动中…"));
});

test("增强版电脑控制与 Claude Desktop 接入的 codex-plus-cu 同时开时，提示二选一", async () => {
  const conflict = { targets: { claudeDesktop: { state: "registered" } } };
  const screen = harness(undefined, conflict);
  screen.render();
  screen.calls[0].resolve({ ...ready, computerUse: true, computerUseSupported: true });
  await tick();
  const warning = (page: Element) => JSON.stringify(page).includes("is-warning");
  assert.ok(warning(screen.render()), "两套都开时应显示冲突提醒");

  const quiet = harness(undefined, { targets: { claudeDesktop: { state: "missing" } } });
  quiet.render();
  quiet.calls[0].resolve({ ...ready, computerUse: true, computerUseSupported: true });
  await tick();
  const page = quiet.render();
  assert.equal(warning(page), false, "未接入时不应显示冲突提醒");
  assert.ok(text(page).includes("二选一"), "仍保留一句二选一的说明");
});

test("Bash 时间限制可分别修改默认值和最大值", async () => {
  const screen = harness();
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
  const screen = harness({ status: { ...ready, bashTimeout: { ...ready.bashTimeout, enabled: true } }, autoDetected: true });
  screen.render();
  screen.render();
  screen.topbar()!.props.onClick();
  screen.calls[0].resolve({ ...ready, running: true, bashTimeout: { ...ready.bashTimeout, enabled: true }, bashTimeoutWarning: "不支持的 Cowork Bash 参数校验结构" });
  await tick();
  const page = screen.render();
  assert.ok(text(page).includes("不支持的 Cowork Bash 参数校验结构"));
  assert.match(text(page), /不支持的 Cowork Bash 参数校验结构/);
  assert.equal(screen.topbar()!.props.disabled, false);
  assert.equal(inputs(page).find((input) => String(input.props.onChange).includes("toggleLocalization"))!.props.disabled, false);
  assert.equal(inputs(page).find((input) => String(input.props.onChange).includes("toggleVoiceInput"))!.props.disabled, false);
});

test("Cowork 已准备但未确认加载时显示中性提示，检测后保持可见且不阻止启动", async () => {
  const pendingStatus = {
    ...ready,
    bashTimeout: { ...ready.bashTimeout, enabled: true },
    bashTimeoutWarning: "已准备，暂未确认。将在增强版启动后自动确认。",
  };
  const screen = harness({ status: pendingStatus, autoDetected: true });
  screen.render();
  let page = screen.render();
  assert.match(text(page), /Bash时限解锁增强: 已准备，暂未确认。将在增强版启动后自动确认。/);
  assert.doesNotMatch(text(page), /Cowork Bash 时限增强未生效/);
  assert.equal(screen.topbar()!.props.disabled, false);
  buttons(page).find((button) => text(button) === "检测 Claude")!.props.onClick();
  assert.equal(screen.calls[0].command, "claude_localization_status");
  screen.calls[0].resolve(pendingStatus);
  await tick();
  page = screen.render();
  assert.match(text(page), /Bash时限解锁增强: 已准备，暂未确认。将在增强版启动后自动确认。/);
  assert.doesNotMatch(text(page), /Cowork Bash 时限增强未生效/);
  assert.equal(screen.topbar()!.props.disabled, false);
});

test("启动后自动确认并撤下顶部黄字提示，每次启动重新确认且不重置输入草稿", async () => {
  const pendingStatus = { ...ready, running: true, bashTimeout: { ...ready.bashTimeout, enabled: true },
    bashTimeoutPending: true, bashTimeoutWarning: "已准备，暂未确认。将在增强版启动后自动确认。" };
  const screen = harness({ status: ready, autoDetected: true });
  screen.render(); screen.render();
  screen.topbar()!.props.onClick();
  screen.calls[0].resolve(pendingStatus);
  await tick();
  let page = screen.render();
  assert.match(text(page), /已准备，暂未确认/);
  assert.ok(text(page).indexOf("已准备，暂未确认") < text(page).indexOf("重新制作增强版"));
  assert.ok(JSON.stringify(page).indexOf("claude-l10n-warning") < JSON.stringify(page).indexOf("claude-l10n-actions"));
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
  screen.topbar()!.props.onClick();
  screen.calls[2].resolve(pendingStatus);
  await tick();
  page = screen.render();
  assert.match(text(page), /已准备，暂未确认/);
  assert.equal(screen.timerCount(), 1);
});

test("重新启动时，上一次尚在返回的确认不能撤下本次启动提示", async () => {
  const pendingStatus = { ...ready, running: true, bashTimeout: { ...ready.bashTimeout, enabled: true },
    bashTimeoutPending: true, bashTimeoutWarning: "已准备，暂未确认。将在增强版启动后自动确认。" };
  const screen = harness({ status: pendingStatus, autoDetected: true });
  screen.render(); screen.render();
  screen.runTimer();
  screen.topbar()!.props.onClick();
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
    bashTimeoutPending: true, bashTimeoutWarning: "已准备，暂未确认。将在增强版启动后自动确认。" }, autoDetected: true };
  const screen = harness(memo);
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
  const screen = harness({ status: ready, autoDetected: true });
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
  const screen = harness({ status: saved, autoDetected: true });
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
  const screen = harness({ status: saved, autoDetected: true });
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
  const screen = harness(memo);
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
  assert.equal(inputs(harness(memo).render()).find((item) => item.props["aria-label"] === "修改最大等待时间")!.props.checked, false);
});

test("Bash 秒数允许清空后重输，失焦才显示规范值并保存数字", async () => {
  const saved = { ...ready, bashTimeout: { ...ready.bashTimeout, enabled: true } };
  const screen = harness({ status: saved, autoDetected: true });
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
