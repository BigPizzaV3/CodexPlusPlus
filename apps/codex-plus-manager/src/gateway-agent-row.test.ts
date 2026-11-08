import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const source = readFileSync(new URL("./GatewayAgents.tsx", import.meta.url), "utf8");
const row = source.slice(source.indexOf('className="gw-agent-row cx-row"'));
const handler = row.match(/onClick=\{\(event\) => \{([\s\S]*?)\}\}/)?.[1];
assert.ok(handler, "Agent 行必须有点击处理器");

const click = new Function("event", "connected", "onToggleExpand", handler.replaceAll(" as HTMLElement", ""));

function dispatch({ inside, interactive = false, connected = true }: { inside: boolean; interactive?: boolean; connected?: boolean }) {
  let toggles = 0;
  const target = { closest: () => interactive };
  const currentTarget = { contains: () => inside };
  click({ target, currentTarget }, connected, () => { toggles += 1; });
  return toggles;
}

test("模型选择器 Portal 的点击不能切换 Agent 详情", () => {
  assert.equal(dispatch({ inside: false }), 0);
});

test("Agent 行主体可展开，交互控件和未接入行不触发展开", () => {
  assert.equal(dispatch({ inside: true }), 1);
  assert.equal(dispatch({ inside: true, interactive: true }), 0);
  assert.equal(dispatch({ inside: true, connected: false }), 0);
});
