import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const source = readFileSync(new URL("./GatewayAgents.tsx", import.meta.url), "utf8");

test("Agent 接入：状态用短标签、整行点击展开", () => {
  const row = source.slice(source.indexOf('className="gw-agent-row"'), source.indexOf("<ModelPicker", source.indexOf('className="gw-agent-row"')));
  assert.match(row, /className="gw-agent-badge"/);
  assert.match(row, /onToggleExpand\(\);/);
  assert.doesNotMatch(source, /打开「接入」后|的模型列表里有 \{1\} 个模型|tf\("模型 \{0\}"/);
});

test("模型下拉底部只有一个「用默认（不经 Codex++）」：它就是断开并还原，不再重复放一个同效果的按钮", () => {
  const source = readFileSync(new URL("./GatewayAgents.tsx", import.meta.url), "utf8");
  assert.doesNotMatch(source, /t\("断开并还原配置"\)/);
  assert.equal(source.match(/onClick=\{onDisconnect\}/g)?.length, 2, "下拉里一个、展开详情里一个");
});
