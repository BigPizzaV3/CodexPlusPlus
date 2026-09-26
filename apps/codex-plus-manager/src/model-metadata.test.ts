import assert from "node:assert";
import { describe, it } from "node:test";
import { isValidAutoCompactPercent, normalizeAutoCompactEditing, normalizeAutoCompactPercent } from "./auto-compact.ts";
import {
  builtinEntryToImportDocument,
  builtinMetadataQueryState,
  builtinRowBackfillValue,
  cancelActiveImportDraft,
  createActiveImportDraft,
  rematchActiveImportDraft,
  updateActiveImportDraft,
  clearModelMetadataForSlug,
  importDocumentSyncPatch,
  importPanelControls,
  importSaveDecision,
  metadataMatchesBuiltin,
  metadataSourceTags,
  modelMetadataKey,
  parseModelRowName,
  modelSlugFromRowName,
  parseModelMetadataDocument,
  parseModelMetadataMap,
  remapModelMetadataSlugs,
  replaceModelMetadataForSlug,
  resolveModelMetadataRowKey,
  retainModelMetadataForSlugs,
  serializeModelMetadataDocument,
  suffixWindowString,
  suffixWindowTokens,
  synchronizeModelMetadataDocumentContextWindow,
  synchronizeModelMetadataDocumentLimits,
  synchronizeModelMetadataDocumentLimitsPreview,
} from "./model-metadata.ts";

