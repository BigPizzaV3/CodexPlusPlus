import assert from "node:assert/strict";
import test from "node:test";
import { agentTabGroups, agentTarget, visibleGroupRoutes } from "./page-groups.ts";

type R = "overview" | "relay" | "proxy" | "claude" | "enhance";
const owner: Record<R, string | undefined> = { overview: undefined, relay: "codex", proxy: "codex", claude: "claude", enhance: "codex" };
const toolOf = (route: R) => owner[route];
const groups: Array<{ id: string; routes: R[] }> = [
  { id: "overview", routes: ["overview"] },
  { id: "providers", routes: ["relay", "proxy"] },
  { id: "enhance", routes: ["claude", "enhance"] },
];

test("按当前 Agent 过滤分组里的页面", () => {
  assert.deepEqual(visibleGroupRoutes(groups[1], toolOf, "codex"), ["relay", "proxy"]);
  assert.deepEqual(visibleGroupRoutes(groups[1], toolOf, "claude"), []);
  assert.deepEqual(visibleGroupRoutes(groups[2], toolOf, "claude"), ["claude"]);
  assert.deepEqual(visibleGroupRoutes(groups[0], toolOf, "grok"), ["overview"]);
});

test("Agent 的页签分组去掉空分组", () => {
  assert.deepEqual(
    agentTabGroups(groups, toolOf, "claude").map((entry) => [entry.group.id, entry.routes]),
    [["overview", ["overview"]], ["enhance", ["claude"]]],
  );
  assert.deepEqual(agentTabGroups(groups, toolOf, "codex").flatMap((entry) => entry.routes), ["overview", "relay", "proxy", "enhance"]);
});

test("点 Agent 回到上次看的页面，看不到时回落脚页", () => {
  assert.equal(agentTarget(["overview", "relay", "proxy"], "proxy", "overview"), "proxy");
  assert.equal(agentTarget(["overview", "relay"], undefined, "overview"), "overview");
  assert.equal(agentTarget(["overview", "claude"], "enhance", "claude"), "claude", "记住的页面不属于这个 Agent 时回落脚页");
});
