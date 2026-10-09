import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const source = readFileSync(new URL("../../../assets/inject/renderer-inject/94-typing-effects.js", import.meta.url), "utf8");
type Listener = (event: Record<string, unknown>) => void;

function fixture(initialMode = "rainbow") {
  const documentListeners = new Map<string, Set<Listener>>();
  const windowListeners = new Map<string, Set<Listener>>();
  const mediaListeners = new Set<Listener>();
  const frames = new Map<number, (time: number) => void>();
  const microtasks: (() => void)[] = [];
  const created: FakeElement[] = [];
  const drawing: string[] = [];
  const settings = { typingEffect: initialMode };
  let now = 100, frameId = 0, rootZoom = 1;
  const add = (map: Map<string, Set<Listener>>, name: string, handler: Listener) => {
    if (!map.has(name)) map.set(name, new Set());
    map.get(name)!.add(handler);
  };
  const remove = (map: Map<string, Set<Listener>>, name: string, handler: Listener) => map.get(name)?.delete(handler);
  const drawingContext = {
    setTransform() {}, clearRect() { drawing.push("clear"); }, save() {}, restore() {}, translate() {}, rotate() {},
    fillRect() { drawing.push("spark"); }, beginPath() {}, moveTo() {}, lineTo() {}, closePath() {},
    fill() { drawing.push("star"); }, globalAlpha: 1, fillStyle: "",
  };

  class FakeElement {
    tagName: string;
    nodeType = 1;
    attrs: Record<string, string> = {};
    style: Record<string, string> = {};
    children: FakeElement[] = [];
    parentElement: FakeElement | null = null;
    textContent = "";
    disabled = false;
    extension = false;
    rect = { left: 100, top: 200, width: 240, height: 100, right: 340, bottom: 300 };
    offsetWidth = 240;
    offsetHeight = 100;
    clientWidth = 238;
    constructor(tag: string) { this.tagName = tag.toUpperCase(); }
    get isConnected(): boolean { return this === documentElement || !!this.parentElement?.isConnected; }
    get childNodes() { return this.children; }
    setAttribute(name: string, value: string) { this.attrs[name] = value; }
    getAttribute(name: string) { return this.attrs[name] ?? null; }
    appendChild(child: FakeElement) { child.parentElement = this; this.children.push(child); return child; }
    remove() { if (this.parentElement) this.parentElement.children = this.parentElement.children.filter(child => child !== this); this.parentElement = null; }
    matches(selector: string): boolean {
      return selector.split(",").some(part => {
        const item = part.trim();
        if (item === "textarea") return this.tagName === "TEXTAREA";
        if (item === ":disabled") return this.disabled;
        if (item === ".composer-footer") return this.attrs.class === "composer-footer";
        if (item.startsWith(".ProseMirror")) return this.attrs.class === "ProseMirror" && this.attrs.contenteditable === "true";
        if (item.startsWith('[role="textbox"]')) return this.attrs.role === "textbox" && this.attrs.contenteditable === "true";
        const attribute = item.match(/^\[([^=\]]+)(?:="([^"]+)")?\]$/);
        return !!attribute && attribute[1] in this.attrs && (!attribute[2] || this.attrs[attribute[1]] === attribute[2]);
      });
    }
    closest(selector: string): FakeElement | null {
      for (let node: FakeElement | null = this; node; node = node.parentElement) if (node.matches(selector)) return node;
      return null;
    }
    contains(node: unknown): boolean {
      for (let current = node as FakeElement | null; current; current = current.parentElement) if (current === this) return true;
      return false;
    }
    querySelector(selector: string): FakeElement | null {
      for (const child of this.children) {
        if (child.matches(selector)) return child;
        const descendant = child.querySelector(selector);
        if (descendant) return descendant;
      }
      return null;
    }
    getBoundingClientRect() {
      if (this.tagName === "CANVAS") return { left: 0, top: 0, width: 1024, height: 768, right: 1024, bottom: 768 };
      if (this.style.left === "-100000px") {
        const width = parseFloat(this.style.width);
        return { left: -100000, top: 0, width: width * rootZoom, height: 100 * rootZoom, right: -100000 + width * rootZoom, bottom: 100 * rootZoom };
      }
      if (this.tagName === "SPAN" && this.parentElement?.style.left === "-100000px") {
        const mirror = this.parentElement;
        const width = parseFloat(mirror.style.width) - 22;
        const columns = Math.max(1, Math.floor(width / 8));
        const lines = mirror.textContent.split("\n");
        const line = lines.at(-1) || "";
        const wrappedRows = mirror.style.whiteSpace === "pre" ? 0 : Math.floor(line.length / columns);
        const column = mirror.style.whiteSpace === "pre" ? line.length : line.length % columns;
        const left = -100000 + (11 + column * 8) * rootZoom;
        const top = (11 + (lines.length - 1 + wrappedRows) * 20) * rootZoom;
        return { left, top, width: rootZoom, height: 20 * rootZoom, right: left + rootZoom, bottom: top + 20 * rootZoom };
      }
      return this.rect;
    }
    getClientRects() { return [this.getBoundingClientRect()]; }
    getContext() { return drawingContext; }
  }
  class Textarea extends FakeElement {
    value = "";
    selectionStart = 0;
    selectionEnd = 0;
    scrollTop = 0;
    scrollLeft = 0;
    wrap = "soft";
    constructor() { super("textarea"); }
  }
  const documentElement = new FakeElement("html");
  const body = documentElement.appendChild(new FakeElement("body"));
  const composer = body.appendChild(new FakeElement("div"));
  composer.setAttribute("data-codex-composer-root", "");
  const editor = composer.appendChild(new Textarea()) as Textarea;
  const document = {
    documentElement, body, activeElement: editor as FakeElement, hidden: false,
    createElement(tag: string) { const element = new FakeElement(tag); created.push(element); return element; },
    addEventListener(name: string, handler: Listener) { add(documentListeners, name, handler); },
    removeEventListener(name: string, handler: Listener) { remove(documentListeners, name, handler); },
  };
  const media = {
    matches: false,
    addEventListener(_name: string, handler: Listener) { mediaListeners.add(handler); },
    removeEventListener(_name: string, handler: Listener) { mediaListeners.delete(handler); },
  };
  let selection: unknown;
  const window = {
    devicePixelRatio: 2,
    matchMedia: () => media,
    getSelection: () => selection,
    addEventListener(name: string, handler: Listener) { add(windowListeners, name, handler); },
    removeEventListener(name: string, handler: Listener) { remove(windowListeners, name, handler); },
    __codexPlusTypingEffectsRuntime: undefined as { dispose(): void } | undefined,
    testApi: undefined as unknown as {
      install(): void; sync(): void; caret(editor: FakeElement): { x: number; y: number };
      state: { particles: { x: number; y: number; star: boolean }[]; canvas: FakeElement | null; composition: unknown };
    },
  };
  const context = createContext({
    window, document, HTMLTextAreaElement: Textarea,
    getComputedStyle: (node: FakeElement) => ({
      fontFamily: "monospace", fontSize: "16px", lineHeight: "20px", letterSpacing: "0px", direction: "ltr",
      paddingTop: "10px", paddingBottom: "10px", paddingLeft: "10px", paddingRight: "10px",
      borderTopWidth: "1px", borderBottomWidth: "1px", borderLeftWidth: "1px", borderRightWidth: "1px",
      ...node.style,
    }),
    codexPlusSettings: () => settings,
    visibleElement: (node: FakeElement) => node.rect.width > 0 && node.rect.height > 0,
    isExtensionUiNode: (node: FakeElement) => node.extension || !!node.closest("[data-codex-plus-ext]"),
    registerCodexPlusExtensionSelector: (selector: string) => { assert.equal(selector, '[data-codex-plus-ext="builtin-typing-effects"]'); },
    performance: { now: () => now },
    requestAnimationFrame: (handler: (time: number) => void) => { frames.set(++frameId, handler); return frameId; },
    cancelAnimationFrame: (id: number) => frames.delete(id),
    queueMicrotask: (callback: () => void) => microtasks.push(callback),
  });
  const load = () => {
    runInContext(`(function() { ${source}\nwindow.testApi = { install: installCodexPlusTypingEffects, sync: syncCodexPlusTypingEffects, caret: codexPlusTypingEffectCaret, state: codexPlusTypingEffects }; })();`, context);
    return window.testApi;
  };
  const dispatch = (name: string, extra: Record<string, unknown> = {}, target: FakeElement = document.activeElement) => {
    const event = { target, isTrusted: true, ...extra };
    for (const listener of documentListeners.get(name) || []) listener(event);
  };
  const insert = (text: string, extra: Record<string, unknown> = {}) => {
    editor.value += text;
    editor.selectionStart = editor.selectionEnd = editor.value.length;
    dispatch("input", { inputType: "insertText", data: text, ...extra }, editor);
  };
  const frame = () => {
    now += 16;
    const pending = [...frames];
    frames.clear();
    for (const [, callback] of pending) callback(now);
  };
  const api = load();
  api.install();
  return {
    api, load, window, document, editor, composer, body, created, drawing, frames, documentListeners, windowListeners, mediaListeners,
    FakeElement, dispatch, insert, frame, settings,
    advance: (milliseconds: number) => { now += milliseconds; },
    flush: () => { while (microtasks.length) microtasks.shift()!(); },
    reduced: (value: boolean) => { media.matches = value; for (const listener of mediaListeners) listener({}); },
    zoom: (value: number) => { rootZoom = value; },
    selection: (value: unknown) => { selection = value; },
    windowEvent: (name: string) => { for (const listener of windowListeners.get(name) || []) listener({}); },
  };
}

