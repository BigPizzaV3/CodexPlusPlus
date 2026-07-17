import assert from "node:assert";
import { describe, it } from "node:test";
import type { RelayProfile } from "./App.tsx";
import {
  buildModelWindows,
  modelWindowRowsFromProfile,
  modelWindowsMapToText,
  modelWindowsTextToMap,
  serializeModelWindowRows,
  mergeModelWindowRows,
  type ModelWindowRow,
} from "./model-windows.ts";

// 类型检查：确保 RelayProfile 包含 modelWindows 和 modelVlm 字段
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
  modelVlm: "",
  modelReasoningSupport: "",
  vlmApiKey: "",
  vlmModel: "",
  vlmBaseUrl: "",
  vlmProtocol: "chatCompletions",
  userAgent: "",
};

void _profileTypeCheck;

describe("model-windows helpers", () => {
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
        { model: "a", window: "1M", textOnly: false, noReasoning: false },
        { model: "b", window: "", textOnly: false, noReasoning: false },
        { model: "c", window: "200K", textOnly: false, noReasoning: false },
      ],
    );
  });

  it("modelWindowRowsFromProfile 解析 modelVlm 标记", () => {
    assert.deepStrictEqual(
      modelWindowRowsFromProfile("a\nb\nc", '{}', '{"a":"vlm","b":"strip"}'),
      [
        { model: "a", window: "", textOnly: true, noReasoning: false },
        { model: "b", window: "", textOnly: true, noReasoning: false },
        { model: "c", window: "", textOnly: false, noReasoning: false },
      ],
    );
  });

  it("serializeModelWindowRows 从行控件生成 modelList、modelWindows 和 modelVlm", () => {
    assert.deepStrictEqual(
      serializeModelWindowRows([
        { model: "a", window: "1M", textOnly: true, noReasoning: false },
        { model: "", window: "400K", textOnly: false, noReasoning: false },
        { model: "b", window: "", textOnly: false, noReasoning: false },
      ], false),
      {
        modelList: "a\nb",
        modelWindows: '{"a":"1M"}',
        modelVlm: '{"a":"strip"}',
        modelReasoningSupport: "{}",
      },
    );
  });

  it("mergeModelWindowRows 追加上游模型时跳过已有模型并保留窗口和图片处理", () => {
    assert.deepStrictEqual(
      mergeModelWindowRows(
        [
          { model: "deepseek-v4-flash", window: "1M", textOnly: true, noReasoning: false },
          { model: "  ", window: "", textOnly: false, noReasoning: false },
        ],
        [
          { model: "deepseek-v4-flash", window: "", textOnly: false, noReasoning: false },
          { model: "deepseek-v4-pro", window: "", textOnly: true, noReasoning: false },
          { model: " deepseek-v4-pro ", window: "200K", textOnly: false, noReasoning: false },
        ],
      ),
      [
        { model: "deepseek-v4-flash", window: "1M", textOnly: true, noReasoning: false },
        { model: "deepseek-v4-pro", window: "", textOnly: true, noReasoning: false },
      ],
    );
  });
});

describe("checkbox serialization", () => {
  it("textOnly + VL configured -> vlm; textOnly + VL not configured -> strip", () => {
    const rows: ModelWindowRow[] = [
      { model: "deepseek-v4", window: "1M", textOnly: true, noReasoning: false },
      { model: "glm-5.2", window: "", textOnly: true, noReasoning: false },
      { model: "minimax", window: "", textOnly: false, noReasoning: false },
    ];
    const withVl = serializeModelWindowRows(rows, true);
    assert.deepStrictEqual(JSON.parse(withVl.modelVlm), { "deepseek-v4": "vlm", "glm-5.2": "vlm" });
    const noVl = serializeModelWindowRows(rows, false);
    assert.deepStrictEqual(JSON.parse(noVl.modelVlm), { "deepseek-v4": "strip", "glm-5.2": "strip" });
  });

  it("noReasoning -> modelReasoningSupport map", () => {
    const rows: ModelWindowRow[] = [
      { model: "kimi-k2.6", window: "", textOnly: false, noReasoning: true },
    ];
    const s = serializeModelWindowRows(rows, false);
    assert.deepStrictEqual(JSON.parse(s.modelReasoningSupport), { "kimi-k2.6": false });
  });

  it("migrates old modelVlm vlm/strip -> textOnly=true", () => {
    const rows = modelWindowRowsFromProfile(
      "deepseek-v4\nglm-5.2\nminimax",
      "{}",
      '{"deepseek-v4":"vlm","glm-5.2":"strip"}',
      "{}",
    );
    assert.strictEqual(rows[0].textOnly, true);
    assert.strictEqual(rows[1].textOnly, true);
    assert.strictEqual(rows[2].textOnly, false);
  });
});
