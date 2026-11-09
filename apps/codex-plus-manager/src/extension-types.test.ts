import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { normalize } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, it } from "node:test";
import ts from "typescript";

const declarationPath = fileURLToPath(new URL("../../../types/codex-plus-extensions.d.ts", import.meta.url));
const declarations = parse(declarationPath);
const apiSource = parse(fileURLToPath(new URL("../../../assets/inject/renderer-inject/91-extension-api.js", import.meta.url)));
const contextSource = parse(fileURLToPath(new URL("../../../assets/inject/renderer-inject/91-extension-runtime.js", import.meta.url)));

function parse(path: string) {
  return ts.createSourceFile(path, readFileSync(path, "utf8"), ts.ScriptTarget.Latest, true);
}

function findNode<T extends ts.Node>(root: ts.Node, match: (node: ts.Node) => node is T): T {
  let found: T | undefined;
  function visit(node: ts.Node) {
    if (found) return;
    if (match(node)) found = node;
    else ts.forEachChild(node, visit);
  }
  visit(root);
  assert.ok(found, `找不到契约节点：${root.getSourceFile().fileName}`);
  return found;
}

function memberName(member: { name?: ts.PropertyName }) {
  assert.ok(member.name && (ts.isIdentifier(member.name) || ts.isStringLiteral(member.name)), "公开成员需要静态名称");
  return member.name.text;
}

function interfaceMembers(name: string) {
  const declaration = findNode(declarations, (node): node is ts.InterfaceDeclaration =>
    ts.isInterfaceDeclaration(node) && node.name.text === name);
  return declaration.members.map(memberName);
}

function runtimeRoutes() {
  const declaration = findNode(apiSource, (node): node is ts.VariableDeclaration =>
    ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.name.text === "codexPlusExtensionRoutes");
  const initializer = declaration.initializer;
  assert.ok(initializer && ts.isNewExpression(initializer) && initializer.expression.getText() === "Set");
  const routes = initializer.arguments?.[0];
  assert.ok(routes && ts.isArrayLiteralExpression(routes), "路由白名单必须是显式数组");
  return routes.elements.map((route) => {
    assert.ok(ts.isStringLiteral(route), "公开路由需要字符串字面量");
    return route.text;
  });
}

function compileUsage(source: string) {
  const fixturePath = fileURLToPath(new URL("./extension-types.fixture.ts", import.meta.url));
  const options: ts.CompilerOptions = {
    strict: true, noEmit: true, types: [], target: ts.ScriptTarget.ES2022,
    lib: ["lib.es2022.d.ts", "lib.dom.d.ts"],
  };
  const host = ts.createCompilerHost(options);
  const getSourceFile = host.getSourceFile;
  host.getSourceFile = (fileName, languageVersion, onError, shouldCreateNewSourceFile) =>
    normalize(fileName) === normalize(fixturePath)
      ? ts.createSourceFile(fileName, source, languageVersion, true)
      : getSourceFile(fileName, languageVersion, onError, shouldCreateNewSourceFile);
  const program = ts.createProgram([declarationPath, fixturePath], options, host);
  const diagnostics = ts.getPreEmitDiagnostics(program);
  assert.equal(diagnostics.length, 0, ts.formatDiagnostics(diagnostics, {
    getCanonicalFileName: (name) => name, getCurrentDirectory: () => "", getNewLine: () => "\n",
  }));
}

