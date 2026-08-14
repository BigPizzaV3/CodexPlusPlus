import assert from "node:assert";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";
import type { RelayProfile } from "./App.tsx";
import {
  buildModelWindows,
  isValidModelWindow,
  modelWindowRowsFromProfile,
  modelWindowRowsValidationError,
  modelWindowsMapToText,
  modelWindowsTextToMap,
  serializeModelWindowRows,
  mergeModelWindowRows,
} from "./model-windows.ts";

// 类型检查：确保 RelayProfile 包含 modelWindows、modelMetadata 和 modelVlm 字段
const _profileTypeCheck: RelayProfile = {
  id: "test",
  name: "",
  model: "",
  baseUrl: "",
  upstreamBaseUrl: "",
  apiKey: "",
  protocol: "responses",
  relayMode: "official",
  officialMixApiKey: false,
  hideOfficialUsageAlert: false,
  testModel: "",
  configContents: "",
  authContents: "",
  useCommonConfig: true,
  contextSelection: { mcpServers: [], skills: [], plugins: [] },
  contextSelectionInitialized: true,
  contextWindow: "",
  autoCompactLimit: "",
  modelList: "",
  modelWindows: "",
  modelAutoCompact: "",
  modelMetadata: "",
  modelVlm: "",
  vlmApiKey: "",
  vlmModel: "",
  vlmBaseUrl: "",
  userAgent: "",
  sub2apiEnabled: false,
  sub2apiMultiplier: "",
};

void _profileTypeCheck;

