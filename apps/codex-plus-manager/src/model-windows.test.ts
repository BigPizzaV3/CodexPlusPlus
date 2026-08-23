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
  noAuth: false,
  hideOfficialUsageAlert: false,
  testModel: "",
  configContents: "",
  authContents: "",
  useCommonConfig: true,
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
        { model: "a", window: "1M", autoCompact: "90%", imageHandling: "send-as-is" },
        { model: "b", window: "", autoCompact: "90%", imageHandling: "send-as-is" },
        { model: "c", window: "200K", autoCompact: "90%", imageHandling: "send-as-is" },
      ],
    );
  });

  it("modelWindowRowsFromProfile 解析 modelVlm 标记", () => {
    assert.deepStrictEqual(
      modelWindowRowsFromProfile("a\nb\nc", '{}', '{"a":"vlm","b":"strip"}'),
      [
        { model: "a", window: "", autoCompact: "90%", imageHandling: "vlm" },
        { model: "b", window: "", autoCompact: "90%", imageHandling: "strip" },
        { model: "c", window: "", autoCompact: "90%", imageHandling: "send-as-is" },
      ],
    );
  });

  it("modelWindowRowsFromProfile 忽略损坏 map 中的非字符串值", () => {
    assert.deepStrictEqual(
      modelWindowRowsFromProfile(
        "a\nb",
        '{"a":1048576,"b":"512K"}',
        '{"a":true,"b":"strip"}',
        '{"a":90,"b":"80%"}',
      ),
      [
        { model: "a", window: "", autoCompact: "90%", imageHandling: "send-as-is" },
        { model: "b", window: "512K", autoCompact: "80%", imageHandling: "strip" },
      ],
    );
  });

  it("modelWindowRowsFromProfile 不因 null map 崩溃", () => {
    assert.deepStrictEqual(
      modelWindowRowsFromProfile("a\nb", "null", "{}", "null"),
      [
        { model: "a", window: "", autoCompact: "90%", imageHandling: "send-as-is" },
        { model: "b", window: "", autoCompact: "90%", imageHandling: "send-as-is" },
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
        modelAutoCompact: '{"a":"90%","b":"90%"}',
      },
    );
  });

  it("删除模型后序列化保存载荷不会保留已删除模型", () => {
    const rows = [
      { model: "keep", window: "1M", autoCompact: "90%", imageHandling: "send-as-is" as const },
      { model: "remove", window: "200K", autoCompact: "80%", imageHandling: "vlm" as const },
    ];
    const saved = serializeModelWindowRows(rows.filter((row) => row.model !== "remove"));
    assert.deepStrictEqual(saved, {
      modelList: "keep",
      modelWindows: '{"keep":"1M"}',
      modelVlm: "{}",
      modelAutoCompact: '{"keep":"90%"}',
    });
  });

  it("mergeModelWindowRows 追加上游模型时跳过已有模型并保留窗口和图片处理", () => {
    assert.deepStrictEqual(
      mergeModelWindowRows(
        [
          { model: "deepseek-v4-flash", window: "1M", autoCompact: "90%", imageHandling: "vlm" },
          { model: "  ", window: "", autoCompact: "", imageHandling: "send-as-is" },
        ],
        [
          { model: "deepseek-v4-flash", window: "", autoCompact: "", imageHandling: "send-as-is" },
          { model: "deepseek-v4-pro", window: "", autoCompact: "", imageHandling: "vlm" },
          { model: " deepseek-v4-pro ", window: "200K", autoCompact: "", imageHandling: "send-as-is" },
        ],
      ),
      [
        { model: "deepseek-v4-flash", window: "1M", autoCompact: "90%", imageHandling: "vlm" },
        { model: "deepseek-v4-pro", window: "", autoCompact: "", imageHandling: "vlm" },
      ],
    );
  });
});
