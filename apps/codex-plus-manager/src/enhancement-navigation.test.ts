import assert from "node:assert/strict";
import { test } from "node:test";
import { managerNavigationDestination } from "./enhancement-navigation.ts";

test("old settings links with a service section open the enhancement page", () => {
  for (const section of ["stepwise", "dictation"] as const) {
    const intent = { page: "settings", section } as const;
    assert.deepEqual(managerNavigationDestination(intent), { route: "enhance" });
    assert.deepEqual(intent, { page: "settings", section });
  }
});

test("enhancement links open the enhancement page whatever the section", () => {
  assert.deepEqual(managerNavigationDestination({ page: "enhance" }), { route: "enhance" });
  for (const section of ["stepwise", "dictation"] as const) {
    assert.deepEqual(managerNavigationDestination({ page: "enhance", section }), { route: "enhance" });
  }
});

test("ordinary manager settings links still open settings", () => {
  assert.deepEqual(managerNavigationDestination({ page: "settings" }), { route: "settings" });
});

test("plugin market navigation opens its own page", () => {
  assert.deepEqual(managerNavigationDestination({ page: "pluginMarket" }), { route: "pluginMarket" });
  assert.deepEqual(managerNavigationDestination({ page: "pluginMarket", section: "dictation" }), { route: "pluginMarket" });
});
