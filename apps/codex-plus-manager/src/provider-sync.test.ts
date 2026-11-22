import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { modelSlug, pickerOutcome, pickerRows, upstreamDiff } from "./provider-sync-model.ts";

const read = (name: string) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");

test("从上游同步的差异按弹窗里当前的列表算，比对忽略大小写和 [1M] 后缀，上游重名只算一次（#2785）", () => {
  assert.equal(modelSlug(" Claude-Opus-4-6[1M] "), "claude-opus-4-6");
  assert.deepEqual(upstreamDiff(["b", "A", "big[1M]", "old"], ["a", "B", "BIG", "new-1", "new-1", "NEW-2"]), {
    added: ["new-1", "NEW-2"],
    removed: ["old"],
  });
});

test("编辑弹窗「从上游获取」的勾选列表：已有的默认勾上、取消即移除，新增的默认不勾（#2785）", () => {
  const rows = pickerRows(["a", "custom", "big[1M]"], ["A", "BIG", "new-1", "new-2"]);
  assert.deepEqual(rows, [
    { model: "a", inList: true, upstream: true },
    { model: "custom", inList: true, upstream: false },
    { model: "big[1M]", inList: true, upstream: true },
    { model: "new-1", inList: false, upstream: true },
    { model: "new-2", inList: false, upstream: true },
  ]);
  // 初始勾选 = 现有列表：什么都不改。
  assert.deepEqual(pickerOutcome(rows, new Set(["a", "custom", "big[1M]"])), { models: ["a", "custom", "big[1M]"], added: 0, removed: 0 });
  // 取消 custom、勾上 new-2。
  assert.deepEqual(pickerOutcome(rows, new Set(["a", "big[1M]", "new-2"])), { models: ["a", "big[1M]", "new-2"], added: 1, removed: 1 });
});

test("编辑弹窗不论上游模型多少都弹勾选列表，不再直接全部加入", () => {
  const form = read("ProviderFormDialog.tsx");
  assert.doesNotMatch(form, /FETCHED_DIRECT_LIMIT/);
  assert.doesNotMatch(read("provider-form-model.ts"), /FETCHED_DIRECT_LIMIT/);
  assert.match(form, /setPicker\(\{ rows: pickerRows\(models, fetched\), selected: new Set\(models\), query: "", fetched: fetched\.length \}\)/);
  const picker = form.slice(form.indexOf("function ModelPicker("));
  assert.match(picker, /t\("将移除"\)/, "取消勾选的已有模型标出「将移除」");
  assert.match(picker, /t\("全选新增"\)/);
  assert.match(picker, /t\("全不选新增"\)/);
});

test("模型弹窗里的「从上游同步」：先拉预览、不写入；下架的默认勾选移除，新增的默认不勾、可搜索、全选 / 全不选", () => {
  const dialog = read("ProviderModelsDialog.tsx");
  assert.match(dialog, /invoke<PreviewResult>\("preview_gateway_provider_models_sync", \{ id: entry\.id \}\)/);
  assert.match(dialog, /setRemove\(new Set\(diff\.removed\)\);\r?\n\s*setAdd\(new Set\(\)\);/);
  assert.match(dialog, /t\("全选"\)/);
  assert.match(dialog, /t\("全不选"\)/);
  assert.match(dialog, /tf\("默认模型 \{0\} 将被移除，改用 \{1\}。"/);
  assert.match(dialog, /if \(event\.key === "Enter"\) event\.preventDefault\(\);/, "搜索框回车不提交");
  assert.match(dialog, /tf\("只显示前 \{0\} 个，共 \{1\} 个；用搜索缩小范围。"/, "超过 500 个时写明只显示了前 500 个");
  assert.match(dialog, /className="gw-modal-backdrop gw-portal" \{\.\.\.backdropProps\}/);
  assert.doesNotMatch(dialog, /check_gateway_provider_models/, "三个拉上游模型的入口只剩表单的 fetch_provider_models 和这里的预览");
});

test("编辑弹窗里可以打开上下文窗口弹窗（复用详情的弹窗），新添加时不显示", () => {
  const form = read("ProviderFormDialog.tsx");
  assert.match(form, /\{windowsEntry && onEditWindows \? \(/);
  assert.match(form, /onClick=\{\(\) => onEditWindows\(models\)\}/);
  const providers = read("GatewayProviders.tsx");
  assert.match(providers, /onEditWindows=\{\(models\) => setFormWindows\(models\)\}/);
  assert.match(providers, /<ProviderWindowsDialog\r?\n\s*busy=\{busy !== null\}\r?\n\s*entry=\{\{ \.\.\.editingEntry, models: formWindows \}\}/);
  assert.match(providers, /withUnlistedWindows\(editingEntry, formWindows, modelWindows\)/);
  // 新添加的表单不传窗口入口。
  assert.doesNotMatch(providers, /<ProviderFormDialog agentPicks=\{agentPicks\}[^\n]*onEditWindows/);
});

test("新加的弹窗样式挂在 .gw-portal 下（弹窗是 portal，正式版里组件 CSS 先于 styles.css 加载）", () => {
  const css = read("ProviderFormDialog.css");
  for (const selector of [".pf-picker-tag", ".pf-picker-empty", ".pf-sync-summary", ".pf-windows-entry", ".pf-windows-summary"]) {
    const rules = css.split("\n").filter((line) => line.includes(selector) && line.trim().endsWith("{"));
    assert.ok(rules.length, selector);
    for (const rule of rules) assert.match(rule, /^\.gw-portal /, rule);
  }
});
