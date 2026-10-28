import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const source = readFileSync(new URL("./GatewayProviders.tsx", import.meta.url), "utf8");
const css = readFileSync(new URL("./GatewayPage.css", import.meta.url), "utf8");

test("供应商池排序用指针事件：桌面端 webview 拦截 HTML5 拖放，draggable 拖不动", () => {
  assert.doesNotMatch(source, /^\s+draggable\b/m);
  assert.doesNotMatch(source, /onDragStart|onDrop=/);
  assert.match(source, /onPointerDown=\{onGripDown\}/);
  assert.match(source, /addEventListener\("pointermove", move\)/);
});

test("供应商行整行点击展开，「添加供应商」在右上角", () => {
  assert.match(source, /className="gw-pool-row"\s+onClick=/);
  assert.match(source, /gw-provider-search-row">[\s\S]*?gw-primary-button[\s\S]*?添加供应商/);
  assert.doesNotMatch(source, /gw-add-row/);
});

test("明细最后一行在正式版样式顺序下也用自适应标签列，不压到按钮", () => {
  assert.match(css, /\.shell \.gw-detail-row\.gw-detail-foot \{\s*grid-template-columns: auto minmax\(0, 1fr\);/);
});
