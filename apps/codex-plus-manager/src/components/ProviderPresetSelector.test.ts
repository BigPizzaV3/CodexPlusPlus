import { describe, it } from "node:test";
import * as assert from "node:assert/strict";
import { shouldRenderBedrockPresetButton } from "../bedrock-config.ts";

// 示例单元测试：验证 `ProviderPresetSelector` 中 Bedrock 入口按钮的渲染判定。
//
// 组件本身依赖 `@/i18n` 别名与 JSX，无法在 Node 原生 `--experimental-strip-types`
// 环境下直接加载（Node 不识别 `.tsx`），因此把渲染分支的开关抽成纯函数
// `shouldRenderBedrockPresetButton` 并由组件复用，测试直接验证该函数。
describe("ProviderPresetSelector Bedrock preset button visibility", () => {
  it("returns true when onSelectBedrock is a function (button renders)", () => {
    const callback = () => {};
    assert.strictEqual(shouldRenderBedrockPresetButton(callback), true);
  });

  it("returns false when onSelectBedrock is undefined (button hidden)", () => {
    assert.strictEqual(shouldRenderBedrockPresetButton(undefined), false);
  });

  it("returns true for any function reference regardless of body", () => {
    // 组件调用点只依赖回调是否存在，不关心内部实现
    assert.strictEqual(
      shouldRenderBedrockPresetButton(() => {
        throw new Error("should never be called by the predicate");
      }),
      true,
    );
  });
});
