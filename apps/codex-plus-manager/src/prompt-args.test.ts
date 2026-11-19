import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";

// 内置拓展「自定义 prompt 参数」：回车发送 /prompts:名字 参数 时按 Codex CLI 规则展开占位符。
const source = readFileSync(new URL("../../../assets/inject/renderer-inject/96-prompt-args.js", import.meta.url), "utf8");

function load(overrides: Record<string, unknown> = {}) {
  const calls: string[] = [];
  const timers: Array<() => void> = [];
  const editor = { innerText: "", id: "editor" };
  const context: Record<string, any> = vm.createContext({
    codexPlusBackendSettingsLoaded: true,
    settings: { promptArgs: true },
    codexPlusSettings: () => context.settings,
    prompts: [] as unknown[],
    postJson: async (path: string) => {
      calls.push(`post:${path}`);
      return { status: "ok", prompts: context.prompts };
    },
    showToast: (message: string, options: { type?: string } = {}) => calls.push(`toast:${options.type || ""}:${message}`),
    codexPlusComposerEditorFrom: (target: unknown) => (target === editor ? editor : null),
    codexPlusClearComposer: () => {
      calls.push("clear");
      editor.innerText = "";
    },
    codexPlusDictationSendButton: () => ({ click: () => calls.push(`send:${editor.innerText}`) }),
    document: {
      execCommand: (command: string, _ui: boolean, text: string) => {
        calls.push(`${command}:${text}`);
        editor.innerText += text;
        return true;
      },
    },
    setTimeout: (fn: () => void) => timers.push(fn),
    Date,
    Map,
    Promise,
    ...overrides,
  });
  vm.runInContext(`${source}\nthis.cache = codexPlusPromptArgsCache;`, context);
  return { context, calls, timers, editor };
}

const expand = (content: string, args: string) => load().context.codexPlusPromptArgsExpand({ id: "p", content }, args);

test("参数按空白拆分，引号包住的算一个", () => {
  const { context } = load();
  const split = (text: string) => [...context.codexPlusPromptArgsSplit(text)];
  assert.deepEqual(split(`a  b\t"c d" 'e "f"' "g \\"h\\"" ""`), ["a", "b", "c d", `e "f"`, `g "h"`, ""]);
  assert.deepEqual(split(`KEY="x y" z`), ["KEY=x y", "z"]);
  assert.deepEqual(split("   "), []);
});

test("位置参数：$1…$9 与 $ARGUMENTS，缺的换成空，$$ 是字面 $", () => {
  assert.deepEqual({ ...expand("Fix $1 in $2 ($9) costs $$5", `bug "src/a b.ts"`) }, { ok: true, text: "Fix bug in src/a b.ts () costs $5", hint: "" });
  assert.equal(expand("All: $ARGUMENTS!", `one "two three"`).text, "All: one two three!");
});

test("具名参数：名字=值 替换，缺了或写法不对就报错不发送", () => {
  assert.equal(expand("Review $BRANCH against $BASE", "BRANCH=feat BASE=main").text, "Review feat against main");
  const missing = expand("Review $BRANCH against $BASE", "BRANCH=feat");
  assert.equal(missing.ok, false);
  assert.match(missing.error, /缺少参数：BASE/);
  const bad = expand("Review $BRANCH", "BRANCH=feat stray");
  assert.equal(bad.ok, false);
  assert.match(bad.error, /stray/);
  // 具名模式下 $1 不是占位符，原样保留。
  assert.equal(expand("$BRANCH $1", "BRANCH=x").text, "x $1");
});

test("正文里的 $HOME 这类没按 名字=值 传时，不当具名参数拦截", () => {
  const result = expand("Run in $HOME: $1", "build");
  assert.equal(result.ok, true);
  assert.equal(result.text, "Run in $HOME: build");
  assert.match(result.hint, /\$HOME/);
  // 没有可替换的占位符：和原生一样接在正文后面。
  assert.deepEqual({ ...expand("Explain this.", "  foo bar ") }, { ok: true, text: "Explain this. foo bar", hint: "" });
});

