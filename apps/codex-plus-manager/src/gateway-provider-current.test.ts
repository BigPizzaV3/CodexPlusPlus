import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import vm from "node:vm";
import { agentModels, blankBinding, disabledGroup, draftFromBinding, isFavoriteModel, modelDisplay, ownPickerSection, pickerSections, providerEnabled, qualifiedProvider, servingProviders, type GatewayPayload, type OwnModels } from "./gateway-model.ts";
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
  const agent = { agent: "codex", name: "Codex", enabled: true, defaultModel: "GROUP/daily" };
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
      disabledGroup, modelDisplay, agentModels, draftFromBinding, providerEnabled,
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

test("停用的默认供应商显示实际回退模型，恢复后显示原选择", () => {
  const selection = { providerId: "a", upstreamModel: "example-model", providerSource: "" };
  const binding = { ...blankBinding(), enabled: true, model: "a/example-model", defaultModelSelection: selection };
  const data: GatewayPayload = {
    ...payload,
    gateway: { ...payload.gateway, bindings: { codex: binding } },
    pool: [
      { ...payload.pool[0], enabled: false },
      { ...payload.pool[1], models: ["fallback-model"] },
    ],
  };
  const agent = { agent: "codex", name: "Codex", enabled: true, defaultModel: "fallback-model" };
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
      disabledGroup, modelDisplay, agentModels, draftFromBinding, providerEnabled,
      ownCurrent: () => null, visibleOwnModels: () => null,
      t: (value: string) => value, tf: (value: string, values: string[]) => value.replace("{0}", values[0]).replace("{1}", values[1]),
      AgentIcon: "AgentIcon", ModelViewGlyph: "ModelViewGlyph", AutoTag: "AutoTag", ChevronDown: "ChevronDown",
      Check: "Check", AlertTriangle: "AlertTriangle", Floating: "Floating", ModelList: "ModelList",
      AgentDisconnectDialog: "AgentDisconnectDialog",
    });
    const render = (current: GatewayPayload, defaultModel: string, defaultSelectionStatus?: string, defaultModelDiffers = false) => name === "ModelPicker"
      ? exports.ModelPicker({ agent: { ...agent, defaultModel, defaultSelectionStatus, defaultModelDiffers }, payload: current, draft: draftFromBinding(binding), connected: true, open: false })
      : exports.PanelAgentRow({ agent: { ...agent, defaultModel, defaultSelectionStatus, defaultModelDiffers }, payload: current, onRun: () => {} });
    const fallback = render(data, "fallback-model");
    const picker = elements(fallback).find((element) => element.props?.className?.includes("gw-picker-button"));
    assert.match(picker?.props.title, /原默认模型「Example-Model」不可用，网关默认回退为 Fallback-Model/);
    assert.equal(elements(fallback).find((element) => element.type === "ModelList")?.props.selectedModel, "fallback-model");
    assert.equal(elements(fallback).find((element) => element.type === "ModelList")?.props.selectedModelSelection, null);
    const noFallback = render(data, "");
    const emptyPicker = elements(noFallback).find((element) => element.props?.className?.includes("gw-picker-button"));
    assert.match(emptyPicker?.props.title, /原默认模型「Example-Model」不可用，网关默认回退为 未设置/);
    assert.equal(elements(emptyPicker).find((element) => element.type === "ModelViewGlyph")?.props.view.title, "");
    assert.equal(elements(noFallback).find((element) => element.type === "ModelList")?.props.selectedModel, "");
    const invalid = render(data, "a/example-model", "invalid");
    const invalidPicker = elements(invalid).find((element) => element.props?.className?.includes("gw-picker-button"));
    assert.equal(invalidPicker?.props.title, undefined, `${name} 不应把实际来源冲突显示为回退`);
    assert.equal(elements(invalidPicker).find((element) => element.type === "ModelViewGlyph")?.props.view.issue, "invalid");
    assert.equal(elements(invalid).find((element) => element.type === "ModelList")?.props.selectedModel, "a/example-model");
    const available = render(data, "a/example-model", "available");
    const availablePicker = elements(available).find((element) => element.props?.className?.includes("gw-picker-button"));
    assert.equal(availablePicker?.props.title, undefined);
    assert.equal(elements(availablePicker).find((element) => element.type === "ModelViewGlyph")?.props.view.issue, undefined);
    const extensionFallback = render(data, "extension/model", "fallback", true);
    const extensionPicker = elements(extensionFallback).find((element) => element.props?.className?.includes("gw-picker-button"));
    const extensionView = elements(extensionPicker).find((element) => element.type === "ModelViewGlyph")?.props.view;
    assert.equal(extensionView.title, "extension/model");
    assert.equal(extensionView.sub, "", "不能根据本地池猜测拓展模型的供应商");
    assert.equal(elements(extensionFallback).find((element) => element.type === "ModelList")?.props.selectedModel, "");
    const enabledCollision = {
      ...data,
      pool: data.pool.map((entry) => entry.id === "a" ? { ...entry, enabled: true } : { ...entry, models: ["a/example-model"] }),
    };
    const collision = render(enabledCollision, "example-model", "fallback");
    const collisionPicker = elements(collision).find((element) => element.props?.className?.includes("gw-picker-button"));
    assert.match(collisionPicker?.props.title, /原默认模型「Example-Model」不可用/);
    assert.equal(elements(collision).find((element) => element.type === "ModelList")?.props.selectedModelSelection, null);
    const overridden = render(enabledCollision, "example-model", "fallback", true);
    assert.equal(elements(overridden).find((element) => element.type === "ModelList")?.props.selectedModel, "");
    const restored = { ...data, pool: data.pool.map((entry) => entry.id === "a" ? { ...entry, enabled: true } : entry) };
    const active = render(restored, "a/example-model");
    assert.equal(elements(active).find((element) => element.props?.className?.includes("gw-picker-button"))?.props.title, undefined);
    assert.equal(elements(active).find((element) => element.type === "ModelList")?.props.selectedModel, "a/example-model");
  }
});

