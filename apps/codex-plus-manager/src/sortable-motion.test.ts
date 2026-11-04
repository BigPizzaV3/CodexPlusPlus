import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");

test("直连供应商拖动时跟手，松手后有位移动画", () => {
  const card = source.slice(source.indexOf("function SortableRelayProfileCard("), source.indexOf("function RelayProfileDetail("));
  assert.match(card, /transform: CSS\.Transform\.toString\(transform\)/);
  assert.match(card, /transition: isDragging \? "none" : transition \?\? "transform 200ms ease"/);
});

test("模型排序使用稳定行 ID，并在落位时过渡", () => {
  const entry = source.slice(source.indexOf("function SortableModelWindowEntry("), source.indexOf("function relaySectionLinks("));
  assert.match(entry, /transition: sortable\.isDragging \? "none" : sortable\.transition \?\? "transform 200ms ease"/);
  assert.match(source, /items=\{modelWindowRows\.map\(\(row\) => row\.sortId\)\}/);
  assert.match(source, /<SortableModelWindowEntry id=\{row\.sortId\} key=\{row\.sortId\}>/);
  assert.match(source, /findIndex\(\(row\) => row\.sortId === active\.id\)/);
});