test("typing effects are opt-in and ignore non-typing edits outside the native composer", () => {
  const disabled = fixture("off");
  disabled.insert("a");
  disabled.dispatch("input", { inputType: "deleteContentBackward", data: null });
  assert.equal(disabled.created.length, 0);
  assert.equal(disabled.frames.size, 0);
  disabled.settings.typingEffect = "unknown";
  disabled.api.sync(); disabled.insert("b");
  assert.equal(disabled.created.length, 0);

  for (const event of [
    { inputType: "insertFromPaste", data: "paste" }, { inputType: "insertFromDrop", data: "drop" },
    { inputType: "deleteContentForward", data: null }, { inputType: "deleteWordForward", data: null },
    { inputType: "deleteSoftLineForward", data: null }, { inputType: "deleteByCut", data: null },
    { inputType: "historyUndo", data: null }, { inputType: "historyRedo", data: null },
    { inputType: "deleteContentBackward", data: null, isTrusted: false },
    { inputType: "deleteContentBackward", data: "", isComposing: true, isTrusted: false },
    { inputType: "insertText", data: "a", isTrusted: false },
    { inputType: "insertCompositionText", data: "中", isComposing: true, isTrusted: false },
  ]) {
    const f = fixture(); f.dispatch("input", event);
    assert.equal(f.created.length, 0, JSON.stringify(event));
  }
  const f = fixture();
  f.editor.extension = true; f.insert("extension");
  f.editor.extension = false; delete f.composer.attrs["data-codex-composer-root"]; f.insert("outside");
  assert.equal(f.created.length, 0);
});

