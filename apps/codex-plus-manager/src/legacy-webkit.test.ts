/**
 * 旧版系统 WebKit 兼容（#2753：旧 Intel Mac 上 2.x 打开就是黑屏）。
 * 直接扫源码 / 构建配置，防止又把旧 WebKit 解析不了的语法、没有的 API 带回来。
 */
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import test from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import ts from "typescript";
import { maskAccountName, maskAccountText } from "./gateway-model.ts";
import { ReleaseNotes } from "./ReleaseNotes.ts";

function readSource(path: string) {
  return readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
}

test("源码里不用正则后行断言：旧 WebKit 解析不了，构建也降不了，模块一加载就抛 SyntaxError", () => {
  const dir = new URL("./", import.meta.url);
  const files = readdirSync(dir).filter((name) => /\.(ts|tsx)$/.test(name) && !name.endsWith(".test.ts"));
  assert.ok(files.length > 50);
  const offenders: string[] = [];
  for (const name of files) {
    const source = ts.createSourceFile(name, readSource(`./${name}`), ts.ScriptTarget.Latest, true, name.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
    const visit = (node: ts.Node) => {
      const isRegexLiteral = node.kind === ts.SyntaxKind.RegularExpressionLiteral;
      // new RegExp("...") 的字面量参数同样检查。
      const isRegexCtorArg = ts.isStringLiteralLike(node) && ts.isNewExpression(node.parent) && node.parent.expression.getText(source) === "RegExp";
      if ((isRegexLiteral || isRegexCtorArg) && /\(\?<[=!]/.test(node.getText(source))) {
        offenders.push(`${name}:${source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1}`);
      }
      ts.forEachChild(node, visit);
    };
    visit(source);
  }
  assert.deepEqual(offenders, []);
});

test("邮箱打码改写成不带后行断言后行为不变：已打码的尾巴不再打一次，行首和句中的邮箱都能识别", () => {
  assert.equal(maskAccountName("ChatGPT Pro · 2649340134@qq.com"), "ChatGPT Pro · 26****34@qq.com");
  assert.equal(maskAccountText("me@example.com 和 you@example.org"), "m***@example.com 和 y***@example.org");
  // 邮箱前面的那个字符被捕获后要原样放回，不能吃掉。
  const masked = maskAccountText("（备用 ab.cd@x.io）");
  assert.match(masked, /^（备用 a[^@\s]*\*[^@\s]*@x\.io）$/);
  for (const text of ["联系 ab.cd@x.io", "已添加「ChatGPT Pro · me@example.com」", "a@b.co,c@d.com"]) {
    const once = maskAccountText(text);
    assert.notEqual(once, text, text);
    assert.equal(maskAccountText(once), once, text);
  }
});

test("更新说明的表格按未转义的 | 切列（不再用后行断言）", () => {
  const html = renderToStaticMarkup(createElement(ReleaseNotes, {
    markdown: "| 项目 | 说明 |\n| --- | --- |\n| a\\|b | c |",
    onOpenUrl: () => {},
  }));
  const cells = [...html.matchAll(/<t[dh][^>]*>([\s\S]*?)<\/t[dh]>/g)].map((match) => match[1]);
  assert.deepEqual(cells.length, 4, html);
  assert.match(cells[2], /a\\\|b|a\|b/);
  assert.equal(cells[3], "c");
});

test("构建目标显式钉在 safari14（BigInt 的下限），不跟着 Vite 默认值漂移", () => {
  const config = readSource("../vite.config.ts");
  assert.match(config, /target: \["es2020", "safari14", "chrome87", "edge88", "firefox78"\],/);
  assert.match(config, /cssTarget: \["safari14", "chrome87", "edge88", "firefox78"\],/);
});

test("启动兜底：API 补丁最先执行；渲染出错有错误边界；模块加载失败由 index.html 的普通脚本接住，不再黑屏", () => {
  const main = readSource("./main.tsx");
  const firstImport = main.split("\n").find((line) => line.startsWith("import "));
  assert.equal(firstImport, 'import "./legacy-webkit";');
  assert.match(main, /class StartupErrorBoundary extends Component/);
  assert.match(main, /static getDerivedStateFromError\(/);
  // 各窗口界面按需加载，加载完包在错误边界里渲染；分块加载失败也交给错误边界显示，不留黑屏。
  assert.match(main, /<StartupErrorBoundary>\n\s*<Root \/>\n\s*<\/StartupErrorBoundary>/);
  assert.match(main, /<StartupErrorBoundary>\n\s*<LoadFailure error=\{error\} \/>\n\s*<\/StartupErrorBoundary>/);
  assert.doesNotMatch(main, /^import \{ (App|TrayPanel|CuPipWindow) \}/m);

  const html = readSource("../index.html");
  const inline = html.indexOf("<script>");
  const moduleScript = html.indexOf('<script type="module" src="/src/main.tsx"></script>');
  assert.ok(inline > 0 && inline < moduleScript, "兜底脚本必须是普通脚本且排在入口模块前面");
  const fallback = html.slice(inline, html.indexOf("</script>", inline));
  assert.match(fallback, /window\.addEventListener\("error"/);
  assert.match(fallback, /root\.childElementCount > 0\) return;/);
  // 兜底脚本自己只用最老的语法，旧 WebKit 一定能跑。
  assert.doesNotMatch(fallback, /=>|\blet\b|\bconst\b|`|\?\.|\?\?/);
});

test("缺 Object.hasOwn / Array#at / String#at 时补上，行为与标准一致", async () => {
  const saved = {
    hasOwn: Object.getOwnPropertyDescriptor(Object, "hasOwn"),
    arrayAt: Object.getOwnPropertyDescriptor(Array.prototype, "at"),
    stringAt: Object.getOwnPropertyDescriptor(String.prototype, "at"),
  };
  try {
    delete (Object as { hasOwn?: unknown }).hasOwn;
    delete (Array.prototype as { at?: unknown }).at;
    delete (String.prototype as { at?: unknown }).at;
    await import(`./legacy-webkit.ts?polyfill=${Date.now()}`);
    const hasOwn = (Object as unknown as { hasOwn: (object: unknown, key: PropertyKey) => boolean }).hasOwn;
    assert.equal(hasOwn({ a: 1 }, "a"), true);
    assert.equal(hasOwn({ a: 1 }, "toString"), false);
    assert.throws(() => hasOwn(null, "a"), TypeError);
    const items = [1, 2, 3] as unknown as { at: (index: number) => number | undefined };
    assert.equal(items.at(-1), 3);
    assert.equal(items.at(0), 1);
    assert.equal(items.at(3), undefined);
    assert.equal(items.at(-4), undefined);
    assert.equal(("a/b/c" as unknown as { at: (index: number) => string | undefined }).at(-1), "c");
    assert.equal(Object.keys([]).includes("at"), false, "补丁不可枚举");
  } finally {
    for (const [target, key, descriptor] of [
      [Object, "hasOwn", saved.hasOwn],
      [Array.prototype, "at", saved.arrayAt],
      [String.prototype, "at", saved.stringAt],
    ] as const) {
      if (descriptor) Object.defineProperty(target, key, descriptor);
    }
  }
});

test("没有 HTMLDialogElement 时给 <dialog> 补 showModal / close / open，Esc 先派发 cancel", async () => {
  const listeners: Record<string, (event: { key: string }) => void> = {};
  const appended: Array<{ textContent: string }> = [];
  class FakeUnknownElement {
    tagName = "DIALOG";
    attrs = new Map<string, string>();
    events: string[] = [];
    hasAttribute(name: string) { return this.attrs.has(name); }
    setAttribute(name: string, value: string) { this.attrs.set(name, value); }
    removeAttribute(name: string) { this.attrs.delete(name); }
    dispatchEvent(event: Event) { this.events.push(event.type); return true; }
  }
  const dialogs: FakeUnknownElement[] = [];
  const globals = globalThis as Record<string, unknown>;
  const savedGlobals = { window: globals.window, document: globals.document, HTMLUnknownElement: globals.HTMLUnknownElement };
  globals.window = {};
  globals.HTMLUnknownElement = FakeUnknownElement;
  globals.document = {
    head: { appendChild: (node: { textContent: string }) => appended.push(node) },
    createElement: () => ({ setAttribute() {}, textContent: "" }),
    addEventListener: (type: string, listener: (event: { key: string }) => void) => { listeners[type] = listener; },
    querySelectorAll: () => dialogs.filter((dialog) => dialog.hasAttribute("open") && dialog.hasAttribute("data-codex-plus-dialog")),
  };
  try {
    await import(`./legacy-webkit.ts?dialog=${Date.now()}`);
    const dialog = new FakeUnknownElement() as FakeUnknownElement & { open: boolean; showModal: () => void; close: () => void };
    dialogs.push(dialog);
    assert.equal(dialog.open, false);
    dialog.showModal();
    assert.equal(dialog.open, true);
    listeners.keydown({ key: "Escape" });
    assert.deepEqual(dialog.events, ["cancel", "close"]);
    assert.equal(dialog.open, false);
    dialog.close();
    assert.deepEqual(dialog.events, ["cancel", "close"], "已关闭的不再派发 close");
    assert.match(appended[0].textContent, /dialog:not\(\[open\]\)\{display:none\}/);
  } finally {
    globals.window = savedGlobals.window;
    globals.document = savedGlobals.document;
    globals.HTMLUnknownElement = savedGlobals.HTMLUnknownElement;
  }
});
