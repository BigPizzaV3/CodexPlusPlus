import assert from "node:assert";
import { describe, it } from "node:test";
import type { RelayProfile } from "./App.tsx";
import {
  buildModelWindows,
  modelWindowRowsFromProfile,
  modelWindowRowsValidationError,
  modelWindowsMapToText,
  modelWindowsTextToMap,
  serializeModelWindowRows,
  mergeModelWindowRows,
  reorderModelWindowRows,
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
  hideOfficialUsageAlert: false,
  testModel: "",
  configContents: "",
  authContents: "",
  useCommonConfig: true,
  contextSelection: { mcpServers: [], skills: [], plugins: [] },
  contextSelectionInitialized: false,
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
  customHeaders: [],
  sub2apiEnabled: false,
  noAuth: false,
  sub2apiMultiplier: "",
  standardOpenaiProtocol: false,
  rateLimitCooldownEnabled: false,
  channelQueueEnabled: false,
  channelRequestsPerMinute: 20,
  cooldownErrorStatuses: [429, 500],
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

  it("reorderModelWindowRows 移动模型时保留整行配置且不修改原数组", () => {
    const rows = [
      { model: "a", window: "1M", autoCompact: "90%", imageHandling: "send-as-is" as const },
      { model: "b", window: "200K", autoCompact: "80%", imageHandling: "strip" as const },
      { model: "c", window: "", autoCompact: "70%", imageHandling: "vlm" as const },
    ];

    assert.deepStrictEqual(reorderModelWindowRows(rows, 0, 2), [rows[1], rows[2], rows[0]]);
    assert.strictEqual(serializeModelWindowRows(reorderModelWindowRows(rows, 0, 2)).modelList, "b\nc\na");
    assert.deepStrictEqual(rows, [
      { model: "a", window: "1M", autoCompact: "90%", imageHandling: "send-as-is" },
      { model: "b", window: "200K", autoCompact: "80%", imageHandling: "strip" },
      { model: "c", window: "", autoCompact: "70%", imageHandling: "vlm" },
    ]);
  });
});

// issue #2345：行名可带 [1M] 后缀，而运行期查 model_vlm / model_windows 用的是
// 剥掉后缀的 model 字符串。读写两侧 key 不归一化时，保存后重新读回会把图片处理
// 静默退回「原样发送」——用户表现为「保存之后全部变成原样发送图片」。
describe("model-windows 模型 key 归一化（issue #2345）", () => {
  it("带后缀的行名序列化后 map key 用规范 slug，选中的图片处理读回不丢", () => {
    const rows = [
      { model: "deepseek-v4-pro[1M]", window: "200K", autoCompact: "80%", imageHandling: "vlm" as const },
    ];
    const saved = serializeModelWindowRows(rows);
    assert.deepStrictEqual(saved, {
      // model_list 保留用户原样行名（codex 侧靠后缀识别窗口）
      modelList: "deepseek-v4-pro[1M]",
      modelWindows: '{"deepseek-v4-pro":"200K"}',
      modelVlm: '{"deepseek-v4-pro":"vlm"}',
      modelAutoCompact: '{"deepseek-v4-pro":"80%"}',
    });
    // 保存后 App.tsx 的 effect 会把这份载荷重新读回成行，必须还原成 vlm
    assert.deepStrictEqual(
      modelWindowRowsFromProfile(saved.modelList, saved.modelWindows, saved.modelVlm, saved.modelAutoCompact),
      [{ model: "deepseek-v4-pro[1M]", window: "200K", autoCompact: "80%", imageHandling: "vlm" }],
    );
  });

  it("历史数据里带后缀的 key 仍能命中不带后缀的行名（兼容回退）", () => {
    assert.strictEqual(
      modelWindowRowsFromProfile("deepseek-v4-pro", "{}", '{"deepseek-v4-pro[1M]":"vlm"}')[0].imageHandling,
      "vlm",
    );
  });

  it("不带后缀的 key 能命中带后缀的行名（读侧归一化）", () => {
    assert.strictEqual(
      modelWindowRowsFromProfile("deepseek-v4-pro[1M]", "{}", '{"deepseek-v4-pro":"strip"}')[0].imageHandling,
      "strip",
    );
  });

  it("供应商 slug 大小写不一致时仍能命中", () => {
    assert.strictEqual(
      modelWindowRowsFromProfile("GLM-5.3", "{}", '{"glm-5.3":"vlm"}')[0].imageHandling,
      "vlm",
    );
    assert.strictEqual(
      modelWindowRowsFromProfile("GLM-5.3[1M]", "{}", '{"glm-5.3":"vlm"}')[0].imageHandling,
      "vlm",
    );
  });

  it("落盘 key 保留原始大小写——上游按大小写敏感全字匹配 model_vlm", () => {
    // 归一化只做「去后缀」，不能顺手 toLowerCase：上游请求仍发 GLM-5.3，
    // 写成 glm-5.3 会让 vision.rs 的 image_handling_mode 查表落空。
    const saved = serializeModelWindowRows([
      { model: "GLM-5.3[1M]", window: "200K", autoCompact: "80%", imageHandling: "vlm" },
    ]);
    assert.deepStrictEqual(JSON.parse(saved.modelVlm), { "GLM-5.3": "vlm" });
    assert.deepStrictEqual(JSON.parse(saved.modelWindows), { "GLM-5.3": "200K" });
    assert.deepStrictEqual(JSON.parse(saved.modelAutoCompact), { "GLM-5.3": "80%" });
  });

  it("三张配置 map 的 key 都按规范 slug 落盘", () => {
    const saved = serializeModelWindowRows([
      { model: "a[1M]", window: "300K", autoCompact: "70%", imageHandling: "strip" },
    ]);
    assert.deepStrictEqual(JSON.parse(saved.modelWindows), { a: "300K" });
    assert.deepStrictEqual(JSON.parse(saved.modelVlm), { a: "strip" });
    assert.deepStrictEqual(JSON.parse(saved.modelAutoCompact), { a: "70%" });
  });

  it("同一模型的带后缀与不带后缀两种写法判重", () => {
    const rows = [
      { model: "deepseek-v4-pro[1M]", window: "", autoCompact: "90%", imageHandling: "send-as-is" as const },
      { model: "deepseek-v4-pro", window: "", autoCompact: "90%", imageHandling: "send-as-is" as const },
    ];
    assert.deepStrictEqual(modelWindowRowsValidationError(rows), {
      code: "duplicateModel",
      model: "deepseek-v4-pro",
    });
    // 序列化侧同样只保留第一条，避免两条行写进同一张 map 的同一个 key 互相覆盖
    assert.deepStrictEqual(serializeModelWindowRows(rows).modelList, "deepseek-v4-pro[1M]");
  });
});