test("backspace emits at the updated textarea caret without requiring input data or changing editing state", () => {
  for (const mode of ["rainbow", "fireworks", "stars"]) {
    for (const inputType of ["deleteContentBackward", "deleteWordBackward", "deleteSoftLineBackward", "deleteHardLineBackward"]) {
      const f = fixture(mode);
      const expectedCount = mode === "fireworks" ? 26 : mode === "stars" ? 10 : 16;
      f.editor.value = "abde";
      f.editor.selectionStart = f.editor.selectionEnd = 2;
      f.dispatch("input", { inputType, data: null });
      assert.equal(f.api.state.particles.length, expectedCount, inputType);
      assert.equal(f.api.state.particles[0].x, 127, "the burst follows the caret after deleting in the middle of the text");
      assert.equal(f.api.state.particles[0].y, 222);
      assert.equal(f.editor.value, "abde");
      assert.equal(f.editor.selectionStart, 2);
      assert.equal(f.editor.selectionEnd, 2);
      assert.equal(f.document.activeElement, f.editor);
      assert.equal(f.editor.children.length, 0);

      f.advance(50);
      f.editor.value = "ade";
      f.editor.selectionStart = f.editor.selectionEnd = 1;
      f.dispatch("input", { inputType, data: "" });
      assert.equal(f.api.state.particles.length, expectedCount * 2, "an empty data string also permits backspace effects");
      assert.equal(f.api.state.particles[expectedCount].x, 119);
      assert.equal(f.editor.value, "ade");
      assert.equal(f.editor.selectionStart, 1);
      assert.equal(f.frames.size, 1);
    }
  }
});

test("backspace reads the current ProseMirror selection and preserves the edited DOM", () => {
  const f = fixture();
  const editor = f.composer.appendChild(new f.FakeElement("div"));
  editor.attrs.class = "ProseMirror"; editor.attrs.contenteditable = "true"; editor.textContent = "你好界";
  f.document.activeElement = editor;
  const text = { nodeType: 3, parentElement: editor };
  let collapsed = 0;
  const range = {
    cloneRange: () => ({ collapse: () => { collapsed++; }, getClientRects: () => [{ left: 175, right: 175, top: 220, height: 20 }] }),
  };
  const selection = { rangeCount: 1, anchorNode: text, focusNode: text, getRangeAt: () => range };
  f.selection(selection);
  f.dispatch("input", { inputType: "deleteContentBackward", data: null }, editor);
  assert.equal(f.api.state.particles.length, 16);
  assert.equal(f.api.state.particles[0].x, 175);
  assert.equal(f.api.state.particles[0].y, 231);
  assert.equal(collapsed, 1);
  assert.equal(editor.textContent, "你好界");
  assert.equal(editor.children.length, 0);
  assert.equal(f.window.getSelection(), selection);
  assert.equal(f.document.activeElement, editor);
});

