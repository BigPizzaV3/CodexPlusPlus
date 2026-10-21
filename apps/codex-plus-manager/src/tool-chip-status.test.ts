/**
 * 工具切换按钮（左侧图标栏）不再渲染「运行中」小绿点。
 *
 * 运行状态已经由顶栏的「Codex 运行中」徽标表达，图标角标重复且抢视线，
 * 按用户要求移除。这里直接扫源码，防止角标被无意加回来。
 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const appSource = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
const stylesSource = readFileSync(new URL("./styles.css", import.meta.url), "utf8");

test("工具切换按钮不再渲染运行中小绿点", () => {
  assert.equal(appSource.includes("tool-chip-running-dot"), false);
  assert.equal(stylesSource.includes(".tool-chip-running-dot"), false);
});
