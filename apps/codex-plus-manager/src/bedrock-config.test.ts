import { describe, it } from "node:test";
import * as assert from "node:assert/strict";
import * as fc from "fast-check";
import { bedrockAllowsProviderTesting, bedrockAwsProfileConfigureCommand, bedrockAwsProfileLoginCommand, bedrockLongTermApiKeyCommand, bedrockShortTermApiKeyGuidanceText, bedrockValidationError, bedrockValidationErrorForBottom, deriveBedrockConfigFromConfigText, isBedrockRelayProfile, resolveBedrockAfterDerive, RESERVED_MODEL_PROVIDER_IDS_MIRROR, type BedrockConfig } from "./bedrock-config.ts";

// Feature: amazon-bedrock-provider, Property 12: 测试/诊断按钮可见性判定
// **Validates: Requirements 9.1, 9.2**
describe("Property 12: bedrockAllowsProviderTesting", () => {
  it("returns false iff bedrock exists and authMode === 'awsProfile'", () => {
    const bedrockConfigArb: fc.Arbitrary<BedrockConfig | null | undefined> = fc.oneof(
      // null
      fc.constant(null),
      // undefined
      fc.constant(undefined),
      // bearerToken mode
      fc.record({
        authMode: fc.constant("bearerToken" as const),
        providerId: fc.string(),
        region: fc.string(),
        awsProfile: fc.string(),
        iamUserName: fc.string(),
        iamKeyValidityDays: fc.string(),
      }),
      // awsProfile mode
      fc.record({
        authMode: fc.constant("awsProfile" as const),
        providerId: fc.string(),
        region: fc.string(),
        awsProfile: fc.string(),
        iamUserName: fc.string(),
        iamKeyValidityDays: fc.string(),
      }),
    );

    fc.assert(
      fc.property(bedrockConfigArb, (bedrock) => {
        const profile = { bedrock } as { bedrock?: BedrockConfig | null };
        const result = bedrockAllowsProviderTesting(profile);

        // returns false iff bedrock exists AND authMode === "awsProfile"
        const shouldBeFalse = bedrock != null && bedrock.authMode === "awsProfile";
        assert.strictEqual(result, !shouldBeFalse);
      }),
      { numRuns: 100 },
    );
  });
});


// Feature: amazon-bedrock-provider, Property 8: Bearer Token 生成与识别往返
// **Validates: Requirements 10.2**
describe("Property 8: Bearer Token generation and recognition roundtrip (TS)", () => {
  it("deriveBedrockConfigFromConfigText correctly recognizes bearer token configs", () => {
    // Reserved provider IDs that cannot be used
    const reserved = ["amazon-bedrock", "openai", "ollama", "lmstudio", "oss", "ollama-chat"];

    // Generate non-reserved provider IDs: lowercase letter followed by alphanumeric chars
    const providerIdArb = fc.stringMatching(/^[a-z][a-z0-9]{1,12}$/)
      .filter(id => !reserved.includes(id));
    // Generate non-empty region strings
    const regionArb = fc.stringMatching(/^[a-z][a-z0-9-]{2,15}$/);

    fc.assert(
      fc.property(providerIdArb, regionArb, (providerId, region) => {
        // Construct a config.toml text matching what complete_bedrock_bearer_token_config generates
        const configText = [
          `model_provider = "${providerId}"`,
          `web_search = "disabled"`,
          "",
          `[model_providers.${providerId}]`,
          `name = "${providerId}"`,
          `wire_api = "responses"`,
          `requires_openai_auth = true`,
          `base_url = "https://bedrock-mantle.${region}.api.aws/openai/v1"`,
          `experimental_bearer_token = "some-test-key"`,
          "",
        ].join("\n");

        const result = deriveBedrockConfigFromConfigText(configText);

        assert.notStrictEqual(result, null, "Should recognize as Bedrock config");
        assert.strictEqual(result!.authMode, "bearerToken");
        assert.strictEqual(result!.providerId, providerId);
        assert.strictEqual(result!.region, region);
      }),
      { numRuns: 100 },
    );
  });
});