test("all effects originate at the caret, preserve editing state, and only animate live particles", () => {
  for (const mode of ["rainbow", "fireworks", "stars"]) {
    const f = fixture(mode);
    f.insert("a");
    const expectedCount = mode === "fireworks" ? 26 : mode === "stars" ? 10 : 16;
    assert.equal(f.api.state.particles.length, expectedCount);
    assert.equal(f.api.state.particles[0].x, 119);
    assert.equal(f.api.state.particles[0].y, 222);
    assert.equal(f.frames.size, 1);
    assert.equal(f.editor.value, "a");
    assert.equal(f.editor.selectionStart, 1);
    assert.equal(f.document.activeElement, f.editor);
    assert.equal(f.editor.children.length, 0);
    assert.equal(f.api.state.canvas?.attrs["aria-hidden"], "true");
    assert.equal(f.api.state.canvas?.attrs["data-codex-plus-ext"], "builtin-typing-effects");
    assert.equal(f.api.state.canvas?.style.pointerEvents, "none");
    assert.equal(f.body.children.length, 1, "offscreen mirror is removed after reading the caret");
    f.frame();
    assert.ok(f.drawing.includes(mode === "stars" ? "star" : "spark"));
    for (let index = 0; index < 60; index++) f.frame();
    assert.equal(f.api.state.particles.length, 0);
    assert.equal(f.api.state.canvas, null);
    assert.equal(f.frames.size, 0);
  }
});

test("textarea caret mirrors typography, wrapping, scrolling and CSS zoom", () => {
  const f = fixture();
  f.editor.value = "first\nabc";
  f.editor.selectionEnd = f.editor.value.length;
  f.editor.scrollLeft = 4; f.editor.scrollTop = 10;
  f.editor.rect = { left: 100, top: 200, width: 480, height: 200, right: 580, bottom: 400 };
  f.zoom(2);
  const point = f.api.caret(f.editor);
  assert.equal(point.x, 162);
  assert.equal(point.y, 264);
  const mirror = f.created.find(node => node.tagName === "DIV")!;
  assert.equal(mirror.style.fontFamily, "monospace");
  assert.equal(mirror.style.lineHeight, "20px");
  assert.equal(mirror.style.width, "240px");
  assert.equal(mirror.style.whiteSpace, "pre-wrap");
  assert.equal(mirror.isConnected, false);
  f.editor.value = "a".repeat(30); f.editor.selectionEnd = 30; f.editor.scrollLeft = f.editor.scrollTop = 0;
  assert.equal(f.api.caret(f.editor).y, 284, "wrapped cursor uses the next visual line");
  f.editor.wrap = "off";
  assert.equal(f.api.caret(f.editor).y, 244, "nowrap follows the native textarea wrap attribute");
  f.editor.wrap = "soft"; f.editor.clientWidth = 198; f.editor.value = "a".repeat(25); f.editor.selectionEnd = 25;
  assert.equal(f.api.caret(f.editor).y, 284, "a vertical scrollbar narrows the mirror's wrapping width");
  assert.equal(f.created.filter(node => node.tagName === "DIV").at(-1)?.style.width, "200px");
});

test("contenteditable reads a cloned Selection Range without changing the native DOM or selection", () => {
  const f = fixture();
  const editor = f.composer.appendChild(new f.FakeElement("div"));
  editor.attrs.class = "ProseMirror"; editor.attrs.contenteditable = "true"; editor.textContent = "你好";
  f.document.activeElement = editor;
  const text = { nodeType: 3, parentElement: editor };
  let collapsed = 0;
  const range = {
    cloneRange: () => ({ collapse: () => { collapsed++; }, getClientRects: () => [{ left: 175, right: 175, top: 220, height: 20 }] }),
  };
  f.selection({ rangeCount: 1, anchorNode: text, focusNode: text, getRangeAt: () => range });
  f.dispatch("input", { inputType: "insertText", data: "好" }, editor);
  assert.equal(f.api.state.particles[0].x, 175);
  assert.equal(f.api.state.particles[0].y, 231);
  assert.equal(collapsed, 1);
  assert.equal(editor.textContent, "你好");
  assert.equal(editor.children.length, 0);
  assert.equal(f.created.some(node => node.tagName === "DIV"), false);
});

test("contenteditable empty paragraphs emit on the current line when a collapsed Range has no geometry", () => {
  const f = fixture();
  const editor = f.composer.appendChild(new f.FakeElement("div"));
  editor.attrs.class = "ProseMirror"; editor.attrs.contenteditable = "true";
  const paragraph = editor.appendChild(new f.FakeElement("p"));
  paragraph.rect = { left: 110, top: 250, width: 210, height: 20, right: 320, bottom: 270 };
  f.document.activeElement = editor;
  f.selection({
    rangeCount: 1, anchorNode: paragraph, focusNode: paragraph,
    getRangeAt: () => ({ cloneRange: () => ({
      endContainer: paragraph, endOffset: 0, collapse() {}, getClientRects: () => [],
      getBoundingClientRect: () => ({ left: 0, top: 0, right: 0, height: 0 }),
    }) }),
  });
  f.dispatch("input", { inputType: "insertParagraph", data: null }, editor);
  assert.equal(f.api.state.particles[0].x, 120);
  assert.equal(f.api.state.particles[0].y, 271);
  assert.equal(editor.children.length, 1, "no marker is inserted into the native empty paragraph");
  f.advance(50);
  editor.rect = { left: 100, top: 200, width: 480, height: 200, right: 580, bottom: 400 };
  paragraph.rect = { left: 120, top: 300, width: 420, height: 40, right: 540, bottom: 340 };
  paragraph.style.paddingLeft = "4px"; paragraph.style.paddingTop = "0px"; paragraph.style.lineHeight = "24px";
  f.dispatch("input", { inputType: "insertParagraph", data: null }, editor);
  assert.equal(f.api.state.particles[16].x, 128, "empty-line padding follows the editor's 2x CSS zoom");
  assert.equal(f.api.state.particles[16].y, 326.4, "the current paragraph's lineHeight takes priority and follows CSS zoom");
});

