import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import vm from "node:vm";

const ts = createRequire(import.meta.url)("typescript");

type Element = { type: unknown; props: Record<string, any> };

function elements(value: unknown): Element[] {
  if (Array.isArray(value)) return value.flatMap(elements);
  if (!value || typeof value !== "object" || !("props" in value)) return [];
  const element = value as Element;
  return [element, ...elements(element.props.children)];
}

function picker(onPick: (item: { model: string }) => void = () => {}) {
  const source = readFileSync(new URL("./GatewayAgents.tsx", import.meta.url), "utf8");
  const file = ts.createSourceFile("GatewayAgents.tsx", source, ts.ScriptTarget.ES2022, true, ts.ScriptKind.TSX);
  const component = file.statements.find((node: any) => ts.isFunctionDeclaration(node) && node.name?.text === "ModelList");
  assert.ok(component);
  const compiled = ts.transpileModule(component.getText(file), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX },
  }).outputText;
  const hooks: any[] = [];
  let cursor = 0;
  const exports: Record<string, any> = {};
  const jsx = (type: unknown, props: Record<string, any>) => ({ type, props });
  const pool = [
    { id: "a", name: "Provider A", baseUrl: "https://a.invalid" },
    { id: "b", name: "Provider B", baseUrl: "https://b.invalid" },
  ];
  const groups = [
    { key: "provider:a", title: "Provider A", items: [{ model: "alpha", label: "Alpha", sub: "Provider A" }] },
    { key: "provider:b", title: "Provider B", items: [{ model: "beta", label: "Beta", sub: "Provider B" }] },
    { key: "own", title: "自带模型", items: [{ model: "own", label: "Own", sub: "自带模型", direct: "example" }] },
    { key: "current", title: "当前选择", items: [{ model: "current", label: "Current", sub: "当前选择" }] },
  ];
  vm.runInNewContext(compiled, {
    exports,
    require: (name: string) => {
      assert.equal(name, "react/jsx-runtime");
      return { jsx, jsxs: jsx };
    },
    useState: (initial: unknown) => {
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
    useContext: () => ({}),
    GatewayNavContext: {},
    pickerSections: (_payload: unknown, query: string) => groups
      .map((group) => ({ ...group, items: group.items.filter((item) => !query || item.label.toLowerCase().includes(query.toLowerCase())) }))
      .filter((group) => group.items.length),
    ownPickerSection: () => null,
    withOwnSection: (sections: unknown) => sections,
    avatarOf: () => "",
    loadFavorites: () => [],
    isFavoriteModel: () => false,
    persistFavorites: () => {},
    t: (value: string) => value,
    tf: (value: string, values: unknown[]) => value.replace(/\{(\d+)\}/g, (_, index) => String(values[Number(index)])),
    Avatar: "Avatar", Search: "Search", Star: "Star", Waypoints: "Waypoints",
    Shuffle: "Shuffle", ChevronRight: "ChevronRight", Check: "Check",
    GROUP_MODEL_PREFIX: "group/",
  });
  return {
    render: () => {
      cursor = 0;
      return exports.ModelList({
        payload: { pool, gateway: {} },
        exclude: [],
        selectedModel: "",
        onPick,
      }) as Element;
    },
  };
}

function section(tree: Element, key: string) {
  const match = elements(tree).find((element) => element.props["data-section"] === key);
  assert.ok(match, key);
  return match;
}

function heading(tree: Element, key: string) {
  const match = elements(section(tree, key)).find((element) => element.props.className === "gw-mp-heading-toggle");
  assert.ok(match, key);
  return match;
}

test("供应商默认折叠，其他模型保持可见；重新打开后恢复默认", () => {
  const view = picker();
  let tree = view.render();
  assert.equal(heading(tree, "provider:a").props["aria-expanded"], false);
  assert.equal(heading(tree, "provider:b").props["aria-expanded"], false);
  assert.equal(heading(tree, "own").props["aria-expanded"], true);
  assert.equal(heading(tree, "current").props["aria-expanded"], true);
  heading(tree, "provider:a").props.onClick();
  tree = view.render();
  assert.equal(heading(tree, "provider:a").props["aria-expanded"], true);
  assert.equal(heading(tree, "provider:b").props["aria-expanded"], false);
  assert.equal(heading(picker().render(), "provider:a").props["aria-expanded"], false);
});

