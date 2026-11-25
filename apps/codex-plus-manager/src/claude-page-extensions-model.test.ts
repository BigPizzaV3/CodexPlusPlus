import assert from "node:assert/strict";
import { test } from "node:test";
import { compareVersions, countUpdates, marketAction, matchesQuery, sortMarket, type PageExtension, type PageMarketEntry, pageRuntimeSummary } from "./claude-page-extensions-model.ts";

const entry = (id: string, version: string): PageMarketEntry => ({
  id, version, name: id, author: "", description: "", source: "official", repo: "a/b", path: "", commit: "0".repeat(40), url: "",
});
const local = (id: string, version: string, extra: Partial<PageExtension> = {}): PageExtension => ({
  id, version, name: id, author: "", description: "", builtin: false, enabled: true, ...extra,
});

test("版本号逐段比较，预发布版在正式版前", () => {
  assert.equal(compareVersions("1.10.0", "1.9.9"), 1);
  assert.equal(compareVersions("1.0.0", "1.0.0"), 0);
  assert.equal(compareVersions("1.0.0-beta.1", "1.0.0"), -1);
  assert.equal(compareVersions("1.0.0+build", "1.0.0"), 0);
  assert.equal(compareVersions("2.0", "1.9.9"), 1);
});

test("市场按钮：安装、更新、已安装、内置", () => {
  const installed = [local("theme", "1.0.0"), local("wide-chat", "1.0.0", { builtin: true }), local("broken", "1.0.0", { error: "清单坏了" })];
  assert.deepEqual(marketAction(entry("new", "1.0.0"), installed), { kind: "install" });
  assert.deepEqual(marketAction(entry("theme", "1.1.0"), installed), { kind: "update", from: "1.0.0" });
  assert.deepEqual(marketAction(entry("theme", "1.0.0"), installed), { kind: "installed" });
  assert.deepEqual(marketAction(entry("theme", "0.9.0"), installed), { kind: "installed" });
  assert.deepEqual(marketAction(entry("wide-chat", "9.0.0"), installed), { kind: "builtin" });
  assert.deepEqual(marketAction(entry("broken", "1.0.0"), installed), { kind: "update", from: "1.0.0" }, "清单坏了的可以重装修好");
  assert.equal(countUpdates([entry("theme", "1.1.0"), entry("new", "1.0.0")], installed), 1);
});

test("搜索不分大小写，空查询全匹配", () => {
  assert.equal(matchesQuery("", "x"), true);
  assert.equal(matchesQuery("WIDE", "wide-chat", null), true);
  assert.equal(matchesQuery("pet", "wide-chat", "加宽"), false);
});

test("市场排序：默认保持目录顺序，按名称，按星数（没有星数的在后）", () => {
  const list = [{ ...entry("b", "1.0.0"), name: "B" }, { ...entry("a", "1.0.0"), name: "A", stars: 3 }, { ...entry("c", "1.0.0"), name: "C", stars: 9 }];
  assert.deepEqual(sortMarket(list, "default").map((item) => item.id), ["b", "a", "c"]);
  assert.deepEqual(sortMarket(list, "name").map((item) => item.id), ["a", "b", "c"]);
  assert.deepEqual(sortMarket(list, "stars").map((item) => item.id), ["c", "a", "b"]);
  assert.deepEqual(list.map((item) => item.id), ["b", "a", "c"], "不改原数组");
});

test("a codexpp package's row says how it is doing in the Claude page", () => {
  const item = (extra: Partial<PageExtension>): PageExtension => ({
    id: "demo", name: "Demo", version: "1.0.0", author: "", description: "", builtin: false, enabled: true,
    format: "codexpp", capabilities: ["page"], notes: [], configSchema: [], config: {}, runtime: null, ...extra,
  });
  const runtime = (extra: object) => ({ status: "loaded", error: "", unsupported: [], loadedAt: "", ...extra });
  // 页面扩展、没启用的、清单有误的：不显示运行情况。
  assert.equal(pageRuntimeSummary(item({ format: "manifest" })), null);
  assert.equal(pageRuntimeSummary(item({ format: undefined })), null, "老版本的列表没有 format，按页面扩展");
  assert.equal(pageRuntimeSummary(item({ enabled: false })), null);
  assert.equal(pageRuntimeSummary(item({ error: "bad" })), null);
  assert.deepEqual(pageRuntimeSummary(item({})), { kind: "pending", detail: "" });
  assert.deepEqual(pageRuntimeSummary(item({ runtime: runtime({ status: "loading" }) })), { kind: "pending", detail: "" });
  assert.deepEqual(pageRuntimeSummary(item({ runtime: runtime({}) })), { kind: "loaded", detail: "" });
  assert.deepEqual(pageRuntimeSummary(item({ runtime: runtime({ status: "failed", error: "SyntaxError: x\n    at y" }) })), { kind: "failed", detail: "SyntaxError: x" });
  assert.deepEqual(pageRuntimeSummary(item({ runtime: runtime({ error: "page: boom" }) })), { kind: "error", detail: "page: boom" });
  assert.deepEqual(pageRuntimeSummary(item({ runtime: runtime({ unsupported: ["registerComposerAction", "registerSlot:composer.above"] }) })), {
    kind: "partial", detail: "registerComposerAction、registerSlot:composer.above",
  });
});