describe("model metadata helpers", () => {
  it("metadata map keys normalize suffixes and casing across every mutation", () => {
    const initial = JSON.stringify({ "DeepSeek-V4-Pro[1M]": { temperature: 0.2 } });
    const normalized = parseModelMetadataMap(initial);
    assert.deepStrictEqual(normalized, { "deepseek-v4-pro": { temperature: 0.2 } });
    assert.strictEqual(modelMetadataKey(" DeepSeek-V4-Pro[1M] "), "deepseek-v4-pro");

    const replaced = replaceModelMetadataForSlug(initial, "deepseek-v4-pro", { top_p: 0.8 });
    assert.deepStrictEqual(parseModelMetadataMap(replaced), { "deepseek-v4-pro": { top_p: 0.8 } });
    assert.strictEqual(clearModelMetadataForSlug(replaced, "DEEPSEEK-V4-PRO[512K]"), "");
    assert.deepStrictEqual(
      parseModelMetadataMap(remapModelMetadataSlugs(initial, [{
        previousSlug: "DeepSeek-V4-Pro[1M]",
        nextSlug: "DEEPSEEK-V4-PRO-NEW[2M]",
      }])),
      { "deepseek-v4-pro-new": { temperature: 0.2 } },
    );
    assert.strictEqual(retainModelMetadataForSlugs(initial, ["deepseek-v4-pro[1M]"]),
      JSON.stringify({ "deepseek-v4-pro": { temperature: 0.2 } }));
  });

  it("自动压缩编辑把数字保持在百分号前并允许清空", () => {
    assert.strictEqual(normalizeAutoCompactEditing("90%5", "90%"), "905%");
    assert.strictEqual(normalizeAutoCompactEditing("9%", "90%"), "9");
    assert.strictEqual(normalizeAutoCompactEditing("90", "90%"), "90");
    assert.strictEqual(normalizeAutoCompactEditing("", "90%"), "");
  });

  it("解析单模型并保留供应商字段", () => {
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
    // 窗口字段由「上下文窗口」列统一管辖，不进 metadata map（issue #2191）。
    assert.deepStrictEqual(result.value.metadata, {
      priority: 2,
      truncation_policy: { mode: "tokens", limit: 10000 },
      vendor_extension: ["kept"],
    });
    assert.deepStrictEqual(result.value.ignoredFields, []);
  });

  it("max_context_window 优先于 context_window", () => {
    const result = parseModelMetadataDocument(JSON.stringify({
      slug: "model-a",
      context_window: 272_000,
      max_context_window: 1_000_000,
    }), "model-a");
    assert.strictEqual(result.ok, true);
    if (!result.ok) return;
    assert.strictEqual(result.value.contextWindow, "1000000");
    assert.deepStrictEqual(result.value.metadata, {});
  });

  it("仅 max_context_window 的文档也能提取窗口", () => {
    const result = parseModelMetadataDocument(JSON.stringify({
      slug: "model-a",
      max_context_window: 600_000,
    }), "model-a");
    assert.strictEqual(result.ok, true);
    if (!result.ok) return;
    assert.strictEqual(result.value.contextWindow, "600000");
    assert.deepStrictEqual(result.value.metadata, {});
  });

  it("仅 max_context_window 时压缩百分比按该窗口计算", () => {
    const result = parseModelMetadataDocument(JSON.stringify({
      slug: "model-a",
      max_context_window: 1_000_000,
      auto_compact_token_limit: 800_000,
    }), "model-a");
    assert.strictEqual(result.ok, true);
    if (!result.ok) return;
    assert.strictEqual(result.value.contextWindow, "1000000");
    assert.strictEqual(result.value.autoCompactPercent, "80%");
    assert.deepStrictEqual(result.value.metadata, {});
  });

  it("窗口字段为非法值时报出对应字段名", () => {
    const result = parseModelMetadataDocument(
      '{"slug":"model-a","context_window":1,"max_context_window":-5}',
      "model-a",
    );
    assert.strictEqual(result.ok, false);
    if (result.ok) return;
    assert.match(result.error, /max_context_window/);
  });

  it("编辑窗口时同步文档里的 max_context_window", () => {
    const synchronized = synchronizeModelMetadataDocumentLimits(
      '{"slug":"model-a","context_window":272000,"max_context_window":1000000,"vendor":true}',
      "model-a",
      "600000",
      "",
    );
    assert.deepStrictEqual(JSON.parse(synchronized ?? "null"), {
      slug: "model-a",
      context_window: 600_000,
      max_context_window: 600_000,
      vendor: true,
    });
  });

  it("编辑窗口清空时文档里的 max_context_window 同步置 null", () => {
    const synchronized = synchronizeModelMetadataDocumentContextWindow(
      '{"slug":"model-a","context_window":100,"max_context_window":100}',
      "model-a",
      "",
    );
    assert.deepStrictEqual(JSON.parse(synchronized ?? "null"), {
      slug: "model-a",
      context_window: null,
      max_context_window: null,
    });
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
      "model-a": { supports_search_tool: true, priority: 2 },
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

  it("保留 Codex++ 已填写的显示名称，其他 metadata 采用最新导入值", () => {
    const replaced = replaceModelMetadataForSlug(
      '{"model-a":{"display_name":"我的模型名","vendor":"old"}}',
      "model-a",
      { display_name: "供应商模型名", vendor: "new", supports_search_tool: true },
    );
    assert.deepStrictEqual(JSON.parse(replaced), {
      "model-a": {
        display_name: "我的模型名",
        vendor: "new",
        supports_search_tool: true,
      },
    });
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

  it("空比例保持 Codex 默认行为并保留字段位置", () => {
    const document = synchronizeModelMetadataDocumentLimits(
      '{"slug":"model-a","context_window":100,"auto_compact_token_limit":90}',
      "model-a",
      "200",
      "",
    );
    assert.deepStrictEqual(JSON.parse(document ?? "null"), {
      slug: "model-a",
      context_window: 200,
      auto_compact_token_limit: null,
    });
  });

  it("自动压缩清空后重新输入不改变 JSON 字段顺序", () => {
    const source = '{"slug":"model-a","context_window":100,"auto_compact_token_limit":90,"vendor":true}';
    const cleared = synchronizeModelMetadataDocumentLimits(source, "model-a", "100", "");
    assert.ok(cleared);
    const refilled = synchronizeModelMetadataDocumentLimits(cleared!, "model-a", "100", "80%");
    assert.ok(refilled);
    assert.deepStrictEqual(Object.keys(JSON.parse(refilled!)), [
      "slug",
      "context_window",
      "auto_compact_token_limit",
      "vendor",
    ]);
    assert.strictEqual(JSON.parse(refilled!).auto_compact_token_limit, 80);
  });

  it("压缩百分比保存再打开时始终把 context_window 放在前面", () => {
    const source = '{"slug":"model-a","vendor":true,"auto_compact_token_limit":90,"context_window":100}';
    const saved = synchronizeModelMetadataDocumentLimits(source, "model-a", "200", "80%");
    assert.ok(saved);
    assert.deepStrictEqual(Object.keys(JSON.parse(saved!)), [
      "slug",
      "context_window",
      "auto_compact_token_limit",
      "vendor",
    ]);
    const reopened = parseModelMetadataDocument(saved!, "model-a");
    assert.strictEqual(reopened.ok, true);
    if (reopened.ok) assert.strictEqual(reopened.value.contextWindow, "200");
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

  it("窗口清空时保留 context_window 字段位置", () => {
    const document = synchronizeModelMetadataDocumentContextWindow(
      '{"slug":"model-a","context_window":100,"priority":1}',
      "model-a",
      "",
    );
    assert.deepStrictEqual(JSON.parse(document ?? "null"), { slug: "model-a", context_window: null, priority: 1 });
  });

  it("context_window 为 null 时按未设置处理", () => {
    const result = parseModelMetadataDocument(
      '{"slug":"model-a","context_window":null,"vendor":true}',
      "model-a",
    );
    assert.strictEqual(result.ok, true);
    if (result.ok) assert.strictEqual(result.value.contextWindow, null);
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

  it("导入时 slug 匹配忽略大小写", () => {
    // 供应商 Model Key 大小写不统一（GLM-5.3-FlashX），界面填大写也应匹配。
    const result = parseModelMetadataDocument(
      JSON.stringify({
        models: [{ slug: "glm-5.3-flashx", context_window: 1_048_576, max_context_window: 1_048_576 }],
      }),
      "GLM-5.3-FlashX",
    );
    assert.strictEqual(result.ok, true);
    if (!result.ok) return;
    assert.strictEqual(result.value.slug, "GLM-5.3-FlashX");
    assert.strictEqual(result.value.contextWindow, "1048576");
  });

  it("同步文档窗口时 slug 匹配也忽略大小写", () => {
    const synchronized = synchronizeModelMetadataDocumentLimits(
      '{"slug":"glm-5.3-flashx","context_window":262144}',
      "GLM-5.3-FlashX",
      "1M",
      "80%",
    );
    assert.ok(synchronized);
    const parsed = JSON.parse(synchronized!);
    assert.strictEqual(parsed.slug, "glm-5.3-flashx");
    assert.strictEqual(parsed.context_window, 1_000_000);
    assert.strictEqual(parsed.auto_compact_token_limit, 800_000);
  });

  it("文档内多个大小写变体 slug 视为歧义", () => {
    const result = parseModelMetadataDocument(
      JSON.stringify({ models: [{ slug: "model-a" }, { slug: "MODEL-A" }] }),
      "model-a",
    );
    assert.strictEqual(result.ok, false);
    if (result.ok) return;
    assert.match(result.error, /多个/);
  });

  it("内置条目转导入文档可往返解析且窗口正确", () => {
    // 官方 gpt 系：预填 codex 默认运行窗口（272000），不是能力上限 872000——
    // 用户导入 872000 会把运行窗口改成上限，改变默认行为
    const gpt = builtinEntryToImportDocument({
      slug: "gpt-5.6-sol",
      display_name: "GPT-5.6-Sol",
      context_window: 272_000,
      max_context_window: 872_000,
    });
    assert.match(gpt, /"context_window": 272000/);
    assert.doesNotMatch(gpt, /872000/);
    const parsedGpt = parseModelMetadataDocument(gpt, "gpt-5.6-sol");
    assert.strictEqual(parsedGpt.ok, true);
    if (parsedGpt.ok) assert.strictEqual(parsedGpt.value.contextWindow, "272000");

    // 供应商场景：ctx 与 max 同值时不受影响；托管字段不进 metadata
    const document = builtinEntryToImportDocument({
      slug: "kimi-k3",
      display_name: "Kimi K3",
      context_window: 1_048_576,
      max_context_window: 1_048_576,
      auto_compact_token_limit: 100_000,
      supported_reasoning_levels: [{ effort: "high", description: "Enhanced" }],
    });
    const parsed = parseModelMetadataDocument(document, "kimi-k3");
    assert.strictEqual(parsed.ok, true);
    if (!parsed.ok) return;
    assert.strictEqual(parsed.value.contextWindow, "1048576");
    assert.deepStrictEqual(parsed.value.metadata, {
      display_name: "Kimi K3",
      supported_reasoning_levels: [{ effort: "high", description: "Enhanced" }],
    });
  });

  it("预填文档的 slug 与用户输入的模型名拼写一致", () => {
    // 用户行名拼写（如 DeepSeek-V4-Flash）与资产 slug（deepseek-v4-flash）
    // 大小写不同时，预填文档与解析结果都应保留用户的拼写；
    // 保存到 metadata map 的 key 由调用方用行名，链路全程与行名一致。
    const entry = { slug: "deepseek-v4-flash", display_name: "DeepSeek-V4-Flash", context_window: 1_048_576 };
    const document = builtinEntryToImportDocument(entry, "DeepSeek-V4-Flash");
    assert.match(document, /"slug": "DeepSeek-V4-Flash"/);
    const parsed = parseModelMetadataDocument(document, "DeepSeek-V4-Flash");
    assert.strictEqual(parsed.ok, true);
    if (parsed.ok) assert.strictEqual(parsed.value.slug, "DeepSeek-V4-Flash");

    // 带后缀的行名：剥成规范 slug 后同样保留用户拼写
    const suffixed = builtinEntryToImportDocument(entry, "DeepSeek-V4-Flash[1M]");
    assert.match(suffixed, /"slug": "DeepSeek-V4-Flash"/);

    // 不传行名时保持资产行为（bundled 展示等场景）
    const fallbackDocument = builtinEntryToImportDocument(entry);
    assert.match(fallbackDocument, /"slug": "deepseek-v4-flash"/);
  });

  it("元数据来源标签覆盖全部用户场景", () => {
    const match = { matched: true, source: "GLM", entry: { slug: "glm-5.3" } };
    const fallback = { matched: false, fallback: { slug: "gpt-5.5", context_window: 272_000 } };
    // 标签直接下发纯文本（不再过 t()/tf()），原样透传即可
    const render = (tags: ReturnType<typeof metadataSourceTags>) => tags.map((tag) => ({
      kind: tag.kind,
      tone: tag.tone,
      text: tag.text,
      title: tag.title,
    }));

    // 纯命中（打开导入区，内置预填）：只显示匹配标签
    assert.deepStrictEqual(
      render(metadataSourceTags({ slug: "glm-5.3", imported: false, builtinMatch: match, builtinIndexSlug: { source: "GLM" } })),
      [{ kind: "match", text: "匹配：GLM", title: "内置元数据：GLM", tone: "builtin" }],
    );
    // 命中 + 自定义（保存过/老版本已配置）：匹配与自定义并列
    assert.deepStrictEqual(
      render(metadataSourceTags({ slug: "glm-5.3", imported: true, builtinMatch: match, builtinIndexSlug: { source: "GLM" } })).map(t => t.text),
      ["匹配：GLM", "自定义"],
    );
    // 回退态（无内置）：回退标签
    assert.deepStrictEqual(
      render(metadataSourceTags({ slug: "nope", imported: false, builtinMatch: fallback, builtinIndexSlug: undefined })).map(t => t.text),
      ["回退：gpt-5.5"],
    );
    // 回退 + 自定义：自定义已覆盖，不再显示"回退"（避免误导为还在用 gpt-5.5）
    assert.deepStrictEqual(
      render(metadataSourceTags({ slug: "nope", imported: true, builtinMatch: fallback, builtinIndexSlug: undefined })).map(t => t.text),
      ["自定义"],
    );
    // match 数据未返回时用索引兜底（行级渲染路径）
    assert.deepStrictEqual(
      render(metadataSourceTags({ slug: "glm-5.3", imported: false, builtinMatch: null, builtinIndexSlug: { source: "GLM" } })).map(t => t.text),
      ["匹配：GLM"],
    );
    // 全无：回退（fallbackSlug 由后端实时下发，不写死）
    assert.deepStrictEqual(
      render(metadataSourceTags({ slug: "nope", imported: false, builtinMatch: null, builtinIndexSlug: undefined, fallbackSlug: "gpt-5.5" })).map(t => t.text),
      ["回退：gpt-5.5"],
    );
    // 自定义 fallback 标签文案可定制（fallback 源变化时）
    assert.deepStrictEqual(
      render(metadataSourceTags({ slug: "nope", imported: false, builtinMatch: null, builtinIndexSlug: undefined, fallbackSlug: "gpt-5.4" })).map(t => t.text),
      ["回退：gpt-5.4"],
    );
    // match 未命中但 entry 缺失时不应产生匹配标签（脏数据防御）
    assert.deepStrictEqual(
      render(metadataSourceTags({ slug: "glm-5.3", imported: false, builtinMatch: { matched: false, source: "GLM" }, builtinIndexSlug: undefined, fallbackSlug: "gpt-5.5" })).map(t => t.text),
      ["回退：gpt-5.5"],
    );
  });

  it("导入文档写回模型行的规则覆盖", () => {
    // 窗口不同才写；相同不写（避免多余 state 更新）
    assert.deepStrictEqual(
      importDocumentSyncPatch({ window: "", autoCompact: "" }, { contextWindow: "500000", autoCompactPercent: null }),
      { window: "500000" },
    );
    assert.deepStrictEqual(
      importDocumentSyncPatch({ window: "500000", autoCompact: "" }, { contextWindow: "500000", autoCompactPercent: null }),
      {},
    );
    // 压缩比不同才写
    assert.deepStrictEqual(
      importDocumentSyncPatch({ window: "500000", autoCompact: "90%" }, { contextWindow: "500000", autoCompactPercent: "80%" }),
      { autoCompact: "80%" },
    );
    // JSON 未声明压缩比（null）：不动行里的值
    assert.deepStrictEqual(
      importDocumentSyncPatch({ window: "500000", autoCompact: "90%" }, { contextWindow: "600000", autoCompactPercent: null }),
      { window: "600000" },
    );
    // 空预览（粘贴清空/粘贴失败）：整体 no-op
    assert.deepStrictEqual(
      importDocumentSyncPatch({ window: "500000", autoCompact: "90%" }, { contextWindow: null, autoCompactPercent: null }),
      {},
    );
  });

  it("保存按钮判定：保存匹配到的内置数据不该产生自定义覆盖", () => {
    // 关键回归：面板内容就是内置条目的复刻时，保存的目标态是「用内置」，
    // 而不是把内置复制成一份自定义配置。
    const builtin = { display_name: "Kimi K3", prefer_websockets: false };

    // 1) 内置预填、未编辑、当前无自定义 → 不写自定义；保存键仍可点（=确认用内置并关面板）
    assert.deepStrictEqual(
      importSaveDecision({ parseOk: true, documentBlank: false, imported: false, matchesBuiltin: true }),
      { needsSave: false, effect: "none", label: "保存此模型", title: "内容与内置元数据一致，保存后继续使用内置元数据" },
    );

    // 2) 内置预填、未编辑、已有自定义 → 内容是内置，保存 = 放弃自定义
    assert.deepStrictEqual(
      importSaveDecision({ parseOk: true, documentBlank: false, imported: true, matchesBuiltin: true }),
      { needsSave: true, effect: "builtin", label: "保存此模型", title: "内容与内置元数据一致，保存后使用内置元数据" },
    );

    // 3) 真的改过内容（与内置不等价）+ 无自定义 → 写成自定义覆盖
    assert.deepStrictEqual(
      importSaveDecision({ parseOk: true, documentBlank: false, imported: false, matchesBuiltin: false }),
      { needsSave: true, effect: "custom", label: "保存为自定义配置", title: "当前为内置元数据预览的修改版；保存后将成为该模型的自定义配置，生成时覆盖内置" },
    );
    // 4) 改过内容 + 已有自定义 → 更新覆盖
    assert.deepStrictEqual(
      importSaveDecision({ parseOk: true, documentBlank: false, imported: true, matchesBuiltin: false }),
      { needsSave: true, effect: "custom", label: "更新此模型配置", title: "保存当前内容为该模型的自定义配置" },
    );

    // 5) 空文本 + 已有自定义 → 放弃自定义（与 2 等效，都是恢复内置）
    assert.deepStrictEqual(
      importSaveDecision({ parseOk: true, documentBlank: true, imported: true, matchesBuiltin: false }),
      { needsSave: true, effect: "builtin", label: "保存此模型", title: "保存后清除该模型的自定义配置，生成时回退默认模板" },
    );
    // 6) 空文本 + 无自定义 → 没有可保存的内容
    assert.deepStrictEqual(
      importSaveDecision({ parseOk: true, documentBlank: true, imported: false, matchesBuiltin: false }),
      { needsSave: false, effect: "none", label: "保存此模型", title: "没有可保存的内容" },
    );

    // 7) 解析失败：一律不可点（红条已说明原因），不能把坏 JSON 存进去
    assert.deepStrictEqual(
      importSaveDecision({ parseOk: false, documentBlank: false, imported: false, matchesBuiltin: false }),
      { needsSave: false, effect: "none", label: "保存此模型", title: "JSON 无法解析，修复后即可保存" },
    );

    // metadataMatchesBuiltin：字段顺序不影响相等；基线与面板同走 parse 管道时
    // 两侧形状一致（含窗口字段），全字段比较（见「全字段等价」用例）
    assert.strictEqual(metadataMatchesBuiltin(builtin, { ...builtin }), true);
    assert.strictEqual(metadataMatchesBuiltin(builtin, { prefer_websockets: false, display_name: "Kimi K3" }), true);
    assert.strictEqual(metadataMatchesBuiltin(builtin, { display_name: "Kimi K3", prefer_websockets: true }), false);
    assert.strictEqual(metadataMatchesBuiltin(builtin, { display_name: "Kimi K3" }), false);
    assert.strictEqual(metadataMatchesBuiltin(null, builtin), false);
    assert.strictEqual(metadataMatchesBuiltin(builtin, null), false);
  });

  it("导入区按钮组恒定可用性判定（不再随状态出现/消失）", () => {
    const slug = "kimi-k3";
    const builtinDoc = builtinEntryToImportDocument({ slug, context_window: 1_048_576 });
    const control = (patch: Partial<Parameters<typeof importPanelControls>[0]>) => importPanelControls({
      slug,
      document: builtinDoc,
      imported: false,
      parseOk: true,
      matched: true,
      matchesBuiltin: true,
      ...patch,
    });

    // 四个按钮永远都在，只是能不能点——这是「点一个键不少一个键」的前提
    const base = control({});
    for (const key of ["rematch", "clear", "cancel", "save"] as const) {
      assert.ok(base[key], `缺少按钮判定：${key}`);
      assert.ok(typeof base[key].disabled === "boolean", `${key} 未给出 disabled`);
      assert.ok(typeof base[key].title === "string" && base[key].title.length > 0, `${key} 未给出 title`);
    }

    // 「重新匹配后保存」的最常见路径：内容是内置复刻、无自定义 →
    // 保存键保持可点（点=确认用内置并关闭面板，不写自定义）。
    // 清除恒可点（2026-09-26 语义：只清空面板文档，不摘配置、不关面板）
    assert.strictEqual(base.rematch.disabled, false);
    assert.strictEqual(base.clear.disabled, false);
    assert.strictEqual(base.clear.title, "清空面板内容（不影响已保存的配置）");
    assert.strictEqual(base.cancel.disabled, false);
    assert.strictEqual(base.save.disabled, false);
    assert.strictEqual(base.save.title, "内容与内置元数据一致，保存后继续使用内置元数据");

    // 已有自定义 + 内容是内置复刻：保存变「恢复内置」
    const withCustom = control({ imported: true });
    assert.strictEqual(withCustom.save.disabled, false);
    assert.strictEqual(withCustom.save.label, "保存此模型");

    // 内容真的改过（与内置不等价）：保存变可点，文案「保存为自定义配置」
    const edited = control({ matchesBuiltin: false });
    assert.strictEqual(edited.save.disabled, false);
    assert.strictEqual(edited.save.label, "保存为自定义配置");

    // 改了模型名导致未命中内置：重新匹配置灰，但按钮本身不消失
    const unmatched = control({ matched: false });
    assert.strictEqual(unmatched.rematch.disabled, true);
    assert.match(unmatched.rematch.title, /没有内置元数据可匹配/);
    assert.strictEqual(unmatched.rematch.title.length > 0, true);

    // 模型名为空：重新匹配置灰并说明原因
    assert.strictEqual(control({ slug: "" }).rematch.disabled, true);
    assert.match(control({ slug: "" }).rematch.title, /请先填写模型名称/);

    // 解析失败：保存置灰，title 指向 JSON 问题而不是「无需保存」
    const broken = control({ parseOk: false });
    assert.strictEqual(broken.save.disabled, true);
    assert.match(broken.save.title, /JSON 无法解析/);

    // 文本框被清空且已有自定义：保存变「恢复内置」
    const cleared = control({ document: "", imported: true, matchesBuiltin: false });
    assert.strictEqual(cleared.save.label, "保存此模型");
    assert.strictEqual(cleared.save.disabled, false);
    // 清空且没有自定义：保存键仍可点（仅解析失败才置灰），点了只关面板不写
    assert.strictEqual(control({ document: "", matchesBuiltin: false }).save.disabled, false);
  });

  // ── [1M] 后缀检测与适配（issue #2279）────────────────────────────────
  // 模型行名是用户原样输入的字符串，[1M] 后缀的含义就是「该模型上下文窗口」。
  // 导入面板/标签/实时同步都在 slug 层面工作，必须先剥后缀再比较。
  it("suffixWindowTokens 解析 [1M]/[256K]/[123] 并拒绝非法写法", () => {
    assert.strictEqual(suffixWindowTokens("1M"), 1_000_000);
    assert.strictEqual(suffixWindowTokens("256K"), 256_000);
    assert.strictEqual(suffixWindowTokens("128k"), 128_000);
    assert.strictEqual(suffixWindowTokens("123"), 123);
    // 非法后缀一律 null：不能把非法值悄悄当成 0 或 NaN 传下去
    assert.strictEqual(suffixWindowTokens(""), null);
    assert.strictEqual(suffixWindowTokens("0K"), null);
    assert.strictEqual(suffixWindowTokens("abc"), null);
    assert.strictEqual(suffixWindowTokens("1.5M"), null);
    assert.strictEqual(suffixWindowTokens("-1M"), null);
  });

  it("modelSlugFromRowName 剥掉合法后缀、保留非法后缀原文", () => {
    assert.strictEqual(modelSlugFromRowName("deepseek-v4-pro[1M]"), "deepseek-v4-pro");
    assert.strictEqual(modelSlugFromRowName("  glm-5.3[256K]  "), "glm-5.3");
    assert.strictEqual(modelSlugFromRowName("GPT-5.6-SOL[1M]"), "GPT-5.6-SOL");
    // 无后缀：trim 后原样返回
    assert.strictEqual(modelSlugFromRowName("  kimi-k3 "), "kimi-k3");
    // 非法后缀不当成后缀处理，整串当 slug（与 Rust parse_model_suffix 一致）
    assert.strictEqual(modelSlugFromRowName("foo[bar]"), "foo[bar]");
    assert.strictEqual(modelSlugFromRowName("foo[1M"), "foo[1M");
    assert.strictEqual(modelSlugFromRowName("foo[0K]"), "foo[0K]");
    // 空串
    assert.strictEqual(modelSlugFromRowName(""), "");
  });

  it("parseModelRowName 一次返回原名、规范 slug、窗口和 map key", () => {
    assert.deepStrictEqual(parseModelRowName(" DeepSeek-V4-Pro[1M] "), {
      rawName: " DeepSeek-V4-Pro[1M] ",
      canonicalSlug: "DeepSeek-V4-Pro",
      suffixWindow: "1000000",
      key: "deepseek-v4-pro",
    });
    assert.deepStrictEqual(parseModelRowName("foo[bar]"), {
      rawName: "foo[bar]",
      canonicalSlug: "foo[bar]",
      suffixWindow: null,
      key: "foo[bar]",
    });
  });

  it("suffixWindowString 给出后缀对应的窗口字符串", () => {
    assert.strictEqual(suffixWindowString("deepseek-v4-pro[1M]"), "1000000");
    assert.strictEqual(suffixWindowString("glm-5.3[256K]"), "256000");
    assert.strictEqual(suffixWindowString("kimi-k3"), null);
    assert.strictEqual(suffixWindowString("foo[bar]"), null);
  });

  it("内置预填 + parse 全链路支持带 [1M] 后缀的模型行名", () => {
    // 复现 issue #2279：行名带后缀时，内置文档写规范 slug，而 parse 的 targetSlug
    // 曾是带后缀行名 → 必然报「文档中没有找到当前模型 slug」。
    const entry = {
      slug: "deepseek-v4-pro",
      display_name: "DeepSeek-V4-Pro",
      context_window: 1_048_576,
      max_context_window: 1_048_576,
      apply_patch_tool_type: "freeform",
    };
    const document = builtinEntryToImportDocument(entry);
    const parsed = parseModelMetadataDocument(document, "deepseek-v4-pro[1M]");
    assert.ok(parsed.ok, "带 [1M] 后缀的行名应能匹配到内置文档");
    assert.strictEqual(parsed.value.slug, "deepseek-v4-pro[1M]");
    assert.strictEqual(parsed.value.metadata.display_name, "DeepSeek-V4-Pro");
    assert.strictEqual(parsed.value.contextWindow, "1048576");

    // 大小写 + 后缀组合也要命中
    const sol = parseModelMetadataDocument(
      builtinEntryToImportDocument({ slug: "gpt-5.6-sol", display_name: "GPT-5.6-Sol" }),
      "GPT-5.6-SOL[1M]",
    );
    assert.ok(sol.ok, "大小写变体 + 后缀应命中");
  });

  it("实时同步（窗口/压缩）同样按规范 slug 匹配带后缀行名", () => {
    const entry = { slug: "glm-5.3", display_name: "glm-5.3" };
    const document = builtinEntryToImportDocument(entry);
    const synced = synchronizeModelMetadataDocumentContextWindow(document, "glm-5.3[1M]", "512000");
    assert.ok(synced, "窗口同步应命中带后缀的行名");
    const doc = JSON.parse(synced!);
    assert.strictEqual(doc.models[0].context_window, 512000);

    const limits = synchronizeModelMetadataDocumentLimits(document, "glm-5.3[1M]", "512000", "80%");
    assert.ok(limits, "压缩同步应命中带后缀的行名");
    assert.strictEqual(JSON.parse(limits!).models[0].auto_compact_token_limit, 409600);
  });

  it("metadataMatchesBuiltin 全字段等价：窗口/压缩偏离内置值同样算编辑", () => {
    const base = { display_name: "Kimi K3", prefer_websockets: false, context_window: 1_048_576 };
    // 内容一致（键顺序无关）→ 内置复刻
    assert.strictEqual(metadataMatchesBuiltin(
      base,
      { prefer_websockets: false, context_window: 1_048_576, display_name: "Kimi K3" },
    ), true);
    // 修复核心回归：仅改窗口/压缩也是用户编辑 → 自定义（旧口径只比事实字段，误判 true）
    assert.strictEqual(metadataMatchesBuiltin({ ...base, context_window: 999_999 }, base), false);
    assert.strictEqual(metadataMatchesBuiltin({ ...base, max_context_window: 872_000 }, base), false);
    assert.strictEqual(metadataMatchesBuiltin({ ...base, auto_compact_token_limit: 943_718 }, base), false);
    // 事实字段不同 → 不等价
    assert.strictEqual(metadataMatchesBuiltin({ ...base, prefer_websockets: true }, base), false);
    assert.strictEqual(metadataMatchesBuiltin({ ...base, apply_patch_tool_type: "freeform" }, base), false);
    // 空对象不是合法基线：否则「纯模板条目」会与任意窗口-only 编辑判等
    assert.strictEqual(metadataMatchesBuiltin({}, base), false);
    assert.strictEqual(metadataMatchesBuiltin(base, {}), false);
    assert.strictEqual(metadataMatchesBuiltin({}, {}), false);
    // null / undefined 任一侧 → false
    assert.strictEqual(metadataMatchesBuiltin(null, base), false);
    assert.strictEqual(metadataMatchesBuiltin(base, null), false);
  });

  it("仅改窗口的端到端：判定落自定义，保存写入不静默删除", () => {
    // 基线 = 内置条目经同一 parse 管道的 documentEntry（含窗口字段）
    const entry = { slug: "glm-5.3", display_name: "glm-5.3", context_window: 1_048_576 };
    const document = builtinEntryToImportDocument(entry, "glm-5.3");
    const parsed = parseModelMetadataDocument(document, "glm-5.3");
    assert.ok(parsed.ok);
    const baseline = parsed.value.documentEntry;
    // 未编辑 / 重新匹配重建（同一 entry、同一行名）→ 全字段相等 → 仍判内置
    assert.strictEqual(metadataMatchesBuiltin(baseline, baseline), true);
    const retrimmed = parseModelMetadataDocument(
      builtinEntryToImportDocument(entry, "glm-5.3"), "glm-5.3",
    );
    assert.ok(retrimmed.ok);
    assert.strictEqual(metadataMatchesBuiltin(retrimmed.value.documentEntry, baseline), true);

    // 行窗口改成 999999 → 文档同步 → documentEntry 偏离内置 → 自定义
    const synced = synchronizeModelMetadataDocumentLimitsPreview(document, "glm-5.3", "999999", "90%");
    assert.ok(synced);
    assert.strictEqual(metadataMatchesBuiltin(synced.preview.documentEntry, baseline), false);
    const decision = importSaveDecision({
      parseOk: true,
      documentBlank: false,
      imported: false,
      matchesBuiltin: metadataMatchesBuiltin(synced.preview.documentEntry, baseline),
    });
    assert.strictEqual(decision.effect, "custom");
    assert.strictEqual(decision.label, "保存为自定义配置");
    // 有事实字段的保存照常写入
    const saved = replaceModelMetadataForSlug("", "glm-5.3", synced.preview.metadata);
    assert.deepStrictEqual(JSON.parse(saved), { "glm-5.3": { display_name: "glm-5.3" } });
  });

  it("空事实字段的自定义也落盘：窗口-only 保存不再静默删除", () => {
    // 纯模板型条目（只有窗口字段）：过滤后事实字段为空对象
    const parsed = parseModelMetadataDocument(
      builtinEntryToImportDocument({ slug: "tpl", context_window: 272_000 }, "tpl"),
      "tpl",
    );
    assert.ok(parsed.ok);
    assert.strictEqual(Object.keys(parsed.value.metadata).length, 0);
    // 旧实现走 delete 分支返回 ""，保存按钮承诺「自定义」实际 no-op
    const saved = replaceModelMetadataForSlug("", "tpl", parsed.value.metadata);
    assert.deepStrictEqual(JSON.parse(saved), { tpl: {} });
    // 读侧不再抹掉空条目：徽标/清除判定能看见这份自定义
    assert.ok(parseModelMetadataMap(saved).tpl);
    // 删除语义只归 clearModelMetadataForSlug
    assert.strictEqual(clearModelMetadataForSlug(saved, "tpl"), "");
    // 空基线防护：模板条目本身（无事实字段基线是 {} 场景已在比较函数内拦截），
    // 这里锁「窗口-only 编辑对模板条目也判不等价」
    assert.strictEqual(metadataMatchesBuiltin(
      { ...parsed.value.documentEntry, context_window: 999_999 },
      parsed.value.documentEntry,
    ), false);
  });

  it("builtinRowBackfillValue 回填空窗口/压缩列且尊重后缀与已填值", () => {
    // 窗口取内置值；压缩回落 Codex++ 默认 90%（当前内置资产该字段均为 null）
    assert.deepStrictEqual(
      builtinRowBackfillValue("kimi-k3", "", "", { context_window: 1_048_576, auto_compact_token_limit: null }),
      { window: "1048576", autoCompact: "90%" },
    );
    // 空白列等同空列
    assert.deepStrictEqual(
      builtinRowBackfillValue("kimi-k3", "  ", "  ", { context_window: 1_048_576, auto_compact_token_limit: null }),
      { window: "1048576", autoCompact: "90%" },
    );
    // 用户已填的值是显式意图，不覆盖
    assert.deepStrictEqual(
      builtinRowBackfillValue("kimi-k3", "512000", "85%", { context_window: 1_048_576, auto_compact_token_limit: null }),
      { window: null, autoCompact: null },
    );
    // [1M] 后缀是显式意图，整行不回填
    assert.deepStrictEqual(
      builtinRowBackfillValue("kimi-k3[1M]", "", "", { context_window: 1_048_576, auto_compact_token_limit: null }),
      { window: null, autoCompact: null },
    );
    // 未命中内置：不回填
    assert.deepStrictEqual(
      builtinRowBackfillValue("unknown-model", "", "", undefined),
      { window: null, autoCompact: null },
    );
    // 厂商给了压缩值时优先用内置换算（943718/1048576 → 90%）
    assert.deepStrictEqual(
      builtinRowBackfillValue("kimi-k3", "", "", { context_window: 1_048_576, auto_compact_token_limit: 943_718 }),
      { window: "1048576", autoCompact: "90%" },
    );
    // 窗口缺失：窗口列不动，压缩仍回落默认（压缩列存百分比，不依赖窗口）
    assert.deepStrictEqual(
      builtinRowBackfillValue("kimi-k3", "", "", { context_window: null, auto_compact_token_limit: null }),
      { window: null, autoCompact: "90%" },
    );
  });

  it("resolveModelMetadataRowKey 按行解析配置落点（含 pending rename）", () => {    const map = { "kimi-k3": { vendor: "x" }, "deepseek-v4-pro": { vendor: "y" } };
    // 现名命中用现名
    assert.strictEqual(resolveModelMetadataRowKey(map, { current: "kimi-k3" }), "kimi-k3");
    // 现名无配置、origin 有 → 改名尚未提交（map 还挂在原名下）时回退
    assert.strictEqual(resolveModelMetadataRowKey(map, { current: "kimi-k3-turbo", origin: "Kimi-K3[1M]" }), "kimi-k3");
    // 两者都没有 → null
    assert.strictEqual(resolveModelMetadataRowKey(map, { current: "unknown", origin: "missing" }), null);
    assert.strictEqual(resolveModelMetadataRowKey(map, { current: "unknown" }), null);
    // 后缀/大小写变体归一到同一 key
    assert.strictEqual(resolveModelMetadataRowKey(map, { current: "DEEPSEEK-V4-PRO[1M]", origin: "" }), "deepseek-v4-pro");
    // 行名被清空时回退 origin
    assert.strictEqual(resolveModelMetadataRowKey(map, { current: "", origin: "deepseek-v4-pro" }), "deepseek-v4-pro");
    assert.strictEqual(resolveModelMetadataRowKey(map, { current: "" }), null);
  });

  it("清除恒可点：只清空面板文档，不受 imported/解析/匹配状态影响", () => {
    const options = {
      slug: "kimi-k3",
      document: "",
      imported: false,
      parseOk: false,
      matched: false,
      matchesBuiltin: false,
    };
    const controls = importPanelControls(options);
    assert.strictEqual(controls.clear.disabled, false);
    assert.strictEqual(controls.clear.title, "清空面板内容（不影响已保存的配置）");
    // 有自定义配置、内容与内置一致等任何状态下都不置灰
    assert.strictEqual(importPanelControls({ ...options, imported: true, parseOk: true, matched: true, matchesBuiltin: true }).clear.disabled, false);
  });

  it("来源徽标直接下发纯文本，文案由 helper 统一维护", () => {
    const [match] = metadataSourceTags({
      slug: "kimi-k3[1M]",
      imported: false,
      builtinMatch: { matched: true, source: "Kimi", entry: { slug: "kimi-k3" } },
      builtinIndexSlug: undefined,
    });
    assert.deepStrictEqual(match, {
      kind: "match",
      tone: "builtin",
      text: "匹配：Kimi",
      title: "内置元数据：Kimi",
    });
    const [fallback] = metadataSourceTags({
      slug: "unknown",
      imported: false,
      builtinMatch: { matched: false },
      builtinIndexSlug: undefined,
      fallbackSlug: "gpt-5.5",
    });
    assert.deepStrictEqual(fallback, {
      kind: "fallback",
      tone: "fallback",
      text: "回退：gpt-5.5",
      title: "无内置元数据，生成时回退 gpt-5.5 官方模板",
    });
    // 命中内置 + 有自定义时并列两个标签，custom 在第二个
    const tags = metadataSourceTags({
      slug: "kimi-k3",
      imported: true,
      builtinMatch: { matched: true, source: "Kimi", entry: { slug: "kimi-k3" } },
      builtinIndexSlug: undefined,
    });
    assert.strictEqual(tags.length, 2);
    const custom = tags[1];
    assert.strictEqual(custom.kind, "custom");
    assert.strictEqual(custom.text, "自定义");
    assert.strictEqual(custom.title, "已导入自定义元数据，生成时覆盖内置（Kimi）");

    // 命中内置但无自定义：只有 match 一个标签
    const onlyMatch = metadataSourceTags({
      slug: "kimi-k3",
      imported: false,
      builtinMatch: { matched: true, source: "Kimi", entry: { slug: "kimi-k3" } },
      builtinIndexSlug: undefined,
    });
    assert.strictEqual(onlyMatch.length, 1);
    assert.strictEqual(onlyMatch[0].kind, "match");
    assert.strictEqual(onlyMatch[0].text, "匹配：Kimi");
  });

  it("内置查询区分命中、未命中和命令失败三态", () => {
    const matched = builtinMetadataQueryState({ matched: true, source: "Kimi", entry: { slug: "kimi-k3" } });
    assert.strictEqual(matched.status, "matched");
    const miss = builtinMetadataQueryState({ matched: false });
    assert.strictEqual(miss.status, "miss");
    const failed = builtinMetadataQueryState(null, new Error("IPC unavailable"));
    assert.deepStrictEqual(failed, { status: "error", error: "IPC unavailable" });
  });

  it("activeImportDraft 事务：名称不入事务，取消只回滚窗口/压缩", () => {
    const draft = createActiveImportDraft({ index: 2, rowName: "GPT-5.6-SOL[1M]", window: "1000000", autoCompact: "80%" });
    // 面板身份只用 index；originalSlug 是行已删除时的兜底身份，不再有
    // canonicalSlug 双轨（那是改名时面板卸载/清除锁死的根源）
    assert.strictEqual(draft.originalSlug, "GPT-5.6-SOL[1M]");
    assert.ok(!("canonicalSlug" in draft), "canonicalSlug 双轨应已删除");
    const edited = updateActiveImportDraft(draft, { document: "{}" });
    assert.strictEqual(edited.originalSlug, "GPT-5.6-SOL[1M]", "编辑文档不得污染兜底身份");
    const cancelled = cancelActiveImportDraft(edited);
    assert.deepStrictEqual(cancelled.rowPatch, { window: "1000000", autoCompact: "80%" });
    // 重新匹配只替换面板内容，originalSlug 保持打开面板时的行名——
    // 它是 pending rename 旧 key 的兜底，被改名污染后 resolveModelMetadataRowKey 就失明了
    const rematched = rematchActiveImportDraft(edited, "{\"models\":[]}", null);
    assert.strictEqual(rematched.originalSlug, "GPT-5.6-SOL[1M]");
    assert.strictEqual(rematched.document, "{\"models\":[]}");
  });
});
