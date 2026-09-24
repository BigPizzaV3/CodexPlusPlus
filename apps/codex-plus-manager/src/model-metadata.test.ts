import assert from "node:assert";
import { describe, it } from "node:test";
import { isValidAutoCompactPercent, normalizeAutoCompactEditing, normalizeAutoCompactPercent } from "./auto-compact.ts";
import {
  builtinEntryToImportDocument,
  clearModelMetadataForSlug,
  importDocumentSyncPatch,
  importPanelControls,
  importSaveDecision,
  metadataMatchesBuiltin,
  metadataSourceTags,
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

  it("元数据来源标签覆盖全部用户场景", () => {
    const match = { matched: true, source: "GLM", entry: { slug: "glm-5.3" } };
    const fallback = { matched: false, fallback: { slug: "gpt-5.5", context_window: 272_000 } };

    // 纯命中（打开导入区，内置预填）：只显示匹配标签
    assert.deepStrictEqual(
      metadataSourceTags({ slug: "glm-5.3", imported: false, builtinMatch: match, builtinIndexSlug: { source: "GLM" } }),
      [{ kind: "match", text: "匹配：GLM", title: "内置元数据：GLM", tone: "builtin" }],
    );
    // 命中 + 自定义（保存过/老版本已配置）：匹配与自定义并列
    assert.deepStrictEqual(
      metadataSourceTags({ slug: "glm-5.3", imported: true, builtinMatch: match, builtinIndexSlug: { source: "GLM" } }).map(t => t.text),
      ["匹配：GLM", "自定义"],
    );
    // 回退态（无内置）：回退标签
    assert.deepStrictEqual(
      metadataSourceTags({ slug: "nope", imported: false, builtinMatch: fallback, builtinIndexSlug: undefined }).map(t => t.text),
      ["回退：gpt-5.5"],
    );
    // 回退 + 自定义：自定义已覆盖，不再显示"回退"（避免误导为还在用 gpt-5.5）
    assert.deepStrictEqual(
      metadataSourceTags({ slug: "nope", imported: true, builtinMatch: fallback, builtinIndexSlug: undefined }).map(t => t.text),
      ["自定义"],
    );
    // match 数据未返回时用索引兜底（行级渲染路径）
    assert.deepStrictEqual(
      metadataSourceTags({ slug: "glm-5.3", imported: false, builtinMatch: null, builtinIndexSlug: { source: "GLM" } }).map(t => t.text),
      ["匹配：GLM"],
    );
    // 全无：回退
    assert.deepStrictEqual(
      metadataSourceTags({ slug: "nope", imported: false, builtinMatch: null, builtinIndexSlug: undefined }).map(t => t.text),
      ["回退：gpt-5.5"],
    );
    // 自定义 fallback 标签文案可定制（fallback 源变化时）
    assert.deepStrictEqual(
      metadataSourceTags({ slug: "nope", imported: false, builtinMatch: null, builtinIndexSlug: undefined, fallbackSlug: "gpt-5.4" }).map(t => t.text),
      ["回退：gpt-5.4"],
    );
    // match 未命中但 entry 缺失时不应产生匹配标签（脏数据防御）
    assert.deepStrictEqual(
      metadataSourceTags({ slug: "glm-5.3", imported: false, builtinMatch: { matched: false, source: "GLM" }, builtinIndexSlug: undefined }).map(t => t.text),
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

    // 1) 内置预填、未编辑、当前无自定义 → 已在使用内置，无需保存
    assert.deepStrictEqual(
      importSaveDecision({ parseOk: true, documentBlank: false, imported: false, matchesBuiltin: true }),
      { needsSave: false, effect: "none", label: "保存此模型", title: "已在使用内置元数据，无需保存" },
    );

    // 2) 内置预填、未编辑、已有自定义 → 内容是内置，保存 = 放弃自定义
    assert.deepStrictEqual(
      importSaveDecision({ parseOk: true, documentBlank: false, imported: true, matchesBuiltin: true }),
      { needsSave: true, effect: "builtin", label: "恢复内置", title: "内容与内置元数据一致，保存后改用内置元数据" },
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
      { needsSave: true, effect: "builtin", label: "恢复内置", title: "保存后清除该模型的自定义配置，改用内置元数据" },
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

    // metadataMatchesBuiltin：字段顺序不影响相等；窗口字段不参与（由行管辖）
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
      matchedSource: "Kimi",
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
    // 保存置灰并说明原因，状态行保持内置态（不会翻成自定义）
    assert.strictEqual(base.rematch.disabled, false);
    assert.strictEqual(base.clear.disabled, true);
    assert.strictEqual(base.cancel.disabled, false);
    assert.strictEqual(base.save.disabled, true);
    assert.strictEqual(base.save.title, "已在使用内置元数据，无需保存");
    assert.strictEqual(base.status.tone, "builtin");
    assert.match(base.status.text, /内置元数据（Kimi）/);
    // 状态行必须点明实时写回与可撤销，避免用户以为只有保存才生效
    assert.match(base.status.text, /实时生效/);

    // 已有自定义 + 内容是内置复刻：清除可用、保存变「恢复内置」，状态行预告保存后恢复内置
    const withCustom = control({ imported: true });
    assert.strictEqual(withCustom.clear.disabled, false);
    assert.strictEqual(withCustom.save.disabled, false);
    assert.strictEqual(withCustom.save.label, "恢复内置");
    assert.match(withCustom.status.text, /保存后恢复内置/);

    // 内容真的改过（与内置不等价）：保存变可点，文案「保存为自定义配置」
    const edited = control({ matchesBuiltin: false });
    assert.strictEqual(edited.save.disabled, false);
    assert.strictEqual(edited.save.label, "保存为自定义配置");
    assert.match(edited.status.text, /保存后：该模型改用这份自定义配置/);

    // 改了模型名导致未命中内置：重新匹配置灰，但按钮本身不消失
    const unmatched = control({ matched: false, matchedSource: undefined });
    assert.strictEqual(unmatched.rematch.disabled, true);
    assert.match(unmatched.rematch.title, /没有内置元数据可匹配/);
    assert.strictEqual(unmatched.rematch.title.length > 0, true);
    assert.strictEqual(unmatched.status.tone, "fallback");
    assert.match(unmatched.status.text, /回退 gpt-5\.5/);

    // 模型名为空：重新匹配置灰并说明原因
    assert.strictEqual(control({ slug: "" }).rematch.disabled, true);
    assert.match(control({ slug: "" }).rematch.title, /请先填写模型名称/);

    // 解析失败：保存置灰，title 指向 JSON 问题而不是「无需保存」
    const broken = control({ parseOk: false });
    assert.strictEqual(broken.save.disabled, true);
    assert.match(broken.save.title, /JSON 无法解析/);

    // 文本框被清空且已有自定义：保存变「恢复内置」，状态行预告恢复内置
    const cleared = control({ document: "", imported: true, matchesBuiltin: false });
    assert.strictEqual(cleared.save.label, "恢复内置");
    assert.strictEqual(cleared.save.disabled, false);
    assert.match(cleared.status.text, /保存后恢复内置/);
    // 清空且没有自定义：没什么可做的，保存置灰
    assert.strictEqual(control({ document: "", matchesBuiltin: false }).save.disabled, true);

    // fallbackSlug 可覆盖（回退模板变化时不用改代码）
    assert.match(
      control({ matched: false, matchedSource: undefined, fallbackSlug: "gpt-5.4" }).status.text,
      /回退 gpt-5\.4/,
    );
  });
});
