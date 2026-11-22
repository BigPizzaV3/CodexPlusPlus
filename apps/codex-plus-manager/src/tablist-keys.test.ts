import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { nextTabIndex } from "./tablist-keys.ts";

test("页签方向键：左右循环移动，Home / End 到头尾，其它键不处理", () => {
  assert.equal(nextTabIndex("ArrowRight", 0, 3), 1);
  assert.equal(nextTabIndex("ArrowRight", 2, 3), 0);
  assert.equal(nextTabIndex("ArrowLeft", 0, 3), 2);
  assert.equal(nextTabIndex("Home", 2, 3), 0);
  assert.equal(nextTabIndex("End", 0, 3), 2);
  assert.equal(nextTabIndex("Enter", 0, 3), null);
  assert.equal(nextTabIndex("ArrowRight", 0, 0), null);
});

test("用量页的两排页签都接方向键，只有选中的页签在 Tab 序列里；明细行空格也能打开", () => {
  for (const name of ["UsageTabs.tsx", "GatewayUsage.tsx"]) {
    const source = readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
    assert.match(source, /onKeyDown=\{onTabListKeyDown\} role="tablist"/, name);
    assert.match(source, /tabIndex=\{(value === tab\.id|split === id) \? 0 : -1\}/, name);
  }
  const usage = readFileSync(new URL("./GatewayUsage.tsx", import.meta.url), "utf8");
  assert.match(usage, /event\.key === "Enter" \|\| event\.key === " "/);
  assert.match(usage, /data-loading=\{loading \|\| undefined\}/);
});
