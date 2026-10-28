import assert from "node:assert/strict";
import { readdirSync } from "node:fs";
import test from "node:test";
import { BRAND_ICON_KEYS, brandIconIsColor, brandIconKey } from "./provider-brands.ts";

test("按名字或地址识别厂商图标", () => {
  assert.equal(brandIconKey("DeepSeek"), "deepseek-color");
  assert.equal(brandIconKey("Zhipu GLM"), "zhipu-color");
  assert.equal(brandIconKey("Z.ai"), "zai");
  assert.equal(brandIconKey("Kimi (China)"), "kimi-color");
  assert.equal(brandIconKey("哈基米 国模", "https://api.moonshot.cn/v1"), "kimi-color");
  assert.equal(brandIconKey("Anthropic"), "claude-color");
  assert.equal(brandIconKey("Grok (SuperGrok)"), "grok");
  assert.equal(brandIconKey("xAI"), "xai");
  assert.equal(brandIconKey("LM Studio"), "lmstudio");
  assert.equal(brandIconKey("火山引擎 Ark"), "volcengine-color");
  assert.equal(brandIconKey("我的中转"), null);
  assert.equal(brandIconIsColor("deepseek-color"), true);
  assert.equal(brandIconIsColor("openai"), false);
});

test("规则里用到的图标都已打包", () => {
  const files = new Set(readdirSync(new URL("./assets/providers/", import.meta.url)).map((name) => name.replace(/\.svg$/, "")));
  for (const key of BRAND_ICON_KEYS) assert.ok(files.has(key), key);
});