test("Chinese IME emits while composing and does not repeat when final input precedes or follows compositionend", () => {
  for (const finalInputFirst of [false, true]) {
    const f = fixture();
    f.dispatch("compositionstart");
    f.insert("你", { inputType: "insertCompositionText", isComposing: true });
    assert.equal(f.api.state.particles.length, 16, "preedit input must show effects before selecting a Chinese candidate");
    f.advance(50);
    if (finalInputFirst) f.dispatch("input", { inputType: "insertCompositionText", data: "你", isComposing: false });
    f.dispatch("compositionend", { data: "你" });
    if (!finalInputFirst) f.dispatch("input", { inputType: "insertText", data: "你", isComposing: false });
    f.flush();
    assert.equal(f.api.state.particles.length, 16);
    f.advance(100);
    f.dispatch("input", { inputType: "insertText", data: "你", isComposing: false });
    assert.equal(f.api.state.particles.length, 16, "duplicate final input must not create another burst");
    f.insert("好");
    assert.equal(f.api.state.particles.length, 32, "ordinary typing after the IME commit still works");
  }
});

test("native Chinese preedit input works with an untrusted compositionend and no separate final input", () => {
  for (const contenteditable of [false, true]) {
    const f = fixture();
    const editor = contenteditable ? f.composer.appendChild(new f.FakeElement("div")) : f.editor;
    if (contenteditable) {
      editor.attrs.class = "ProseMirror"; editor.attrs.contenteditable = "true";
      f.document.activeElement = editor;
      const text = { nodeType: 3, parentElement: editor };
      f.selection({
        rangeCount: 1, anchorNode: text, focusNode: text,
        getRangeAt: () => ({ cloneRange: () => ({
          collapse() {}, getClientRects: () => [{ left: 175, right: 175, top: 220, height: 20 }],
        }) }),
      });
    }
    f.dispatch("compositionstart", {}, editor);
    for (const [index, data] of ["n", "ni", "你好"].entries()) {
      f.dispatch("compositionupdate", { data }, editor);
      if (contenteditable) editor.textContent = data;
      else { f.editor.value = data; f.editor.selectionStart = f.editor.selectionEnd = data.length; }
      f.dispatch("input", { inputType: "insertCompositionText", data, isComposing: true }, editor);
      assert.equal(f.api.state.particles.length, (index + 1) * 16, "each changed preedit string emits immediately");
      f.advance(50);
      f.dispatch("input", { inputType: "insertCompositionText", data, isComposing: true }, editor);
      assert.equal(f.api.state.particles.length, (index + 1) * 16, "duplicate preedit input does not emit again");
      f.advance(50);
    }
    f.dispatch("compositionend", { data: "你好", isTrusted: false }, editor);
    f.flush();
    assert.equal(f.api.state.particles.length, 48, "the observed native inputs remain sufficient when the end event is synthetic");
    assert.equal(contenteditable ? editor.textContent : f.editor.value, "你好");
    assert.equal(f.document.activeElement, editor);
  }
});