test("接入后的空绑定和停用组使用网关默认模型，拓展成员不误标为本地供应商", () => {
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
      disabledGroup, modelDisplay, agentModels, draftFromBinding, providerEnabled,
      ownCurrent: () => "Direct model", visibleOwnModels: () => null,
      t: (value: string) => value, tf: (value: string, values: string[]) => value.replace("{0}", values[0]).replace("{1}", values[1]),
      AgentIcon: "AgentIcon", ModelViewGlyph: "ModelViewGlyph", AutoTag: "AutoTag", ChevronDown: "ChevronDown",
      Check: "Check", AlertTriangle: "AlertTriangle", Floating: "Floating", ModelList: "ModelList",
      AgentDisconnectDialog: "AgentDisconnectDialog",
    });
    const render = (model: string, defaultModel: string, differs: boolean, enabled = true) => {
      const binding = { ...blankBinding(), enabled, model };
      const data: GatewayPayload = {
        ...payload,
        gateway: { ...payload.gateway, bindings: { codex: binding } },
        pool: [{ ...payload.pool[0], models: ["m1"] }],
        groups: [{ id: "disabled", name: "Disabled", mode: "inOrder", members: [], disabled: true }],
      };
      const agent = { agent: "codex", name: "Codex", enabled, defaultModel, defaultModelDiffers: differs };
      return name === "ModelPicker"
        ? exports.ModelPicker({ agent, payload: data, draft: draftFromBinding(binding), connected: enabled, open: false })
        : exports.PanelAgentRow({ agent, payload: data, onRun: () => {} });
    };
    const inspect = (tree: any) => {
      const picker = elements(tree).find((element) => element.props?.className?.includes("gw-picker-button"));
      return {
        picker,
        view: elements(picker).find((element) => element.type === "ModelViewGlyph")?.props.view,
        selected: elements(tree).find((element) => element.type === "ModelList")?.props.selectedModel,
      };
    };
    const overridden = inspect(render("", "m2", true));
    assert.equal(overridden.view.title, "m2", name);
    assert.equal(overridden.view.sub, "", name);
    assert.equal(overridden.selected, "", name);
    const extensionOnly = inspect(render("", "extension/model", true));
    assert.equal(extensionOnly.view.title, "extension/model", name);
    assert.equal(extensionOnly.selected, "", name);
    const empty = inspect(render("", "", false));
    assert.equal(empty.view.title, "", name);
    assert.equal(empty.selected, "", name);
    const group = inspect(render("group/disabled", "m1", true));
    assert.equal(group.view.title, "m1", name);
    assert.equal(group.view.sub, "", name);
    assert.equal(group.selected, "", name);
    assert.match(group.picker.props.title, /网关默认回退为 m1/, name);
    const direct = inspect(render("", "m2", true, false));
    assert.ok(elements(direct.picker).some((element) => element.props?.children === "Direct model"), name);
  }
});

