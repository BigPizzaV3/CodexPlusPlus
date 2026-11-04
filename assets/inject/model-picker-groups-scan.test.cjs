// #2883：菜单关闭时的轮询轮次不能每秒做几十次强制布局。
// 诊断（codexPlusModelPickerReportUnrecognized）必须先过廉价 DOM 过滤、
// 且有 2 秒节流；只有真有候选单选菜单时才碰 visibleElement 的 getBoundingClientRect。
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { test } = require("node:test");
const vm = require("node:vm");

const path = require("node:path");
const source = readFileSync(path.join(__dirname, "renderer-inject", "71-model-picker-groups.js"), "utf8");

// 抽出函数源码单独执行，给它一整套可控的桩，避免拉起整个分片。
const fnSource = (() => {
  const start = source.indexOf("function codexPlusModelPickerReportUnrecognized()");
  assert.ok(start > 0, "shard 里要有 codexPlusModelPickerReportUnrecognized");
  const end = source.indexOf("function codexPlusModelPickerScan()", start);
  assert.ok(end > start);
  return source.slice(start, end);
})();

function fakeMenu(rect, { radios = [], inExtension = false } = {}) {
  return {
    closest: (selector) => (inExtension && selector === "[data-codex-plus-ext]" ? {} : null),
    querySelectorAll: (selector) => (selector === '[role="menuitemradio"]' ? radios : []),
    getBoundingClientRect: () => {
      fakeMenu.rectCalls += 1;
      return rect;
    },
  };
}
fakeMenu.rectCalls = 0;

function makeContext(menus) {
  const state = { unrecognizedReported: false };
  const ctx = vm.createContext({
    codexPlusModelPickerGroupsState: state,
    document: { querySelectorAll: () => menus },
    // visibleElement 走真实 Codex 的做法：看布局盒子。
    visibleElement: (el) => { const r = el.getBoundingClientRect(); return r.width > 0 && r.height > 0; },
    codexPlusModelPickerCatalogExactMap: () => new Map([["raw\u0000m1", 1], ["raw\u0000m2", 1]]),
    codexPlusModelPickerNormalizeLoose: (t) => t,
    sendCodexPlusDiagnostic: () => Promise.resolve(),
    Promise,
    Date,
  });
  vm.runInContext(`var codexPlusModelPickerReportUnrecognized; ${fnSource}`, ctx);
  return { ctx, state };
}

const visibleRect = { width: 10, height: 10 };

test("菜单里没有单选项时连续扫描零次布局读取", () => {
  fakeMenu.rectCalls = 0;
  const menus = [fakeMenu(visibleRect), fakeMenu(visibleRect), fakeMenu(visibleRect)];
  const { ctx } = makeContext(menus);
  for (let i = 0; i < 20; i += 1) ctx.codexPlusModelPickerReportUnrecognized();
  assert.equal(fakeMenu.rectCalls, 0, `布局读取 ${fakeMenu.rectCalls} 次`);
});

test("有候选菜单时 2 秒内最多检查一次布局", () => {
  fakeMenu.rectCalls = 0;
  const menu = fakeMenu(visibleRect, { radios: [{ textContent: "zz" }, { textContent: "yy" }] });
  const { ctx } = makeContext([menu]);
  ctx.codexPlusModelPickerReportUnrecognized();
  const afterFirst = fakeMenu.rectCalls;
  assert.ok(afterFirst >= 1, "候选菜单要被检查");
  for (let i = 0; i < 10; i += 1) ctx.codexPlusModelPickerReportUnrecognized();
  assert.equal(fakeMenu.rectCalls, afterFirst, "2 秒节流内不再读布局");
});

test("拓展自己插入的菜单不触发布局读取", () => {
  fakeMenu.rectCalls = 0;
  const menus = [fakeMenu(visibleRect, { radios: [{}, {}], inExtension: true })];
  const { ctx } = makeContext(menus);
  ctx.codexPlusModelPickerReportUnrecognized();
  assert.equal(fakeMenu.rectCalls, 0);
});
