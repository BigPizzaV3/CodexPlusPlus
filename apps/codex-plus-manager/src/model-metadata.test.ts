import assert from "node:assert";
import { describe, it } from "node:test";
import { isValidAutoCompactPercent, normalizeAutoCompactPercent } from "./auto-compact.ts";
import {
  clearModelMetadataForSlug,
  parseModelMetadataDocument,
  parseModelMetadataMap,
  remapModelMetadataSlugs,
  replaceModelMetadataForSlug,
  retainModelMetadataForSlugs,
  serializeModelMetadataDocument,
  synchronizeModelMetadataDocumentContextWindow,
  synchronizeModelMetadataDocumentLimits,
  synchronizeModelMetadataDocumentLimitsPreview,
} from "./model-metadata.ts";

describe("model metadata helpers", () => {
  it("解析单模型并保护专用字段", () => {
    const result = parseModelMetadataDocument(JSON.stringify({
      slug: "model-a",
      context_window: 1_000_000,
      auto_compact_token_limit: 800_000,
      max_context_window: 1_000_000,
      priority: 2,
      truncation_policy: { mode: "tokens", limit: 10000 },
      vendor_extension: ["kept"],
    }), "model-a");
    assert.strictEqual(result.ok, true);
    if (!result.ok) return;
    assert.strictEqual(result.value.contextWindow, "1000000");
    assert.strictEqual(result.value.autoCompactPercent, "80%");
    assert.deepStrictEqual(result.value.metadata, {
      truncation_policy: { mode: "tokens", limit: 10000 },
      vendor_extension: ["kept"],
    });
    assert.deepStrictEqual(result.value.ignoredFields, ["max_context_window", "priority"]);
  });

  it("支持 export/module 包装但不会执行 JavaScript", () => {
    assert.strictEqual(
      parseModelMetadataDocument('export default {"slug":"model-a"};', "model-a").ok,
      true,
    );
    assert.strictEqual(
      parseModelMetadataDocument('module.exports = {"models":[{"slug":"model-a"}]};', "model-a").ok,
      true,
    );
    assert.strictEqual(parseModelMetadataDocument("export default getModels();", "model-a").ok, false);
  });

  it("导入多模型文档时只匹配精确 slug", () => {
    const result = parseModelMetadataDocument(
      JSON.stringify({ models: [{ slug: "model-a", marker: "a" }, { slug: "model-b", marker: "b" }] }),
      "model-b",
    );
    assert.strictEqual(result.ok, true);
    if (result.ok) assert.deepStrictEqual(result.value.metadata, { marker: "b" });
  });

  it("替换、清除、保留和 slug 重命名只影响 metadata map", () => {
    const replaced = replaceModelMetadataForSlug(
      '{"model-a":{"old":true},"other":{"keep":true}}',
      "model-a",
      { supports_search_tool: true, priority: 2 },
    );
    assert.deepStrictEqual(JSON.parse(replaced), {
      "model-a": { supports_search_tool: true },
      other: { keep: true },
    });
    assert.strictEqual(clearModelMetadataForSlug(replaced, "model-a"), '{"other":{"keep":true}}');
    assert.strictEqual(
      remapModelMetadataSlugs('{"a":{"x":1},"b":{"x":2}}', [
        { previousSlug: "a", nextSlug: "b" },
        { previousSlug: "b", nextSlug: "c" },
      ]),
      '{"b":{"x":1},"c":{"x":2}}',
    );
    assert.strictEqual(
      retainModelMetadataForSlugs('{"a":{"x":1},"deleted":{"x":2}}', ["a"]),
      '{"a":{"x":1}}',
    );
  });

  it("模型窗口和比例使用十进制 K/M 及 half-up 舍入", () => {
    const document = serializeModelMetadataDocument("model-a", { vendor: "x" }, "1M", "80%");
    assert.deepStrictEqual(JSON.parse(document), {
      models: [{ slug: "model-a", context_window: 1_000_000, auto_compact_token_limit: 800_000, vendor: "x" }],
    });
    const rounded = synchronizeModelMetadataDocumentLimits(
      '{"slug":"tiny","context_window":3}',
      "tiny",
      "3",
      "50%",
    );
    assert.strictEqual(JSON.parse(rounded ?? "null").auto_compact_token_limit, 2);
  });

  it("空比例保持 Codex 默认行为并移除专用阈值", () => {
    const document = synchronizeModelMetadataDocumentLimits(
      '{"slug":"model-a","context_window":100,"auto_compact_token_limit":90}',
      "model-a",
      "200",
      "",
    );
    assert.deepStrictEqual(JSON.parse(document ?? "null"), { slug: "model-a", context_window: 200 });
  });

  it("预览在修改窗口后保留显式高精度比例", () => {
    const synchronized = synchronizeModelMetadataDocumentLimitsPreview(
      '{"slug":"model-a","context_window":272000,"auto_compact_token_limit":229376}',
      "model-a",
      "800000",
      "84.329412%",
    );
    assert.ok(synchronized);
    assert.strictEqual(synchronized?.preview.autoCompactPercent, "84%");
    assert.strictEqual(synchronized?.preview.autoCompactCalculationPercent, "84.329412%");
    assert.strictEqual(JSON.parse(synchronized?.document ?? "null").auto_compact_token_limit, 674635);
  });

  it("窗口清空时删除 context_window", () => {
    const document = synchronizeModelMetadataDocumentContextWindow(
      '{"slug":"model-a","context_window":100,"priority":1}',
      "model-a",
      "",
    );
    assert.deepStrictEqual(JSON.parse(document ?? "null"), { slug: "model-a", priority: 1 });
  });

  it("前端比例校验与 Rust 语法一致", () => {
    for (const value of ["90", "84.5%", "0.000001", "100%", ""]) {
      assert.strictEqual(isValidAutoCompactPercent(value), true, value);
    }
    for (const value of ["0", "101%", "90%%", ".5", "1.1234567"]) {
      assert.strictEqual(isValidAutoCompactPercent(value), false, value);
      assert.strictEqual(normalizeAutoCompactPercent(value), value);
    }
  });

  it("坏 metadata map 在 UI 侧不抛异常", () => {
    assert.deepStrictEqual(parseModelMetadataMap("not-json"), {});
  });
});