// Feature: amazon-bedrock-provider, Property 9: AWS Profile 生成与识别往返
// **Validates: Requirements 10.1**
describe("Property 9: AWS Profile generation and recognition roundtrip (TS)", () => {
  it("deriveBedrockConfigFromConfigText correctly recognizes AWS Profile configs", () => {
    // Generate non-empty region strings
    const regionArb = fc.stringMatching(/^[a-z][a-z0-9-]{2,15}$/);
    // Generate AWS profile names (possibly empty)
    const awsProfileArb = fc.stringMatching(/^[a-zA-Z0-9_-]{0,20}$/);

    fc.assert(
      fc.property(regionArb, awsProfileArb, (region, awsProfile) => {
        // Construct a config.toml matching what complete_bedrock_aws_profile_config generates
        const profileLine = awsProfile.trim() ? `profile = "${awsProfile.trim()}"` : "";
        const configLines = [
          `model_provider = "amazon-bedrock"`,
          "",
          `[model_providers.amazon-bedrock.aws]`,
          `region = "${region}"`,
        ];
        if (profileLine) configLines.push(profileLine);
        configLines.push("");
        const configText = configLines.join("\n");

        const result = deriveBedrockConfigFromConfigText(configText);

        assert.notStrictEqual(result, null, "Should recognize as Bedrock config");
        assert.strictEqual(result!.authMode, "awsProfile");
        assert.strictEqual(result!.region, region);
        const expectedProfile = awsProfile.trim() || "";
        assert.strictEqual(result!.awsProfile, expectedProfile);
      }),
      { numRuns: 100 },
    );
  });
});

// 隔离+opt-in 回归保护
describe("deriveBedrockConfigFromConfigText - non-Bedrock configs", () => {
  it("returns null for standard custom provider config", () => {
    const config = `model_provider = "custom"\nmodel = "gpt-4"\n\n[model_providers.custom]\nname = "custom"\nbase_url = "https://example.com"\nwire_api = "responses"\nrequires_openai_auth = true\n`;
    assert.strictEqual(deriveBedrockConfigFromConfigText(config), null);
  });

  it("returns null for OpenAI-style config", () => {
    const config = `model_provider = "openai"\nmodel = "gpt-4"\n\n[model_providers.openai]\nname = "openai"\nbase_url = "https://api.openai.com/v1"\nwire_api = "responses"\nrequires_openai_auth = true\n`;
    assert.strictEqual(deriveBedrockConfigFromConfigText(config), null);
  });

  it("returns null for config without model_provider", () => {
    const config = `model = "some-model"\n\n[model_providers.foo]\nname = "foo"\nbase_url = "https://foo.example.com"\n`;
    assert.strictEqual(deriveBedrockConfigFromConfigText(config), null);
  });

  it("returns null for requires_openai_auth but non-bedrock base_url", () => {
    const config = `model_provider = "myapi"\n\n[model_providers.myapi]\nname = "myapi"\nbase_url = "https://api.myservice.com/v1"\nrequires_openai_auth = true\n`;
    assert.strictEqual(deriveBedrockConfigFromConfigText(config), null);
  });

  it("returns null for empty config", () => {
    assert.strictEqual(deriveBedrockConfigFromConfigText(""), null);
  });
});

// 保留字列表一致性测试
describe("Reserved provider ID list consistency", () => {
  it("TS RESERVED_MODEL_PROVIDER_IDS_MIRROR matches Rust RESERVED_MODEL_PROVIDER_IDS", () => {
    // This list must match the Rust constant in crates/codex-plus-core/src/relay_config.rs
    const rustReservedIds = [
      "amazon-bedrock",
      "openai",
      "ollama",
      "lmstudio",
      "oss",
      "ollama-chat",
    ];

    // Compare as sets (order-independent)
    const tsSet = new Set(RESERVED_MODEL_PROVIDER_IDS_MIRROR);
    const rustSet = new Set(rustReservedIds);

    assert.strictEqual(tsSet.size, rustSet.size, "Lists should have the same length");
    for (const id of rustReservedIds) {
      assert.ok(tsSet.has(id), `TS list is missing "${id}" from Rust list`);
    }
    for (const id of RESERVED_MODEL_PROVIDER_IDS_MIRROR) {
      assert.ok(rustSet.has(id), `TS list has extra "${id}" not in Rust list`);
    }
  });
});


