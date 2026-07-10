import assert from "node:assert";
import { describe, it } from "node:test";
import type { RelayProfile } from "./App.tsx";
import {
  buildModelWindows,
  createModelWindowRow,
  modelWindowRowsFromProfile,
  modelWindowsMapToText,
  modelWindowsTextToMap,
  serializeModelWindowRows,
  mergeModelWindowRows,
  modelWindowTokenError,
  validateModelWindowRows,
} from "./model-windows.ts";

// 类型检查：确保 RelayProfile 包含 modelWindows 字段
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
      modelWindowRowsFromProfile("a\nb\nc", '{"a":"1M","c":"200K"}')
        .map((row) => `${row.model}:${row.window}`),
      ["a:1M", "b:", "c:200K"],
    );
  });

  it("serializeModelWindowRows 从行控件生成 modelList 和 modelWindows", () => {
    assert.deepStrictEqual(
      serializeModelWindowRows([
        createModelWindowRow("a", "1M"),
        createModelWindowRow("", "400K"),
        createModelWindowRow("b", ""),
      ]),
      {
        modelList: "a\nb",
        modelWindows: '{"a":"1M"}',
      },
    );
  });

  it("mergeModelWindowRows 追加上游模型时跳过已有模型并保留窗口", () => {
    assert.deepStrictEqual(
      mergeModelWindowRows(
        [
          createModelWindowRow("deepseek-v4-flash", "1M"),
          createModelWindowRow("  ", ""),
        ],
        [
          createModelWindowRow("deepseek-v4-flash", ""),
          createModelWindowRow("deepseek-v4-pro", ""),
          createModelWindowRow(" deepseek-v4-pro ", "200K"),
        ],
      ).map((row) => `${row.model}:${row.window}`),
      ["deepseek-v4-flash:1M", "deepseek-v4-pro:"],
    );
  });

  it("严格接受整数和 K/M 后缀，拒绝小数及其他后缀", () => {
    assert.strictEqual(modelWindowTokenError("200K"), null);
    assert.strictEqual(modelWindowTokenError("1m"), null);
    assert.strictEqual(modelWindowTokenError("1000000"), null);
    assert.ok(modelWindowTokenError("200KB"));
    assert.ok(modelWindowTokenError("1.5M"));
    assert.ok(modelWindowTokenError("0"));
  });

  it("拒绝没有模型名称的窗口，并保留已有行 id", () => {
    const row = createModelWindowRow("a", "1M");
    assert.strictEqual(mergeModelWindowRows([row], [])[0].id, row.id);
    assert.ok(validateModelWindowRows([createModelWindowRow("", "1M")]));
  });
});
