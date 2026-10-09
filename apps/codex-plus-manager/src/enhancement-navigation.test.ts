import assert from "node:assert/strict";
import { test } from "node:test";
import { managerNavigationDestination } from "./enhancement-navigation.ts";

test("existing settings links open their complete enhancement configuration", () => {
  for (const section of ["stepwise", "dictation"] as const) {
    const intent = { page: "settings", section } as const;
    assert.deepEqual(managerNavigationDestination(intent), { route: "enhance", section });
    assert.deepEqual(intent, { page: "settings", section });
  }
});

test("enhancement links preserve the requested section", () => {
  assert.deepEqual(managerNavigationDestination({ page: "enhance" }), { route: "enhance", section: null });
  for (const section of ["stepwise", "dictation"] as const) {
    assert.deepEqual(managerNavigationDestination({ page: "enhance", section }), { route: "enhance", section });
  }
});

test("ordinary manager settings links still open settings", () => {
  assert.deepEqual(managerNavigationDestination({ page: "settings" }), { route: "settings", section: null });
});

test("plugin market navigation opens its own page without an enhancement section", () => {
  assert.deepEqual(managerNavigationDestination({ page: "pluginMarket" }), { route: "pluginMarket", section: null });
  assert.deepEqual(managerNavigationDestination({ page: "pluginMarket", section: "dictation" }), { route: "pluginMarket", section: null });
});
