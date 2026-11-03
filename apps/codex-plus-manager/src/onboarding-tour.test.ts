import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import * as model from "./onboarding-model.ts";
import * as focusModel from "./agent-focus-model.ts";

const source = ts.transpileModule(readFileSync(new URL("./OnboardingTour.tsx", import.meta.url), "utf8"), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX, target: ts.ScriptTarget.ES2022 },
}).outputText;

type Element = { type: string; props: Record<string, any> };

/** 运行真实组件回调和布局 effect；DOM 尺寸由浏览器视口约束的固定夹具提供。 */
function harness() {
  const hooks: any[] = [];
  const cleanups = new Map<number, () => void>();
  const effects: Array<() => void> = [];
  const listeners = new Map<string, Set<() => void>>();
  const closed: boolean[] = [];
  let cursor = 0;
  let dirty = false;
  let escape = () => {};
  const window = {
    innerWidth: 1200, innerHeight: 800,
    addEventListener: (name: string, callback: () => void) => {
      if (!listeners.has(name)) listeners.set(name, new Set());
      listeners.get(name)!.add(callback);
    },
    removeEventListener: (name: string, callback: () => void) => listeners.get(name)?.delete(callback),
  };
  const card = {
    get offsetWidth() { return Math.min(380, window.innerWidth - 24); },
    get offsetHeight() { return Math.min(200, window.innerHeight - 24); },
  };
  const document = {
    body: {},
    querySelector: () => null,
    addEventListener: window.addEventListener,
    removeEventListener: window.removeEventListener,
  };
  const equal = (left: unknown[] | undefined, right: unknown[]) => left?.length === right.length
    && left.every((value, index) => Object.is(value, right[index]));
  const jsx = (type: string, props: Record<string, any>) => {
    if (props.ref) props.ref.current = card;
    return { type, props };
  };
  const react = {
    useState: (initial: any) => {
      const index = cursor++;
      if (!(index in hooks)) hooks[index] = typeof initial === "function" ? initial() : initial;
      return [hooks[index], (next: any) => {
        const value = typeof next === "function" ? next(hooks[index]) : next;
        if (!Object.is(hooks[index], value)) {
          hooks[index] = value;
          dirty = true;
        }
      }];
    },
    useRef: (initial: any) => {
      const index = cursor++;
      if (!(index in hooks)) hooks[index] = { current: initial };
      return hooks[index];
    },
    useCallback: (callback: () => void, deps: unknown[]) => {
      const index = cursor++;
      if (!equal(hooks[index]?.deps, deps)) hooks[index] = { callback, deps };
      return hooks[index].callback;
    },
    useLayoutEffect: (effect: () => void | (() => void), deps: unknown[]) => {
      const index = cursor++;
      if (equal(hooks[index], deps)) return;
      hooks[index] = deps;
      effects.push(() => {
        cleanups.get(index)?.();
        cleanups.delete(index);
        const cleanup = effect();
        if (cleanup) cleanups.set(index, cleanup);
      });
    },
  };
  const modules: Record<string, unknown> = {
    react,
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "react-dom": { createPortal: (child: unknown) => child },
    "lucide-react": { ArrowLeft: "ArrowLeft", ArrowRight: "ArrowRight", X: "X" },
    "./i18n": {
      t: (text: string) => text,
      tf: (text: string, args: unknown[]) => text.replace(/\{(\d+)\}/g, (_, index) => String(args[Number(index)])),
    },
    "./command-palette-model": { paletteShortcut: "Ctrl K" },
    "./onboarding-model": model,
    "./agent-focus-model": focusModel,
    "./gateway-model": { agentName: (agent: string) => agent },
    "./use-escape-layer": { useEscapeLayer: (_enabled: boolean, close: () => void) => { escape = close; } },
    "./use-modal-focus": { useModalFocus: () => {} },
    "./OnboardingTour.css": {},
  };
  const exports: Record<string, any> = {};
  vm.runInNewContext(source, {
    exports, window, document,
    require: (name: string) => { assert.ok(name in modules, name); return modules[name]; },
  });
  return {
    closed,
    render: () => {
      let tree!: Element;
      for (let pass = 0; pass < 6; pass += 1) {
        cursor = 0;
        dirty = false;
        tree = exports.OnboardingTour({ focus: "all", onClose: (finished: boolean) => closed.push(finished) });
        while (effects.length) effects.shift()!();
        if (!dirty) return tree;
      }
      assert.fail("layout effects did not settle");
    },
    resize: (width: number, height: number) => {
      window.innerWidth = width; window.innerHeight = height;
      for (const listener of listeners.get("resize") ?? []) listener();
    },
    escape: () => escape(),
    unmount: () => cleanups.forEach((cleanup) => cleanup()),
    listenerCount: () => [...listeners.values()].reduce((count, entries) => count + entries.size, 0),
  };
}

