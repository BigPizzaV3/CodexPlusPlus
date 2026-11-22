import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { it } from "node:test";
import { initialActiveOption, isSelectNavKey, moveActiveOption } from "./app-select-model.ts";

it("方向键在可选项之间移动，跳过禁用项，到头不回绕", () => {
  const disabled = [false, true, false, false, true];
  assert.equal(moveActiveOption(disabled, 0, "ArrowDown"), 2);
  assert.equal(moveActiveOption(disabled, 2, "ArrowDown"), 3);
  assert.equal(moveActiveOption(disabled, 3, "ArrowDown"), 3);
  assert.equal(moveActiveOption(disabled, 3, "ArrowUp"), 2);
  assert.equal(moveActiveOption(disabled, 2, "ArrowUp"), 0);
  assert.equal(moveActiveOption(disabled, 0, "ArrowUp"), 0);
  assert.equal(moveActiveOption(disabled, 2, "Home"), 0);
  assert.equal(moveActiveOption(disabled, 0, "End"), 3);
  assert.equal(moveActiveOption(disabled, -1, "ArrowDown"), 0);
  assert.equal(moveActiveOption(disabled, -1, "ArrowUp"), 3);
  assert.equal(moveActiveOption([true, true], 0, "ArrowDown"), -1);
  assert.equal(isSelectNavKey("End"), true);
  assert.equal(isSelectNavKey("Tab"), false);
});

it("打开时高亮选中项，选中项不可选时高亮第一个可选项", () => {
  assert.equal(initialActiveOption([false, false, false], 2), 2);
  assert.equal(initialActiveOption([true, false, false], 0), 1);
  assert.equal(initialActiveOption([true, false], -1), 1);
});

it("AppSelect 是可键盘操作的列表框：焦点留在按钮上，选项不进 Tab 顺序", () => {
  const source = readFileSync(new URL("./AppSelect.tsx", import.meta.url), "utf8");
  assert.match(source, /role="combobox"/);
  assert.match(source, /aria-activedescendant=\{open && activeIndex >= 0 \? optionId\(activeIndex\) : undefined\}/);
  assert.match(source, /aria-controls=\{open \? listboxId : undefined\}/);
  assert.match(source, /role="listbox"/);
  assert.match(source, /role="option"/);
  assert.match(source, /tabIndex=\{-1\}/);
  assert.match(source, /if \(event\.key === "Enter" \|\| event\.key === " "\)/);
});