// Regression: 保留字冲突不能同时以 inline + 底部两条红字重复展示。
// `bedrockValidationErrorForBottom` 必须跳过"该标识符与保留供应商冲突"这一类错误，
// 让保留字冲突的错误只由 Provider ID 字段下方的 inline `<p>` 展示；
// 其它字段（region / apiKey）空缺时底部仍需展示。
describe("bedrockValidationErrorForBottom skips reserved-provider-id conflict", () => {
  const reservedBedrock = (region: string): BedrockConfig => ({
    authMode: "bearerToken",
    providerId: "openai", // 命中保留字
    region,
    awsProfile: "",
    iamUserName: "",
    iamKeyValidityDays: "90",
  });

  it("reserved provider id alone: bedrockValidationError reports it, bedrockValidationErrorForBottom returns null", () => {
    const bedrock = reservedBedrock("us-east-2");
    // 完整校验返回保留字冲突文本（保存按钮门控依赖该返回值）
    assert.strictEqual(bedrockValidationError(bedrock, "sk-key"), "该标识符与保留供应商冲突");
    // 底部聚合校验跳过保留字冲突，返回 null（因为其他字段都合法）
    assert.strictEqual(bedrockValidationErrorForBottom(bedrock, "sk-key"), null);
  });

  it("reserved provider id + empty region: bottom surfaces region error, inline shows conflict", () => {
    const bedrock = reservedBedrock("");
    // 完整校验先命中 region 空（region 优先级最高）
    assert.strictEqual(bedrockValidationError(bedrock, "sk-key"), "region 为必填项");
    // 底部聚合校验也能报出 region 空错误（保留字冲突由 inline 单独展示）
    assert.strictEqual(bedrockValidationErrorForBottom(bedrock, "sk-key"), "region 为必填项");
  });

  it("reserved provider id + empty apiKey (region OK): bottom surfaces apiKey error", () => {
    const bedrock = reservedBedrock("us-east-2");
    // 完整校验先命中保留字冲突（在 apiKey 之前）
    assert.strictEqual(bedrockValidationError(bedrock, ""), "该标识符与保留供应商冲突");
    // 底部跳过保留字冲突，报出 apiKey 空错误
    assert.strictEqual(bedrockValidationErrorForBottom(bedrock, ""), "Bedrock API Key 为必填项");
  });

  it("non-reserved provider id: bottom surfaces exactly the same error as full validation", () => {
    const bedrock: BedrockConfig = {
      authMode: "bearerToken",
      providerId: "my-valid-bedrock",
      region: "",
      awsProfile: "",
      iamUserName: "",
      iamKeyValidityDays: "90",
    };
    assert.strictEqual(bedrockValidationError(bedrock, "sk-key"), "region 为必填项");
    assert.strictEqual(bedrockValidationErrorForBottom(bedrock, "sk-key"), "region 为必填项");
  });
});

// Regression: bedrockLongTermApiKeyCommand 对非法天数输入 fallback 到 90，
// 避免把 abc / -5 之类的字符串原样嵌入到示例命令里让用户拷去 shell 报错。
describe("bedrockLongTermApiKeyCommand rejects invalid validity days", () => {
  it("non-numeric days fallback to 90", () => {
    const cmd = bedrockLongTermApiKeyCommand("me", "abc");
    assert.ok(cmd.includes("--credential-age-days 90"), `Expected fallback to 90, got: ${cmd}`);
    assert.ok(!cmd.includes("abc"), `Should not embed non-numeric literal, got: ${cmd}`);
  });

  it("negative-looking days fallback to 90", () => {
    const cmd = bedrockLongTermApiKeyCommand("me", "-5");
    assert.ok(cmd.includes("--credential-age-days 90"), `Expected fallback to 90, got: ${cmd}`);
    assert.ok(!cmd.includes("-5"), `Should not embed negative literal, got: ${cmd}`);
  });

  it("valid positive integer preserved as-is", () => {
    const cmd = bedrockLongTermApiKeyCommand("me", "30");
    assert.ok(cmd.includes("--credential-age-days 30"), `Expected preserved value, got: ${cmd}`);
  });

  it("empty days fallback to 90", () => {
    const cmd = bedrockLongTermApiKeyCommand("me", "");
    assert.ok(cmd.includes("--credential-age-days 90"), `Expected fallback to 90, got: ${cmd}`);
  });

  it("leading-zero days rejected as non-canonical (fallback to 90)", () => {
    // 090 之类非规范正整数虽然形式上是数字，但 create-service-specific-credential 不接受
    const cmd = bedrockLongTermApiKeyCommand("me", "090");
    assert.ok(cmd.includes("--credential-age-days 90"), `Expected fallback to 90, got: ${cmd}`);
  });
});

