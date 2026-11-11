import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import vm from "node:vm";
import { agentModels, blankBinding, disabledGroup, draftFromBinding, isFavoriteModel, modelDisplay, ownPickerSection, pickerSections, qualifiedProvider, servingProviders, type GatewayPayload, type OwnModels } from "./gateway-model.ts";
import { modelSlugFromRowName } from "./model-metadata.ts";

const source = readFileSync(new URL("./GatewayAgents.tsx", import.meta.url), "utf8").replaceAll("\r\n", "\n");
const ts = createRequire(import.meta.url)("typescript");
const file = ts.createSourceFile("GatewayAgents.tsx", source, ts.ScriptTarget.ES2022, true, ts.ScriptKind.TSX);
const component = file.statements.find((node: any) => ts.isFunctionDeclaration(node) && node.name?.text === "ModelList");
assert.ok(component);
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

function elements(element: any): any[] {
  if (!element || typeof element !== "object") return [];
  if (Array.isArray(element)) return element.flatMap(elements);
  return [element, ...elements(element.props?.children)];
}

test("自带同名模型不遮住按供应商池路由的当前选择", () => {
  const compiled = ts.transpileModule(component.getText(file), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX },
  }).outputText;
  const exports: Record<string, any> = {};
  const jsx = (type: unknown, props: Record<string, any>) => ({ type, props });
  vm.runInNewContext(compiled, {
    exports,
    require: () => ({ jsx, jsxs: jsx }),
    useState: (initial: any) => [typeof initial === "function" ? initial() : initial, () => {}],
    useRef: (initial: unknown) => ({ current: initial }),
    useMemo: (fn: () => unknown) => fn(),
    useEffect: () => {},
    useContext: () => ({}), GatewayNavContext: {},
    pickerSections, ownPickerSection, withOwnSection, isFavoriteModel, modelDisplay, qualifiedProvider, servingProviders, modelSlugFromRowName,
    avatarOf: (name: string) => name, loadFavorites: () => [], persistFavorites: () => {},
    t: (value: string) => value, tf: (value: string) => value,
    Avatar: "Avatar", Search: "Search", Star: "Star", Waypoints: "Waypoints",
    Shuffle: "Shuffle", ChevronRight: "ChevronRight", Check: "Check", GROUP_MODEL_PREFIX: "group/",
  });
  const tree = exports.ModelList({
    payload, exclude: [], selectedModel: "example-model", own, onPick: () => {}, onPickOwn: () => {}, pinProvider: true,
  });
  assert.ok(elements(tree).find((element) => element.props?.["data-section"] === "own"));
  const current = elements(tree).find((element) => element.props?.["data-section"] === "current");
  assert.ok(current);
  const selected = elements(current).find((element) => element.props?.role === "option");
  assert.equal(selected?.props["aria-selected"], true);
  assert.equal(elements(selected).find((element) => element.type === "strong")?.props.title, "example-model");
});

test("默认模型带窗口后缀时仍勾选对应供应商，而非同名供应商", () => {
  const compiled = ts.transpileModule(component.getText(file), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX },
  }).outputText;
  const exports: Record<string, any> = {};
  const jsx = (type: unknown, props: Record<string, any>) => ({ type, props });
  const hooks: any[] = [];
  let cursor = 0;
  vm.runInNewContext(compiled, {
    exports,
    require: () => ({ jsx, jsxs: jsx }),
    useState: (initial: any) => {
      const index = cursor++;
      if (!(index in hooks)) hooks[index] = typeof initial === "function" ? initial() : initial;
      return [hooks[index], (value: any) => { hooks[index] = typeof value === "function" ? value(hooks[index]) : value; }];
    },
    useRef: (initial: unknown) => {
      const index = cursor++;
      if (!(index in hooks)) hooks[index] = { current: initial };
      return hooks[index];
    },
    useMemo: (fn: () => unknown) => fn(),
    useEffect: () => {},
    useContext: () => ({}), GatewayNavContext: {},
    pickerSections, ownPickerSection, withOwnSection, isFavoriteModel, modelDisplay, qualifiedProvider, servingProviders, modelSlugFromRowName,
    avatarOf: (name: string) => name, loadFavorites: () => [], persistFavorites: () => {},
    t: (value: string) => value, tf: (value: string) => value,
    Avatar: "Avatar", Search: "Search", Star: "Star", Waypoints: "Waypoints",
    Shuffle: "Shuffle", ChevronRight: "ChevronRight", Check: "Check", GROUP_MODEL_PREFIX: "group/",
  });
  const render = (model: string, upstreamModel?: string, providerRemoved = false, data = payload, ownModels?: OwnModels) => {
    cursor = 0;
    return exports.ModelList({
      payload: data, exclude: [], selectedModel: model,
      selectedModelSelection: upstreamModel ? { providerId: "a", upstreamModel, providerSource: "", providerRemoved } : null,
      own: ownModels, onPickOwn: ownModels ? () => {} : undefined, onPick: () => {}, pinProvider: true,
    });
  };
  for (const key of ["provider:a", "provider:b"]) {
    const section = elements(render("a/example-model", "example-model")).find((element) => element.props?.["data-section"] === key);
    const heading = elements(section).find((element) => element.props?.className === "gw-mp-heading-toggle");
    assert.ok(heading, key);
    heading.props.onClick();
  }
  const selected = (model: string, upstreamModel?: string, providerRemoved = false, data = payload, ownModels?: OwnModels) =>
    elements(render(model, upstreamModel, providerRemoved, data, ownModels))
      .filter((element) => element.type === "section")
      .flatMap((section) => elements(section).filter((element) => element.props?.role === "option" && element.props["aria-selected"])
        .map(() => section.props["data-section"]));

  assert.deepEqual(selected("a/example-model[1M]", "example-model"), ["provider:a"]);
  assert.deepEqual(selected("a/example-model", "example-model[1M]"), ["provider:a"]);
  assert.deepEqual(selected("a/example-model[1M]", "example-model[1M]", true), []);
  assert.deepEqual(selected("a/example-model[1M]"), ["provider:a"]);
  const windowed = { ...payload, pool: payload.pool.map((entry) => ({ ...entry, models: ["example-model[1M]"] })) };
  assert.deepEqual(selected("a/example-model", undefined, false, windowed), ["provider:a"]);
  const direct = { ...own, current: "example-model[1M]" };
  assert.deepEqual(selected("a/example-model", "example-model", false, payload, direct), ["provider:a"]);
});

