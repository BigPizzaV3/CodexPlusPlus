const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const vm = require("node:vm");
const dictationSource = readFileSync(`${__dirname}/renderer-inject/93-dictation.js`, "utf8");
const source = readFileSync(`${__dirname}/renderer-inject/93-prompt-enhance.js`, "utf8");
const flush = () => new Promise(resolve => setImmediate(resolve));

// 极简 ProseMirror 文档：只模拟提示词增强用到的读、替换、追加与选区。
function proseMirror(lines, extra = []) {
  const textNode = text => ({ isText: true, type: { name: "text" }, text });
  const paragraph = text => ({ type: { name: "paragraph" }, children: text ? [textNode(text)] : [] });
  const doc = blocks => ({
    blocks,
    get content() { return { size: blocks.length * 2 }; },
    textBetween() { return blocks.map(block => block.children.map(child => child.text ?? child.attrs?.label ?? "").join("")).join("\n"); },
    descendants(visit) { for (const block of blocks) if (visit(block) !== false) for (const child of block.children) visit(child); },
  });
  const schema = {
    nodes: { paragraph: { create: (_, child) => ({ type: { name: "paragraph" }, children: child ? [child] : [] }) } },
    text: textNode,
  };
  const selection = { constructor: { atEnd: current => ({ from: current.content.size, atEnd: true }) } };
  const view = { isDestroyed: false, dispatched: [], state: { doc: doc(lines.map(paragraph)), schema, selection } };
  if (extra.length) view.state.doc.blocks[0].children.push(...extra);
  Object.defineProperty(view.state, "tr", { get() {
    const tr = {
      doc: view.state.doc, steps: [],
      replaceWith(from, to, nodes) { tr.steps.push(["replaceWith", from, to]); tr.doc = doc(nodes); return tr; },
      insert(at, nodes) { tr.steps.push(["insert", at]); tr.doc = doc([...tr.doc.blocks, ...nodes]); return tr; },
      insertText(text) { tr.steps.push(["insertText", text]); return tr; },
      setSelection(value) { tr.selection = value; return tr; },
      scrollIntoView() { return tr; },
    };
    return tr;
  } });
  view.dispatch = tr => { view.dispatched.push(tr); view.state.doc = tr.doc; };
  return view;
}

function fixture({ editorKind = "prosemirror", lines = ["修一下登录页"], extra = [], respond } = {}) {
  const requests = [], toasts = [], events = [], appended = [];
  let resolveResponse;
  class Textarea {
    constructor(value) { this._value = value; this.dataset = {}; this.isConnected = true; }
    get value() { return this._value; }
    set value(value) { this._value = value; }
    setSelectionRange(from, to) { this.selectionStart = from; this.selectionEnd = to; }
    focus() { document.activeElement = this; }
    dispatchEvent(event) { events.push(event.type); }
    closest() { return null; }
    matches() { return false; }
  }
  let editor;
  if (editorKind === "textarea") editor = new Textarea(lines.join("\n"));
  else {
    editor = { isConnected: true, dataset: {}, focus() { document.activeElement = editor; }, closest: () => null, matches: () => false };
    const view = proseMirror(lines, extra);
    view.dom = editor;
    editor.pmViewDesc = { view };
  }
  const document = {
    body: { append: node => appended.push(node) }, head: { append() {} }, activeElement: null,
    querySelectorAll: () => [], getElementById: () => null,
    addEventListener() {}, removeEventListener() {},
    createElement: () => ({ dataset: {}, style: {}, append() {}, setAttribute() {}, addEventListener() {}, replaceChildren() {} }),
    execCommand: () => { throw Error("prompt enhance must use a native transaction or controlled textarea"); },
  };
  const window = { location: { href: "app://-/thread/one" }, innerWidth: 1200, innerHeight: 800, addEventListener() {}, removeEventListener() {} };
  const context = vm.createContext({
    window, document, navigator: {}, HTMLTextAreaElement: Textarea, Event, console, crypto: { randomUUID: () => "id" },
    requestAnimationFrame: () => {}, setInterval, clearInterval, setTimeout,
    registerCodexPlusExtensionSelector() {}, visibleElement: () => true, isExtensionUiNode: () => false,
    codexPlusBackendSettingsLoaded: true,
    codexPlusBackendSettings: { promptEnhance: { enabled: true }, dictation: { enabled: false } },
    showToast: message => toasts.push(message), loadOptionalCodexAppModule: async () => ({}),
    postJson: async (path, payload) => {
      requests.push({ path, payload });
      if (respond) return respond(payload);
      return new Promise(resolve => { resolveResponse = resolve; });
    },
  });
  vm.runInContext(`${dictationSource}\n${source}\n globalThis.testApi = {
    state: codexPlusPromptEnhance, read: codexPlusPromptEnhanceRead, write: codexPlusPromptEnhanceWrite,
    start: startCodexPlusPromptEnhance, apply: applyCodexPlusPromptEnhance, close: closeCodexPlusPromptEnhance,
    blocker: codexPlusPromptEnhanceReplaceBlocker, enabled: codexPlusPromptEnhanceEnabled,
  };`, context);
  context.renderCodexPlusPromptEnhance = () => {};
  // 跳过 DOM 布局查询，直接把 footer 映射到测试输入框。
  context.codexPlusDictationScope = () => ({});
  context.codexPlusDictationEditor = () => editor;
  return { ...context.testApi, context, editor, requests, toasts, events, resolve: value => resolveResponse(value) };
}

