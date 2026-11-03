import assert from "node:assert/strict";
import { test } from "node:test";
import {
  extensionEnabledForHost,
  extensionHostForTool,
  extensionsForHost,
  extensionStateForHost,
} from "./extension-host-model.ts";

test("shared extension page selects Claude explicitly and keeps the Codex default", () => {
  assert.equal(extensionHostForTool("claude"), "claude");
  for (const tool of ["codex", "grok", "opencode", ""]) {
    assert.equal(extensionHostForTool(tool), "codex");
  }
});

test("Claude lists only declared packages while Codex keeps all existing entries", () => {
  const entries = [
    { key: "user:legacy.js", enabled: true, status: "loaded" },
    { key: "builtin:codex", enabled: true, status: "loaded", package: {} },
    { key: "user:null", enabled: true, status: "failed", package: { claudeDesktop: null } },
    { key: "user:claude", enabled: false, status: "not_loaded", package: { claudeDesktop: { tools: ["status"] } } },
    { key: "user:both", enabled: true, status: "loaded", package: { claudeDesktop: { buttons: ["status"] } } },
  ];
  const before = structuredClone(entries);
  const codex = extensionsForHost(entries, "codex");
  const claude = extensionsForHost(entries, "claude");
  assert.deepEqual(codex.map((entry) => entry.key), entries.map((entry) => entry.key));
  assert.deepEqual(claude.map((entry) => entry.key), ["user:claude", "user:both"]);
  assert.equal(claude[0], entries[3]);
  assert.deepEqual(entries, before);
});

test("each host uses its own extension switch with a legacy inventory fallback", () => {
  const codexOnlyEnabled = { enabled: true, claude_enabled: false, status: "loaded" };
  const claudeOnlyEnabled = { enabled: false, claude_enabled: true, status: "disabled" };
  assert.equal(extensionEnabledForHost(codexOnlyEnabled, "codex"), true);
  assert.equal(extensionEnabledForHost(codexOnlyEnabled, "claude"), false);
  assert.equal(extensionEnabledForHost(claudeOnlyEnabled, "codex"), false);
  assert.equal(extensionEnabledForHost(claudeOnlyEnabled, "claude"), true);
  assert.equal(extensionEnabledForHost({ enabled: true, status: "" }, "claude"), true);
  assert.equal(extensionEnabledForHost({ enabled: false, status: "" }, "claude"), false);
});

test("Claude never reports the Codex runtime status as its own", () => {
  for (const status of ["loaded", "loading", "failed", "disabled", "not_loaded", ""]) {
    const script = { enabled: false, claude_enabled: true, status, error: "Codex runtime failed" };
    assert.equal(extensionStateForHost(script, "claude", true), "notLoaded", status);
    assert.equal(extensionStateForHost(script, "claude", false), "globalOff", status);
    assert.equal(extensionStateForHost({ ...script, claude_enabled: false }, "claude", true), "off", status);
    assert.equal(extensionStateForHost({ ...script, claude_enabled: false }, "claude", false), "off", status);
  }
});

test("Codex status continues to respect its own switch, global setting and runtime", () => {
  const script = { enabled: true, claude_enabled: false, status: "loaded" };
  assert.equal(extensionStateForHost(script, "codex", true), "loaded");
  assert.equal(extensionStateForHost(script, "codex", false), "globalOff");
  assert.equal(extensionStateForHost({ ...script, enabled: false }, "codex", true), "off");
  assert.equal(extensionStateForHost({ ...script, status: "loading" }, "codex", true), "loading");
  assert.equal(extensionStateForHost({ ...script, status: "failed" }, "codex", true), "failed");
  assert.equal(extensionStateForHost({ ...script, status: "not_loaded" }, "codex", true), "notLoaded");
});