// Feature: amazon-bedrock-provider, Property 4: Region 必填校验
// **Validates: Requirements 2.4, 5.4**
describe("Property 4: Region required validation (frontend)", () => {
  it("returns error for empty/whitespace region, passes for non-empty region", () => {
    const authModeArb = fc.constantFrom("bearerToken" as const, "awsProfile" as const);
    // Empty or whitespace-only region (empty string or only spaces/tabs/newlines)
    const emptyRegionArb = fc.stringMatching(/^[ \t\n]{0,10}$/);
    // Non-empty region
    const nonEmptyRegionArb = fc.stringMatching(/^[a-z][a-z0-9-]{2,15}$/);

    fc.assert(
      fc.property(authModeArb, emptyRegionArb, (authMode, emptyRegion) => {
        const bedrock = {
          authMode,
          providerId: "my-valid-provider",
          region: emptyRegion,
          awsProfile: "",
          iamUserName: "",
          iamKeyValidityDays: "90",
        };
        const result = bedrockValidationError(bedrock, "valid-api-key");
        assert.notStrictEqual(result, null, `Expected error for empty region "${emptyRegion}" with authMode=${authMode}`);
      }),
      { numRuns: 100 }
    );

    fc.assert(
      fc.property(authModeArb, nonEmptyRegionArb, (authMode, region) => {
        const bedrock = {
          authMode,
          providerId: "my-valid-provider",
          region,
          awsProfile: "",
          iamUserName: "",
          iamKeyValidityDays: "90",
        };
        // For bearerToken, all other required fields are valid
        // For awsProfile, region is the only required field
        const result = bedrockValidationError(bedrock, "valid-api-key");
        // The region check itself should not produce an error
        if (result !== null) {
          // If there IS an error, it should NOT be about region
          assert.ok(!result.includes("region"), `Got unexpected region error: "${result}" for non-empty region "${region}"`);
        }
      }),
      { numRuns: 100 }
    );
  });
});

// Feature: amazon-bedrock-provider, Property 5: Bedrock API Key 必填校验（Bearer Token 路径）
// **Validates: Requirements 3.4**
describe("Property 5: Bedrock API Key required validation (Bearer Token)", () => {
  it("returns error for empty/whitespace API key in bearerToken mode, passes for non-empty", () => {
    // Empty or whitespace-only API key
    const emptyKeyArb = fc.array(fc.constantFrom(" ", "\t", "\n"), { minLength: 0, maxLength: 10 }).map(arr => arr.join(""));
    // Non-empty API key
    const nonEmptyKeyArb = fc.string({ minLength: 1 }).filter(s => s.trim().length > 0);

    fc.assert(
      fc.property(emptyKeyArb, (emptyKey) => {
        const bedrock = {
          authMode: "bearerToken" as const,
          providerId: "my-valid-provider",
          region: "us-east-2",
          awsProfile: "",
          iamUserName: "",
          iamKeyValidityDays: "90",
        };
        const result = bedrockValidationError(bedrock, emptyKey);
        assert.notStrictEqual(result, null, `Expected error for empty API key "${emptyKey}"`);
      }),
      { numRuns: 100 }
    );

    fc.assert(
      fc.property(nonEmptyKeyArb, (apiKey) => {
        const bedrock = {
          authMode: "bearerToken" as const,
          providerId: "my-valid-provider",
          region: "us-east-2",
          awsProfile: "",
          iamUserName: "",
          iamKeyValidityDays: "90",
        };
        const result = bedrockValidationError(bedrock, apiKey);
        // Should pass validation (null means no error)
        assert.strictEqual(result, null, `Got unexpected error: "${result}" for non-empty API key`);
      }),
      { numRuns: 100 }
    );
  });
});

