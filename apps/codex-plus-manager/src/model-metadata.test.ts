import assert from "node:assert";
import { describe, it } from "node:test";
import { isValidAutoCompactPercent, normalizeAutoCompactPercent } from "./auto-compact.ts";
import {
  clearModelMetadataForSlug,
  parseModelMetadataDocument,
  remapModelMetadataSlugs,
  parseModelMetadataMap,
  renameModelMetadataSlug,
  replaceModelMetadataForSlug,
  retainModelMetadataForSlugs,
  serializeModelMetadataDocument,
  synchronizeModelMetadataDocumentLimits,
  synchronizeModelMetadataDocumentContextWindow,
  updateModelMetadataField,
} from "./model-metadata.ts";

describe("model metadata helpers", () => {
  it("空配置保持为没有高级覆盖", () => {
    assert.deepStrictEqual(parseModelMetadataMap(""), {});
    assert.strictEqual(updateModelMetadataField("", "gpt-5.5", "tool_mode", null), "");
  });

  it("按字段增量更新并在最后一个字段清除后恢复默认", () => {
    const initial = '{"deepseek-v4-pro":{"supports_search_tool":true,"priority":2}}';
    const updated = updateModelMetadataField(initial, "deepseek-v4-pro", "priority", 3);
    assert.deepStrictEqual(JSON.parse(updated), {
      "deepseek-v4-pro": { supports_search_tool: true, priority: 3 },
    });
    assert.strictEqual(
      updateModelMetadataField('{"deepseek-v4-pro":{"priority":2}}', "deepseek-v4-pro", "priority", null),
      "",
    );
  });

  it("解析 models 文档，提取窗口并保留嵌套和未知字段", () => {
    const result = parseModelMetadataDocument(JSON.stringify({
      models: [
        {
          slug: "deepseek-v4-flash",
          context_window: 1048576,
          max_context_window: 1048576,
          use_responses_lite: false,
          truncation_policy: { mode: "tokens", limit: 10000 },
          model_messages: { instructions_template: "hello" },
          vendor_extension: ["kept"],
        },
      ],
    }), "deepseek-v4-flash");

    assert.strictEqual(result.ok, true);
    if (!result.ok) return;
    assert.strictEqual(result.value.contextWindow, "1048576");
    assert.strictEqual(result.value.autoCompactPercent, "90%");
    assert.deepStrictEqual(result.value.metadata, {
      max_context_window: 1048576,
      use_responses_lite: false,
      truncation_policy: { mode: "tokens", limit: 10000 },
      model_messages: { instructions_template: "hello" },
      vendor_extension: ["kept"],
    });
  });

  it("支持常见 model.js JSON 包装但不执行 JavaScript", () => {
    const exported = parseModelMetadataDocument(
      'export default {"slug":"deepseek-v4-pro","priority":2};',
      "deepseek-v4-pro",
    );
    assert.strictEqual(exported.ok, true);

    const commonJs = parseModelMetadataDocument(
      'module.exports = {"models":[{"slug":"deepseek-v4-pro","priority":2}]};',
      "deepseek-v4-pro",
    );
    assert.strictEqual(commonJs.ok, true);

    const executable = parseModelMetadataDocument(
      'export default getModels();',
      "deepseek-v4-pro",
    );
    assert.strictEqual(executable.ok, false);
  });

  it("从多模型文档中只截取与当前 slug 完全匹配的模型", () => {
    const matched = parseModelMetadataDocument(
      JSON.stringify({
        models: [
          { slug: "deepseek-v4-flash", priority: 1, provider_marker: "flash" },
          { slug: "deepseek-v4-pro", priority: 2, provider_marker: "pro" },
        ],
      }),
      "deepseek-v4-flash",
    );
    assert.strictEqual(matched.ok, true);
    if (!matched.ok) return;
    assert.deepStrictEqual(matched.value.metadata, {
      priority: 1,
      provider_marker: "flash",
    });
  });

  it("当前 slug 不在多模型文档中时报告可用 slug", () => {
    const result = parseModelMetadataDocument(
      '{"models":[{"slug":"deepseek-v4-flash"},{"slug":"deepseek-v4-pro"}]}',
      "missing-model",
    );
    assert.strictEqual(result.ok, false);
    if (result.ok) return;
    assert.match(result.error, /deepseek-v4-flash/);
    assert.match(result.error, /deepseek-v4-pro/);
  });

  it("接受第三方模型明确声明的推理与 verbosity 能力", () => {
    const result = parseModelMetadataDocument(JSON.stringify({
      slug: "deepseek-v4-pro",
      support_verbosity: true,
      default_verbosity: "low",
      default_reasoning_level: "high",
      supported_reasoning_levels: [
        { effort: "low", description: "Fast" },
        { effort: "high", description: "Deep" },
        { effort: "max", description: "Maximum" },
      ],
    }), "deepseek-v4-pro");
    assert.strictEqual(result.ok, true);
  });

  it("接受模型自定义推理档位且不要求默认值必须出现在列表中", () => {
    const result = parseModelMetadataDocument(JSON.stringify({
      slug: "deepseek-v4-pro",
      default_reasoning_level: "provider-extreme",
      supported_reasoning_levels: [
        { effort: "low", description: "Fast" },
        { effort: "provider-ultra", description: "Provider-defined" },
      ],
    }), "deepseek-v4-pro");
    assert.strictEqual(result.ok, true);
  });

  it("允许供应商独立声明默认 verbosity", () => {
    const result = parseModelMetadataDocument(JSON.stringify({
      slug: "deepseek-v4-pro",
      default_verbosity: "provider-concise",
    }), "deepseek-v4-pro");
    assert.strictEqual(result.ok, true);
  });

  it("只拒绝会破坏 Codex 目录结构的推理字段", () => {
    const result = parseModelMetadataDocument(JSON.stringify({
      slug: "deepseek-v4-pro",
      supported_reasoning_levels: [{ effort: "low" }],
    }), "deepseek-v4-pro");
    assert.strictEqual(result.ok, false);
    if (result.ok) return;
    assert.match(result.error, /effort 和 description/);
  });

  it("导入内容整体替换当前模型配置，同时保留其他模型", () => {
    const replaced = replaceModelMetadataForSlug(
      '{"deepseek-v4-pro":{"priority":1,"custom":"remove"},"other":{"priority":9}}',
      "deepseek-v4-pro",
      { priority: 2, supports_search_tool: true },
    );
    assert.deepStrictEqual(JSON.parse(replaced), {
      other: { priority: 9 },
      "deepseek-v4-pro": { priority: 2, supports_search_tool: true },
    });
  });

  it("把已导入字段重新格式化成当前模型的 models.json", () => {
    const document = serializeModelMetadataDocument(
      "deepseek-v4-flash",
      { priority: 1, truncation_policy: { mode: "tokens", limit: 10000 } },
      "1M",
      "80",
    );
    assert.deepStrictEqual(JSON.parse(document), {
      models: [{
        slug: "deepseek-v4-flash",
        context_window: 1_000_000,
        auto_compact_token_limit: 800_000,
        priority: 1,
        truncation_policy: { mode: "tokens", limit: 10000 },
      }],
    });
  });

  it("导入 auto_compact_token_limit 时换算成百分比并从元数据中剥离", () => {
    const result = parseModelMetadataDocument(JSON.stringify({
      slug: "gpt-5.6-sol",
      context_window: 272000,
      auto_compact_token_limit: 229376,
      priority: 1,
    }), "gpt-5.6-sol");
    assert.strictEqual(result.ok, true);
    if (!result.ok) return;
    assert.strictEqual(result.value.autoCompactPercent, "84.329412%");
    assert.deepStrictEqual(result.value.metadata, { priority: 1 });
  });

  it("自动压缩百分比显示值会补上百分号", () => {
    assert.strictEqual(normalizeAutoCompactPercent("90"), "90%");
    assert.strictEqual(normalizeAutoCompactPercent("84.5%"), "84.5%");
    assert.strictEqual(normalizeAutoCompactPercent(""), "");
  });

  it("自动压缩百分比语法与 Rust 后端保持一致", () => {
    for (const valid of ["90", "84.5%", "0.000001", "100%", "1.123456%", ""]) {
      assert.strictEqual(isValidAutoCompactPercent(valid), true, valid);
    }
    for (const invalid of [".5", "1e2", "0x10", "+90", "1.1234567", "0", "101%", "90%%"]) {
      assert.strictEqual(isValidAutoCompactPercent(invalid), false, invalid);
      assert.strictEqual(normalizeAutoCompactPercent(invalid), invalid);
    }
  });

  it("修改百分比时同步更新当前模型的 auto_compact_token_limit", () => {
    const synchronized = synchronizeModelMetadataDocumentLimits(JSON.stringify({
      models: [
        { slug: "deepseek-v4-flash", context_window: 1_000_000, auto_compact_token_limit: 900_000 },
        { slug: "deepseek-v4-pro", context_window: 272_000, auto_compact_token_limit: 244_800 },
      ],
    }), "deepseek-v4-flash", "800K", "75%");
    assert.ok(synchronized);
    assert.deepStrictEqual(JSON.parse(synchronized ?? "null"), {
      models: [
        { slug: "deepseek-v4-flash", context_window: 800_000, auto_compact_token_limit: 600_000 },
        { slug: "deepseek-v4-pro", context_window: 272_000, auto_compact_token_limit: 244_800 },
      ],
    });
  });

  it("手动窗口变化时只同步当前模型的 context_window", () => {
    const synchronized = synchronizeModelMetadataDocumentContextWindow(JSON.stringify({
      models: [
        { slug: "deepseek-v4-flash", context_window: 272000, priority: 1 },
        { slug: "deepseek-v4-pro", context_window: 1048576, priority: 2 },
      ],
    }), "deepseek-v4-flash", "1M");
    assert.ok(synchronized);
    assert.deepStrictEqual(JSON.parse(synchronized), {
      models: [
        { slug: "deepseek-v4-flash", context_window: 1_000_000, priority: 1 },
        { slug: "deepseek-v4-pro", context_window: 1_048_576, priority: 2 },
      ],
    });
  });

  it("清空手动窗口时从当前模型文本移除 context_window", () => {
    const synchronized = synchronizeModelMetadataDocumentContextWindow(
      '{"slug":"deepseek-v4-flash","context_window":1048576,"priority":1}',
      "deepseek-v4-flash",
      "",
    );
    assert.deepStrictEqual(JSON.parse(synchronized ?? "null"), {
      slug: "deepseek-v4-flash",
      priority: 1,
    });
  });

  it("原样保留官方及未知字段并静默补充 reasoning 兼容字段", () => {
    const result = parseModelMetadataDocument(JSON.stringify({
      slug: "deepseek-v4-pro",
      prefer_websockets: false,
      reasoning_summary_format: "experimental",
      supports_reasoning_summaries: true,
      base_instructions: "same",
      model_messages: { instructions_template: "same" },
    }), "deepseek-v4-pro");
    assert.strictEqual(result.ok, true);
    if (!result.ok) return;
    assert.strictEqual(result.value.metadata.prefer_websockets, false);
    assert.strictEqual(result.value.metadata.reasoning_summary_format, "experimental");
    assert.strictEqual(result.value.metadata.supports_reasoning_summaries, true);
    assert.strictEqual(result.value.metadata.supports_reasoning_summary_parameter, true);
    assert.strictEqual(Object.hasOwn(result.value, "diagnostics"), false);
  });

  it("清除时只移除当前模型的高级覆盖", () => {
    assert.strictEqual(
      clearModelMetadataForSlug('{"a":{"priority":1},"b":{"priority":2}}', "a"),
      '{"b":{"priority":2}}',
    );
  });

  it("模型改名时迁移元数据并以当前模型配置替换旧目标", () => {
    assert.strictEqual(
      renameModelMetadataSlug('{"a":{"priority":1},"b":{"priority":2}}', "a", "c"),
      '{"b":{"priority":2},"c":{"priority":1}}',
    );
    assert.strictEqual(
      renameModelMetadataSlug('{"a":{"priority":1},"b":{"priority":2}}', "a", "b"),
      '{"b":{"priority":1}}',
    );
  });

  it("冲突解除后按原始快照批量迁移链式改名", () => {
    assert.strictEqual(
      remapModelMetadataSlugs(
        '{"a":{"priority":1},"b":{"priority":2}}',
        [
          { previousSlug: "a", nextSlug: "b" },
          { previousSlug: "b", nextSlug: "c" },
        ],
      ),
      '{"b":{"priority":1},"c":{"priority":2}}',
    );
  });

  it("拆分历史重复 slug 时为新名称复制并保留共享元数据", () => {
    assert.strictEqual(
      remapModelMetadataSlugs(
        '{"a":{"priority":1}}',
        [
          { previousSlug: "a", nextSlug: "b" },
          { previousSlug: "a", nextSlug: "a" },
        ],
      ),
      '{"a":{"priority":1},"b":{"priority":1}}',
    );
  });

  it("保存前只保留仍在模型列表中的元数据", () => {
    assert.strictEqual(
      retainModelMetadataForSlugs('{"a":{"priority":1},"deleted":{"priority":2}}', ["a"]),
      '{"a":{"priority":1}}',
    );
  });
});
