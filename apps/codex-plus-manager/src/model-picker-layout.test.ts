import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const agents = readFileSync(new URL("./GatewayAgents.tsx", import.meta.url), "utf8");
const css = readFileSync(new URL("./GatewayPage.css", import.meta.url), "utf8");
const listSource = agents.slice(agents.indexOf("export function ModelList"), agents.indexOf("/** Agent 行右侧的模型下拉。 */"));

test("模型下拉：左侧竖排图标栏 + 右侧搜索与模型列表，聚合页默认模型选择器不混入直连自带组和断开按钮", () => {
  assert.match(listSource, /className="gw-mp-rail"/);
  assert.match(listSource, /className="gw-mp-main"/);
  assert.match(listSource, /className="gw-mp-foot"/);
  assert.doesNotMatch(listSource, /gw-mp-filters/);
  assert.doesNotMatch(listSource, /gw-via/);
  assert.doesNotMatch(agents, /className="gw-popover-item" data-selected=\{!connected\}/);
  const agentPicker = agents.slice(agents.indexOf("export function AgentModelPicker"), agents.indexOf("function ModelPicker("));
  assert.doesNotMatch(agentPicker, /onPickOwn=/, "聚合页只选本地服务默认模型，不再重复列出「直连自带」同名供应商");
  assert.doesNotMatch(agentPicker, /onDisconnect=/, "聚合页切回直连已有两段开关，不再放无效的「用默认」按钮");
});

test("左侧图标栏竖排可滚动、选中项右侧竖条高亮，搜索栏聚焦底部下划线高亮", () => {
  const rule = (selector: string) => {
    const start = css.indexOf(`${selector} {`);
    assert.notEqual(start, -1, selector);
    return css.slice(start, css.indexOf("}", start));
  };
  const mp = rule(".gw-portal .gw-mp");
  assert.match(mp, /flex-direction: row/);
  const rail = rule(".gw-portal .gw-mp-rail");
  assert.match(rail, /flex-direction: column/);
  assert.match(rail, /overflow-y: auto/);
  const activeBar = rule('.gw-portal .gw-mp-rail-btn[data-active="true"]::after');
  assert.match(activeBar, /background:/);
  const searchFocus = rule(".gw-portal .gw-mp-search:focus-within");
  assert.match(searchFocus, /border-bottom-color:/);
});

test("模型下拉样式挂在 portal 前缀下，生产环境加载顺序里也能压过旧样式", () => {
  for (const name of ["gw-mp-rail", "gw-mp-rail-btn", "gw-mp-main", "gw-mp-item", "gw-mp-foot", "gw-mp-action"]) {
    assert.match(css, new RegExp(`\\.gw-portal \\.${name}\\b`));
  }
});