describe("拓展公开类型契约", () => {
  it("完整路由类型与运行时白名单严格一致", () => {
    const declaration = findNode(declarations, (node): node is ts.TypeAliasDeclaration =>
      ts.isTypeAliasDeclaration(node) && node.name.text === "CodexPlusExtensionRoute");
    assert.ok(ts.isUnionTypeNode(declaration.type));
    const declared = declaration.type.types.map((type) => {
      assert.ok(ts.isLiteralTypeNode(type) && ts.isStringLiteral(type.literal), "路由类型不能放宽为任意字符串");
      return type.literal.text;
    });
    assert.deepEqual(declared.sort(), runtimeRoutes().sort(), "新增或删除公开路由时必须同步类型声明");
  });

  it("公开 API 成员均有声明，专用生成器接口单独维护", () => {
    const build = findNode(apiSource, (node): node is ts.FunctionDeclaration =>
      ts.isFunctionDeclaration(node) && node.name?.text === "buildCodexPlusExtensionApi");
    const result = build.body?.statements.find(ts.isReturnStatement)?.expression;
    assert.ok(result && ts.isObjectLiteralExpression(result));
    // dshClient 仅供生成的 dsh 配套拓展包调用，不属于普通脚本公开契约。
    const runtime = result.properties.map(memberName).filter((name) => name !== "dshClient");
    assert.deepEqual(interfaceMembers("CodexPlusExtensionApi").sort(), runtime.sort());
  });

  it("实例成员均有声明，内部方法与动态服务各自显式区分", () => {
    const instantiate = findNode(contextSource, (node): node is ts.FunctionDeclaration =>
      ts.isFunctionDeclaration(node) && node.name?.text === "codexPlusInstantiateExtension");
    const ctx = findNode(instantiate, (node): node is ts.VariableDeclaration =>
      ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.name.text === "ctx").initializer;
    assert.ok(ctx && ts.isObjectLiteralExpression(ctx));
    const runtime = ctx.properties.map(memberName).filter((name) => name !== "__tryConfigUpdate");
    // inject 服务与 fiber 在构建完 ctx 后动态挂载，不是对象字面量里的成员。
    const dynamic = new Set(["sessionQuery", "sessions", "fiber"]);
    const declared = interfaceMembers("CodexPlusExtensionContext").filter((name) => !dynamic.has(name));
    assert.deepEqual(declared.sort(), runtime.sort());
  });

  it("公开路由、回调及生命周期可以按真实用法通过严格编译", () => {
    compileUsage(`
      const api = window.codexPlus;
      declare const ctx: CodexPlusExtensionContext;
      ${runtimeRoutes().map((route) => `void api.call(${JSON.stringify(route)}); void ctx.call(${JSON.stringify(route)});`).join("\n")}
      const session = api.currentSession();
      const sessionId: string = session.id;
      const title: string = session.title;
      const hostId: string = session.hostId;
      const remote: boolean = session.remote;
      // @ts-expect-error 当前会话是冻结的快照。
      session.title = "changed";
      const removeMenu: CodexPlusDispose = api.registerMenuItem({
        label: "Enabled", toggleValue: () => true,
        onChange(next) { const enabled: boolean = next; },
      });
      ctx.registerMenuItem({ label: "Open", buttonLabel: "Show", onActivate({ close }) { close(); } });
      ctx.registerMenuItem({ order: 1200, customMetadata: "retained", onActivate() {} });
      const settings: CodexPlusSettingsPanelDefinition = {
        render({ container, config, saveConfig }) {
          const element: HTMLElement = container;
          const enabled: unknown = config.enabled;
          const saved: Promise<Record<string, unknown>> = saveConfig({ enabled: true });
          return () => element.replaceChildren();
        },
      };
      const removeSettings: CodexPlusDispose = api.registerSettingsPanel(settings);
      const removeInstanceSettings: CodexPlusDispose = ctx.registerSettingsPanel(settings);
      ctx.registerCommand({
        name: "inspect",
        async run(args, { sessionId, editor, insertText, setText, toast }) {
          const id: string = sessionId;
          const argument: string = args;
          const target: HTMLElement = editor;
          const inserted: boolean = insertText(id);
          const replaced: boolean = setText(argument);
          toast(id);
        },
      });
      const cleanup = () => {};
      const syncEffect: CodexPlusDispose = ctx.effect(() => cleanup);
      const asyncEffect: CodexPlusDispose = ctx.effect(async () => cleanup);
      ctx.effect(() => {});
      ctx.effect(async () => {});
      // @ts-expect-error 内部写路由不属于公开白名单。
      void api.call("/settings/set");
      // @ts-expect-error ctx 同样不能调用内部写路由。
      void ctx.call("/delete");
      // @ts-expect-error 开关回调接收 boolean。
      ctx.registerMenuItem({ label: "Invalid", onChange(next: string) {} });
      // @ts-expect-error effect 只能返回清理函数或 void。
      ctx.effect(async () => 42);
      // @ts-expect-error 设置面板的 render 只支持同步返回清理函数。
      api.registerSettingsPanel({ async render() {} });
      // @ts-expect-error 生成器专用接口不属于普通脚本契约。
      api.dshClient;
      // @ts-expect-error 配置更新试探方法仅供内核调用。
      ctx.__tryConfigUpdate;
    `);
  });
});