// Feature: amazon-bedrock-provider, Property 6: Long-Term API Key 命令生成
// **Validates: Requirements 7.1, 7.4**
describe("Property 6: Long-Term API Key command generation", () => {
  it("command contains required parts for any non-empty user and days", () => {
    // 非空 IAM 用户名（含空白字符也允许，trim 后非空即可）
    const userArb = fc.stringMatching(/^\s{0,3}[a-zA-Z][a-zA-Z0-9_-]{0,20}\s{0,3}$/);
    // 有效期天数字符串（数字，非空非空白）
    const daysArb = fc.stringMatching(/^\s{0,3}[1-9][0-9]{0,3}\s{0,3}$/);

    fc.assert(
      fc.property(userArb, daysArb, (user, days) => {
        const cmd = bedrockLongTermApiKeyCommand(user, days);
        const trimmedUser = user.trim();
        const trimmedDays = days.trim();
        assert.ok(cmd.includes("aws iam create-service-specific-credential"), `Missing subcommand in: ${cmd}`);
        assert.ok(cmd.includes(`--user-name ${trimmedUser}`), `Missing --user-name in: ${cmd}`);
        assert.ok(cmd.includes(`--credential-age-days ${trimmedDays}`), `Missing --credential-age-days in: ${cmd}`);
      }),
      { numRuns: 100 }
    );
  });
});

// Feature: amazon-bedrock-provider, Requirements 8.4, 8.5: AWS SSO commands concatenation
describe("AWS SSO commands concatenation", () => {
  it("concatenated text contains both aws configure sso and aws sso login lines", () => {
    const configureCmd = bedrockAwsProfileConfigureCommand("my-dev");
    const loginCmd = bedrockAwsProfileLoginCommand("my-dev");
    const combined = `${configureCmd}\n${loginCmd}`;
    assert.ok(
      combined.includes("aws configure sso"),
      `Combined text should contain "aws configure sso", got: ${combined}`,
    );
    assert.ok(
      combined.includes("aws sso login"),
      `Combined text should contain "aws sso login", got: ${combined}`,
    );
  });

  it("empty profile also produces both commands in concatenation", () => {
    const configureCmd = bedrockAwsProfileConfigureCommand("");
    const loginCmd = bedrockAwsProfileLoginCommand("");
    const combined = `${configureCmd}\n${loginCmd}`;
    assert.ok(combined.includes("aws configure sso"));
    assert.ok(combined.includes("aws sso login"));
  });
});

// Feature: amazon-bedrock-provider, Property 7: AWS Profile 配置/登录命令生成
// **Validates: Requirements 8.1, 8.2, 8.3**
describe("Property 7: AWS Profile configure/login command generation", () => {
  it("commands include --profile when non-empty, plain command when empty/whitespace", () => {
    // 任意 profile 名称：可为空、可含空白、可含普通标识符字符
    const profileArb = fc.oneof(
      fc.constant(""),
      fc.stringMatching(/^[ \t\n]{0,5}$/),  // whitespace only
      fc.stringMatching(/^\s{0,3}[a-zA-Z][a-zA-Z0-9_-]{0,20}\s{0,3}$/),  // valid trimmed
    );

    fc.assert(
      fc.property(profileArb, (profile) => {
        const trimmed = profile.trim();
        const configCmd = bedrockAwsProfileConfigureCommand(profile);
        const loginCmd = bedrockAwsProfileLoginCommand(profile);
        if (trimmed) {
          assert.ok(configCmd.includes(`aws configure sso --profile ${trimmed}`),
            `configure command should include --profile ${trimmed}, got: ${configCmd}`);
          assert.ok(loginCmd.includes(`aws sso login --profile ${trimmed}`),
            `login command should include --profile ${trimmed}, got: ${loginCmd}`);
        } else {
          assert.strictEqual(configCmd, "aws configure sso",
            `configure command should be plain for empty profile, got: ${configCmd}`);
          assert.strictEqual(loginCmd, "aws sso login",
            `login command should be plain for empty profile, got: ${loginCmd}`);
        }
      }),
      { numRuns: 100 }
    );
  });
});

