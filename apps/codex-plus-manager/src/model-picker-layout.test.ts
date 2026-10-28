import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const agents = readFileSync(new URL("./GatewayAgents.tsx", import.meta.url), "utf8");
const css = readFileSync(new URL("./GatewayPage.css", import.meta.url), "utf8");
const listSource = agents.slice(agents.indexOf("export function ModelList"), agents.indexOf("/** Agent 行右侧的模型下拉。 */"));

test("模型下拉：横向筛选替代左侧图标栏，操作放在底部，不再每行重复「经 Codex++」", () => {
  assert.match(listSource, /className="gw-mp-filters"/);
  assert.match(listSource, /className="gw-mp-foot"/);
  assert.doesNotMatch(listSource, /gw-popover-rail/);
  assert.doesNotMatch(listSource, /gw-via/);
  assert.doesNotMatch(agents, /className="gw-popover-item" data-selected=\{!connected\}/);
});

test("模型下拉样式挂在 portal 前缀下，生产环境加载顺序里也能压过旧样式", () => {
  for (const name of ["gw-mp-filters", "gw-mp-item", "gw-mp-foot", "gw-mp-action"]) {
    assert.match(css, new RegExp(`\\.gw-portal \\.${name}\\b`));
  }
});
