import assert from "node:assert/strict";
import { it } from "node:test";
import { readFile } from "node:fs/promises";

/**
 * 模型选择器供应商入口（聚合供应商场景）的契约与行为测试。
 *
 * DOM 相关逻辑用最小替身验证，真实浏览器交互另由实测覆盖。这里钉死三件事：
 *   1. 与后端约定的字段名不能漂（`codex_plus_group` / `codex_plus_group_id`）；
 *   2. 影响别人的行为约束：不搬动 React 管理的行、隐藏原生标签要退出无障碍树、
 *      单供应商不接管、聚合目录里"默认成员"那一组要能看懂；
 *   3. 供应商入口与模型列表切换、自检重建和清理不能丢状态或留下隐藏行。
 */

const SHARD_URL = new URL(
  "../../../assets/inject/renderer-inject/71-model-picker-groups.js",
  import.meta.url,
);
const MANIFEST_URL = new URL(
  "../../../assets/inject/renderer-inject/manifest.json",
  import.meta.url,
);

async function readShard() {
  return await readFile(SHARD_URL, "utf8");
}

function functionSource(source: string, name: string) {
  const body = source.match(new RegExp(`^  function ${name}\\([^]*?^  \\}`, "m"))?.[0];
  assert.ok(body, `${name} 必须存在`);
  return body;
}

it("供应商入口分片在 manifest 里排在模型目录之后、并在组装产物里", async () => {
  const manifest = JSON.parse(await readFile(MANIFEST_URL, "utf8")) as {
    fragments: Array<{ name: string }>;
  };
  const names = manifest.fragments.map((fragment) => fragment.name);
  const index = names.indexOf("71-model-picker-groups.js");
  assert.ok(index > 0, "分片必须登记进 manifest.json");
  assert.ok(
    names.indexOf("70-model-catalog.js") < index,
    "要排在 70-model-catalog.js 之后：它依赖 codexModelCatalog 快照",
  );
  assert.ok(
    names.indexOf("99-tail.js") > index,
    "必须排在 IIFE 收尾之前，否则代码不在主闭包里",
  );

  const artifact = await readFile(
    new URL("../../../assets/inject/renderer-inject.js", import.meta.url),
    "utf8",
  );
  assert.match(artifact, /codexPlusModelPickerGroupsRuntime/, "组装产物里要有本分片");
});

it("供应商入口读的字段名与后端发布一致", async () => {
  const source = await readShard();
  // 后端 aggregate_catalog.rs 发布 / model_suffix.rs 放行的两个键。
  assert.match(source, /entry\.codex_plus_group\b/);
  assert.match(source, /entry\.codex_plus_group_id\b/);
});