test("同名停用组不遮住所选供应商模型的冲突提示", () => {
  const selection = { providerId: "GROUP", upstreamModel: "daily", providerSource: "" };
  const binding = { ...blankBinding(), enabled: true, model: "GROUP/daily", defaultModelSelection: selection };
  const data: GatewayPayload = {
    ...payload,
    gateway: { ...payload.gateway, bindings: { codex: binding } },
    pool: [{ ...payload.pool[0], id: "GROUP", name: "Group provider" }],
    groups: [{ id: "daily", name: "Daily", mode: "inOrder", members: [], disabled: true }],
  };
  const agent = { agent: "codex", name: "Codex", enabled: true, defaultModel: "example-model" };
  for (const [path, name] of [["GatewayAgents.tsx", "ModelPicker"], ["TrayPanel.tsx", "PanelAgentRow"]]) {
    const content = readFileSync(new URL(`./${path}`, import.meta.url), "utf8");
    const parsed = ts.createSourceFile(path, content, ts.ScriptTarget.ES2022, true, ts.ScriptKind.TSX);
    const target = parsed.statements.find((node: any) => ts.isFunctionDeclaration(node) && node.name?.text === name);
    assert.ok(target);
    const compiled = ts.transpileModule(`export ${target.getText(parsed)}`, {
      compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX },
    }).outputText;
    const exports: Record<string, any> = {};
    const jsx = (type: unknown, props: Record<string, any>) => ({ type, props });
    vm.runInNewContext(compiled, {
      exports,
      require: () => ({ jsx, jsxs: jsx, Fragment: "Fragment" }),
      useState: (initial: unknown) => [initial, () => {}], useRef: (initial: unknown) => ({ current: initial }),
      disabledGroup, modelDisplay, agentModels, draftFromBinding,
      ownCurrent: () => null, visibleOwnModels: () => null,
      t: (value: string) => value, tf: (value: string) => value,
      AgentIcon: "AgentIcon", ModelViewGlyph: "ModelViewGlyph", AutoTag: "AutoTag", ChevronDown: "ChevronDown",
      Check: "Check", AlertTriangle: "AlertTriangle", Floating: "Floating", ModelList: "ModelList",
      AgentDisconnectDialog: "AgentDisconnectDialog",
    });
    const tree = name === "ModelPicker"
      ? exports.ModelPicker({ agent, payload: data, draft: draftFromBinding(binding), connected: true, open: false })
      : exports.PanelAgentRow({ agent, payload: data, onRun: () => {} });
    const picker = elements(tree).find((element) => element.props?.className?.includes("gw-picker-button"));
    assert.equal(picker?.props.title, undefined, `${name} 不应声称回退到其他模型`);
    assert.equal(elements(picker).find((element) => element.type === "ModelViewGlyph")?.props.view.issue, "conflict", name);
    assert.equal(elements(tree).find((element) => element.type === "ModelList")?.props.selectedModel, "GROUP/daily", name);
  }
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
  const picker = () => elements(render()).find((element) => element.type === "ModelPicker");
  picker().props.onPick({ model: "next", providerModel: "provider/next" });
  assert.equal(calls[0][2].binding.model, "provider/next");
  assert.equal(picker().props.draft.model, "provider/saved");
  finish(false);
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.equal(picker().props.draft.model, "provider/saved");
});
