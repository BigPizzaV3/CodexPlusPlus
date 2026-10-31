import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import {
  changeRows,
  defaultPicked,
  ignoredFor,
  normalizePreview,
  reasoningPicks,
  windowPicks,
  type MetaSyncPreview,
} from "./provider-meta-sync-model.ts";
import { EN_BACKEND_PATTERNS, EN_PLAIN, EN_TEMPLATE } from "./i18n-en.ts";

const read = (name: string) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");

const preview: MetaSyncPreview = normalizePreview({
  upstreamModels: 3,
  matched: 3,
  withWindow: 2,
  withReasoning: 2,
  windows: [
    { model: "gpt-x", window: 1_000_000, source: "context_length", options: [300_000, 600_000, 1_000_000], current: 272_000, currentSource: "builtin", manual: false },
    { model: "mine", window: 400_000, source: "max_input_tokens", options: [], current: 128_000, currentSource: "custom", manual: true },
  ],
  reasoning: [
    { model: "gpt-x", levels: ["low", "high", "max"], defaultLevel: "high", source: "reasoning_efforts", defaultSource: "reasoning_default_effort", currentLevels: ["low", "medium", "high"], currentDefault: null, currentSource: "builtin", manual: false },
    { model: "mine", levels: ["low", "high"], defaultLevel: null, source: "reasoning_efforts", defaultSource: "", currentLevels: ["high"], currentDefault: "high", currentSource: "custom", manual: true },
  ],
  ignored: [
    { model: "gpt-x", field: "reasoning_efforts", value: "turbo", reason: "unknownLevel" },
    { model: "gpt-x", field: "max_context_tokens", value: "0", reason: "invalidWindow" },
    { model: "mine", field: "reasoning.efforts", value: "{}", reason: "invalidType" },
  ],
});

test("同步上下文窗口 / 思考等级：手动设置的默认不勾，其余默认勾上（#2807）", () => {
  assert.deepEqual([...defaultPicked(preview, "windows")], ["gpt-x"]);
  assert.deepEqual([...defaultPicked(preview, "reasoning")], ["gpt-x"]);
});

test("差异行列出新值、旧值、来源字段和可选档位（#2807）", () => {
  const windows = changeRows(preview, "windows");
  assert.deepEqual(windows[0], {
    model: "gpt-x",
    next: "1M",
    current: "272K",
    currentSource: "builtin",
    source: "context_length",
    manual: false,
    options: "300K / 600K / 1M",
  });
  assert.equal(windows[1].manual, true);
  const reasoning = changeRows(preview, "reasoning");
  assert.equal(reasoning[0].next, "low, high, max · 默认 high");
  assert.equal(reasoning[0].current, "low, medium, high");
  assert.equal(reasoning[0].source, "reasoning_efforts · reasoning_default_effort");
  assert.equal(reasoning[1].current, "high · 默认 high");
});

test("勾选结果只带勾上的模型；勾上手动值才覆盖（#2807）", () => {
  assert.deepEqual(windowPicks(preview, new Set(["gpt-x"])), [{ model: "gpt-x", window: 1_000_000 }]);
  assert.deepEqual(windowPicks(preview, new Set(["gpt-x", "mine"])).map((pick) => pick.model), ["gpt-x", "mine"]);
  assert.deepEqual(reasoningPicks(preview, new Set(["mine"])), [{ model: "mine", levels: ["low", "high"], defaultLevel: null }]);
  assert.deepEqual(windowPicks(preview, new Set()), []);
});

test("被忽略的字段按类别分开提示（#2807）", () => {
  assert.deepEqual(ignoredFor(preview, "windows").map((note) => note.field), ["max_context_tokens"]);
  assert.deepEqual(ignoredFor(preview, "reasoning").map((note) => note.field), ["reasoning_efforts", "reasoning.efforts"]);
  assert.deepEqual(normalizePreview(undefined).windows, []);
});

test("供应商详情的上下文窗口、思考等级两行都有「从上游同步」，订阅账号不显示（#2807）", () => {
  const source = read("GatewayProviders.tsx");
  assert.match(source, /const metaSyncable = !entry\.account && entry\.models\.length > 0;/);
  assert.match(source, /onClick=\{\(\) => setMetaSync\("windows"\)\}/);
  assert.match(source, /onClick=\{\(\) => setMetaSync\("reasoning"\)\}/);
  assert.match(source, /"apply_gateway_provider_windows_sync", \{ id: entry\.id, picks \}/);
  assert.match(source, /"apply_gateway_provider_reasoning_sync", \{ id: entry\.id, picks \}/);
  const dialog = read("ProviderMetaSyncDialog.tsx");
  // 打开时只预览，确认后才写入；初始勾选按手动值区分。
  assert.match(dialog, /invoke<PreviewResult>\("preview_gateway_provider_meta_sync", \{ id: entry\.id \}\)/);
  assert.match(dialog, /setPicked\(defaultPicked\(preview, kind\)\);\r?\n/);
  assert.doesNotMatch(dialog, /apply_gateway_provider_/);
  const lib = read("../src-tauri/src/lib.rs");
  for (const command of ["preview_gateway_provider_meta_sync", "apply_gateway_provider_windows_sync", "apply_gateway_provider_reasoning_sync"]) {
    assert.match(lib, new RegExp(`gateway_commands::${command},`));
  }
});

test("同步上下文窗口 / 思考等级的文案都有英文（#2807）", () => {
  const sources = read("ProviderMetaSyncDialog.tsx") + read("GatewayProviders.tsx") + read("provider-meta-sync-model.ts");
  const plain = [...read("ProviderMetaSyncDialog.tsx").matchAll(/\bt\("([^"]+)"\)/g)].map((match) => match[1]);
  for (const key of plain) assert.ok(EN_PLAIN[key], `缺英文：${key}`);
  const templates = [...read("ProviderMetaSyncDialog.tsx").matchAll(/\btf\("([^"]+)"/g)].map((match) => match[1]);
  for (const key of templates) assert.ok(EN_TEMPLATE[key], `缺英文：${key}`);
  for (const label of ["手动设置", "上次同步", "供应商默认", "内置资料", "未设置", "窗口数值不合理", "不是 Codex 支持的档位", "默认档不在支持的档位里", "字段类型不对"]) {
    assert.ok(sources.includes(`"${label}"`), label);
    assert.ok(EN_PLAIN[label], `缺英文：${label}`);
  }
  for (const message of ["已从上游同步 2 个模型的上下文窗口。", "已从上游同步 1 个模型的思考等级。", "读取上游模型资料失败：x", "同步思考等级失败：x"]) {
    assert.ok(EN_BACKEND_PATTERNS.some(([pattern]) => pattern.test(message)), message);
  }
});
