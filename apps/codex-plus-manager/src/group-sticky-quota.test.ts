import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import {
  formatRemaining,
  globMatches,
  groupModeLabel,
  groupStickyLabel,
  memberLabel,
  patternMatches,
  quotaRemaining,
  quotaWindowLabel,
  speedOf,
  type GatewayPayload,
} from "./gateway-model.ts";

const read = (name: string) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");

function payload(): GatewayPayload {
  return {
    gateway: { version: 1, bindings: {}, keyPolicy: { mode: "sticky" }, restoreOnExit: false },
    port: 1,
    helperRunning: true,
    pool: [
      { id: "openrouter", name: "OpenRouter", baseUrl: "https://openrouter.ai/api/v1", protocol: "chatCompletions", official: false, models: ["meta/llama-4:free", "anthropic/claude-opus-5.5"], keyCount: 1, enabled: true },
      { id: "zen", name: "Zen", baseUrl: "https://zen.test/v1", protocol: "chatCompletions", official: false, models: ["big-pickle-free"], keyCount: 1, enabled: true },
      { id: "off", name: "Off", baseUrl: "https://off.test/v1", protocol: "chatCompletions", official: false, models: ["x:free"], keyCount: 1, enabled: false },
    ],
    groups: [],
    agents: [],
    health: { keys: [], providers: [] },
    speeds: [{ relayId: "zen", model: "big-pickle-free", ms: 820, samples: 3 }],
    codexProviders: [],
    claudeProviders: [],
    grokProfiles: [],
  };
}

test("通配符与后端一致：* 匹配任意字符（含 /），不区分大小写；按「供应商 id/模型」或模型名匹配，关掉的供应商不算", () => {
  assert.equal(globMatches("openrouter/*:free", "OpenRouter/meta/llama-4:free"), true);
  assert.equal(globMatches("a.b*", "axb"), false, "点号不是正则");
  assert.equal(globMatches("deepseek", "deepseek-v4"), false);
  assert.deepEqual(patternMatches(payload(), "*:free"), ["openrouter/meta/llama-4:free"]);
  assert.deepEqual(patternMatches(payload(), "*free"), ["openrouter/meta/llama-4:free", "zen/big-pickle-free"]);
  assert.equal(memberLabel(payload(), "*free"), "*free", "通配符成员原样显示");
});

test("额度：取最紧的窗口，过了重置时间按满额；窗口名", () => {
  const quota = { windows: [{ windowMinutes: 300, usedPercent: 30, resetsAtMs: 1000 }, { windowMinutes: 10080, usedPercent: 85, resetsAtMs: 5000 }], updatedAtMs: 0 };
  assert.equal(Math.round(quotaRemaining(quota, 0) * 100), 15);
  assert.equal(Math.round(quotaRemaining(quota, 6000) * 100), 100);
  assert.deepEqual(quotaWindowLabel(quota.windows[0]), { text: "5 小时", args: [] });
  assert.deepEqual(quotaWindowLabel({ windowMinutes: null, usedPercent: 0, resetsAtMs: null }), { text: "本月", args: [] });
  assert.deepEqual(quotaWindowLabel({ windowMinutes: 2880, usedPercent: 0, resetsAtMs: null }), { text: "{0} 天", args: [2] });
  assert.equal(formatRemaining(3 * 86_400_000 + 5 * 3_600_000), "3d5h", "每周额度的重置时间写到天");
});

test("速度：按供应商和小写模型名查", () => {
  assert.equal(speedOf(payload(), "zen", "Big-Pickle-Free"), 820);
  assert.equal(speedOf(payload(), "openrouter", "x"), null);
});

test("模型组编辑器：五种模式、三种会话固定、通配符成员；供应商池显示额度并能刷新", () => {
  assert.equal(groupModeLabel("usage"), "额度优先");
  assert.equal(groupModeLabel("fast"), "速度优先");
  assert.equal(groupStickyLabel(undefined), "缓存期内固定", "旧数据没有 sticky 时按默认");
  const page = read("GatewayPage.tsx");
  assert.match(page, /\{ value: "usage", label: t\("额度优先"\) \}/);
  assert.match(page, /\{ value: "fast", label: t\("速度优先"\) \}/);
  assert.match(page, /const stickies: GroupSticky\[\] = \["cache", "session", "off"\]/);
  assert.match(page, /onSave\(\{ id: group\?\.id \?\? "", name: name\.trim\(\), mode, sticky, members \}\)/);
  assert.match(page, /patternMatches\(payload, member\.model\)/);
  const providers = read("GatewayProviders.tsx");
  assert.match(providers, /"refresh_gateway_quota", \{ id: entry\.id \}/);
  assert.match(providers, /<QuotaBadge quota=\{entry\.quota\} \/>/);
  assert.match(read("../src-tauri/src/lib.rs"), /gateway_commands::refresh_gateway_quota,/);
});
