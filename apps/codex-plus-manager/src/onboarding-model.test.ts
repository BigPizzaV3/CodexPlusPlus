import assert from "node:assert/strict";
import test from "node:test";
import { placeCard, shouldShowTour, TOUR_STEPS, TOUR_VERSION } from "./onboarding-model.ts";

test("首次使用和引导升级后自动弹出，看过当前版本就不再弹", () => {
  assert.equal(shouldShowTour(null), true);
  assert.equal(shouldShowTour(String(TOUR_VERSION - 1)), true);
  assert.equal(shouldShowTour("garbage"), true);
  assert.equal(shouldShowTour(String(TOUR_VERSION)), false);
});

test("步骤：第一步是欢迎卡（无目标），其余步骤都有唯一目标", () => {
  assert.equal(TOUR_STEPS[0].target, undefined);
  const targets = TOUR_STEPS.slice(1).map((step) => step.target);
  assert.ok(targets.every(Boolean));
  assert.equal(new Set(targets).size, targets.length);
  assert.equal(new Set(TOUR_STEPS.map((step) => step.id)).size, TOUR_STEPS.length);
});

test("说明卡优先放目标右侧，放不下换左侧，再不行放下方，且不出屏幕", () => {
  const viewport = { width: 1200, height: 800 };
  const card = { width: 320, height: 180 };
  assert.deepEqual(placeCard({ left: 10, top: 100, right: 210, bottom: 130 }, card, viewport), { left: 224, top: 25, side: "right" });
  assert.equal(placeCard({ left: 900, top: 100, right: 1190, bottom: 130 }, card, viewport).side, "left");
  const bottom = placeCard({ left: 100, top: 700, right: 1150, bottom: 760 }, card, viewport);
  assert.equal(bottom.side, "bottom");
  assert.ok(bottom.top + card.height <= viewport.height - 12);
});

test("导览只有一套文案：顺序跟着侧栏走，欢迎卡不提网关，Agent 一步指到侧栏的「本地服务」", () => {
  // 顺序跟着侧栏：万物皆拓展 → Agent → 搜索 → 设置。
  assert.deepEqual(TOUR_STEPS.map((step) => step.id), ["welcome", "management", "agents", "palette", "settings"]);
  assert.doesNotMatch(TOUR_STEPS[0].body, /网关/);
  // 本地服务是侧栏第一组里的一个入口，不在设置里。
  assert.match(TOUR_STEPS.find((step) => step.id === "agents")!.body, /到侧栏的「本地服务」里配置/);
  // 使用模式已经取消，文案里不再有按 Agent 名字填的占位符。
  for (const step of TOUR_STEPS) assert.doesNotMatch(step.title + step.body, /\{\d+\}/);
});
