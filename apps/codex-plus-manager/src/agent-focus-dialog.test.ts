import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import * as model from "./agent-focus-model.ts";

const source = ts.transpileModule(readFileSync(new URL("./AgentFocusDialog.tsx", import.meta.url), "utf8"), {
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
const ready = (names: string[] = []) => ({ status: "ok", agents: names.map((name) => ({ name, enabled: true })) });

function harness(initial: model.AgentFocus = null) {
  const hooks: any[] = [];
  const cleanups = new Map<number, () => void>();
  const timers = new Map<number, () => void>();
  const loads: Array<ReturnType<typeof deferred<unknown>>> = [];
  const saves: Array<{ mode: model.FocusMode; result: ReturnType<typeof deferred<boolean>> }> = [];
  let cursor = 0;
  let timerId = 0;
  let cancellations = 0;
  let focused = "";
  let escape = () => {};
  const exports: Record<string, any> = {};
  const jsx = (type: string, props: Record<string, any>) => {
    if (props.ref) props.ref.current = {
      querySelector: (selector: string) => ({ focus: () => { focused = selector; } }),
    };
    return { type, props };
  };
  const react = {
    useState: (initialValue: unknown) => {
      const index = cursor++;
      if (!(index in hooks)) hooks[index] = typeof initialValue === "function" ? (initialValue as () => unknown)() : initialValue;
      return [hooks[index], (value: unknown) => {
        hooks[index] = typeof value === "function" ? (value as (current: unknown) => unknown)(hooks[index]) : value;
      }];
    },
    useRef: (initialValue: unknown) => {
      const index = cursor++;
      if (!(index in hooks)) hooks[index] = { current: initialValue };
      return hooks[index];
    },
    useEffect: (effect: () => void | (() => void), deps: unknown[]) => {
      const index = cursor++;
      if (hooks[index]?.every((value: unknown, i: number) => Object.is(value, deps[i]))) return;
      cleanups.get(index)?.();
      cleanups.delete(index);
      hooks[index] = deps.slice();
      const cleanup = effect();
      if (typeof cleanup === "function") cleanups.set(index, cleanup);
    },
  };
  const modules: Record<string, unknown> = {
    react,
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "react-dom": { createPortal: (child: unknown) => child },
    "@tauri-apps/api/core": { invoke: (command: string) => {
      assert.equal(command, "load_gateway");
      const request = deferred<unknown>();
      loads.push(request);
      return request.promise;
    } },
    "lucide-react": Object.fromEntries(["Check", "LayoutGrid", "LoaderCircle", "RefreshCw", "Sparkles"].map((name) => [name, name])),
    "./i18n": { t: (text: string) => text, tf: (text: string, args: string[]) => text.replace(/\{(\d+)\}/g, (_, index) => args[Number(index)]) },
    "./agent-focus-model": model,
    "./HubOverview": { AgentGlyph: "AgentGlyph" },
    "./use-escape-layer": { useEscapeLayer: (_active: boolean, close: () => void) => { escape = close; } },
    "./use-modal-focus": { useModalFocus: () => {} },
    "./OnboardingTour.css": {},
  };
  vm.runInNewContext(source, {
    exports, document: { body: {} },
    window: {
      setTimeout: (callback: () => void) => { const id = ++timerId; timers.set(id, callback); return id; },
      clearTimeout: (id: number) => timers.delete(id),
    },
    require: (name: string) => {
      assert.ok(name in modules, `unexpected import ${name}`);
      return modules[name];
    },
  });
  return {
    loads, saves,
    render: () => {
      cursor = 0;
      return exports.AgentFocusDialog({
        initial,
        onDone: (mode: model.FocusMode) => {
          const result = deferred<boolean>();
          saves.push({ mode, result });
          return result.promise;
        },
        onCancel: () => { cancellations += 1; },
      }) as Element;
    },
    escape: () => escape(),
    cancellations: () => cancellations,
    focused: () => focused,
    expireLoad: () => {
      const item = timers.entries().next().value;
      assert.ok(item);
      timers.delete(item[0]);
      item[1]();
    },
    unmount: () => cleanups.forEach((cleanup) => cleanup()),
    timerCount: () => timers.size,
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
function button(tree: Element, label: string) {
  const found = children(tree).find((element) => element.type === "button" && text(element) === label);
  assert.ok(found, label);
  return found;
}
function radio(tree: Element, mode: string) {
  const found = children(tree).find((element) => element.props?.["data-mode"] === mode);
  assert.ok(found, mode);
  return found;
}

test("首次 Esc 仅暂不设置，不暗中提交全部功能", () => {
  const f = harness();
  const tree = f.render();
  assert.equal(radio(tree, "codex").props["aria-checked"], true);
  assert.equal(button(tree, "稍后设置").props.disabled, false);
  f.escape();
  assert.equal(f.cancellations(), 1);
  assert.equal(f.saves.length, 0);
  f.unmount();
  assert.equal(f.timerCount(), 0);
});

test("增强模式等网关状态确认，加载失败后可重试并显示已有连接", async () => {
  const f = harness();
  radio(f.render(), "enhance").props.onClick();
  let tree = f.render();
  assert.equal(button(tree, "完成").props.disabled, true);
  button(tree, "完成").props.onClick();
  assert.equal(f.saves.length, 0);
  f.loads[0].reject(new Error("fixture unavailable"));
  await flush();
  tree = f.render();
  assert.match(text(tree), /无法确认已有网关连接/);
  assert.equal(button(tree, "完成").props.disabled, true);
  button(tree, "重试").props.onClick();
  f.render();
  f.loads[1].resolve(ready(["Codex", "Claude Code"]));
  await flush();
  tree = f.render();
  assert.match(text(tree), /Codex、Claude Code 目前经 Codex\+\+ 网关接入/);
  assert.match(text(tree), /继续运行并管理对应配置/);
  assert.equal(button(tree, "完成").props.disabled, false);
  f.unmount();
});

test("加载超时可重试，旧请求晚到不会覆盖新的连接状态", async () => {
  const f = harness();
  radio(f.render(), "enhance").props.onClick();
  f.expireLoad();
  button(f.render(), "重试").props.onClick();
  f.render();
  f.loads[1].resolve(ready(["Claude Code"]));
  await flush();
  f.loads[0].resolve(ready());
  await flush();
  assert.match(text(f.render()), /Claude Code 目前经 Codex\+\+ 网关接入/);
  assert.equal(f.timerCount(), 0);
  f.unmount();
});

test("网关返回失败或缺少连接列表时，不能当成没有既有连接", async () => {
  for (const result of [{ status: "failed", agents: [] }, { status: "ok" }]) {
    const f = harness("enhance");
    f.render();
    f.loads[0].resolve(result);
    await flush();
    const tree = f.render();
    assert.match(text(tree), /无法确认已有网关连接/);
    assert.equal(button(tree, "完成").props.disabled, true);
    button(tree, "完成").props.onClick();
    assert.equal(f.saves.length, 0);
    f.unmount();
  }
});

test("保存期间禁止重复提交和退出，失败保留选择可直接重试", async () => {
  const f = harness("all");
  radio(f.render(), "enhance").props.onClick();
  f.loads[0].resolve(ready());
  await flush();
  const finish = button(f.render(), "完成");
  finish.props.onClick();
  finish.props.onClick();
  f.escape();
  let tree = f.render();
  assert.equal(f.saves.length, 1);
  assert.equal(f.cancellations(), 0);
  assert.equal(button(tree, "取消").props.disabled, true);
  assert.equal(radio(tree, "all").props.disabled, true);
  f.saves[0].result.resolve(false);
  await flush();
  tree = f.render();
  assert.match(text(tree), /模式尚未切换/);
  assert.equal(radio(tree, "enhance").props["aria-checked"], true);
  button(tree, "重试保存").props.onClick();
  assert.equal(f.saves[1].mode, "enhance");
  f.saves[1].result.resolve(true);
  await flush();
  assert.doesNotMatch(text(f.render()), /模式尚未切换/);
  f.unmount();
});

test("保存抛错后解除等待状态，保留所选模式并允许重试或取消", async () => {
  const f = harness("codex");
  const tree = f.render();
  button(tree, "完成").props.onClick();
  f.saves[0].result.reject(new Error("fixture disconnected"));
  await flush();
  const failed = f.render();
  assert.match(text(failed), /模式尚未切换/);
  assert.equal(radio(failed, "codex").props["aria-checked"], true);
  assert.equal(button(failed, "重试保存").props.disabled, false);
  assert.equal(button(failed, "取消").props.disabled, false);
  f.escape();
  assert.equal(f.cancellations(), 1);
  f.unmount();
});

test("模式单选支持方向键和首尾键，只有当前选项进入 Tab 顺序", () => {
  const f = harness();
  let prevented = 0;
  const key = (mode: string, name: string) => radio(f.render(), mode).props.onKeyDown({ key: name, nativeEvent: {}, preventDefault: () => { prevented += 1; } });
  key("codex", "ArrowDown");
  let tree = f.render();
  assert.equal(radio(tree, "enhance").props.tabIndex, 0);
  assert.equal(radio(tree, "codex").props.tabIndex, -1);
  assert.equal(f.focused(), '[data-mode="enhance"]');
  key("enhance", "End");
  assert.equal(radio(f.render(), "all").props["aria-checked"], true);
  key("all", "ArrowRight");
  assert.equal(radio(f.render(), "codex").props["aria-checked"], true);
  key("codex", "ArrowLeft");
  assert.equal(radio(f.render(), "all").props["aria-checked"], true);
  key("all", "Home");
  tree = f.render();
  assert.equal(radio(tree, "codex").props["aria-checked"], true);
  assert.equal(prevented, 5);
  f.unmount();
});

test("输入法和组合键不会改变模式选择", () => {
  const f = harness();
  for (const overrides of [
    { nativeEvent: { isComposing: true } },
    { altKey: true },
    { ctrlKey: true },
    { metaKey: true },
    { defaultPrevented: true },
  ]) {
    radio(f.render(), "codex").props.onKeyDown({
      key: "ArrowRight",
      nativeEvent: {},
      preventDefault: () => assert.fail("不应拦截输入法或组合键"),
      ...overrides,
    });
    assert.equal(radio(f.render(), "codex").props["aria-checked"], true);
  }
  f.unmount();
});
