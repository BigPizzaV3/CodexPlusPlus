import assert from "node:assert";
import { describe, it } from "node:test";
import { parseModelMetadataMap } from "./model-metadata.ts";
import {
  defaultReasoningLevelFromMetadata,
  formatReasoningLevelsLabel,
  normalizeReasoningLevels,
  reasoningLevelsFromMetadata,
  resolveModelReasoningLevels,
  setModelReasoningLevelsForSlug,
} from "./model-reasoning.ts";

describe("model reasoning levels", () => {
  it("label collapses contiguous runs of three or more canonical levels", () => {
    assert.equal(
      formatReasoningLevelsLabel(["low", "medium", "high", "xhigh", "max"]),
      "low → max",
    );
    assert.equal(
      formatReasoningLevelsLabel(["none", "low", "medium", "high", "xhigh", "max"]),
      "none, low → max",
    );
    // 两档不折叠；跨越未选档位（minimal）不合并区间。
    assert.equal(formatReasoningLevelsLabel(["none", "high"]), "none, high");
    assert.equal(
      formatReasoningLevelsLabel(["low", "medium", "high", "minimal"]),
      "low, medium, high, minimal",
    );
    assert.equal(formatReasoningLevelsLabel([]), "");
  });

  it("normalization keeps known efforts in canonical order and drops junk", () => {
    assert.deepStrictEqual(
      normalizeReasoningLevels(["max", "low", "bogus", "low", "", "high"]),
      ["low", "high", "max"],
    );
    assert.deepStrictEqual(
      normalizeReasoningLevels([{ effort: "xhigh" }, { effort: "nope" }, 3]),
      ["xhigh"],
    );
    assert.deepStrictEqual(normalizeReasoningLevels(undefined), []);
  });

  it("metadata readers distinguish missing declarations from explicit ones", () => {
    assert.equal(reasoningLevelsFromMetadata({}), null);
    assert.equal(reasoningLevelsFromMetadata(undefined), null);
    assert.deepStrictEqual(reasoningLevelsFromMetadata({ supported_reasoning_levels: [] }), []);
    assert.deepStrictEqual(
      reasoningLevelsFromMetadata({ supported_reasoning_levels: [{ effort: "high" }, "none"] }),
      ["none", "high"],
    );
    assert.equal(defaultReasoningLevelFromMetadata({ default_reasoning_level: "high" }), "high");
    assert.equal(defaultReasoningLevelFromMetadata({ default_reasoning_level: "bogus" }), null);
  });

  it("custom declarations win over built-in levels and keep their default", () => {
    const state = resolveModelReasoningLevels(
      { supported_reasoning_levels: [{ effort: "none" }, { effort: "high" }], default_reasoning_level: "high" },
      ["low", "medium", "high"],
    );
    assert.deepStrictEqual(state, { levels: ["none", "high"], overridden: true, defaultLevel: "high" });
    // 内置默认档不回填，避免打开面板就产生覆盖。
    assert.deepStrictEqual(
      resolveModelReasoningLevels(undefined, ["low", "medium", "high"]),
      { levels: ["low", "medium", "high"], overridden: false, defaultLevel: null },
    );
    assert.deepStrictEqual(resolveModelReasoningLevels(undefined, null), {
      levels: [],
      overridden: false,
      defaultLevel: null,
    });
  });

  it("writing levels stores canonical efforts with descriptions", () => {
    const written = setModelReasoningLevelsForSlug("", "deepseek-v4-pro", ["max", "low"], "max");
    assert.deepStrictEqual(parseModelMetadataMap(written), {
      "deepseek-v4-pro": {
        supported_reasoning_levels: [
          { effort: "low", description: "Fast responses with lighter reasoning" },
          { effort: "max", description: "Maximum reasoning depth for the hardest problems" },
        ],
        default_reasoning_level: "max",
      },
    });
  });

  it("writing respects suffixed row names and preserves other metadata", () => {
    const initial = JSON.stringify({
      "DeepSeek-V4-Pro[1M]": { codex_plus_fast_support: "supported", temperature: 0.2 },
      "other-model": { supported_reasoning_levels: [{ effort: "high" }] },
    });
    const written = setModelReasoningLevelsForSlug(initial, "DeepSeek-V4-Pro[1M]", ["high"], null);
    const map = parseModelMetadataMap(written);
    assert.deepStrictEqual(map["deepseek-v4-pro"], {
      codex_plus_fast_support: "supported",
      temperature: 0.2,
      supported_reasoning_levels: [
        { effort: "high", description: "Greater reasoning depth for complex problems" },
      ],
    });
    // 其他模型条目原样保留。
    assert.deepStrictEqual(map["other-model"], { supported_reasoning_levels: [{ effort: "high" }] });
  });

  it("dropping the default when it leaves the selection, and clearing removes the keys", () => {
    const initial = setModelReasoningLevelsForSlug("", "m1", ["low", "high"], "high");
    const withoutDefault = setModelReasoningLevelsForSlug(initial, "m1", ["low"], "high");
    assert.deepStrictEqual(parseModelMetadataMap(withoutDefault), {
      m1: {
        supported_reasoning_levels: [
          { effort: "low", description: "Fast responses with lighter reasoning" },
        ],
      },
    });
    // 清空全部档位 = 放弃自定义声明：两个键都移除，空条目一起删除。
    assert.equal(setModelReasoningLevelsForSlug(withoutDefault, "m1", [], null), "");
    // 原始条目同样只有思考等级，清空后整体删除并回退内置。
    assert.equal(setModelReasoningLevelsForSlug(initial, "m1", [], null), "");
    // 其他字段仍在时保留条目，只移除思考等级相关键。
    const preserved = setModelReasoningLevelsForSlug(
      JSON.stringify({ m1: { display_name: "M1", supported_reasoning_levels: [{ effort: "low" }] } }),
      "m1",
      [],
      null,
    );
    assert.deepStrictEqual(parseModelMetadataMap(preserved), { m1: { display_name: "M1" } });
  });
});