test("Chinese IME backspace emits for each changed preedit text and deduplicates repeated deletion input", () => {
  for (const contenteditable of [false, true]) {
    for (const data of [null, ""]) {
      const f = fixture();
      const editor = contenteditable ? f.composer.appendChild(new f.FakeElement("div")) : f.editor;
      if (contenteditable) {
        editor.attrs.class = "ProseMirror"; editor.attrs.contenteditable = "true";
        f.document.activeElement = editor;
        const text = { nodeType: 3, parentElement: editor };
        f.selection({
          rangeCount: 1, anchorNode: text, focusNode: text,
          getRangeAt: () => ({ cloneRange: () => ({
            collapse() {}, getClientRects: () => [{ left: 175, right: 175, top: 220, height: 20 }],
          }) }),
        });
      }
      const setText = (value: string) => {
        if (contenteditable) editor.textContent = value;
        else { f.editor.value = value; f.editor.selectionStart = f.editor.selectionEnd = value.length; }
      };
      f.dispatch("compositionstart", {}, editor);
      setText("ni");
      f.dispatch("input", { inputType: "insertCompositionText", data: "ni", isComposing: true }, editor);
      assert.equal(f.api.state.particles.length, 16);

      for (const [index, value] of ["n", ""].entries()) {
        f.advance(50);
        setText(value);
        f.dispatch("input", { inputType: "deleteContentBackward", data, isComposing: true }, editor);
        assert.equal(f.api.state.particles.length, (index + 2) * 16, "actual preedit deletions emit even when data remains empty or null");
        f.advance(50);
        f.dispatch("input", { inputType: "deleteContentBackward", data, isComposing: true }, editor);
        assert.equal(f.api.state.particles.length, (index + 2) * 16, "unchanged duplicate preedit deletion emits only once");
      }

      f.dispatch("compositionend", { data: "" }, editor); f.flush();
      assert.equal(f.api.state.particles.length, 48, "cancelling the empty composition adds no burst");
      assert.equal(contenteditable ? editor.textContent : f.editor.value, "");
      assert.equal(f.document.activeElement, editor);
      f.advance(50);
      setText("a");
      f.dispatch("input", { inputType: "insertText", data: "a" }, editor);
      assert.equal(f.api.state.particles.length, 64, "typing resumes after deleting and cancelling the preedit text");
    }
  }
});

test("Chinese IME preedit shortening emits when backspace arrives as insertCompositionText", () => {
  const f = fixture();
  f.editor.value = "你好"; f.editor.selectionStart = f.editor.selectionEnd = 2;
  f.dispatch("compositionstart");
  for (const [index, data] of ["ni", "n", ""].entries()) {
    f.editor.value = `你好${data}`;
    f.editor.selectionStart = f.editor.selectionEnd = f.editor.value.length;
    f.dispatch("input", { inputType: "insertCompositionText", data, isComposing: true });
    assert.equal(f.api.state.particles.length, (index + 1) * 16, "shrinking preedit text emits even when it becomes empty and restores the original text");
    f.advance(50);
    f.dispatch("input", { inputType: "insertCompositionText", data, isComposing: true });
    assert.equal(f.api.state.particles.length, (index + 1) * 16, "duplicate shortening input does not repeat the burst");
    f.advance(50);
  }
  assert.equal(f.api.state.particles[32].x, 127, "the final deletion uses the caret after the original Chinese text");
  assert.equal(f.editor.value, "你好");
  assert.equal(f.editor.selectionStart, 2);
  f.dispatch("compositionend", { data: "" }); f.flush();
  assert.equal(f.api.state.particles.length, 48);
});

test("trusted composing backspace can recover when compositionstart was not observed", () => {
  const f = fixture();
  f.editor.value = "n"; f.editor.selectionStart = f.editor.selectionEnd = 1;
  f.dispatch("input", { inputType: "deleteContentBackward", data: null, isComposing: true });
  assert.equal(f.api.state.particles.length, 16);
  f.advance(50);
  f.dispatch("input", { inputType: "deleteContentBackward", data: null, isComposing: true });
  assert.equal(f.api.state.particles.length, 16, "a duplicate recovered deletion does not emit again");
  f.advance(50);
  f.editor.value = ""; f.editor.selectionStart = f.editor.selectionEnd = 0;
  f.dispatch("input", { inputType: "deleteContentBackward", data: null, isComposing: true });
  assert.equal(f.api.state.particles.length, 32);
  assert.equal(f.api.state.particles[16].x, 111, "deleting the last preedit character emits at the empty textarea caret");
  f.dispatch("compositionend", { data: "", isTrusted: false }); f.flush();
  assert.equal(f.api.state.particles.length, 32);
});

test("a final composition input arriving after the compositionend microtask is still observed once", () => {
  const f = fixture();
  f.dispatch("compositionstart");
  f.dispatch("compositionend", { data: "中" });
  f.flush();
  assert.equal(f.api.state.particles.length, 0);
  f.insert("中", { inputType: "insertFromComposition", isComposing: false });
  assert.equal(f.api.state.particles.length, 16, "late final input must not be discarded with an unresolved composition");
  f.advance(100);
  f.dispatch("input", { inputType: "insertFromComposition", data: "中", isComposing: false });
  assert.equal(f.api.state.particles.length, 16, "late duplicate final input must not create another burst");
});

test("trusted composing input can recover when compositionstart was not observed", () => {
  const f = fixture();
  f.insert("你", { inputType: "insertCompositionText", isComposing: true });
  assert.equal(f.api.state.particles.length, 16);
  f.advance(50);
  f.editor.value = "你好"; f.editor.selectionStart = f.editor.selectionEnd = 2;
  f.dispatch("input", { inputType: "insertCompositionText", data: "你好", isComposing: true });
  assert.equal(f.api.state.particles.length, 32);
  f.advance(50);
  f.dispatch("compositionend", { data: "你好", isTrusted: false }); f.flush();
  assert.equal(f.api.state.particles.length, 32, "closing the recovered composition does not repeat its last input");
});

