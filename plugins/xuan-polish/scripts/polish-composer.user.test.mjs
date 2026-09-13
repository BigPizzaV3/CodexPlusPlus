import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";

const source = fs.readFileSync(path.join(import.meta.dirname, "polish-composer.user.js"), "utf8");

test("polish script keeps the original composer workflow in an independent bridge adapter", () => {
  assert.match(source, /MutationObserver/);
  assert.match(source, /\/v1\/polish/);
  assert.match(source, /停止/);
  assert.match(source, /恢复/);
  assert.match(source, /Ctrl\+Enter|ctrlKey/);
  assert.match(source, /127\.0\.0\.1:57324/);
  assert.match(source, /润色中|正在润色|loading/);
});

test("polish script keeps Ctrl+Enter as a toggle shortcut", () => {
  assert.match(source, /if \(event\.ctrlKey && !event\.metaKey\) return true;/);
  assert.match(source, /const activeElement = document\.activeElement;/);
  assert.match(source, /function onPromptOptimizeShortcut\(event\) \{\s*if \(runtime\.disposed\) return;/);
});

test("polish script supports cancellation, restore state and settings without exposing credentials", () => {
  assert.match(source, /optimizeToken|AbortController/);
  assert.match(source, /promptOptimizeState/);
  assert.match(source, /\/v1\/polish\/settings/);
  assert.doesNotMatch(source, /Authorization\s*:/i);
  assert.doesNotMatch(source, /bearer\s+\$?\{/i);
});
