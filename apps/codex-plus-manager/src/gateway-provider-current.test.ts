import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import vm from "node:vm";
import { isFavoriteModel, modelDisplay, ownPickerSection, pickerSections, type GatewayPayload, type OwnModels, type PickerSection } from "./gateway-model.ts";

const source = readFileSync(new URL("./GatewayAgents.tsx", import.meta.url), "utf8").replaceAll("\r\n", "\n");
const ts = createRequire(import.meta.url)("typescript");
const file = ts.createSourceFile("GatewayAgents.tsx", source, ts.ScriptTarget.ES2022, true, ts.ScriptKind.TSX);
const component = file.statements.find((node: any) => ts.isFunctionDeclaration(node) && node.name?.text === "ModelList");
const statement = component?.body?.statements.find((node: any) =>
  ts.isVariableStatement(node) && node.declarationList.declarations.some((entry: any) => entry.name.getText(file) === "sections"));
const declaration = statement?.declarationList.declarations.find((entry: any) => entry.name.getText(file) === "sections");
const callback = declaration?.initializer?.arguments?.[0];
assert.ok(callback && ts.isArrowFunction(callback) && ts.isBlock(callback.body));
const select = new Function(
  "payload", "query", "exclude", "skipGroup", "ownList", "sectionOnly", "favoritesOnly", "favorites",
  "withOwnSection", "pickerSections", "ownPickerSection", "avatarOf", "isFavoriteModel", "selectedModel", "pinProvider", "modelDisplay",
  callback.body.getText(file).slice(1, -1),
);
const ownStart = source.indexOf("function withOwnSection(");
const bodyStart = source.indexOf("{\n", ownStart);
const bodyEnd = source.indexOf("\n}", bodyStart);
assert.ok(ownStart >= 0 && bodyStart > ownStart && bodyEnd > bodyStart);
const withOwnSection = new Function("sections", "own", source.slice(bodyStart + 2, bodyEnd));

const payload = {
  gateway: { version: 1, bindings: {}, keyPolicy: { mode: "sticky" }, restoreOnExit: false },
  port: 57321,
  helperRunning: true,
  pool: ["a", "b"].map((id) => ({
    id, name: id, baseUrl: "", protocol: "responses", official: false,
    models: ["example-model"], keyCount: 1, enabled: true, source: "",
  })),
  groups: [], agents: [], health: { keys: [], providers: [] },
  codexProviders: [], claudeProviders: [], grokProfiles: [],
} as GatewayPayload;
const own: OwnModels = { vendor: "Direct", current: "example-model", models: [{ id: "example-model", label: "Example" }] };

test("自带同名模型不遮住按供应商池路由的当前选择", () => {
  const sections: PickerSection[] = select(
    payload, "", [], undefined, own, null, false, [], withOwnSection, pickerSections,
    ownPickerSection, (name: string) => name, isFavoriteModel, "example-model", true, modelDisplay,
  );
  assert.ok(sections.some((section) => section.key === "own"));
  assert.equal(sections.find((section) => section.key === "current")?.items[0].model, "example-model");
});

test("选择默认模型保存失败时保持已保存的显示", async () => {
  const agentRow = file.statements.find((node: any) => ts.isFunctionDeclaration(node) && node.name?.text === "AgentRow");
  assert.ok(agentRow);
  const compiled = ts.transpileModule(`export ${agentRow.getText(file)}`, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX },
  }).outputText;
  const hooks: any[] = [];
  let cursor = 0;
  let finish!: (saved: boolean) => void;
  const saved = new Promise<boolean>((resolve) => { finish = resolve; });
  const calls: any[] = [];
  const exports: Record<string, any> = {};
  const jsx = (type: unknown, props: Record<string, any>) => ({ type, props });
  vm.runInNewContext(compiled, {
    exports,
    require: () => ({ jsx, jsxs: jsx, Fragment: "Fragment" }),
    useState: (initial: any) => {
      const index = cursor++;
      if (!(index in hooks)) hooks[index] = typeof initial === "function" ? initial() : initial;
      return [hooks[index], (value: any) => { hooks[index] = value; }];
    },
    useEffect: () => {},
    draftFromBinding: (binding: any) => ({ model: binding.model, poolExclude: [] }),
    bindingFromDraft: (draft: unknown) => draft,
    validateDraft: () => null,
    agentDiagnosticPresentation: () => ({ tone: "pending", badge: "", reconnect: false }),
    agentStatusText: () => "",
    agentModels: () => [],
    canReconnectAgent: () => true,
    t: (value: string) => value,
    tf: (value: string) => value,
    CxStatusIcon: "CxStatusIcon", AgentIcon: "AgentIcon", CxBadge: "CxBadge",
    ModelPicker: "ModelPicker", Switch: "Switch", ChevronRight: "ChevronRight",
    AgentDetail: "AgentDetail", AgentDisconnectDialog: "AgentDisconnectDialog",
  });
  const render = () => {
    cursor = 0;
    return exports.AgentRow({
      agent: { agent: "codex", name: "Codex", enabled: true },
      payload: { gateway: { bindings: { codex: { model: "provider/saved" } } } },
      busy: null, expanded: false, onToggleExpand: () => {}, onManage: () => {},
      run: (...args: any[]) => { calls.push(args); return saved; },
      notify: async () => {},
    });
  };
  const elements = (element: any): any[] => {
    if (!element || typeof element !== "object") return [];
    if (Array.isArray(element)) return element.flatMap(elements);
    return [element, ...elements(element.props?.children)];
  };
  const picker = () => elements(render()).find((element) => element.type === "ModelPicker");
  picker().props.onPick({ model: "next", providerModel: "provider/next" });
  assert.equal(calls[0][2].binding.model, "provider/next");
  assert.equal(picker().props.draft.model, "provider/saved");
  finish(false);
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.equal(picker().props.draft.model, "provider/saved");
});
