// #3100：网关供应商池的自动压缩阈值（窗口的百分比）。
//
// 后端把百分比换算成目录里的 `auto_compact_token_limit`（Rust 侧用例见
// crates/codex-plus-core/tests/all/gateway_auto_compact.rs）；这里守输入解析和整表替换的边界：
// 空串 = 没设（不写字段、用 Codex 默认），写错要在保存前拦下来，编辑弹窗只列部分模型时
// 没列出的模型原有阈值不能被清掉。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { parseCompactPercent, withUnlistedAutoCompact } from "./gateway-auto-compact-model.ts";

const read = (name: string) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");

test("阈值输入：空串表示没设，1~100 之间才算合法，写错的返回 NaN", () => {
  assert.equal(parseCompactPercent(""), null);
  assert.equal(parseCompactPercent("   "), null);
  assert.equal(parseCompactPercent("85"), 85);
  assert.equal(parseCompactPercent("85%"), 85);
  assert.equal(parseCompactPercent(" 84.5 "), 84.5);
  assert.equal(parseCompactPercent("1"), 1);
  assert.equal(parseCompactPercent("100"), 100);
  for (const bad of ["0", "100.5", "101", "-5", "85.555", "abc", "8 5", "%"]) {
    assert.ok(Number.isNaN(parseCompactPercent(bad)), bad);
  }
});

test("编辑弹窗按部分模型保存时，没列出的模型原有阈值照旧保留", () => {
  const entry = {
    autoCompact: { models: { "hidden-a": 90, "shown-b": 70 }, fallback: 60 },
  } as never;
  const merged = withUnlistedAutoCompact(entry, ["shown-b", "new-c"], {
    models: { "shown-b": 80, "new-c": 75 },
    fallback: 50,
  });
  // 列出的整表替换；没列出的 hidden-a 原样带上。
  assert.deepEqual(merged, { models: { "hidden-a": 90, "shown-b": 80, "new-c": 75 }, fallback: 50 });
  // 没设过阈值的老供应商也能直接合并。
  assert.deepEqual(withUnlistedAutoCompact({} as never, ["a"], { models: { a: 85 }, fallback: null }), {
    models: { a: 85 },
    fallback: null,
  });
});

test("窗口弹窗真的接了阈值：列里有输入框、非法值阻止保存、保存时传给后端", () => {
  const dialog = read("ProviderWindowsDialog.tsx");
  assert.match(dialog, /t\("自动压缩阈值"\)/);
  assert.match(dialog, /parseCompactPercent\(percents\[FALLBACK\]/);
  assert.match(dialog, /invalidPercent\.length > 0/);
  assert.match(dialog, /onSave\(windows, parseContextWindow\(rows\[FALLBACK\] \?\? ""\) \|\| null, \{/);
  const providers = read("GatewayProviders.tsx");
  assert.match(providers, /autoCompact: withUnlistedAutoCompact\(editingEntry, formWindows, autoCompact\)/);
  assert.match(providers, /"save_gateway_provider_windows", \{ id: entry\.id, windows: modelWindows, fallback, autoCompact \}/);
});