function descendants(element: any): Element[] {
  if (element == null || typeof element !== "object") return [];
  if (Array.isArray(element)) return element.flatMap(descendants);
  return [element, ...descendants(element.props?.children)];
}
function text(element: any): string {
  if (element == null || typeof element === "boolean") return "";
  if (Array.isArray(element)) return element.map(text).join("");
  return typeof element === "object" ? text(element.props?.children) : String(element);
}
function card(tree: Element) {
  return descendants(tree).find((element) => element.props?.className === "tour-card")!;
}
function button(tree: Element, label: string) {
  const found = descendants(tree).find((element) => element.type === "button" && text(element) === label);
  assert.ok(found, label);
  return found;
}
function key(tree: Element, value: string, options: { input?: boolean; composing?: boolean; ctrl?: boolean } = {}) {
  const event = {
    key: value, defaultPrevented: false, ctrlKey: options.ctrl, altKey: false, metaKey: false,
    nativeEvent: { isComposing: options.composing },
    target: { closest: () => options.input ? {} : null },
    preventDefault() { this.defaultPrevented = true; },
  };
  card(tree).props.onKeyDown(event);
  return event;
}

test("Enter 不再推进导览，跳过和上一步保持按钮自己的行为", () => {
  const f = harness();
  let tree = f.render();
  const event = key(tree, "Enter");
  assert.equal(event.defaultPrevented, false);
  assert.match(text(f.render()), /1 \/ /);
  button(tree, "跳过").props.onClick();
  assert.deepEqual(f.closed, [false]);
  button(tree, "开始").props.onClick();
  tree = f.render();
  assert.match(text(tree), /2 \/ /);
  assert.equal(key(tree, "Enter").defaultPrevented, false);
  button(tree, "上一步").props.onClick();
  assert.match(text(f.render()), /1 \/ /);
  assert.equal(key(f.render(), " ").defaultPrevented, false);
  f.unmount();
});

test("方向键只处理导览导航，不干扰输入、输入法或组合快捷键", () => {
  const f = harness();
  let tree = f.render();
  for (const options of [{ input: true }, { composing: true }, { ctrl: true }]) {
    assert.equal(key(tree, "ArrowRight", options).defaultPrevented, false);
    assert.match(text(f.render()), /1 \/ /);
  }
  assert.equal(key(tree, "ArrowRight").defaultPrevented, true);
  tree = f.render();
  assert.match(text(tree), /2 \/ /);
  assert.equal(key(tree, "ArrowLeft").defaultPrevented, true);
  assert.match(text(f.render()), /1 \/ /);
  key(f.render(), "ArrowLeft");
  assert.match(text(f.render()), /1 \/ /, "不能回到首步之前");
  f.unmount();
});

test("最后一步的完成和 Esc 分别报告完成、跳过", () => {
  const f = harness();
  for (let index = 1; index < model.TOUR_STEPS.length; index += 1) key(f.render(), "ArrowRight");
  let tree = f.render();
  button(tree, "完成").props.onClick();
  assert.deepEqual(f.closed, [true]);
  f.escape();
  assert.deepEqual(f.closed, [true, false]);
  f.unmount();
  assert.equal(f.listenerCount(), 0);
});

test("无高亮目标的欢迎卡在窗口缩放后重新居中，窄窗口仍保留边距", () => {
  const f = harness();
  let position = card(f.render()).props.style;
  assert.equal(position.left, 410);
  assert.equal(position.top, 300);
  f.resize(400, 300);
  position = card(f.render()).props.style;
  assert.equal(position.left, 12);
  assert.equal(position.top, 50);
  f.resize(280, 180);
  position = card(f.render()).props.style;
  assert.equal(position.left, 12);
  assert.equal(position.top, 12);
  f.unmount();
});
