// 模型行装饰：价格档位、目录价格映射与厂商图标解析。
// 沿用 model-picker-groups-scan.test.cjs 的做法：抽出分片函数源码用 vm 单独执行 + 桩。
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { test } = require("node:test");
const vm = require("node:vm");
const path = require("node:path");

const source = readFileSync(path.join(__dirname, "renderer-inject", "71-model-picker-groups.js"), "utf8");

const slice = (startMarker, endMarker) => {
  const start = source.indexOf(startMarker);
  assert.ok(start > 0, `分片里要有 ${startMarker}`);
  const end = source.indexOf(endMarker, start);
  assert.ok(end > start, `分片里要有 ${endMarker}`);
  return source.slice(start, end);
};

// A 段：价格解析与目录价格映射（含归一化/拆分/归属辅助函数）。
const sliceA = slice("  function providerNameOf", "  function codexPlusModelPickerRowIdentity");
// B 段：厂商字形表、行图标解析与价格档位。
const sliceB = slice("  const codexPlusModelPickerGlyphs", "  function codexPlusModelPickerPriceNode");

function makeContext(catalog = {}) {
  const ctx = vm.createContext({
    codexModelCatalog: catalog,
    codexPlusModelPickerGroupNameMaxLength: 24,
    codexPlusModelPickerGroupsState: { providerNames: new Map(), defaultProviderName: "", defaultGroupId: "" },
    codexPlusModelPickerText: () => ({ unnamed: "其他供应商" }),
    // 行内没有同名模型时图标回退成供应商配色字母头像。
    codexPlusModelPickerProviderIcon: ({ id, name }) => ({
      label: String(name || "?").slice(0, 2).toUpperCase(),
      color: "#123456",
    }),
    Map, Set, Array, Object, String, Number, JSON, Math, Promise,
  });
  vm.runInContext(`var __a; ${sliceA} __a = true;`, ctx);
  vm.runInContext(`var __b; ${sliceB} __b = true;`, ctx);
  return ctx;
}

const catalog = {
  models: ["cpx-deepseek/deepseek-chat", "cpx-claude/claude-sonnet-4-5", "free-model"],
  modelMetadata: {
    "cpx-deepseek/deepseek-chat": {
      displayName: "DeepSeek-Chat（DeepSeek）",
      codex_plus_group_id: "deepseek",
      codex_plus_price: { input: 0.27, cachedInput: 0.07, output: 1.1, currency: "USD" },
    },
    "cpx-claude/claude-sonnet-4-5": {
      displayName: "Claude Sonnet 4.5（官转）",
      codex_plus_group_id: "claude",
      codex_plus_price: { input: 3, output: 15 },
    },
    "free-model": {
      displayName: "Free Model",
      codex_plus_price: { input: 0, output: 0 },
    },
    "no-price": { displayName: "No Price" },
  },
};

test("目录价格映射：带后缀显示名拆后缀后也能命中", () => {
  const ctx = makeContext(catalog);
  const map = ctx.codexPlusModelPickerCatalogPriceMap();
  assert.equal(map.get("raw\u0000DeepSeek-Chat（DeepSeek）")?.output, 1.1, "原文键");
  assert.equal(map.get(ctx.codexPlusModelPickerNormalizeLoose("DeepSeek-Chat"))?.input, 0.27, "裸名归一化");
  assert.equal(map.get(ctx.codexPlusModelPickerNormalizeLoose("claude-sonnet-4-5"))?.output, 15, "slug 剥前缀");
  assert.equal(map.get(ctx.codexPlusModelPickerNormalizeLoose("Free Model"))?.input, 0, "免费模型有价格");
  assert.equal(map.get(ctx.codexPlusModelPickerNormalizeLoose("No Price")), undefined, "无价目不登记");
});

test("价格档位：输入输出各半混合分档", () => {
  const ctx = makeContext();
  const tier = ctx.codexPlusModelPickerPriceTier;
  assert.equal(tier(null), -1, "无价目不画");
  assert.equal(tier({ input: 0, output: 0 }), 0, "零价画免费");
  assert.equal(tier({ input: 0.27, output: 1.1 }), 2, "deepseek-chat ≈0.7 → 2 段");
  assert.equal(tier({ input: 3, output: 15 }), 4, "claude ≈9 → 4 段");
  assert.equal(tier({ input: 15, output: 60 }), 5, "o 系 → 5 段");
  assert.equal(tier({ input: 0.01, output: 0.01 }), 1, "接近免费仍是有价");
});

test("厂商图标：已知家族给彩色字形，认不出的回退字母", () => {
  const ctx = makeContext();
  const glyph = ctx.codexPlusModelPickerGlyph;
  const openai = glyph({ name: "GPT-6.1 Sol" }).svg;
  assert.ok(glyph({ name: "Claude-Sonnet-4-6" }).svg.includes("Claude") || glyph({ name: "Claude-Sonnet-4-6" }).svg, "claude → svg");
  assert.ok(openai, "gpt → svg");
  assert.equal(glyph({ name: "6.1 Sol" }).svg, openai, "去掉 GPT- 前缀的行也认 OpenAI");
  assert.equal(glyph({ name: "5.3 Codex Spark" }).svg, openai, "codex 系列 → OpenAI");
  assert.equal(glyph({ name: "Mystery", slug: "cpx-a/gpt-5.5" }).svg, openai, "按 slug 认出");
  assert.notEqual(glyph({ name: "gemini-3-pro" }).svg, openai, "gemini 不是 OpenAI");
  assert.ok(glyph({ name: "deepseek-v4" }).svg, "deepseek → svg");
  assert.ok(glyph({ name: "qwen3-max" }).svg, "qwen → svg");
  assert.ok(glyph({ name: "Grok 4.7" }).svg, "grok → svg");
  assert.equal(ctx.codexPlusModelPickerProviderGlyph({ name: "jojocodemax" }), null, "中转账号不认品牌");
  assert.equal(ctx.codexPlusModelPickerProviderGlyph({ name: "a 哥" }), null);
  assert.ok(ctx.codexPlusModelPickerProviderGlyph({ name: "ChatGPT Pro · 26****" }), "ChatGPT Pro → OpenAI");
  const unknown = glyph({ name: "mystery-1", provider: "自家中转", groupId: "self" });
  assert.equal(unknown.color, "#123456", "未知家族 → 供应商配色");
  assert.equal(unknown.letter, "M");
});