// Feature: amazon-bedrock-provider, Requirements 7.2: Short-Term API Key 说明文本
describe("bedrockShortTermApiKeyGuidanceText", () => {
  it("does not contain the Long-Term command and mentions 12 hour validity", () => {
    const text = bedrockShortTermApiKeyGuidanceText();
    assert.ok(
      !text.includes("aws iam create-service-specific-credential"),
      `Short-term guidance should NOT contain the long-term subcommand`,
    );
    assert.ok(text.includes("12"), `Short-term guidance should mention "12" (hours)`);
    assert.ok(text.includes("小时"), `Short-term guidance should mention "小时"`);
  });
});

// Feature: amazon-bedrock-provider, Requirements 9.1, 9.2
// 测试按钮 / Provider Doctor 按钮的可见性在两条 Bedrock 鉴权路径下的差异
// - `SortableRelayProfileCard` 的测试按钮 disabled 表达式：
//     isAggregateRelayProfile(profile) || !bedrockAllowsProviderTesting(profile)
// - `RelayProfileEditor` 对 Bedrock profile 走 <BedrockRelayProfileEditor>（内部不渲染 Provider Doctor）
// 因此只需覆盖以下纯逻辑即可：
//   1) bedrockAllowsProviderTesting(profile) 在 authMode === "awsProfile" 时返回 false（测试按钮禁用）
//   2) bedrockAllowsProviderTesting(profile) 在 authMode === "bearerToken" 时返回 true（测试按钮可用）
//   3) isBedrockRelayProfile(profile) 为 true → 编辑器分支切走，不渲染 Provider Doctor UI
describe("Test button & Provider Doctor visibility per Bedrock authMode", () => {
  it("AWS Profile path: test button predicate returns disabled state", () => {
    const profile = {
      bedrock: {
        authMode: "awsProfile",
        providerId: "",
        region: "us-east-1",
        awsProfile: "default",
        iamUserName: "",
        iamKeyValidityDays: "90",
      } satisfies BedrockConfig,
    };
    // 测试按钮判定：`isAggregateRelayProfile || !bedrockAllowsProviderTesting`
    // 对于非聚合的 AWS Profile Bedrock：!bedrockAllowsProviderTesting === true → 按钮禁用
    assert.strictEqual(
      bedrockAllowsProviderTesting(profile),
      false,
      "AWS Profile mode should NOT allow provider testing",
    );
    // 编辑器分支切走：isBedrockRelayProfile === true → 渲染 BedrockRelayProfileEditor
    // （该组件不渲染 Provider Doctor 按钮）
    assert.strictEqual(
      isBedrockRelayProfile(profile),
      true,
      "Bedrock profile should route to BedrockRelayProfileEditor (no Provider Doctor)",
    );
  });

  it("Bearer Token path: test button predicate returns enabled state", () => {
    const profile = {
      bedrock: {
        authMode: "bearerToken",
        providerId: "my-bedrock",
        region: "us-east-1",
        awsProfile: "",
        iamUserName: "",
        iamKeyValidityDays: "90",
      } satisfies BedrockConfig,
    };
    // 测试按钮判定：bedrockAllowsProviderTesting === true → 按钮可用
    assert.strictEqual(
      bedrockAllowsProviderTesting(profile),
      true,
      "Bearer Token mode should ALLOW provider testing (same as pure API mode)",
    );
    // 编辑器仍然进入 BedrockRelayProfileEditor（同样不渲染 Provider Doctor）
    assert.strictEqual(isBedrockRelayProfile(profile), true);
  });

  it("Non-Bedrock profile: test button predicate depends only on aggregate flag", () => {
    // 模拟纯 API 模式（bedrock 字段为空）
    const profile = { bedrock: null };
    assert.strictEqual(
      bedrockAllowsProviderTesting(profile),
      true,
      "Non-Bedrock profile should allow testing (like pure API)",
    );
    assert.strictEqual(isBedrockRelayProfile(profile), false);
  });
});

