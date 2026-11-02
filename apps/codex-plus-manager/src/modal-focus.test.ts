import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import test from "node:test";
import ts from "typescript";

const source = ts.transpileModule(readFileSync(new URL("./use-modal-focus.ts", import.meta.url), "utf8"), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;

/** 只替换 DOM 的焦点与事件通道，运行真实 hook 的挂载、按键和卸载回调。 */
function harness() {
  const listeners = new Map<string, Set<(event: any) => void>>();
  const frames = new Map<number, () => void>();
  let frameId = 0;
  const doc: any = {
    modals: [] as Element[],
    activeElement: null,
    querySelectorAll: () => doc.modals,
    addEventListener: (name: string, listener: (event: any) => void) => {
      if (!listeners.has(name)) listeners.set(name, new Set());
      listeners.get(name)!.add(listener);
    },
    removeEventListener: (name: string, listener: (event: any) => void) => listeners.get(name)?.delete(listener),
    defaultView: {
      getComputedStyle: (element: Element) => ({ visibility: element.visible ? "visible" : "hidden" }),
      requestAnimationFrame: (run: () => void) => { frames.set(++frameId, run); return frameId; },
      cancelAnimationFrame: (id: number) => frames.delete(id),
    },
  };
  function emit(name: string, event: unknown) {
    for (const listener of listeners.get(name) ?? []) listener(event);
  }
  class Element {
    ownerDocument = doc;
    children: Element[] = [];
    parent: Element | null = null;
    isConnected = true;
    tabIndex = 0;
    disabled = false;
    hidden = false;
    visible = true;
    autofocus = false;
    readonly name: string;
    constructor(name: string) { this.name = name; }
    append(child: Element) { child.parent = this; this.children.push(child); return child; }
    contains(element: Element | null): boolean { return element === this || this.children.some((child) => child.contains(element)); }
    querySelectorAll(): Element[] { return this.children.flatMap((child) => [child, ...child.querySelectorAll()]); }
    getClientRects() { return this.visible ? [{}] : []; }
    matches() { return this.disabled; }
    closest(): Element | null { return this.hidden ? this : this.parent?.closest() ?? null; }
    hasAttribute() { return this.autofocus; }
    focus() {
      if (!this.isConnected || this.disabled || !this.visible) return;
      doc.activeElement = this;
      emit("focusin", { target: this });
    }
  }
  const body = new Element("body");
  body.tabIndex = -1;
  doc.body = body;
  doc.activeElement = body;
  const cleanups: (() => void)[] = [];
  const context = vm.createContext({
    exports: {},
    require: () => ({ useEffect: (effect: () => (() => void) | undefined) => { const cleanup = effect(); if (cleanup) cleanups.push(cleanup); } }),
  });
  vm.runInContext(source, context);
  const open = (root: Element) => {
    doc.modals.push(root);
    context.exports.useModalFocus({ current: root });
    const cleanup = cleanups.at(-1)!;
    return () => {
      cleanup();
      doc.modals = doc.modals.filter((modal: Element) => modal !== root);
      root.isConnected = false;
    };
  };
  return {
    doc, Element, open,
    frame: () => { for (const run of frames.values()) run(); frames.clear(); },
    key: (key: string, shiftKey = false) => {
      const event = { key, shiftKey, defaultPrevented: false, preventDefault() { this.defaultPrevented = true; } };
      emit("keydown", event);
      return event;
    },
  };
}

test("弹层优先聚焦选中项，Tab 和 Shift+Tab 在可用控件间循环", () => {
  const h = harness();
  const trigger = new h.Element("trigger"); trigger.focus();
  const root = new h.Element("dialog"); root.tabIndex = -1;
  const first = root.append(new h.Element("first"));
  const selected = root.append(new h.Element("selected")); selected.autofocus = true;
  const disabled = root.append(new h.Element("disabled")); disabled.disabled = true;
  const last = root.append(new h.Element("last"));
  const close = h.open(root);
  assert.equal(h.doc.activeElement, selected);
  assert.equal(h.key("Enter").defaultPrevented, false, "Enter 应留给原生按钮激活");
  assert.equal(h.key("Tab").defaultPrevented, false, "中间控件沿用原生 Tab 顺序");
  last.focus();
  assert.equal(h.key("Tab").defaultPrevented, true);
  assert.equal(h.doc.activeElement, first);
  assert.equal(h.key("Tab", true).defaultPrevented, true);
  assert.equal(h.doc.activeElement, last);
  close();
  assert.equal(h.doc.activeElement, trigger);
});

test("焦点进入背景时回到弹层，隐藏、禁用和负 tabindex 不参与循环", () => {
  const h = harness();
  const root = new h.Element("dialog");
  const hidden = root.append(new h.Element("hidden")); hidden.hidden = true;
  const negative = root.append(new h.Element("negative")); negative.tabIndex = -1;
  const disabled = root.append(new h.Element("disabled")); disabled.disabled = true;
  const available = root.append(new h.Element("available"));
  const close = h.open(root);
  assert.equal(h.doc.activeElement, available);
  new h.Element("background").focus();
  assert.equal(h.doc.activeElement, available);
  assert.equal(h.key("Tab").defaultPrevented, true);
  assert.equal(h.doc.activeElement, available);
  close();
});

test("新开的确认弹层可接管焦点，关闭后原弹层继续约束 Tab", () => {
  const h = harness();
  const root = new h.Element("dialog");
  const button = root.append(new h.Element("button"));
  const close = h.open(root);
  const alert = new h.Element("alert");
  const confirm = alert.append(new h.Element("confirm"));
  h.doc.modals.push(alert);
  confirm.focus();
  assert.equal(h.doc.activeElement, confirm);
  assert.equal(h.key("Tab").defaultPrevented, false, "不拦截新弹层的键盘操作");
  h.doc.modals = h.doc.modals.filter((modal: unknown) => modal !== alert);
  assert.equal(h.key("Tab").defaultPrevented, true);
  assert.equal(h.doc.activeElement, button);
  close();
});

test("同时登记多个弹层时只由最上层处理，关闭后恢复到下层控件", () => {
  const h = harness();
  const first = new h.Element("first-dialog");
  const firstButton = first.append(new h.Element("first-button"));
  const closeFirst = h.open(first);
  const second = new h.Element("second-dialog");
  const secondButton = second.append(new h.Element("second-button"));
  const closeSecond = h.open(second);
  assert.equal(h.doc.activeElement, secondButton);
  assert.equal(h.key("Tab").defaultPrevented, true);
  assert.equal(h.doc.activeElement, secondButton);
  closeSecond();
  assert.equal(h.doc.activeElement, firstButton);
  closeFirst();
});

test("初次渲染不可见时下一帧补聚焦，卸载后不再抢焦点", () => {
  const h = harness();
  const root = new h.Element("dialog"); root.visible = false;
  const button = root.append(new h.Element("button")); button.visible = false;
  const close = h.open(root);
  root.visible = true; button.visible = true;
  h.frame();
  assert.equal(h.doc.activeElement, button);
  close();
  const background = new h.Element("background");
  background.focus();
  assert.equal(h.doc.activeElement, background);
  assert.equal(h.key("Tab").defaultPrevented, false);
});

test("弹层容器先出现、控件稍后可见时，下一帧聚焦首选按钮", () => {
  const h = harness();
  const root = new h.Element("dialog"); root.tabIndex = -1;
  const button = root.append(new h.Element("button")); button.visible = false; button.autofocus = true;
  const close = h.open(root);
  assert.equal(h.doc.activeElement, root);
  button.visible = true;
  h.frame();
  assert.equal(h.doc.activeElement, button);
  close();
});
