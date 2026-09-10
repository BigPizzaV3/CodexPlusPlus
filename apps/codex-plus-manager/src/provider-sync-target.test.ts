import assert from "node:assert/strict";
import test from "node:test";

import {
  isProviderSyncTargetSelectable,
  preferredProviderSyncTarget,
} from "./provider-sync-target.ts";

test("the exact current provider takes priority over a saved provider", () => {
  const targets = [
    { id: "relay-alpha", isCurrentProvider: true, isResolvable: true },
    { id: "relay-beta", isCurrentProvider: false, isResolvable: true },
  ];

  assert.equal(preferredProviderSyncTarget(targets, "relay-alpha", "relay-beta"), "relay-alpha");
});

test("a resolvable saved provider is used when the current provider is unavailable", () => {
  const targets = [
    { id: "relay-alpha", isCurrentProvider: true, isResolvable: false },
    { id: "relay-beta", isCurrentProvider: false, isResolvable: true },
  ];

  assert.equal(preferredProviderSyncTarget(targets, "relay-alpha", "relay-beta"), "relay-beta");
});

test("history-only and legacy targets fail closed", () => {
  const historyOnly = { id: "relay-history", isCurrentProvider: false, isResolvable: false };
  const legacy = { id: "relay-legacy", isCurrentProvider: false };

  assert.equal(isProviderSyncTargetSelectable(historyOnly), false);
  assert.equal(
    isProviderSyncTargetSelectable(
      legacy as unknown as Parameters<typeof isProviderSyncTargetSelectable>[0],
    ),
    false,
  );
  assert.equal(preferredProviderSyncTarget([historyOnly], "relay-history", "relay-history"), "");
});
