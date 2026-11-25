import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { setImmediate as settle } from "node:timers/promises";
import vm from "node:vm";
import ts from "typescript";
import * as jsxRuntime from "react/jsx-runtime";
import { mergeRotatedAuthContents, rebaseSettingsEdits } from "./settings-rebase.ts";

const read = (file: string) => readFileSync(new URL(file, import.meta.url), "utf8");
const compile = (code: string) => ts.transpileModule(code, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX },
}).outputText;
const dialogCode = compile(read("./AgentConfigEditor.tsx"));
const app = ts.createSourceFile("App.tsx", read("./App.tsx"), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const relayEditor = ts.createSourceFile("RelayEditor.tsx", read("./RelayEditor.tsx"), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const translate = { t: (value: string) => value, tf: (value: string, args: string[]) => value.replace("{0}", args[0]) };

function nodes(tree: any): any[] {
  if (!tree || typeof tree !== "object") return [];
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  return [tree, ...nodes(tree.props.children)];
}

function screenHooks() {
  const state: any[] = [];
  let cursor = 0;
  return {
    useState(initial: unknown) {
      const index = cursor++;
      if (!(index in state)) state[index] = initial;
      return [state[index], (value: unknown) => { state[index] = value; }];
    },
    useEffect(effect: () => void) {
      const index = cursor++;
      if (!state[index]) { state[index] = true; effect(); }
    },
    render(component: () => unknown) {
      cursor = 0;
      return nodes(component());
    },
  };
}

test("Pi 配置弹窗读取原文，保存失败保留草稿，成功才关闭", async (t) => {
  const hooks = screenHooks();
  const close = t.mock.fn();
  let shellDirty: boolean | undefined;
  const draft = '{"theme":"dark"}';
  const saveArgs = { agent: "pi", original: { source: "pi", contents: "{}" }, contents: draft };
  let conflict = true;
  const invoke = async (command: string, args: unknown) => {
    if (command === "read_agent_config") return { source: "pi", contents: "{}", inputFormat: "JSON", editable: true };
    assert.equal(command, "save_agent_config");
    assert.deepEqual(structuredClone(args), saveArgs);
    if (conflict) throw new Error("配置已变更");
  };
  const modules: Record<string, unknown> = {
    "react/jsx-runtime": jsxRuntime,
    "react-dom": { createPortal: (child: unknown) => child },
    react: {
      ...hooks,
      createContext: () => null, useId: t.mock.fn(() => "config-dialog"),
    },
    "./i18n": translate,
    "./cx-layout": { CxLoading: "loading" },
    "./use-modal-shell": {
      useModalShell: ({ onClose, labelledBy, dirty }: { onClose: () => void; labelledBy: string; dirty?: boolean }) => {
        shellDirty = dirty;
        return {
          backdropProps: { onMouseDown: onClose }, dialogProps: { role: "dialog", "aria-labelledby": labelledBy },
          requestClose: onClose,
        };
      },
    },
    "@/components/ui/button": { Button: "button" },
    "@tauri-apps/api/core": { invoke },
  };
  const context = vm.createContext({ exports: {}, document: { body: {} }, require: (name: string) => modules[name] });
  vm.runInContext(dialogCode, context);
  const render = () => hooks.render(() => context.exports.AgentConfigDialog({ agent: "pi", name: "Pi", onClose: close }));
  const textarea = () => render().find((item) => item.type === "textarea").props;
  const save = () => render().find((item) => item.props.children === "保存").props;
  const opening = render();
  assert.ok(opening.some(node => node.type === "loading"));
  const dialog = opening.find(node => node.props.role === "dialog");
  assert.ok(opening.some(node => node.type === "strong" && node.props.id === dialog.props["aria-labelledby"]));
  await settle();
  assert.ok(!render().some(node => node.type === "loading"));
  assert.equal(textarea().value, "{}");
  assert.equal(save().disabled, true);
  assert.equal(shellDirty, false);
  textarea().onChange({ currentTarget: { value: draft } });
  // 有草稿时把 dirty 交给弹窗外壳：Esc / 点遮罩 / 关闭前先确认。
  render();
  assert.equal(shellDirty, true);
  save().onClick();
  render().find(node => node.props.onMouseDown).props.onMouseDown();
  await settle();
  assert.match(render().find((node) => node.props.role === "alert").props.children, /配置已变更/);
  assert.equal(close.mock.callCount(), 0);
  assert.equal(textarea().value, draft);
  conflict = false;
  save().onClick();
  await settle();
  assert.equal(close.mock.callCount(), 1);
});

const appBody = app.statements.filter(ts.isFunctionDeclaration).find((node) => node.name?.text === "App")!.body!;
function appInitializer(name: string) {
  return appBody.statements.filter(ts.isVariableStatement)
    .flatMap(statement => [...statement.declarationList.declarations])
    .find(decl => decl.name.getText(app) === name)!.initializer!.getText(app);
}

test("从 Agent 接入打开 Codex 时定位到通用配置，其他 Agent 仍开自己的弹窗", () => {
  function provider(node: ts.Node): ts.JsxOpeningElement | undefined {
    if (ts.isJsxOpeningElement(node) && node.tagName.getText(app) === "OpenAgentConfig.Provider") return node;
    return ts.forEachChild(node, provider);
  }
  const attribute = provider(appBody)!.attributes.properties[0] as ts.JsxAttribute;
  const handler = (attribute.initializer as ts.JsxExpression).expression!.getText(app);
  const calls: unknown[] = [];
  const open = vm.runInNewContext(compile(`(${handler})`), {
    setRelayCommonOpen: (opened: boolean) => calls.push(opened), setConfigTarget: (agent: unknown) => calls.push(agent),
  });
  open({ agent: "codex", name: "Codex" });
  open({ agent: "pi", name: "Pi" });
  assert.deepEqual(calls, [true, { agent: "pi", name: "Pi" }]);
});

test("Codex 模板保存冲突时保留草稿，不重试，成功后才提示生效条件", async () => {
  const opened = "tool_output_token_limit = 2000\n";
  const draft = "[permissions.work]\ndescription = 'work'\n";
  const settings = {
    revision: "r1", activeRelayId: "external-provider", relayCommonConfigContents: opened, theme: "saved",
    relayProfiles: [],
  };
  let form = { ...settings, theme: "unsaved" };
  let persisted = settings;
  const writes: { settings: typeof settings; expectedRevision: string }[] = [];
  const notices: string[] = [];
  const hooks = screenHooks();
  const context = vm.createContext({
    ...translate,
    useState: hooks.useState, useEffect: hooks.useEffect, exports: {}, require: () => jsxRuntime,
    Button: "button", CxLoading: "loading", Save: "svg", SyncedTextarea: "textarea", errorText: String,
    settingsRef: { current: { status: "ok", settings } }, isSuccessStatus: (status: string) => status === "ok",
    run: async (task: () => unknown) => {
      try { return await task(); } catch (error) { notices.push(String(error)); return null; }
    },
    normalizeSettings: (value: unknown) => value, mergeRotatedAuthContents, rebaseSettingsEdits,
    setSettings() {},
    setSettingsForm: (update: (value: typeof settings) => typeof settings) => { form = update(form); },
    showNotice: (_title: string, message: string, status: string) => notices.push(`${status}: ${message}`),
    invoke: async (command: string, args: { draft?: string; settings: typeof settings; expectedRevision: string }) => {
      if (command === "read_relay_common_config") {
        if (args?.draft === "[bad") throw new Error("TOML 格式无效");
        return structuredClone(persisted);
      }
      assert.equal(command, "save_settings");
      writes.push(structuredClone(args));
      if (writes.length === 1) {
        persisted = { ...settings, revision: "r2", activeRelayId: "another-provider" };
        return { status: "failed", message: "settings_revision_conflict" };
      }
      persisted = { ...args.settings, revision: "r3" };
      return { status: "ok", settings: persisted };
    },
  });
  const page = relayEditor.statements.filter(ts.isFunctionDeclaration).find(node => node.name?.text === "RelayCommonConfigScreen")!;
  const expression = ts.factory.createFunctionExpression(undefined, undefined, page.name, page.typeParameters, page.parameters, page.type, page.body!);
  const screen = vm.runInContext(compile(`(${ts.createPrinter().printNode(ts.EmitHint.Expression, expression, relayEditor)})`), context);
  const save = vm.runInContext(compile(`(${appInitializer("saveCommonTemplate")})`), context);
  const render = () => hooks.render(() => screen({ onSave: save }));
  const textarea = () => render().find(node => node.type === "textarea").props;
  const button = () => render().find(node => node.type === "button" && Array.isArray(node.props.children)).props;
  const clickSave = async () => { button().onClick(); await settle(); };

  assert.ok(render().some(node => node.type === "loading"));
  await settle();
  assert.ok(!render().some(node => node.type === "loading"));
  assert.equal(textarea().value, opened);
  textarea().onValueChange("[bad");
  await clickSave();
  assert.equal(writes.length, 0);
  assert.match(notices.at(-1)!, /TOML/);
  textarea().onValueChange(draft);
  await clickSave();
  assert.equal(writes.length, 1);
  assert.equal(textarea().value, draft);
  assert.match(notices.at(-1)!, /^failed: settings_revision_conflict$/);

  await clickSave();
  assert.deepEqual(writes.map(write => write.expectedRevision), ["r1", "r2"]);
  assert.equal(writes[1].settings.activeRelayId, "another-provider");
  assert.equal(writes[1].settings.relayCommonConfigContents, draft);
  assert.equal(form.theme, "unsaved");
  assert.match(notices.at(-1)!, /^ok: .*请重新应用使用通用配置的供应商/);
  assert.equal(button().disabled, true);
});

test("通用配置页有未保存的修改时，返回列表和侧栏切页都先确认，取消就留在原页", async () => {
  const confirms: string[] = [];
  let answer = false;
  const appConfirm = async (message: string) => { confirms.push(message); return answer; };

  // 页面自己的「返回列表」。
  const hooks = screenHooks();
  const backs: number[] = [];
  const dirtyReports: boolean[] = [];
  const context = vm.createContext({
    ...translate, appConfirm,
    useState: hooks.useState, useEffect: hooks.useEffect, exports: {}, require: () => jsxRuntime,
    Button: "button", CxLoading: "loading", Save: "svg", SyncedTextarea: "textarea", errorText: String,
    invoke: async () => ({ relayCommonConfigContents: "a = 1\n" }),
  });
  const page = relayEditor.statements.filter(ts.isFunctionDeclaration).find(node => node.name?.text === "RelayCommonConfigScreen")!;
  const expression = ts.factory.createFunctionExpression(undefined, undefined, page.name, page.typeParameters, page.parameters, page.type, page.body!);
  const screen = vm.runInContext(compile(`(${ts.createPrinter().printNode(ts.EmitHint.Expression, expression, relayEditor)})`), context);
  const render = () => hooks.render(() => screen({
    onSave: async () => null, onBack: () => backs.push(1), onDirtyChange: (dirty: boolean) => dirtyReports.push(dirty),
  }));
  const back = () => render().find(node => node.type === "button" && node.props.children === "返回列表").props;
  render();
  await settle();
  assert.deepEqual(dirtyReports, [false]);
  back().onClick();
  await settle();
  assert.deepEqual(confirms, []);
  assert.equal(backs.length, 1);
  render().find(node => node.type === "textarea").props.onValueChange("a = 2\n");
  back().onClick();
  await settle();
  assert.equal(confirms.length, 1);
  assert.equal(backs.length, 1);
  answer = true;
  back().onClick();
  await settle();
  assert.equal(backs.length, 2);

  // App 的 setRoute：通用配置页有草稿时侧栏切页先确认。
  confirms.length = 0;
  answer = false;
  const routes: string[] = [];
  const closed: boolean[] = [];
  const relayCommonDirty = { current: true };
  const setRoute = vm.runInNewContext(compile(`(${appInitializer("setRoute")})`), {
    ...translate, appConfirm, relayCommonDirty,
    useCallback: (fn: unknown) => fn,
    setRelayCommonOpen: (open: boolean) => closed.push(open), setRouteState: (route: string) => routes.push(route),
  });
  setRoute("home");
  await settle();
  assert.equal(confirms.length, 1);
  assert.deepEqual(routes, []);
  answer = true;
  setRoute("home");
  await settle();
  assert.deepEqual(routes, ["home"]);
  assert.deepEqual(closed, [false]);
  assert.equal(relayCommonDirty.current, false);
  setRoute("settings");
  assert.equal(confirms.length, 2);
  assert.deepEqual(routes, ["home", "settings"]);
});