test("consecutive final composition inputs without compositionstart each emit while duplicates do not", () => {
  const f = fixture();
  f.insert("你", { inputType: "insertFromComposition", isComposing: false });
  assert.equal(f.api.state.particles.length, 16);
  f.advance(50);
  f.insert("好", { inputType: "insertFromComposition", isComposing: false });
  assert.equal(f.api.state.particles.length, 32, "a changed final input starts a new recovered composition");
  f.advance(50);
  f.dispatch("input", { inputType: "insertFromComposition", data: "好", isComposing: false });
  assert.equal(f.api.state.particles.length, 32, "an unchanged duplicate final input still emits only once");
});

test("a replacement editor can retry its caret while an older composition has a pending frame", () => {
  const f = fixture();
  const oldEditor = f.composer.appendChild(new f.FakeElement("div"));
  oldEditor.attrs.class = "ProseMirror"; oldEditor.attrs.contenteditable = "true";
  f.document.activeElement = oldEditor;
  f.dispatch("compositionstart", {}, oldEditor);
  oldEditor.textContent = "中";
  f.dispatch("input", { inputType: "insertCompositionText", data: "中", isComposing: true }, oldEditor);
  assert.equal(f.frames.size, 1);

  oldEditor.remove();
  const editor = f.composer.appendChild(new f.FakeElement("div"));
  editor.attrs.class = "ProseMirror"; editor.attrs.contenteditable = "true"; editor.textContent = "好";
  f.document.activeElement = editor;
  f.dispatch("input", { inputType: "insertCompositionText", data: "好", isComposing: true }, editor);
  assert.equal(f.frames.size, 1, "the replacement cancels the old frame and schedules its own retry");
  f.frame();
  const text = { nodeType: 3, parentElement: editor };
  f.selection({
    rangeCount: 1, anchorNode: text, focusNode: text,
    getRangeAt: () => ({ cloneRange: () => ({
      collapse() {}, getClientRects: () => [{ left: 250, right: 250, top: 240, height: 20 }],
    }) }),
  });
  f.frame();
  assert.equal(f.api.state.particles.length, 16, "an old pending frame must not block the replacement editor's retry");
  assert.equal(f.api.state.particles[0].x, 250);
  assert.equal(f.api.state.particles[0].y, 251);
  f.dispatch("compositionend", { data: "好", isTrusted: false }, editor); f.flush();
  assert.equal(f.api.state.particles.length, 16);
});

test("IME retries a temporarily unavailable contenteditable selection on the next frame", () => {
  const f = fixture();
  const editor = f.composer.appendChild(new f.FakeElement("div"));
  editor.attrs.class = "ProseMirror"; editor.attrs.contenteditable = "true";
  f.document.activeElement = editor;
  f.dispatch("compositionstart", {}, editor);
  editor.textContent = "中";
  f.dispatch("input", { inputType: "insertCompositionText", data: "中", isComposing: true }, editor);
  f.dispatch("compositionend", { data: "中", isTrusted: false }, editor);
  f.flush();
  assert.equal(f.api.state.particles.length, 0);
  const text = { nodeType: 3, parentElement: editor };
  f.selection({
    rangeCount: 1, anchorNode: text, focusNode: text,
    getRangeAt: () => ({ cloneRange: () => ({
      collapse() {}, getClientRects: () => [{ left: 175, right: 175, top: 220, height: 20 }],
    }) }),
  });
  f.frame();
  assert.equal(f.api.state.particles.length, 16, "the failed caret lookup must not mark the composition as emitted");
  assert.equal(f.api.state.particles[0].x, 175);
  assert.equal(f.api.state.particles[0].y, 231);
  f.advance(100);
  f.dispatch("input", { inputType: "insertText", data: "中", isComposing: false }, editor);
  assert.equal(f.api.state.particles.length, 16, "a later final input does not repeat the recovered burst");
  assert.equal(editor.textContent, "中");
  assert.equal(editor.children.length, 0);
});

test("cancelled compositions and untrusted input cannot generate effects", () => {
  for (const isTrusted of [false, true]) {
    const cancelled = fixture();
    cancelled.dispatch("compositionstart");
    cancelled.dispatch("compositionend", { data: "", isTrusted }); cancelled.flush();
    assert.equal(cancelled.api.state.particles.length, 0);
    cancelled.insert("a");
    assert.equal(cancelled.api.state.particles.length, 16, "cancelled composition must release its pending state");
  }

  for (const started of [false, true]) {
    const f = fixture();
    if (started) f.dispatch("compositionstart");
    f.insert("中", { inputType: "insertCompositionText", isComposing: true, isTrusted: false });
    f.dispatch("compositionend", { data: "中", isTrusted: false });
    f.flush(); f.frame();
    assert.equal(f.api.state.particles.length, 0, "synthetic input and end events must not authorize emission");
    assert.equal(f.api.state.canvas, null);
  }
});

