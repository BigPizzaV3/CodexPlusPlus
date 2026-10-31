import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const app = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");

test("pending provider import is confirmed and dismissed with the fingerprint shown in the dialog", () => {
  const confirm = app.slice(app.indexOf("const confirmPendingProviderImport = async"), app.indexOf("const dismissPendingProviderImport = async"));
  assert.ok(confirm.includes('call<SettingsResult>("confirm_pending_provider_import", { fingerprint })'), "confirm should send the fingerprint");
  // 被新链接覆盖而拒绝时，重新读取 pending 让用户看到新的那条。
  assert.ok(confirm.includes("refreshPendingProviderImport(true)"), "a rejected confirm should reload the pending import");
  const dismiss = app.slice(app.indexOf("const dismissPendingProviderImport = async"), app.indexOf("const refreshLocalSessions = async"));
  assert.ok(dismiss.includes('"dismiss_pending_provider_import", { fingerprint }'), "dismiss should send the fingerprint");
});

test("pool imports warn that the provider starts serving requests right away", () => {
  const dialog = app.slice(app.indexOf("function PendingProviderImportDialog("), app.indexOf("function DreamSkinCommunityLinkDialog("));
  assert.ok(dialog.includes("「添加供应商时自动加入」"), "pool import hint should name the auto-join setting");
  // Windows 检出是 CRLF 行尾，按 \r?\n 匹配。
  assert.match(dialog, /\{toPool \? \(\r?\n\s*<div className="hint-line"/, "hint is only shown for pool imports");
});