test("选择模型或修改详情保存失败时保持已保存的显示", async () => {
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
  let validationError: string | null = null;
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
    validateDraft: () => validationError,
    agentDiagnosticPresentation: () => ({ tone: "pending", badge: "", reconnect: false }),
    agentStatusText: () => "",
    agentModels: () => [],
    canReconnectAgent: () => true,
    t: (value: string) => value,
    tf: (value: string) => value,
    CxStatusIcon: "CxStatusIcon", AgentIcon: "AgentIcon", CxBadge: "CxBadge",
    ModelPicker: "ModelPicker", Switch: "Switch", ChevronRight: "ChevronRight",
    ResetModelSelections: "ResetModelSelections",
    AgentDetail: "AgentDetail", AgentDisconnectDialog: "AgentDisconnectDialog",
  });
  const render = () => {
    cursor = 0;
    return exports.AgentRow({
      agent: { agent: "codex", name: "Codex", enabled: true },
      payload: { gateway: { bindings: { codex: { model: "provider/saved" } } } },
      busy: null, expanded: true, onToggleExpand: () => {}, onManage: () => {},
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
  const detail = () => elements(render()).find((element) => element.type === "AgentDetail");
  validationError = "不能保存";
  detail().props.onSave({ model: "provider/saved", poolExclude: ["provider"] });
  assert.deepEqual(picker().props.draft.poolExclude, []);
  assert.equal(calls.length, 1);
  validationError = null;
  detail().props.onSave({ model: "provider/saved", poolExclude: ["provider"] });
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.deepEqual(picker().props.draft.poolExclude, []);
  assert.equal(calls.length, 2);
});

test("已接入和已断开的 Agent 都能确认重置历史模型选择", async () => {
  const names = ["AgentRow", "ResetModelSelections"];
  const components = names.map((name) =>
    file.statements.find((node: any) => ts.isFunctionDeclaration(node) && node.name?.text === name),
  );
  assert.ok(components.every(Boolean));
  const compiled = ts.transpileModule(components.map((component: any) => `export ${component.getText(file)}`).join("\n"), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX },
  }).outputText;
  const exports: Record<string, any> = {};
  const jsx = (type: unknown, props: Record<string, any>) => ({ type, props });
  const calls: any[][] = [];
  const confirmations: Array<{ message: string; options: Record<string, unknown> }> = [];
  let respond!: (accepted: boolean) => void;
  vm.runInNewContext(compiled, {
    exports,
    require: () => ({ jsx, jsxs: jsx, Fragment: "Fragment" }),
    useState: (initial: any) => [typeof initial === "function" ? initial() : initial, () => {}],
    useEffect: () => {},
    draftFromBinding: () => ({ model: "provider/saved", poolExclude: [] }),
    agentDiagnosticPresentation: () => ({ tone: "pending", badge: "", reconnect: false }),
    agentStatusText: () => "",
    agentModels: () => [],
    canReconnectAgent: () => true,
    t: (value: string) => value,
    tf: (value: string, values: string[]) => value.replace("{0}", values[0]),
    appConfirm: (message: string, options: Record<string, unknown>) => {
      confirmations.push({ message, options });
      return new Promise<boolean>((resolve) => { respond = resolve; });
    },
    CxStatusIcon: "CxStatusIcon", AgentIcon: "AgentIcon", CxBadge: "CxBadge",
    ModelPicker: "ModelPicker", Switch: "Switch", ChevronRight: "ChevronRight",
    AgentDetail: "AgentDetail", AgentDisconnectDialog: "AgentDisconnectDialog", RotateCcw: "RotateCcw",
  });
  const render = (enabled: boolean, hasBinding = true, busy: string | null = null) => exports.AgentRow({
    agent: { agent: "codex", name: "Codex", enabled },
    payload: { gateway: { bindings: hasBinding ? { codex: { model: "provider/saved" } } : {} } },
    busy, expanded: false, onToggleExpand: () => {}, onManage: () => {},
    run: async (...args: any[]) => { calls.push(args); return true; },
    notify: async () => {},
  });
  for (const enabled of [true, false]) {
    const entry = elements(render(enabled)).find((element) => element.type === exports.ResetModelSelections);
    assert.ok(entry, `enabled=${enabled}`);
  }
  assert.equal(elements(render(false, false)).some((element) => element.type === exports.ResetModelSelections), false);
  const busyEntry = elements(render(false, true, "another-action")).find((element) => element.type === exports.ResetModelSelections);
  assert.equal(exports.ResetModelSelections(busyEntry.props).props.disabled, true);

  const entry = elements(render(false)).find((element) => element.type === exports.ResetModelSelections);
  const button = exports.ResetModelSelections(entry.props);
  button.props.onClick();
  assert.match(confirmations[0].message, /历史会话可能按当前路由规则/);
  assert.match(confirmations[0].message, /有效的当前默认模型供应商身份会保留/);
  assert.match(confirmations[0].message, /失效的当前默认选择会清除/);
  assert.equal(confirmations[0].options.title, "重置 Codex 的历史模型选择");
  assert.equal(calls.length, 0);
  respond(false);
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.equal(calls.length, 0);

  button.props.onClick();
  assert.equal(calls.length, 0);
  respond(true);
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.equal(calls.length, 1);
  assert.equal(calls[0][1], "reset_gateway_model_selections");
  assert.equal(calls[0][2].agent, "codex");
  assert.equal(Object.keys(calls[0][2]).length, 1);
});
