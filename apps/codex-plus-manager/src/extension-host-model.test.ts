import assert from "node:assert/strict";
import { test } from "node:test";
import {
  claudeParts,
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

test("a package's Claude side splits into the page part and the Code-session part", () => {
  const page = { scripts: ["page.js"], styles: [], methods: [] };
  assert.deepEqual(claudeParts({ package: { claudeDesktop: { page } } }), { page: true, mods: false });
  assert.deepEqual(claudeParts({ package: { claudeDesktop: { buttons: [{ id: "a" }], events: {}, tools: [] } } }), { page: false, mods: true });
  assert.deepEqual(claudeParts({ package: { claudeDesktop: { page, events: { "session.start": "run" } } } }), { page: true, mods: true });
  assert.deepEqual(claudeParts({ package: { claudeDesktop: null } }), { page: false, mods: false });
  assert.deepEqual(claudeParts({}), { page: false, mods: false });
});

test("Claude reports what the Claude page said about a package's page part, and nothing else", () => {
  const page = { scripts: ["page.js"], styles: [], methods: [] };
  const script = { enabled: false, claude_enabled: true, status: "failed", package: { claudeDesktop: { page } } };
  const status = (extra: object) => ({ status: "loaded", error: "", unsupported: [], loadedAt: "", ...extra });
  assert.equal(extensionStateForHost(script, "claude", true, status({})), "loaded");
  assert.equal(extensionStateForHost(script, "claude", true, status({ status: "failed", error: "boom" })), "failed");
  assert.equal(extensionStateForHost(script, "claude", true, status({ error: "page: boom" })), "failed", "加载成功但运行中出错");
  assert.equal(extensionStateForHost(script, "claude", true, status({ status: "pending" })), "notLoaded");
  assert.equal(extensionStateForHost(script, "claude", true, null), "notLoaded");
  assert.equal(extensionStateForHost(script, "claude", false, status({})), "globalOff");
  assert.equal(extensionStateForHost({ ...script, claude_enabled: false }, "claude", true, status({})), "off");
  // 只有 Code 会话那部分的包：页面的回报和它无关。
  const modsOnly = { ...script, package: { claudeDesktop: { buttons: [{ id: "a" }] } } };
  assert.equal(extensionStateForHost(modsOnly, "claude", true, status({})), "notLoaded");
});