// 保护 onSelectBedrock 中间态：
// 在 `deriveRelayProfileFromFiles` 里必须用 `resolveBedrockAfterDerive` 而不是
// 直接 `deriveBedrockConfigFromConfigText`，否则用户刚点 Amazon Bedrock 按钮
// （bedrock 已被 patch 到 profile 上但 config 还没写 Bedrock 特征）时，
// bedrock 会被无条件覆盖成 null，导致编辑器不切换到 BedrockRelayProfileEditor。
describe("resolveBedrockAfterDerive - preserve UI-set bedrock across derive", () => {
  const bedrockMantleConfig = [
    `model_provider = "mantle"`,
    `web_search = "disabled"`,
    "",
    `[model_providers.mantle]`,
    `name = "mantle"`,
    `wire_api = "responses"`,
    `requires_openai_auth = true`,
    `base_url = "https://bedrock-mantle.us-east-2.api.aws/openai/v1"`,
    `experimental_bearer_token = "some-key"`,
    "",
  ].join("\n");

  const genericCustomConfig = [
    `model_provider = "custom"`,
    "",
    `[model_providers.custom]`,
    `name = "custom"`,
    `wire_api = "responses"`,
    `requires_openai_auth = true`,
    `base_url = ""`,
    "",
  ].join("\n");

  const uiPatchedBedrock: BedrockConfig = {
    authMode: null,
    providerId: "",
    region: "",
    awsProfile: "",
    iamUserName: "",
    iamKeyValidityDays: "90",
  };

  it("returns existing bedrock when config lacks Bedrock features (onSelectBedrock initial state)", () => {
    // 复现 bug 场景：用户刚点 Amazon Bedrock 按钮，
    // profile.bedrock 已被 patch 成 {authMode: null, ...} 的中间态，
    // 但 withGeneratedRelayFiles 生成的 config 还是通用 custom 模板。
    const result = resolveBedrockAfterDerive(uiPatchedBedrock, genericCustomConfig);
    assert.notStrictEqual(result, null, "existing bedrock must NOT be overwritten to null");
    assert.strictEqual(result!.authMode, null);
  });

  it("returns existing bedrock when config is empty (fresh new profile)", () => {
    const result = resolveBedrockAfterDerive(uiPatchedBedrock, "");
    assert.notStrictEqual(result, null);
    assert.strictEqual(result!.authMode, null);
  });

  it("returns derived bedrock when config has Bedrock features (magnetic from disk)", () => {
    // 磁盘 config 有 Bedrock 特征时，即便 existing 也有值，也以推导结果为准，
    // 保留「从磁盘反推 profile」的语义。
    const staleExisting: BedrockConfig = {
      authMode: "awsProfile",
      providerId: "",
      region: "eu-west-1",
      awsProfile: "stale",
      iamUserName: "",
      iamKeyValidityDays: "90",
    };
    const result = resolveBedrockAfterDerive(staleExisting, bedrockMantleConfig);
    assert.notStrictEqual(result, null);
    assert.strictEqual(result!.authMode, "bearerToken");
    assert.strictEqual(result!.providerId, "mantle");
    assert.strictEqual(result!.region, "us-east-2");
  });

  it("returns null when neither existing nor config indicate Bedrock (non-Bedrock profile)", () => {
    // 普通 custom provider profile：既没有 UI 侧的 bedrock patch，config 也没有 Bedrock 特征。
    assert.strictEqual(resolveBedrockAfterDerive(null, genericCustomConfig), null);
    assert.strictEqual(resolveBedrockAfterDerive(undefined, genericCustomConfig), null);
    assert.strictEqual(resolveBedrockAfterDerive(null, ""), null);
  });
});
