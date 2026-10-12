/**
 * @description 自动压缩下拉选项纯函数单测（Node 内置 test runner，与 aggregate-routes.test.ts 同风格）
 * @author Albert_Luo
 * @email 480199976@qq.com
 * @date 2026-10-12
 */

import assert from "node:assert";
import { describe, it } from "node:test";
import { AUTO_COMPACT_PRESETS, autoCompactOptions, DEFAULT_AUTO_COMPACT_PERCENT } from "./auto-compact.ts";

describe("autoCompactOptions", () => {
  it("预设档位按从松到紧排列，并包含默认 90%", () => {
    assert.deepStrictEqual(AUTO_COMPACT_PRESETS, ["50%", "60%", "70%", "75%", "80%", "85%", "90%", "95%"]);
  });

  it("空值回落到默认 90%，不产生重复项", () => {
    const options = autoCompactOptions("");
    assert.deepStrictEqual(options, AUTO_COMPACT_PRESETS);
    assert.strictEqual(new Set(options).size, options.length);
    assert.ok(options.includes(DEFAULT_AUTO_COMPACT_PERCENT));
  });

  it("预设值原样保留", () => {
    assert.deepStrictEqual(autoCompactOptions("80%"), AUTO_COMPACT_PRESETS);
    assert.deepStrictEqual(autoCompactOptions("80"), AUTO_COMPACT_PRESETS);
  });

  it("自定义值追加为最后一项，旧数据不被静默改写", () => {
    const options = autoCompactOptions("87.5");
    assert.deepStrictEqual(options.slice(0, -1), AUTO_COMPACT_PRESETS);
    assert.strictEqual(options.at(-1), "87.5%");
  });
});
