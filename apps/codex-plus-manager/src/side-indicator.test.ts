/** 侧栏选中色块的位置计算。 */
import assert from "node:assert/strict";
import test from "node:test";
import { easeOut, indicatorFrame } from "./side-indicator.ts";

test("色块从起点滑到终点：两端精确落位，中途在两者之间且先快后慢", () => {
  const from = { x: 0, y: 40, width: 200, height: 32 };
  const to = { x: 28, y: 240, width: 172, height: 28 };
  assert.deepEqual(indicatorFrame(from, to, 0), from);
  assert.deepEqual(indicatorFrame(from, to, 1), to);
  const half = indicatorFrame(from, to, 0.5);
  // 缓出：时间过半时已经走了大半程。
  assert.ok(half.y > 140 && half.y < 240);
  assert.ok(half.width < 200 && half.width > 172);
});

test("进度越界时夹到 0–1，不会滑过头", () => {
  assert.equal(easeOut(-1), 0);
  assert.equal(easeOut(2), 1);
  const from = { x: 0, y: 0, width: 10, height: 10 };
  const to = { x: 0, y: 100, width: 10, height: 10 };
  assert.deepEqual(indicatorFrame(from, to, 5), to);
});