describe("model-windows helpers", () => {
  it("每个模型只通过 models.json 整体替换自己的配置", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const metadataSource = readFileSync(new URL("./model-metadata.ts", import.meta.url), "utf8");
    const styles = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
    const english = readFileSync(new URL("./i18n-en.ts", import.meta.url), "utf8");
    assert.match(source, /t\("导入 models\.json"\)/);
    assert.doesNotMatch(source, /已导入 · 上下文 \{0\}/);
    assert.doesNotMatch(source, /\{importLabel\}/);
    assert.match(source, /className="relay-model-config-heading">\{t\("模型配置"\)\}/);
    assert.match(source, /支持多模型文件，自动匹配同名模型/);
    assert.match(source, /t\("自动压缩"\)/);
    assert.match(source, /导入对应模型的 model\.json 字段/);
    assert.match(source, /以下字段由 Codex\+\+ 管理，未导入：\{0\}/);
    assert.doesNotMatch(source, /按 slug 精确匹配当前模型/);
    assert.match(source, /context_window、auto_compact_token_limit 保持同步。/);
    assert.doesNotMatch(source, /可粘贴包含多个模型的 models\.json；系统会/);
    assert.match(source, /t\("替换此模型配置"\)/);
    assert.match(english, /"替换此模型配置": "Replace this model configuration"/);
    assert.match(source, /disabled=\{!metadataImportPreview\}/);
    assert.match(source, /onBlur=\{\(\) => commitModelSlug\(index\)\}/);
    assert.match(source, /modelSlugOriginsRef/);
    assert.match(source, /className="relay-model-metadata-import-flow"/);
    assert.match(source, /normalizeAutoCompactPercent\(event\.currentTarget\.value\)/);
    assert.doesNotMatch(source, /t\("解析预览"\)/);
    assert.doesNotMatch(source, /aria-label=\{t\("取消导入"\)\}/);
    assert.doesNotMatch(source, /className="relay-model-import-heading"/);
    assert.doesNotMatch(source, /已匹配 · 上下文/);
    assert.doesNotMatch(source, /className="relay-model-import-preview"/);
    assert.match(source, /replaceModelMetadataForSlug\(/);
    assert.match(source, /clearModelMetadataForSlug\(/);
    assert.match(source, /t\("清除导入配置"\)/);
    assert.match(source, /清除已导入的模型字段，保留上下文窗口/);
    assert.match(source, /metadataImportTarget\.slug/);
    assert.match(source, /value=\{row\.window\}/);
    assert.doesNotMatch(source, /window: metadataImportPreview\.contextWindow \?\? ""/);
    assert.doesNotMatch(source, /autoCompact: metadataImportPreview\.autoCompactPercent \?\? ""/);
    assert.doesNotMatch(source, /setMetadataImportPreview\(parsed\.value\);\s*updateModelWindowRow\(index/);
    assert.match(source, /const modelRowsError = showApiFields\s*\?/);
    assert.match(source, /relayProfileUsesLiveFiles\(draft\)\s*\?/);
    assert.match(source, /const saved = await onFormChange\(next\)/);
    assert.match(source, /const applied = await actions\.reapplyActiveRelayProfile\(true\)/);
    assert.doesNotMatch(source, /const applied = await actions\.switchRelayProfile\(next, profile\.id\)/);
    assert.match(source, /if \(relaySwitchingRef\.current\)/);
    assert.match(source, /relaySwitchingRef\.current = true/);
    assert.match(source, /relaySwitchingRef\.current = false/);
    assert.doesNotMatch(source, /snapshotActiveRelayFilesBeforeSwitch/);
    assert.match(source, /savingRef\.current = true/);
    assert.match(source, /disabled=\{saving \|\| !!validationError\}/);
    assert.match(source, /profile\.id === form\.activeRelayId/);
    assert.doesNotMatch(source, /effectiveRelayConfigPreview\(normalizedDraft, form, normalizedDraft\)/);
    assert.match(source, /className="relay-model-import-workbench"/);
    assert.match(source, /serializeModelMetadataDocument\(\s*slug,\s*existingMetadata/);
    assert.match(source, /synchronizeModelMetadataDocumentLimitsPreview\(/);
    assert.match(source, /metadataImportPreview\?\.autoCompactPercent \?\? row\.autoCompact/);
    assert.match(source, /originalWindow:/);
    assert.match(source, /originalAutoCompact:/);
    assert.match(source, /placeholder="90%"/);
    assert.match(source, /className="relay-model-import-copy"/);
    assert.doesNotMatch(source, /className="relay-model-drawer"/);
    assert.doesNotMatch(source, /className="relay-model-capability-summary"/);
    assert.doesNotMatch(source, /className="relay-model-minimal-grid"/);
    assert.doesNotMatch(source, /t\("压缩触发 token 数"\)/);
    assert.doesNotMatch(source, /className="relay-model-raw-details"/);
    assert.match(source, /className="relay-field-context-window"/);
    assert.match(source, /className="relay-field-auto-compact"/);
    assert.match(source, /作为该供应商下所有模型的全局上下文上限，并受 catalog 中 max_context_window 约束/);
    assert.match(source, /填写后覆盖该供应商下所有模型的压缩触发 token 数/);
    assert.doesNotMatch(metadataSource, /RECOGNIZED_MODEL_FIELDS/);
    assert.doesNotMatch(metadataSource, /compatibilityFields/);
    assert.match(metadataSource, /MANAGED_MODEL_METADATA_FIELDS/);
    assert.match(metadataSource, /"max_context_window"/);
    assert.match(source, /spaceBelow < estimatedMenuHeight/);
    assert.match(source, /placement === "top" \? "open-top"/);
    assert.match(styles, /\.app-select\.open-top \.app-select-menu/);
    assert.match(styles, /\.relay-model-import-workbench/);
    assert.doesNotMatch(styles, /\.relay-advanced-fields,\s*\.relay-api-fields\s*\{[^}]*overflow: hidden;/);
    assert.match(styles, /\.relay-detail-sticky\s*\{[^}]*margin: 0;/);
    assert.doesNotMatch(styles, /\.relay-detail-sticky\s*\{[^}]*margin: -/);
    assert.match(styles, /@media \(max-width: 600px\)/);
    assert.match(styles, /\.relay-model-row,\s*\.relay-model-row-actions\s*\{[\s\S]*grid-template-columns:/);
    assert.match(styles, /\.relay-model-window-heading\s*\{\s*grid-column: 2;/);
    assert.match(styles, /\.relay-model-compact-heading\s*\{\s*grid-column: 3;/);
    assert.match(styles, /\.relay-model-config-heading\s*\{\s*grid-column: 4 \/ 6;/);
    assert.match(styles, /\.relay-model-import-button\s*\{[\s\S]*width: 36px;[\s\S]*height: 36px;/);
    assert.match(styles, /\.relay-model-row-actions \.app-select\s*\{\s*grid-column: 1;/);
    assert.match(styles, /\.relay-model-row-hint\s*\{\s*grid-column: 2 \/ 6;/);
    assert.match(styles, /\.relay-model-import-copy\s*\{[\s\S]*display: flex;/);
    assert.match(styles, /\.relay-model-import-copy span\s*\{[\s\S]*line-height: 1\.45;/);
    assert.match(styles, /\.relay-model-metadata-import-actions\s*\{[\s\S]*grid-template-columns: minmax\(0, 1fr\) auto;/);
    assert.match(styles, /\.relay-model-metadata-import-flow\s*\{[\s\S]*justify-content: flex-end;/);
  });

  it("modelWindowsMapToText 按 modelList 行顺序输出窗口文本", () => {
    assert.strictEqual(
      modelWindowsMapToText("a\nb\nc", '{"a":"1M","c":"200K"}'),
      "1M\n\n200K",
    );
  });

  it("modelWindowsMapToText 对非法 JSON 返回空字符串", () => {
    assert.strictEqual(modelWindowsMapToText("a\nb", "not-json"), "");
  });

  it("modelWindowsTextToMap 按行组装 model_windows map", () => {
    assert.strictEqual(
      modelWindowsTextToMap("a\nb\nc", "1M\n\n200K"),
      '{"a":"1M","c":"200K"}',
    );
  });

  it("modelWindowsTextToMap 对没有对应窗口的模型不写入 map", () => {
    assert.strictEqual(
      modelWindowsTextToMap("a\nb", "1M"),
      '{"a":"1M"}',
    );
  });

  it("buildModelWindows 行数一致时返回 modelWindows JSON", () => {
    const result = buildModelWindows("deepseek-v4-flash\ndeepseek-v4-pro", "1M\n");
    assert.strictEqual(result.ok, true);
    if (result.ok) {
      assert.strictEqual(result.modelWindows, '{"deepseek-v4-flash":"1M"}');
    }
  });

  it("buildModelWindows 行数不一致时返回错误", () => {
    const result = buildModelWindows("a\nb", "1M");
    assert.strictEqual(result.ok, false);
    if (!result.ok) {
      assert.ok(result.error.includes("2"));
      assert.ok(result.error.includes("1"));
    }
  });

  it("modelWindowRowsFromProfile 把模型和窗口合成同一组行", () => {
    assert.deepStrictEqual(
      modelWindowRowsFromProfile("a\nb\nc", '{"a":"1M","c":"200K"}'),
      [
        { model: "a", window: "1M", autoCompact: "", imageHandling: "send-as-is" },
        { model: "b", window: "", autoCompact: "", imageHandling: "send-as-is" },
        { model: "c", window: "200K", autoCompact: "", imageHandling: "send-as-is" },
      ],
    );
  });

  it("modelWindowRowsFromProfile 解析 modelVlm 标记", () => {
    assert.deepStrictEqual(
      modelWindowRowsFromProfile("a\nb\nc", '{}', '{"a":"vlm","b":"strip"}'),
      [
        { model: "a", window: "", autoCompact: "", imageHandling: "vlm" },
        { model: "b", window: "", autoCompact: "", imageHandling: "strip" },
        { model: "c", window: "", autoCompact: "", imageHandling: "send-as-is" },
      ],
    );
  });

  it("modelWindowRowsFromProfile 为旧自动压缩值补上百分号", () => {
    assert.deepStrictEqual(
      modelWindowRowsFromProfile("a\nb", '{}', '{}', '{"a":"90","b":"84.5%"}'),
      [
        { model: "a", window: "", autoCompact: "90%", imageHandling: "send-as-is" },
        { model: "b", window: "", autoCompact: "84.5%", imageHandling: "send-as-is" },
      ],
    );
  });

  it("serializeModelWindowRows 从行控件生成 modelList、modelWindows 和 modelVlm", () => {
    assert.deepStrictEqual(
      serializeModelWindowRows([
        { model: "a", window: "1M", autoCompact: "", imageHandling: "vlm" },
        { model: "", window: "400K", autoCompact: "", imageHandling: "send-as-is" },
        { model: "b", window: "", autoCompact: "", imageHandling: "send-as-is" },
      ]),
      {
        modelList: "a\nb",
        modelWindows: '{"a":"1M"}',
        modelVlm: '{"a":"vlm"}',
        modelAutoCompact: '{}',
      },
    );
  });

  it("mergeModelWindowRows 追加上游模型时跳过已有模型并保留窗口和图片处理", () => {
    assert.deepStrictEqual(
      mergeModelWindowRows(
        [
          { model: "deepseek-v4-flash", window: "1M", autoCompact: "", imageHandling: "vlm" },
          { model: "  ", window: "", autoCompact: "", imageHandling: "send-as-is" },
        ],
        [
          { model: "deepseek-v4-flash", window: "", autoCompact: "", imageHandling: "send-as-is" },
          { model: "deepseek-v4-pro", window: "", autoCompact: "", imageHandling: "vlm" },
          { model: " deepseek-v4-pro ", window: "200K", autoCompact: "", imageHandling: "send-as-is" },
        ],
      ),
      [
        { model: "deepseek-v4-flash", window: "1M", autoCompact: "", imageHandling: "vlm" },
        { model: "deepseek-v4-pro", window: "", autoCompact: "", imageHandling: "vlm" },
      ],
    );
  });

  it("模型行校验拒绝重复 slug 和后端不接受的百分比格式", () => {
    assert.deepStrictEqual(modelWindowRowsValidationError([
      { model: "a", window: "", autoCompact: "90%", imageHandling: "send-as-is" },
      { model: "a", window: "", autoCompact: "80%", imageHandling: "send-as-is" },
    ]), { code: "duplicateModel", model: "a" });
    assert.deepStrictEqual(modelWindowRowsValidationError([
      { model: "a", window: "", autoCompact: "1e2", imageHandling: "send-as-is" },
    ]), { code: "invalidAutoCompact", model: "a" });
    assert.deepStrictEqual(modelWindowRowsValidationError([
      { model: "a", window: "1.5M", autoCompact: "90%", imageHandling: "send-as-is" },
    ]), { code: "invalidWindow", model: "a" });
    assert.strictEqual(modelWindowRowsValidationError([
      { model: "a", window: "", autoCompact: "84.5%", imageHandling: "send-as-is" },
    ]), null);
  });

  it("上下文窗口语法与 Rust u64 解析保持一致", () => {
      for (const valid of ["", "1", "256K", "1M", "18446744073709551615"]) {
      assert.strictEqual(isValidModelWindow(valid), true, valid);
    }
    for (const invalid of ["0", "1.5M", "abc", "-1", "18446744073709551616"]) {
      assert.strictEqual(isValidModelWindow(invalid), false, invalid);
    }
  });
});