(async () => {
  const read = fixture({ lines: ["第一行", "", "第三行"] });
  assert.deepEqual({ ...read.read(read.editor) }, { text: "第一行\n\n第三行", rich: false });
  const rich = fixture({ extra: [{ type: { name: "mention" }, attrs: { label: "@App.tsx" } }] });
  assert.equal(rich.read(rich.editor).rich, true, "@file chips or attachments must block whole-draft replacement");

  // 确认替换：只发草稿、不碰输入框，确认时一次事务整段替换。
  const flow = fixture({ lines: ["修一下登录页"] });
  const pending = flow.start({});
  await flush();
  assert.deepEqual(JSON.parse(JSON.stringify(flow.requests)), [{ path: "/prompt-enhance/generate", payload: { prompt: "修一下登录页" } }]);
  assert.equal(flow.state.phase, "loading");
  assert.equal(flow.editor.pmViewDesc.view.dispatched.length, 0, "the draft stays untouched while generating");
  flow.resolve({ status: "ok", text: "目标：修复登录页\n要求：保留现有样式" });
  await pending;
  assert.equal(flow.state.phase, "result");
  assert.equal(flow.blocker(), "");
  assert.equal(flow.apply("replace", flow.state.text), true);
  const view = flow.editor.pmViewDesc.view;
  assert.equal(view.dispatched.length, 1, "replacement is a single undoable transaction");
  assert.deepEqual(JSON.parse(JSON.stringify(view.dispatched[0].steps)), [["replaceWith", 0, 2]]);
  assert.equal(view.state.doc.textBetween(), "目标：修复登录页\n要求：保留现有样式");
  assert.equal(view.dispatched[0].selection.atEnd, true, "caret ends after the inserted prompt");
  assert.equal(flow.state.phase, "idle");

  // 取消：不写回，迟到的结果也不再生效。
  const cancel = fixture();
  const cancelled = cancel.start({});
  await flush();
  cancel.close();
  cancel.resolve({ status: "ok", text: "迟到的结果" });
  await cancelled;
  assert.equal(cancel.state.phase, "idle");
  assert.equal(cancel.editor.pmViewDesc.view.dispatched.length, 0, "cancel leaves the draft unchanged");

  // 生成期间用户改了草稿：拒绝整段替换，只允许追加。
  const edited = fixture({ lines: ["原稿"] });
  const editedPending = edited.start({});
  await flush();
  edited.editor.pmViewDesc.view.state.doc.blocks[0].children[0].text = "原稿加了一句";
  edited.resolve({ status: "ok", text: "增强稿" });
  await editedPending;
  assert.match(edited.blocker(), /草稿有改动/);
  assert.equal(edited.apply("replace", "增强稿"), false);
  assert.equal(edited.editor.pmViewDesc.view.dispatched.length, 0);
  assert.equal(edited.apply("append", "增强稿"), true);
  assert.deepEqual(JSON.parse(JSON.stringify(edited.editor.pmViewDesc.view.dispatched[0].steps)), [["insert", 2]]);
  assert.equal(edited.editor.pmViewDesc.view.state.doc.textBetween(), "原稿加了一句\n增强稿");

  // 含 @文件 的草稿同样只能追加。
  const richFlow = fixture({ extra: [{ type: { name: "mention" }, attrs: { label: "@App.tsx" } }], respond: () => ({ status: "ok", text: "增强稿" }) });
  await richFlow.start({});
  assert.match(richFlow.blocker(), /@文件/);
  assert.equal(richFlow.apply("replace", "增强稿"), false);

  // 切换会话后不能写进别的会话。
  const moved = fixture({ respond: () => ({ status: "ok", text: "增强稿" }) });
  await moved.start({});
  moved.context.window.location.href = "app://-/thread/two";
  assert.equal(moved.apply("append", "增强稿"), false);
  assert.equal(moved.editor.pmViewDesc.view.dispatched.length, 0);

  // 失败结果展示错误、不写回。
  const failure = fixture({ respond: () => ({ status: "failed", message: "提示词增强未配置 API Key" }) });
  await failure.start({});
  assert.equal(failure.state.phase, "error");
  assert.equal(failure.state.error, "提示词增强未配置 API Key");

  // 空草稿不发请求。
  const empty = fixture({ lines: ["  "] });
  await empty.start({});
  assert.equal(empty.requests.length, 0);
  assert.equal(empty.toasts.length, 1);

  // textarea 兼容路径：受控 setter + input 事件。
  const textarea = fixture({ editorKind: "textarea", lines: ["旧稿"], respond: () => ({ status: "ok", text: "新稿" }) });
  await textarea.start({});
  assert.equal(textarea.apply("replace", "新稿"), true);
  assert.equal(textarea.editor.value, "新稿");
  assert.deepEqual(textarea.events, ["input"]);
  assert.equal(textarea.write(textarea.editor, "补充", "append"), true);
  assert.equal(textarea.editor.value, "新稿\n\n补充");

  // 关闭开关后入口不可用。
  const disabled = fixture();
  disabled.context.codexPlusBackendSettings.promptEnhance.enabled = false;
  assert.equal(disabled.enabled(), false);
  await disabled.start({});
  assert.equal(disabled.requests.length, 0);
  disabled.context.codexPlusBackendSettings.promptEnhance.enabled = true;
  disabled.context.codexPlusBackendSettings.enhancementsEnabled = false;
  assert.equal(disabled.enabled(), false, "the master enhancement switch pauses prompt enhance");

  console.log("prompt enhance read, preview, replace, append, cancel and stale-draft contracts passed");
})().catch(error => { console.error(error); process.exit(1); });