test("回车发送：展开后清空输入框、写入展开结果、再点发送按钮", async () => {
  const { context, calls, timers, editor } = load();
  context.prompts = [{ id: "fix", content: "Fix $1 now" }];
  await context.codexPlusPromptArgsRefresh(true);
  editor.innerText = "/prompts:fix  the-bug ";
  let prevented = 0;
  const event = { key: "Enter", target: editor, preventDefault: () => prevented++, stopImmediatePropagation: () => prevented++ };
  context.codexPlusPromptArgsKeyDown(event);
  assert.equal(prevented, 2);
  assert.deepEqual(calls.slice(1), ["clear", "insertText:Fix the-bug now"]);
  timers.forEach((fn) => fn());
  assert.equal(calls.at(-1), "send:Fix the-bug now");
});

test("缺参数时拦下回车并提示，输入框保持原样", async () => {
  const { context, calls, timers, editor } = load();
  context.prompts = [{ id: "review", content: "Review $BRANCH vs $BASE" }];
  await context.codexPlusPromptArgsRefresh(true);
  editor.innerText = "/prompts:review BRANCH=x";
  let prevented = false;
  context.codexPlusPromptArgsKeyDown({ key: "Enter", target: editor, preventDefault: () => (prevented = true), stopImmediatePropagation() {} });
  assert.equal(prevented, true);
  assert.equal(editor.innerText, "/prompts:review BRANCH=x");
  assert.match(calls.at(-1)!, /^toast:error:.*缺少参数：BASE/);
  assert.equal(timers.length, 0);
});

test("不该介入的回车交给原生：无参数、未知 prompt、Shift/输入法、设置关闭", async () => {
  const { context, editor } = load();
  context.prompts = [{ id: "fix", content: "Fix $1" }];
  await context.codexPlusPromptArgsRefresh(true);
  const pressed = (text: string, extra: Record<string, unknown> = {}) => {
    editor.innerText = text;
    let prevented = false;
    context.codexPlusPromptArgsKeyDown({ key: "Enter", target: editor, preventDefault: () => (prevented = true), stopImmediatePropagation() {}, ...extra });
    return prevented;
  };
  assert.equal(pressed("/prompts:fix"), false);
  assert.equal(pressed("/prompts:other x"), false);
  assert.equal(pressed("hello /prompts:fix x"), false);
  assert.equal(pressed("/prompts:fix x", { shiftKey: true }), false);
  assert.equal(pressed("/prompts:fix x", { isComposing: true }), false);
  context.settings = { promptArgs: false };
  assert.equal(pressed("/prompts:fix x"), false);
  context.settings = { promptArgs: true };
  assert.equal(pressed("/prompts:fix x"), true);
});

test("清单按需刷新：输入 /prompts: 时超过 3 秒才重读，并发只发一次", async () => {
  const { context, calls, editor } = load();
  await context.codexPlusPromptArgsRefresh(true);
  const posts = () => calls.filter((call) => call.startsWith("post:")).length;
  assert.equal(posts(), 1);
  editor.innerText = "/prompts:";
  context.codexPlusPromptArgsInput({ target: editor });
  assert.equal(posts(), 1, "3 秒内不重读");
  context.cache.at = 0;
  context.codexPlusPromptArgsInput({ target: editor });
  context.codexPlusPromptArgsInput({ target: editor });
  assert.equal(posts(), 2, "并发合并成一次");
  editor.innerText = "hello";
  context.cache.at = 0;
  await Promise.resolve();
  context.codexPlusPromptArgsInput({ target: editor });
  assert.equal(posts(), 2, "不是 /prompts: 开头不读");
});

test("拓展装上时登记捕获阶段的 keydown / input 监听，并预读清单", () => {
  const { context, calls } = load();
  const listened: string[] = [];
  context.codexPlusPromptArgsExtension({ listen: (_target: unknown, type: string, _fn: unknown, options: { capture?: boolean }) => listened.push(`${type}:${options.capture}`) });
  assert.deepEqual(listened, ["keydown:true", "input:true"]);
  assert.deepEqual(calls, ["post:/custom-prompts/list"]);
});
