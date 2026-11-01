import assert from "node:assert/strict";
import test from "node:test";
import {
  AGENT_FOCUS_VERSION,
  agentVisible,
  parseAgentFocus,
  serializeAgentFocus,
  shouldAskAgentFocus,
  singleAgent,
  visibleAgents,
} from "./agent-focus-model.ts";

test("没问过、记录无效或旧版本都要问；答过当前版本就不再问", () => {
  assert.equal(shouldAskAgentFocus(null), true);
  assert.equal(shouldAskAgentFocus("garbage"), true);
  assert.equal(shouldAskAgentFocus(JSON.stringify({ version: AGENT_FOCUS_VERSION, mode: "nope" })), true);
  assert.equal(shouldAskAgentFocus(JSON.stringify({ version: AGENT_FOCUS_VERSION - 1, mode: "codex" })), true);
  assert.equal(shouldAskAgentFocus(serializeAgentFocus("codex")), false);
  assert.equal(shouldAskAgentFocus(serializeAgentFocus("all")), false);
});

test("读写来回一致，未知模式当作没问过", () => {
  assert.equal(parseAgentFocus(serializeAgentFocus("codex")), "codex");
  assert.equal(parseAgentFocus(serializeAgentFocus("all")), "all");
  assert.equal(parseAgentFocus(JSON.stringify({ version: 1, mode: "claude" })), null);
  assert.equal(parseAgentFocus("null"), null);
  assert.equal(parseAgentFocus(null), null);
});

test("只管理 Codex 时只显示 Codex；全部功能或没问过时全部显示", () => {
  for (const focus of [null, "all"] as const) {
    assert.equal(agentVisible(focus, "claude"), true);
    assert.equal(agentVisible(focus, "opencode"), true);
  }
  assert.equal(agentVisible("codex", "codex"), true);
  assert.equal(agentVisible("codex", "claude"), false);
  assert.equal(agentVisible("codex", "grok"), false);
  assert.equal(agentVisible("codex", "opencode"), false);
});

test("只管理 Codex 是单 Agent 模式", () => {
  assert.equal(singleAgent("codex"), "codex");
  assert.equal(singleAgent("all"), null);
  assert.equal(singleAgent(null), null);
});

test("按模式过滤列表；全部功能原样返回", () => {
  const items = [{ agent: "codex" }, { agent: "claude" }, { agent: "opencode" }];
  assert.equal(visibleAgents("all", items), items);
  assert.equal(visibleAgents(null, items), items);
  assert.deepEqual(visibleAgents("codex", items), [{ agent: "codex" }]);
});
