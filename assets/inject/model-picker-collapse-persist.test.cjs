// #3073：折叠状态要跨网关变更/重启保留。
// 从分片里抽出折叠持久化相关函数单独执行，给它一套可控的 localStorage 桩。
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { test } = require("node:test");
const vm = require("node:path") === null ? null : require("node:vm");
const path = require("node:path");

const source = readFileSync(path.join(__dirname, "renderer-inject", "71-model-picker-groups.js"), "utf8");

// 重启场景的关键：换列表节点时不再清空折叠集合。
test("ObserveList 换列表节点时不再清空折叠状态", () => {
  const start = source.indexOf("function codexPlusModelPickerObserveList(list)");
  assert.ok(start > 0, "shard 里要有 codexPlusModelPickerObserveList");
  const end = source.indexOf("function codexPlusModelPickerScheduleRescan()", start);
  const body = source.slice(start, end);
  assert.ok(!body.includes("state.collapsed?.clear()"), "换列表节点不能清空折叠集合");
  assert.ok(body.includes("#3073"), "应注明 issue #3073 的保留语义");
});

function extract(name, endMarker) {
  const start = source.indexOf(`function ${name}()`);
  assert.ok(start > 0, `shard 里要有 ${name}`);
  const end = endMarker
    ? source.indexOf(endMarker, start)
    : source.indexOf("\n  }", start) + 3;
  assert.ok(end > start, `${name} 截取失败`);
  return source.slice(start, end);
}

function makeContext(store) {
  const state = {
    collapsed: new Set(["旧实例交接的组"]),
    viewMode: "list",
    viewLoaded: false,
    collapsedLoaded: false,
  };
  const ctx = vm.createContext({
    codexPlusModelPickerGroupsState: state,
    window: { localStorage: store },
    codexPlusModelPickerCollapsedListStoreKey: "codexPlus.modelPicker.collapsedList.v1",
    codexPlusModelPickerViewStoreKey: "codexPlus.modelPicker.viewMode.v1",
    codexPlusModelPickerLoadCollapsed: () => ctx.codexPlusModelPickerLoadCollapsed(),
    JSON,
  });
  return { ctx, state };
}

test("重启后（新实例）从 localStorage 恢复折叠分组", () => {
  const store = {
    items: { "codexPlus.modelPicker.collapsedList.v1": JSON.stringify(["g1", "g2", ""]) },
    getItem(key) { return this.items[key] ?? null; },
    setItem(key, value) { this.items[key] = value; },
  };
  const { ctx, state } = makeContext(store);
  vm.runInContext(`var codexPlusModelPickerLoadCollapsed; var codexPlusModelPickerPersistCollapsed; var codexPlusModelPickerLoadView; ${
    extract("codexPlusModelPickerLoadCollapsed", "function codexPlusModelPickerPersistCollapsed")
  }${extract("codexPlusModelPickerPersistCollapsed", "function codexPlusModelPickerRememberAttribute")}`, ctx);
  ctx.codexPlusModelPickerLoadCollapsed();
  assert.ok(state.collapsed.has("旧实例交接的组"));
  assert.ok(state.collapsed.has("g1") && state.collapsed.has("g2"), "持久化的折叠 id 要恢复");
  assert.equal(state.collapsed.size, 3, "空串 id 不恢复");
});

test("折叠变更落盘，重注入交接与持久化取并集", () => {
  const store = {
    items: {},
    getItem(key) { return this.items[key] ?? null; },
    setItem(key, value) { this.items[key] = value; },
  };
  const { ctx, state } = makeContext(store);
  vm.runInContext(`var codexPlusModelPickerLoadCollapsed; var codexPlusModelPickerPersistCollapsed; ${
    extract("codexPlusModelPickerLoadCollapsed", "function codexPlusModelPickerPersistCollapsed")
  }${extract("codexPlusModelPickerPersistCollapsed", "function codexPlusModelPickerRememberAttribute")}`, ctx);
  state.collapsed.add("新折叠的组");
  ctx.codexPlusModelPickerPersistCollapsed();
  assert.deepEqual(JSON.parse(store.items["codexPlus.modelPicker.collapsedList.v1"]).sort(), ["新折叠的组", "旧实例交接的组"]);
  // 模拟重启：全新实例只有持久化数据。
  const reloaded = JSON.parse(store.getItem("codexPlus.modelPicker.collapsedList.v1"));
  assert.ok(reloaded.includes("新折叠的组"));
});

test("坏数据不抛错、不污染状态", () => {
  const store = {
    getItem: () => "not-json{",
    setItem() {},
  };
  const { state } = makeContext(store);
  const { ctx } = makeContext(store);
  vm.runInContext(`var codexPlusModelPickerLoadCollapsed; ${extract("codexPlusModelPickerLoadCollapsed", "function codexPlusModelPickerPersistCollapsed")}`, ctx);
  ctx.codexPlusModelPickerLoadCollapsed();
  assert.equal(state.collapsed.size, 1, "只保留交接来的 id");
});