test("供应商筛选展开目标；搜索展开匹配项，清空后恢复手动状态", () => {
  const view = picker();
  let tree = view.render();
  heading(tree, "provider:a").props.onClick();
  tree = view.render();
  const filter = elements(tree).find((element) => element.type === "button" && element.props.title === "Provider B");
  assert.ok(filter);
  filter.props.onClick();
  tree = view.render();
  assert.equal(heading(tree, "provider:b").props["aria-expanded"], true);
  const all = elements(tree).find((element) => element.type === "button" && element.props.children === "全部");
  assert.ok(all);
  all.props.onClick();
  tree = view.render();
  const input = elements(tree).find((element) => element.type === "input");
  assert.ok(input);
  input.props.onChange({ currentTarget: { value: "alpha" } });
  tree = view.render();
  assert.equal(heading(tree, "provider:a").props["aria-expanded"], true);
  heading(tree, "provider:a").props.onClick();
  tree = view.render();
  assert.equal(heading(tree, "provider:a").props["aria-expanded"], false);
  elements(tree).find((element) => element.type === "input")!.props.onChange({ currentTarget: { value: "ALPHA" } });
  tree = view.render();
  assert.equal(heading(tree, "provider:a").props["aria-expanded"], true);
  elements(tree).find((element) => element.type === "input")!.props.onChange({ currentTarget: { value: "" } });
  tree = view.render();
  assert.equal(heading(tree, "provider:a").props["aria-expanded"], true);
  assert.equal(heading(tree, "provider:b").props["aria-expanded"], true);
});

test("筛选按钮向辅助技术暴露当前选择", () => {
  const view = picker();
  const buttons = () => {
    const filters = elements(view.render()).find((element) => element.props.className === "gw-mp-filters");
    assert.ok(filters);
    return elements(filters).filter((element) => element.type === "button");
  };

  let [all, favorites, provider] = buttons();
  assert.equal(all.props["aria-pressed"], true);
  assert.equal(favorites.props["aria-pressed"], false);
  assert.equal(provider.props["aria-pressed"], false);

  favorites.props.onClick();
  [all, favorites, provider] = buttons();
  assert.equal(all.props["aria-pressed"], false);
  assert.equal(favorites.props["aria-pressed"], true);
  assert.equal(provider.props["aria-pressed"], false);

  provider.props.onClick();
  [all, favorites, provider] = buttons();
  assert.equal(all.props["aria-pressed"], false);
  assert.equal(favorites.props["aria-pressed"], false);
  assert.equal(provider.props["aria-pressed"], true);
});

test("收藏按钮的 Enter 不会触发模型选择，模型行的 Enter 仍可选择", () => {
  const picked: string[] = [];
  const view = picker((item) => picked.push(item.model));
  heading(view.render(), "provider:a").props.onClick();
  const modelRow = elements(section(view.render(), "provider:a")).find((element) => element.props.role === "option");
  assert.ok(modelRow);
  const row = {};
  modelRow.props.onKeyDown({ key: "Enter", currentTarget: row, target: {} });
  assert.deepEqual(picked, []);
  modelRow.props.onKeyDown({ key: "Enter", currentTarget: row, target: row });
  assert.deepEqual(picked, ["alpha"]);
});

test("空间不足时模型选择器使用窗口高度，普通浮层保持锚点定位", () => {
  const source = readFileSync(new URL("./GatewayAgents.tsx", import.meta.url), "utf8");
  const file = ts.createSourceFile("GatewayAgents.tsx", source, ts.ScriptTarget.ES2022, true, ts.ScriptKind.TSX);
  const component = file.statements.find((node: any) => ts.isFunctionDeclaration(node) && node.name?.text === "Floating");
  assert.ok(component);
  const compiled = ts.transpileModule(component.getText(file), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX },
  }).outputText;
  const hooks: any[] = [];
  let cursor = 0;
  const exports: Record<string, any> = {};
  const viewport = { innerWidth: 364, innerHeight: 320, addEventListener: () => {}, removeEventListener: () => {} };
  vm.runInNewContext(compiled, {
    exports,
    require: () => ({ jsx: (type: unknown, props: Record<string, any>) => ({ type, props }) }),
    useRef: (initial: unknown) => {
      const index = cursor++;
      if (!(index in hooks)) hooks[index] = { current: initial };
      return hooks[index];
    },
    useState: (initial: unknown) => {
      const index = cursor++;
      if (!(index in hooks)) hooks[index] = initial;
      return [hooks[index], (value: unknown) => { hooks[index] = value; }];
    },
    useLayoutEffect: (effect: () => unknown) => effect(),
    useEffect: () => {},
    useEscapeLayer: () => {},
    createPortal: (element: Element) => element,
    document: { body: {} },
    window: viewport,
  });
  const anchorRef = { current: { getBoundingClientRect: () => ({ top: 150, bottom: 180, right: 300 }) } };
  const render = (minHeight?: number) => {
    const props = { anchorRef, open: true, onClose: () => {}, width: 340, minHeight };
    cursor = 0;
    exports.Floating(props);
    cursor = 0;
    return exports.Floating(props) as Element;
  };
  let panel = render(180);
  assert.equal(panel.props.style.top, 12);
  assert.equal(panel.props.style.maxHeight, 296);
  viewport.innerHeight = 550;
  panel = render(180);
  assert.equal(panel.props.style.top, 186);
  viewport.innerHeight = 320;
  panel = render();
  assert.equal(panel.props.style.bottom, 176);
  assert.equal(panel.props.style.maxHeight, 132);
});