test("pending IME caret retries are bounded and cancelled when the runtime is cleared", () => {
  const pending = () => {
    const f = fixture();
    const editor = f.composer.appendChild(new f.FakeElement("div"));
    editor.attrs.class = "ProseMirror"; editor.attrs.contenteditable = "true";
    f.document.activeElement = editor;
    f.dispatch("compositionstart", {}, editor);
    editor.textContent = "中";
    f.dispatch("input", { inputType: "insertCompositionText", data: "中", isComposing: true }, editor);
    f.dispatch("compositionend", { data: "中", isTrusted: false }, editor); f.flush();
    assert.ok(f.frames.size > 0, "a temporarily unavailable caret schedules a retry");
    return { f, editor };
  };
  const unresolved = pending();
  for (let index = 0; index < 6; index++) unresolved.f.frame();
  assert.equal(unresolved.f.frames.size, 0, "an unavailable caret must not keep an idle animation loop alive");
  assert.equal(unresolved.f.api.state.particles.length, 0);

  for (const reason of ["off", "focus", "window-blur", "hidden", "reduced", "reinjection", "dispose"]) {
    const { f, editor } = pending();
    if (reason === "off") { f.settings.typingEffect = "off"; f.api.sync(); }
    if (reason === "focus") f.dispatch("focusout", {}, editor);
    if (reason === "window-blur") f.windowEvent("blur");
    if (reason === "hidden") { f.document.hidden = true; f.dispatch("visibilitychange"); }
    if (reason === "reduced") f.reduced(true);
    if (reason === "reinjection") f.load().install();
    if (reason === "dispose") f.window.__codexPlusTypingEffectsRuntime?.dispose();
    assert.equal(f.frames.size, 0, reason);
    assert.equal(f.api.state.particles.length, 0, reason);
    assert.equal(f.api.state.canvas, null, reason);
  }
});

test("finishing an earlier particle burst preserves a Chinese composition still in progress", () => {
  const f = fixture(); f.insert("a"); f.advance(50); f.dispatch("compositionstart");
  for (let index = 0; index < 60; index++) f.frame();
  assert.equal(f.api.state.canvas, null);
  assert.ok(f.api.state.composition);
  f.insert("中", { inputType: "insertCompositionText", isComposing: true });
  f.dispatch("compositionend", { data: "中" }); f.flush();
  assert.equal(f.api.state.particles.length, 16);
});

test("rapid typing is throttled, caps particles and keeps one animation frame scheduled", () => {
  const f = fixture("fireworks");
  f.insert("a"); f.insert("b");
  assert.equal(f.api.state.particles.length, 26);
  for (let index = 0; index < 20; index++) { f.advance(40); f.insert("c"); }
  assert.equal(f.api.state.particles.length, 180);
  assert.equal(f.frames.size, 1);
  assert.equal(f.created.filter(node => node.tagName === "CANVAS").length, 1);
});

test("off, focus loss, visibility and reduced motion clear effects immediately", () => {
  for (const reason of ["off", "focus", "window-blur", "hidden", "reduced"]) {
    const f = fixture(); f.insert("a");
    if (reason === "off") { f.settings.typingEffect = "off"; f.api.sync(); }
    if (reason === "focus") f.dispatch("focusout", {}, f.editor);
    if (reason === "window-blur") f.windowEvent("blur");
    if (reason === "hidden") { f.document.hidden = true; f.dispatch("visibilitychange"); }
    if (reason === "reduced") f.reduced(true);
    assert.equal(f.api.state.canvas, null, reason);
    assert.equal(f.api.state.particles.length, 0, reason);
    assert.equal(f.frames.size, 0, reason);
  }
  const f = fixture(); f.reduced(true); f.insert("a");
  assert.equal(f.created.length, 0);
  f.reduced(false); f.insert("b");
  assert.equal(f.api.state.particles.length, 16);
});

test("reinstallation and reinjection do not accumulate delegated listeners or old animation frames", () => {
  const f = fixture(); f.insert("a");
  for (let index = 0; index < 3; index++) f.api.install();
  assert.equal(f.documentListeners.get("input")?.size, 1);
  const oldCanvas = f.api.state.canvas;
  const replacement = f.load();
  replacement.sync(); replacement.install();
  assert.equal(oldCanvas?.isConnected, false);
  assert.equal(f.frames.size, 0);
  assert.equal(f.documentListeners.get("input")?.size, 1);
  assert.equal(f.windowListeners.get("blur")?.size, 1);
  assert.equal(f.mediaListeners.size, 1);
  f.insert("b");
  assert.equal(replacement.state.particles.length, 16);
  f.window.__codexPlusTypingEffectsRuntime?.dispose();
  assert.equal(f.documentListeners.get("input")?.size, 0);
  assert.equal(f.mediaListeners.size, 0);
  assert.equal(f.frames.size, 0);
});