it("不搬动 React 管理的行，只用类名隐藏", async () => {
  const source = await readShard();
  // 行必须留在原位：只允许插标题/工具条、加类名和属性。
  assert.doesNotMatch(source, /appendChild\(entry\.row\)|appendChild\(row\)/);
  assert.doesNotMatch(source, /\.row\.remove\(\)/);
  assert.match(source, /codex-plus-model-group-collapsed\{display:none!important\}/);
  // 保留已发布的隐藏类名，导航只控制可见性，不重排 DOM。
  assert.match(source, /classList\.toggle\("codex-plus-model-group-collapsed"/);
});

it("隐藏原生标签时同步退出无障碍树，避免读屏念两遍", async () => {
  const source = await readShard();
  assert.match(source, /native\.setAttribute\("aria-hidden", "true"\)/);
  assert.match(source, /native\.removeAttribute\("aria-hidden"\)/);
});

it("只有一个供应商时不接管原生列表", async () => {
  const source = await readShard();
  assert.match(source, /if \(groups\.length < 2\) \{/);
  // 单组时要能退回原始状态。
  assert.match(source, /codexPlusModelPickerClear\(list\)/);
  assert.match(source, /list\.removeAttribute\("data-codex-plus-model-groups"\)/);
});

it("默认成员的裸名模型归到默认供应商分组，不产生空标题", async () => {
  const source = await readShard();
  // 分组名解析：优先 codex_plus_group，其次 description 的 "上游 · 供应商"。
  assert.match(source, /const separator = description\.lastIndexOf\(" · "\)/);
  assert.match(source, /codexPlusModelPickerDefaultProviderName/);
  // 空名兜底成固定 id，避免导航串组。
  assert.match(source, /group\.id = "__ungrouped__"/);
});

it("扫描器自己插的节点都带归属标记，避免自喂扫描循环", async () => {
  const source = await readShard();
  assert.match(
    source,
    /registerCodexPlusExtensionSelector\('\[data-codex-plus-ext="builtin-model-picker-groups"\]'\)/,
    "必须登记扩展选择器，否则扫描调度器会把我们插的标题当成 Codex 原生节点",
  );
  const marks = source.match(/data-codex-plus-ext", codexPlusModelPickerGroupsExtMark/g) ?? [];
  assert.ok(marks.length >= 4, "标题、工具条、行、标签都要打标记");
});

it("菜单关闭或组件重建后能自愈，不留下清不掉的残留", async () => {
  const source = await readShard();
  // React 回收注入节点后入口会消失，指纹没变也必须重建。
  assert.match(source, /codexPlusModelPickerHeadersIntact/);
  assert.match(functionSource(source, "codexPlusModelPickerHeadersIntact"), /codex-plus-model-provider-grid/);
  // 恢复时要摘干净自己加的类名与属性。
  assert.match(source, /row\.removeAttribute\(codexPlusModelPickerGroupAttr\)/);
  assert.match(source, /native\.removeAttribute\("data-codex-plus-ext"\)/);
});

it("重注入时与上一个实例交接，不先拆再建造成闪烁", async () => {
  const source = await readShard();
  assert.match(source, /window\.__codexPlusModelPickerGroupsRuntime\?\.dispose\?\.\(\)/);
  const dispose = source.match(/dispose\(\) \{[\s\S]*?\n    \},/)?.[0];
  assert.ok(dispose, "要有 dispose 实现");
  assert.doesNotMatch(dispose, /codexPlusModelPickerClear/, "dispose 不能顺手拆 DOM：重注入会闪");
  assert.match(source, /releaseDom\(\)/, "彻底还原要有单独出口");
});

it("入口使用独立供应商图标，并通过导航显示模型列表", async () => {
  const source = await readShard();
  assert.match(source, /codex-plus-model-provider-grid/);
  assert.match(source, /codex-plus-model-provider-icon/);
  assert.match(source, /function codexPlusModelPickerNavigate\(list, groupId\)/);
  assert.match(source, /function codexPlusModelPickerSyncNavigation\(list, groups\)/);
  assert.doesNotMatch(source, /全部展开|全部折叠/);
  // 入口的无障碍名称仍要包含供应商、模型数和默认身份。
  assert.match(source, /groupAria/);
});

it("只持久化供应商身份，不把当前列表或旧折叠状态带到下一次打开", async () => {
  const source = await readShard();
  const state = {
    providerNames: new Map([["example-model", { name: "供应商 A", parsed: false }]]),
    defaultGroupId: "provider-a",
    activeGroupId: "provider-b",
    collapsed: new Set(["provider-a"]),
    touched: new Set(["provider-b"]),
  };
  let saved: Record<string, unknown> | undefined;
  const persist = new Function(
    "window", "codexPlusModelPickerGroupsState", "codexPlusModelPickerGroupsStoreKey",
    `${functionSource(source, "codexPlusModelPickerPersist")}\nreturn codexPlusModelPickerPersist;`,
  )(
    { localStorage: { setItem: (_key: string, value: string) => { saved = JSON.parse(value); } } },
    state,
    "test-model-picker",
  );
  persist();
  assert.ok(saved);
  assert.equal(saved.defaultGroupId, "provider-a");
  assert.ok(saved.providers);
  for (const key of ["activeGroupId", "collapsed", "touched"]) {
    assert.equal(Object.hasOwn(saved, key), false, `${key} 不应跨菜单保存`);
  }
});

it("兼容旧身份缓存，但忽略其中的展开状态和导航状态", async () => {
  const source = await readShard();
  const state = {
    stored: false,
    providerNames: new Map<string, { name: string; parsed: boolean }>(),
    defaultGroupId: "",
    activeGroupId: "",
  };
  const cached = {
    defaultGroupId: "provider-a",
    providers: {
      first: "供应商 A",
      second: { name: "供应商 B", parsed: true },
    },
    activeGroupId: "provider-b",
    collapsed: ["provider-a"],
    touched: ["provider-b"],
  };
  const store = new Function(
    "window", "codexPlusModelPickerGroupsState", "codexPlusModelPickerGroupsStoreKey",
    `${functionSource(source, "codexPlusModelPickerStore")}\nreturn codexPlusModelPickerStore;`,
  )(
    { localStorage: { getItem: () => JSON.stringify(cached) } },
    state,
    "test-model-picker",
  );
  store();
  assert.equal(state.activeGroupId, "", "打开菜单仍从供应商入口开始");
  assert.equal(state.defaultGroupId, "provider-a");
  assert.deepEqual(state.providerNames.get("first"), { name: "供应商 A", parsed: false });
  assert.deepEqual(state.providerNames.get("second"), { name: "供应商 B", parsed: true, groupId: "" });
  assert.equal(Object.hasOwn(state, "collapsed"), false);
  assert.equal(Object.hasOwn(state, "touched"), false);
});

async function navigationRuntime() {
  const source = await readShard();
  let focused = "";
  const element = (id: string, selected = false) => {
    const attributes = new Map<string, string>();
    const classes = new Set<string>();
    const fields: Record<string, unknown> = {};
    return {
      id, attributes, classes, fields,
      textContent: "", title: "", hidden: false,
      parentElement: null as unknown,
      setAttribute: (name: string, value: string) => attributes.set(name, value),
      querySelector: (selector: string) => fields[selector] ?? null,
      matches: () => selected,
      classList: { toggle: (name: string, enabled: boolean) => enabled ? classes.add(name) : classes.delete(name) },
      focus: () => { focused = id; },
    };
  };
  const headers = new Map(["a", "b"].map((id) => {
    const header = element(`header-${id}`);
    header.fields[".codex-plus-model-group-title"] = element(`title-${id}`);
    header.fields[".codex-plus-model-group-count"] = element(`count-${id}`);
    return [id, header];
  }));
  const grid = element("grid");
  grid.fields.button = headers.get("a");
  const toolbar = element("toolbar");
  const toolbarLabel = element("toolbar-label");
  toolbar.fields[".codex-plus-model-group-toolbar-label"] = toolbarLabel;
  toolbar.fields[".codex-plus-model-group-count"] = element("toolbar-count");
  const list = Object.assign(element("list"), {
    isConnected: true,
    scrollTop: 80,
    querySelector: (selector: string) => {
      if (selector === ".codex-plus-model-provider-grid") return grid;
      if (selector === ".codex-plus-model-group-toolbar") return toolbar;
      if (selector === ".codex-plus-model-group-header") return headers.get("a");
      const id = selector.match(/^\[data-codex-plus-model-group-id="([^"]*)"\]$/)?.[1];
      return id ? headers.get(id) ?? null : null;
    },
  });
  const entry = (groupId: string, id: string, selected = false) => {
    const row = element(id, selected);
    row.parentElement = list;
    return { groupId, provider: `供应商 ${groupId.toUpperCase()}`, row, name: id };
  };
  let rows = [entry("a", "a-1"), entry("b", "b-1", true), entry("a", "a-2")];
  let reads = 0;
  const state = { activeGroupId: "" };
  const functions = [
    "codexPlusModelPickerText",
    "codexPlusModelPickerGroups",
    "codexPlusModelPickerUniqueGroupIds",
    "codexPlusModelPickerSelectedRow",
    "codexPlusModelPickerSyncGroup",
    "codexPlusModelPickerNavigate",
    "codexPlusModelPickerSyncNavigation",
    "cssEscapeValue",
  ].map((name) => functionSource(source, name)).join("\n");
  const api = new Function(
    "document", "navigator", "codexPlusModelPickerGroupsState", "codexPlusModelPickerRows",
    "codexPlusModelPickerDefaultProviderName", "codexPlusModelPickerDefaultGroupId",
    "codexPlusModelPickerSyncDefaultBadge", "codexPlusModelPickerGroupAttr", "codexPlusModelPickerGroupIdAttr",
    `${functions}
      return {
        navigate: codexPlusModelPickerNavigate,
        sync: codexPlusModelPickerSyncNavigation,
        groups: () => codexPlusModelPickerUniqueGroupIds(codexPlusModelPickerGroups(codexPlusModelPickerRows())),
      };`,
  )(
    { documentElement: { lang: "zh-CN" } }, { language: "zh-CN" }, state,
    () => { reads++; return rows; }, () => "供应商 A", () => "a", () => {},
    "data-codex-plus-model-group", "data-codex-plus-model-group-id",
  );
  return {
    api, state, list, grid, toolbar, toolbarLabel, headers, entry,
    get rows() { return rows; },
    set rows(value) { rows = value; },
    get focused() { return focused; },
    get reads() { return reads; },
  };
}

it("从图标进入该供应商的原生模型列表，返回后恢复入口焦点", async () => {
  const runtime = await navigationRuntime();
  const { api, state, list, grid, toolbar, toolbarLabel, headers } = runtime;
  const order = runtime.rows.map((entry) => entry.row);
  const hidden = () => runtime.rows.map((entry) => entry.row.classes.has("codex-plus-model-group-collapsed"));
  api.sync(list, api.groups());
  assert.equal(grid.hidden, false);
  assert.equal(toolbar.hidden, true);
  assert.deepEqual(hidden(), [true, true, true], "供应商首页不混入模型行");
  assert.equal(headers.get("b")?.attributes.get("data-current"), "true", "当前供应商仍有选中标志");

  api.navigate(list, "b");
  assert.equal(state.activeGroupId, "b");
  assert.equal(list.attributes.get("data-codex-plus-model-provider-view"), "models");
  assert.equal(grid.hidden, true);
  assert.equal(toolbar.hidden, false);
  assert.equal(toolbarLabel.textContent, "供应商 B");
  assert.deepEqual(hidden(), [true, false, true]);
  assert.equal(runtime.focused, "b-1", "进入后聚焦选中模型");
  assert.equal(list.scrollTop, 0);

  api.navigate(list, "");
  assert.equal(state.activeGroupId, "");
  assert.equal(grid.hidden, false);
  assert.equal(toolbar.hidden, true);
  assert.equal(runtime.focused, "header-b", "返回原供应商入口");
  assert.deepEqual(hidden(), [true, true, true]);
  assert.deepEqual(runtime.rows.map((entry) => entry.row), order, "原生行保持顺序和身份");
  assert.ok(runtime.rows.every((entry) => entry.row.parentElement === list), "原生行不搬动");
});

it("导航使用当前行快照，当前供应商被移除时退回入口", async () => {
  const runtime = await navigationRuntime();
  const { api, state, list, grid, toolbar } = runtime;
  api.navigate(list, "b");
  runtime.rows = [runtime.entry("a", "new-a"), runtime.entry("b", "new-b", true)];
  api.navigate(list, "b");
  assert.equal(runtime.focused, "new-b", "React 替换行后不能聚焦旧节点");
  assert.equal(runtime.rows[1].row.classes.has("codex-plus-model-group-collapsed"), false);

  runtime.rows = [runtime.rows[0]];
  api.sync(list, api.groups());
  assert.equal(state.activeGroupId, "");
  assert.equal(grid.hidden, false);
  assert.equal(toolbar.hidden, true);
  assert.equal(runtime.focused, "header-a");
  assert.equal(runtime.rows[0].row.classes.has("codex-plus-model-group-collapsed"), true);

  const reads = runtime.reads;
  list.isConnected = false;
  api.navigate(list, "a");
  assert.equal(state.activeGroupId, "");
  assert.equal(runtime.reads, reads, "已关闭的菜单不能再被旧事件操作");
});

it("同一菜单扫描保留当前供应商，重新打开时回到供应商入口", async () => {
  const source = await readShard();
  const firstList = {};
  const secondList = {};
  const cleared: unknown[] = [];
  const observed: unknown[] = [];
  let disconnected = 0;
  class Observer {
    observe(list: unknown) { observed.push(list); }
    disconnect() { disconnected++; }
  }
  const state = { activeList: firstList, activeGroupId: "b", observer: new Observer() };
  const observe = new Function(
    "codexPlusModelPickerGroupsState", "codexPlusModelPickerClear", "MutationObserver",
    "codexPlusModelPickerNeedsRescan", "codexPlusModelPickerScheduleRescan",
    `${functionSource(source, "codexPlusModelPickerObserveList")}\nreturn codexPlusModelPickerObserveList;`,
  )(state, (list: unknown) => cleared.push(list), Observer, () => false, () => {});
  observe(firstList);
  assert.equal(state.activeGroupId, "b", "轮询不能把用户带回首页");
  assert.deepEqual(cleared, []);

  observe(secondList);
  assert.equal(state.activeGroupId, "");
  assert.equal(state.activeList, secondList);
  assert.deepEqual(cleared, [firstList]);
  assert.deepEqual(observed, [secondList]);
  assert.equal(disconnected, 1);

  state.activeGroupId = "a";
  observe(secondList);
  assert.equal(state.activeGroupId, "a");
  assert.deepEqual(observed, [secondList], "同一个菜单只注册一个观察器");
});

it("供应商网格支持方向键，模型列表中的 Escape 返回供应商入口", async () => {
  const source = await readShard();
  let focused = -1;
  const buttons = Array.from({ length: 5 }, (_, index) => ({
    parentElement: {},
    focus: () => { focused = index; },
  }));
  const list = { querySelectorAll: () => buttons };
  const state = { activeGroupId: "" };
  const navigated: unknown[] = [];
  const keydown = new Function(
    "codexPlusModelPickerGroupsState", "codexPlusModelPickerNavigate", "getComputedStyle",
    `${functionSource(source, "codexPlusModelPickerNavigationKeydown")}\nreturn codexPlusModelPickerNavigationKeydown;`,
  )(state, (container: unknown, id: string) => navigated.push([container, id]), () => ({ gridTemplateColumns: "80px 80px 80px" }));
  const press = (key: string, index: number) => {
    let prevented = false;
    let stopped = false;
    keydown({
      key, currentTarget: list, target: { closest: () => buttons[index] },
      preventDefault: () => { prevented = true; }, stopPropagation: () => { stopped = true; },
    });
    return { prevented, stopped };
  };
  assert.deepEqual(press("ArrowDown", 0), { prevented: true, stopped: true });
  assert.equal(focused, 3);
  press("End", 0);
  assert.equal(focused, 4);
  press("Home", 4);
  assert.equal(focused, 0);
  assert.deepEqual(press("Enter", 0), { prevented: false, stopped: true }, "保留按钮原生激活");
  state.activeGroupId = "b";
  assert.deepEqual(press("Escape", 0), { prevented: true, stopped: true });
  assert.deepEqual(navigated, [[list, ""]]);
});

it("提前捕获 Escape 只返回当前供应商子页，首页及菜单外按键继续交给原生", async () => {
  const source = await readShard();
  const child = {};
  const outside = {};
  const list = {
    isConnected: true,
    contains: (target: unknown) => target === child || target === list,
  };
  const state = { activeGroupId: "b", activeList: list as typeof list | null };
  const navigated: unknown[] = [];
  const keydown = new Function(
    "codexPlusModelPickerGroupsState", "codexPlusModelPickerNavigate",
    `${functionSource(source, "codexPlusModelPickerEscapeKeydown")}\nreturn codexPlusModelPickerEscapeKeydown;`,
  )(state, (container: unknown, id: string) => navigated.push([container, id]));
  const press = (overrides: Record<string, unknown> = {}) => {
    let prevented = false;
    let stopped = false;
    keydown({
      key: "Escape", isComposing: false, target: child,
      preventDefault: () => { prevented = true; },
      stopPropagation: () => { stopped = true; },
      ...overrides,
    });
    return { prevented, stopped };
  };
  assert.deepEqual(press(), { prevented: true, stopped: true });
  assert.deepEqual(navigated, [[list, ""]], "先返回供应商入口，阻止原生关闭弹层");
  navigated.length = 0;
  const passthrough = (overrides: Record<string, unknown>, message: string) => {
    assert.deepEqual(press(overrides), { prevented: false, stopped: false }, message);
    assert.deepEqual(navigated, [], message);
  };
  passthrough({ key: "Enter" }, "其它按键不由全局 Escape 监听处理");
  passthrough({ isComposing: true }, "输入法组合期间不接管 Escape");
  passthrough({ target: outside }, "菜单外的 Escape 不受影响");
  state.activeGroupId = "";
  passthrough({}, "供应商首页保留原生关闭行为");
  state.activeGroupId = "b";
  list.isConnected = false;
  passthrough({}, "已断开的菜单不能响应旧事件");
  list.isConnected = true;
  state.activeList = null;
  passthrough({}, "没有活动菜单时不拦截 Escape");
});

it("Escape 在 window 捕获阶段注册，菜单清理和重注入时移除同一监听", async () => {
  const source = await readShard();
  assert.match(
    functionSource(source, "codexPlusModelPickerPatch"),
    /window\.addEventListener\("keydown", codexPlusModelPickerEscapeKeydown, true\)/,
    "必须早于 document 捕获阶段的原生关闭监听",
  );
  assert.match(
    functionSource(source, "codexPlusModelPickerClear"),
    /window\.removeEventListener\("keydown", codexPlusModelPickerEscapeKeydown, true\)/,
    "关闭菜单和退回单供应商时解除全局监听",
  );
  const dispose = source.match(/dispose\(\) \{[\s\S]*?\n    \},/)?.[0];
  assert.ok(dispose);
  assert.match(
    dispose,
    /window\.removeEventListener\("keydown", codexPlusModelPickerEscapeKeydown, true\)/,
    "热重注入不能遗留旧实例的 Escape 监听",
  );
});

it("重注入复用并更新已有样式，不让新版入口继续使用旧折叠样式", async () => {
  const source = await readShard();
  const attributes = new Map<string, string>();
  const style = {
    id: "picker-style", isConnected: true, textContent: "old collapsed layout",
    setAttribute: (name: string, value: string) => attributes.set(name, value),
  };
  const install = new Function(
    "document", "codexPlusModelPickerGroupsStyleId", "codexPlusModelPickerGroupsExtMark",
    "codexPlusModelPickerNativeLabelClass", "codexPlusModelPickerOwnLabelClass", "codexPlusModelPickerFavoriteClass",
    `${functionSource(source, "codexPlusModelPickerInstallStyle")}\nreturn codexPlusModelPickerInstallStyle;`,
  )({
    getElementById: () => style,
    createElement: () => { throw new Error("不应创建重复样式"); },
    head: { appendChild: () => { throw new Error("不应重复挂载已有样式"); } },
  }, "picker-style", "builtin-model-picker-groups", "native-label", "own-label", "favorite");
  install();
  assert.match(style.textContent, /\.codex-plus-model-provider-grid\{display:grid/);
  assert.match(style.textContent, /\.codex-plus-model-provider-grid\[hidden\]/);
  assert.equal(attributes.get("data-codex-plus-ext"), "builtin-model-picker-groups");
  const current = style.textContent;
  install();
  assert.equal(style.textContent, current, "重复注入保持同一份样式");
});

it("默认供应商带「默认」标志，身份来自目录而不是猜名字", async () => {
  const source = await readShard();
  // 顶层 id 优先，其次目录条目的 default 标记，最后才是会话内记住的值。
  assert.match(source, /codex_plus_default_group_id/);
  assert.match(source, /codex_plus_default\b/);
  assert.match(source, /defaultGroupId/);
  // 标志要能随扫描补齐/摘掉，而不是建标题时定一次。
  assert.match(source, /codexPlusModelPickerSyncDefaultBadge/);
  assert.match(source, /codex-plus-model-group-default/);
  assert.match(source, /defaultBadge: "默认"/);
  // 默认分组在 id 去重改写之前先认出来，且按组内任意一行命中判定。
  assert.match(source, /group\.isDefault = !!defaultGroupId/);
  // 默认身份仅用于标志，不应跳过供应商入口直接进入默认模型列表。
  assert.doesNotMatch(source, /function codexPlusModelPickerAutoCollapse/);
});

it("目录条目按别名匹配，且只用它补分组 id、不用它决定显示归属", async () => {
  const source = await readShard();
  // 目录的展示名（GPT-6.1-Sol）与选择器里的行（6.1 Sol）不一样，必须登记别名。
  assert.match(source, /codexPlusModelPickerCatalogAliases/);
  assert.match(source, /replace\(\/\^gpt\[-\\s\]\*\/iu, ""\)/);
  // 早期版本的教训：拿目录展示名去拟合行文本会把同一家切成好几段。
  assert.match(source, /const groupId = known\?\.groupId/);
  // 只有整行精确命中目录（含 `（后缀）` 的完整展示名）时，才用目录里的分组名做显示归属：
  // 网关把同名模型的后几家写成 `名字（供应商 id）`，标题要显示打过码的完整账号名；别名命中仍不决定归属。
  assert.match(source, /const provider = exactKnown\?\.provider \|\| split\.provider \|\| known\?\.provider/);
  // 先按原文比（网关关掉「模型名后带供应商」时同名几份只差零宽字符），再按宽松归一化比。
  assert.match(source, /const exactKnown = exact\.get\(`raw\\u0000\$\{identity\.text\}`\)\s*\|\| exact\.get\(codexPlusModelPickerNormalizeLoose\(identity\.text\)\)/);
  assert.match(source, /const rawKey = `raw\\u0000\$\{String\(displayName\)/);
  // 组内任意一行命中默认分组即认定该组是默认（裸名行的 id 可能来自目录）。
  assert.match(source, /group\.entries\.some\(\(entry\) => entry\.groupId === defaultGroupId/);
});

it("供应商名称允许收缩，计数不靠 auto 外边距定位", async () => {
  const source = await readShard();
  const title = source.match(/\.codex-plus-model-group-title\{([^}]+)\}/)?.[1];
  assert.ok(title, "保留供应商标题的既有类名");
  assert.match(title, /min-width:0/);
  assert.match(title, /overflow:hidden/);
  assert.doesNotMatch(
    source,
    /codex-plus-model-group-count\{flex:0 0 auto;margin-left:auto/,
    "计数不能再用 margin-left:auto 定位",
  );
});

it("默认标志不改变无障碍语义：读屏会念出「默认供应商」", async () => {
  const source = await readShard();
  assert.match(source, /（默认供应商）/);
  assert.match(source, /\(default provider\)/);
});

it("在 Codex++ 里收藏的模型在选择器行尾画星标，取消收藏或还原时拿掉", async () => {
  const source = await readShard();
  assert.match(source, /favorite: entry\.codex_plus_favorite === true/);
  assert.match(source, /favorite: exactKnown\?\.favorite === true/);
  assert.match(source, /codexPlusModelPickerFavoriteClass = "codex-plus-model-group-favorite"/);
  // 星标节点带归属标记，扫描不会把它当成原生标签，也不会自喂循环。
  assert.match(source, /star\.setAttribute\("data-codex-plus-ext", codexPlusModelPickerGroupsExtMark\)/);
  assert.match(source, /content\.querySelector\(`\.\$\{codexPlusModelPickerFavoriteClass\}`\)\?\.remove\(\)/);
});

it("菜单认不出时记一次结构诊断：只在真有目录里的模型时记，不带模型名（#2753）", async () => {
  const source = await readShard();
  const body = source.match(/^  function codexPlusModelPickerReportUnrecognized\(\) \{[^]*?^  \}/m)?.[0];
  assert.ok(body, "诊断函数必须存在");
  const element = (attributes: Record<string, string>, text = "", children: unknown[] = []) => ({
    textContent: text,
    attributes: Object.entries(attributes).map(([name, value]) => ({ name, value })),
    getAttribute: (name: string) => attributes[name] ?? null,
    closest: () => null,
    querySelectorAll: (selector: string) => (selector === '[role="menuitemradio"]' ? children : selector === "[class]" ? children : []),
  });
  const radios = [
    element({ role: "menuitemradio", "data-state": "checked", class: "_Row_abc" }, "GPT-6 Sol（官方）"),
    element({ role: "menuitemradio", class: "_Row_abc" }, "DeepSeek-V4-Pro（DeepSeek）"),
  ];
  const run = (menus: unknown[], catalog: Map<string, unknown>) => {
    const sent: Array<{ event: string; detail: Record<string, unknown> }> = [];
    const state: Record<string, unknown> = {};
    const report = new Function(
      "document", "codexPlusModelPickerGroupsState", "codexPlusModelPickerCatalogExactMap",
      "codexPlusModelPickerNormalizeLoose", "visibleElement", "sendCodexPlusDiagnostic",
      `${body}\nreturn codexPlusModelPickerReportUnrecognized;`,
    )(
      { querySelectorAll: (selector: string) => (selector === '[role="menu"]' ? menus : []) },
      state, () => catalog, (value: string) => value.toLowerCase(), () => true,
      (event: string, detail: Record<string, unknown>) => { sent.push({ event, detail }); },
    );
    report();
    report();
    return sent;
  };
  const catalog = new Map([["gpt-6 sol（官方）", {}]]);
  const menu = element({ role: "menu", "data-side": "top", "data-state": "open", class: "_Menu_x1" }, "", radios);

  const sent = run([menu], catalog);
  assert.equal(sent.length, 1, "每个页面只记一次");
  assert.equal(sent[0].event, "model_picker_groups_menu_unrecognized");
  assert.equal(sent[0].detail.radios, 2);
  assert.equal(sent[0].detail.catalogMatches, 1);
  assert.deepEqual(sent[0].detail.menuData, ["data-side", "data-state"]);
  assert.deepEqual(sent[0].detail.modulePrefixes, ["Menu", "Row"]);
  assert.doesNotMatch(JSON.stringify(sent[0].detail), /GPT|DeepSeek|官方/, "诊断里不能有模型名");

  assert.equal(run([menu], new Map()).length, 0, "目录里没有对应模型（例如别的单选菜单）时不记");
  assert.equal(run([element({ role: "menu" }, "", radios.slice(0, 1))], catalog).length, 0, "只有一个选项的菜单不记");
});

it("认不出归属的行收进同一个「其他供应商」组，不被有名分组切成多段（#2860）", async () => {
  const source = await readShard();
  const groupsOf = new Function(
    `${functionSource(source, "codexPlusModelPickerGroups")}\nreturn codexPlusModelPickerGroups;`,
  )() as (rows: unknown[], defaultProviderName?: string) => Array<{ name: string; entries: unknown[] }>;
  const row = (name: string, provider = "", groupId = "") => ({
    name, provider, groupId, authoritativeGroupId: Boolean(groupId),
  });
  const groups = groupsOf([
    row("a1"),
    row("b1", "供应商 B", "provider-b"),
    row("a2"),
    row("c1", "供应商 C", "provider-c"),
    row("a3"),
  ]);
  const unnamed = groups.filter((group) => !group.name);
  assert.equal(unnamed.length, 1, "无名的行只能有一个分组");
  assert.equal(unnamed[0].entries.length, 3);
  assert.equal(groups.length, 3);
});

it("目录快照缺元数据时用记下的权威归属兜底，后缀猜出来的不套给裸名行（#2860）", async () => {
  const source = await readShard();
  const state = {
    providerNames: new Map<string, { name: string; parsed: boolean; groupId?: string }>([
      ["bare-model", { name: "默认供应商", parsed: false, groupId: "provider-default" }],
      ["shared-model", { name: "别家", parsed: true, groupId: "provider:别家" }],
    ]),
  };
  const build = (catalog: unknown) => new Function(
    "codexModelCatalog", "codexPlusModelPickerGroupsState", "providerNameOf",
    "codexPlusModelPickerCatalogAliases", "codexPlusModelPickerRememberProvider",
    `${functionSource(source, "codexPlusModelPickerCatalogProviderMap")}\nreturn codexPlusModelPickerCatalogProviderMap;`,
  )(
    catalog,
    state,
    (entry: { codex_plus_group?: string }) => entry.codex_plus_group || "",
    (displayName: string) => [displayName],
    () => undefined,
  ) as () => Map<string, { provider?: string; groupId?: string }>;

  // 拉取失败：快照里没有元数据。
  const fallback = build({ models: [] })();
  assert.deepEqual(fallback.get("bare-model"), { provider: "默认供应商", groupId: "provider-default", parsed: false });
  assert.equal(fallback.has("shared-model"), false, "从后缀猜的归属不能兜底给裸名行");

  // 快照正常：目录里的权威值优先于记下的旧值。
  const fresh = build({
    models: ["bare-model"],
    modelMetadata: {
      "bare-model": { displayName: "bare-model", codex_plus_group: "新供应商", codex_plus_group_id: "provider-new" },
    },
  })();
  assert.equal(fresh.get("bare-model")?.provider, "新供应商");
  assert.equal(fresh.get("bare-model")?.groupId, "provider-new");
});

it("模型列表按结构定位（模型行的共同父节点），类名只在没有模型行时兜底（#2893）", async () => {
  const source = await readShard();
  const listIn = new Function(`${functionSource(source, "codexPlusModelPickerListIn")}\nreturn codexPlusModelPickerListIn;`)() as (root: unknown) => unknown;
  type Node = { parentElement: Node | null; contains(other: Node): boolean; querySelectorAll(selector: string): Node[]; querySelector(selector: string): Node | null; name: string };
  const node = (name: string, parent: Node | null): Node => {
    const self: Node = {
      name,
      parentElement: parent,
      contains(other) { for (let n: Node | null = other; n; n = n.parentElement) if (n === self) return true; return false; },
      querySelectorAll: () => [],
      querySelector: () => null,
    };
    return self;
  };
  // 根 → 动画容器 → 列表（类名换成什么都行）→ 三个模型行
  const root = node("root", null);
  const motion = node("motion", root);
  const list = node("list-with-any-class", motion);
  const rows = [node("row1", list), node("row2", list), node("row3", list)];
  root.querySelectorAll = (selector) => (selector.includes("menuitemradio") ? rows : []);
  assert.equal(listIn(root), list, "不看类名，取模型行的共同父节点");

  // 还没有模型行：回落到类名，新旧格式都认，且不误中 ModelListMotion-。
  const fallback = node("class-list", root);
  root.querySelectorAll = () => [];
  let asked = "";
  root.querySelector = (selector) => { asked = selector; return fallback; };
  assert.equal(listIn(root), fallback);
  assert.match(asked, /\[class\*="_ModelList_"\]/);
  assert.match(asked, /\[class\^="ModelList-"\], \[class\*=" ModelList-"\]/);

  // 行分散到面板根下的不同分支（共同父节点退到根）：结构变了，也回落到类名。
  const a = node("a", root);
  const b = node("b", root);
  root.querySelectorAll = (selector) => (selector.includes("menuitemradio") ? [node("r1", a), node("r2", b)] : []);
  assert.equal(listIn(root), fallback);

  const matchesNew = (className: string) => className.startsWith("ModelList-") || className.includes(" ModelList-");
  assert.equal(matchesNew("ModelList-I4a0SH"), true);
  assert.equal(matchesNew("ModelListMotion-iyH7rX"), false);
});
