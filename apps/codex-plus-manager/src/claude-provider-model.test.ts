import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import {
  CLAUDE_PRESETS,
  applyFormFields,
  duplicateClaudeProvider,
  parseSettingsConfig,
  providerBrief,
  providerFromPreset,
  readFormFields,
  setCompat,
  validateProviderDraft,
  type ClaudeProvider,
} from "./claude-provider-model.ts";

const app = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");

test("表单字段写回配置：空值删键，其它键保留，鉴权变量二选一", () => {
  const config = {
    hooks: { Stop: [] },
    env: { ANTHROPIC_BASE_URL: "https://a.example", ANTHROPIC_AUTH_TOKEN: "sk-a", API_TIMEOUT_MS: "300000" },
  };
  const fields = readFormFields(config);
  assert.equal(fields.authField, "ANTHROPIC_AUTH_TOKEN");
  assert.equal(fields.apiKey, "sk-a");

  const next = applyFormFields(config, { ...fields, authField: "ANTHROPIC_API_KEY", apiKey: " sk-b ", model: "glm-5", baseUrl: "" });
  assert.deepEqual(next.hooks, { Stop: [] });
  assert.deepEqual(next.env, { API_TIMEOUT_MS: "300000", ANTHROPIC_API_KEY: "sk-b", ANTHROPIC_MODEL: "glm-5" });
  assert.equal(readFormFields(next).authField, "ANTHROPIC_API_KEY");
  // 原对象不被改动。
  assert.equal(config.env.ANTHROPIC_AUTH_TOKEN, "sk-a");
});

test("兼容开关按约定值写入和删除", () => {
  const on = setCompat({ env: {} }, "CLAUDE_CODE_AUTO_MODE_SERVER", "0", true);
  assert.deepEqual(on.env, { CLAUDE_CODE_AUTO_MODE_SERVER: "0" });
  assert.deepEqual(setCompat(on, "CLAUDE_CODE_AUTO_MODE_SERVER", "0", false).env, {});
});

test("JSON 编辑框只接受对象，env 值只能是标量", () => {
  assert.equal(parseSettingsConfig("{").ok, false);
  assert.equal(parseSettingsConfig("[]").ok, false);
  assert.equal(parseSettingsConfig('{"env": []}').ok, false);
  assert.equal(parseSettingsConfig('{"env": {"A": {}}}').ok, false);
  const parsed = parseSettingsConfig('{"env": {"A": "1", "B": 2, "C": true}, "model": "x"}');
  assert.ok(parsed.ok);
});

test("预设：id 唯一、地址是 https、不带推广参数，官方预设不写地址", () => {
  const ids = new Set(CLAUDE_PRESETS.map((preset) => preset.id));
  assert.equal(ids.size, CLAUDE_PRESETS.length);
  for (const preset of CLAUDE_PRESETS) {
    assert.doesNotMatch(preset.websiteUrl, /aff=|utm_|ic=|track_id|\/i\//, preset.name);
    const base = preset.env.ANTHROPIC_BASE_URL;
    if (preset.category === "official") assert.equal(base, undefined);
    else assert.match(base ?? "", /^https:\/\//, preset.name);
  }
  const deepseek = providerFromPreset(CLAUDE_PRESETS.find((preset) => preset.id === "deepseek")!);
  assert.equal(deepseek.id, "");
  assert.equal(providerBrief(deepseek).endpoint, "api.deepseek.com/anthropic");
  assert.equal(validateProviderDraft(deepseek), null);
});

test("复制供应商去掉 id 与 CC Switch 来源，配置是深拷贝", () => {
  const source: ClaudeProvider = {
    id: "claude-1", name: "Kimi", websiteUrl: "", notes: "", createdAt: 1, ccsSourceId: "x",
    settingsConfig: { env: { ANTHROPIC_BASE_URL: "https://kimi.example" } },
  };
  const copy = duplicateClaudeProvider(source, "副本");
  assert.equal(copy.id, "");
  assert.equal(copy.name, "Kimi 副本");
  assert.equal(copy.ccsSourceId, undefined);
  (copy.settingsConfig.env as Record<string, string>).ANTHROPIC_BASE_URL = "changed";
  assert.equal(source.settingsConfig.env?.ANTHROPIC_BASE_URL, "https://kimi.example");
});

test("校验：名称必填，Base URL 必须是 http(s)", () => {
  const draft: ClaudeProvider = { id: "", name: " ", websiteUrl: "", notes: "", createdAt: 0, settingsConfig: { env: {} } };
  assert.ok(validateProviderDraft(draft));
  draft.name = "A";
  assert.equal(validateProviderDraft(draft), null);
  draft.settingsConfig = { env: { ANTHROPIC_BASE_URL: "api.example.com" } };
  assert.ok(validateProviderDraft(draft));
});

test("Claude 的供应商配置挂在 Claude 菜单，切到 Claude 先落在这一页", () => {
  assert.match(app, /\{ id: "claudeRelay", get label\(\) \{ return t\("供应商配置"\); \}, icon: KeyRound, tool: "claude" \}/);
  const sections = app.slice(app.indexOf("const navigationSections"), app.indexOf("const globalNavigationRoutes"));
  assert.match(sections, /"claudeRelay", "claude"/);
  assert.match(app, /if \(currentRoute\?\.tool !== "claude"\) setRoute\("claudeRelay"\);/);
  assert.match(app, /route === "claudeRelay" \? \(\s*<ClaudeProviderPage/);
});
