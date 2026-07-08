import { describe, it } from "node:test";
import * as assert from "node:assert/strict";
import {
  bedrockShouldRenderPathFields,
  bedrockBearerTokenFieldKeys,
  bedrockAwsProfileFieldKeys,
  bedrockModelNeedsWarning,
  bedrockAwsProfileCommandBlock,
  bedrockValidationError,
  type BedrockConfig,
} from "../bedrock-config.ts";

// 示例单元测试：验证 `BedrockRelayProfileEditor` 的渲染分支与提示逻辑。
//
// 组件本身依赖 `@/i18n` 别名与 JSX，无法在 Node 原生 `--experimental-strip-types`
// 环境下直接加载（Node 不识别 `.tsx`），因此把渲染分支的开关、字段集合、提示可见性、
// 命令拼接等抽成纯函数放在 `bedrock-config.ts`，测试直接验证这些纯函数。
describe("BedrockRelayProfileEditor rendering logic", () => {
  // Requirement 1.2: authMode === null 时不渲染任何路径专属字段
  it("authMode === null: no path-specific fields render", () => {
    const bedrock: BedrockConfig = {
      authMode: null,
      providerId: "",
      region: "us-east-1",
      awsProfile: "",
      iamUserName: "",
      iamKeyValidityDays: "90",
    };
    assert.strictEqual(bedrockShouldRenderPathFields(bedrock), false);
  });

  it("bedrock === null/undefined: no path-specific fields render", () => {
    assert.strictEqual(bedrockShouldRenderPathFields(null), false);
    assert.strictEqual(bedrockShouldRenderPathFields(undefined), false);
  });

  it("authMode === 'bearerToken' triggers path-specific fields rendering", () => {
    const bedrock: BedrockConfig = {
      authMode: "bearerToken",
      providerId: "my-bedrock",
      region: "us-east-1",
      awsProfile: "",
      iamUserName: "",
      iamKeyValidityDays: "90",
    };
    assert.strictEqual(bedrockShouldRenderPathFields(bedrock), true);
  });

  it("authMode === 'awsProfile' triggers path-specific fields rendering", () => {
    const bedrock: BedrockConfig = {
      authMode: "awsProfile",
      providerId: "",
      region: "us-east-1",
      awsProfile: "default",
      iamUserName: "",
      iamKeyValidityDays: "90",
    };
    assert.strictEqual(bedrockShouldRenderPathFields(bedrock), true);
  });

  // Requirement 1.3: Bearer Token 分支字段集合
  it("bearerToken field set matches Requirement 1.3", () => {
    const keys = bedrockBearerTokenFieldKeys();
    assert.deepStrictEqual(
      [...keys].sort(),
      ["apiKey", "iamKeyValidityDays", "iamUserName", "providerId", "region"],
    );
  });

  // Requirement 1.4: AWS Profile 分支字段集合
  it("awsProfile field set matches Requirement 1.4", () => {
    const keys = bedrockAwsProfileFieldKeys();
    assert.deepStrictEqual([...keys].sort(), ["awsProfile", "region"]);
  });

  // Requirement 6.1: 非 openai.* 模型时提示可见，但保存按钮仍可点击
  it("model warning is visible for non-openai.* model but save button remains enabled", () => {
    // 触发提示
    assert.strictEqual(bedrockModelNeedsWarning("anthropic.claude-3"), true);
    assert.strictEqual(bedrockModelNeedsWarning("claude-3-sonnet"), true);
    assert.strictEqual(bedrockModelNeedsWarning("meta.llama3"), true);

    // 保存按钮阻塞只由 bedrockValidationError 决定；非 openai.* 前缀不进入校验
    const bedrock: BedrockConfig = {
      authMode: "awsProfile",
      providerId: "",
      region: "us-east-1",
      awsProfile: "default",
      iamUserName: "",
      iamKeyValidityDays: "90",
    };
    assert.strictEqual(
      bedrockValidationError(bedrock, ""),
      null,
      "AWS Profile mode with valid region should not have validation errors",
    );
  });

  it("openai.* model does not trigger the warning", () => {
    assert.strictEqual(bedrockModelNeedsWarning("openai.gpt-oss-120b-1:0"), false);
    assert.strictEqual(bedrockModelNeedsWarning("openai.gpt-5.5"), false);
  });

  it("empty/whitespace model does not trigger the warning", () => {
    assert.strictEqual(bedrockModelNeedsWarning(""), false);
    assert.strictEqual(bedrockModelNeedsWarning("   "), false);
    assert.strictEqual(bedrockModelNeedsWarning("\t\n"), false);
  });

  // Requirement 8.4: AWS Profile 分支命令文本区域同时包含 configure sso 与 sso login
  it("AWS Profile command block contains both 'aws configure sso' and 'aws sso login'", () => {
    const block = bedrockAwsProfileCommandBlock("my-dev");
    assert.ok(block.includes("aws configure sso"), `Missing 'aws configure sso' in: ${block}`);
    assert.ok(block.includes("aws sso login"), `Missing 'aws sso login' in: ${block}`);
    // 同一段文本同时展示两行（Requirement 8.4）
    assert.ok(block.includes("\n"), `Command block should have both commands on separate lines: ${block}`);
  });

  it("AWS Profile command block with named profile includes --profile flag on both lines", () => {
    const block = bedrockAwsProfileCommandBlock("my-dev");
    assert.ok(block.includes("aws configure sso --profile my-dev"));
    assert.ok(block.includes("aws sso login --profile my-dev"));
  });

  it("AWS Profile command block with empty profile still contains both plain commands", () => {
    const block = bedrockAwsProfileCommandBlock("");
    assert.ok(block.includes("aws configure sso"));
    assert.ok(block.includes("aws sso login"));
    // 空 profile 时不应出现 --profile 参数
    assert.ok(!block.includes("--profile"), `Empty profile should not include --profile flag: ${block}`);
  });
});
