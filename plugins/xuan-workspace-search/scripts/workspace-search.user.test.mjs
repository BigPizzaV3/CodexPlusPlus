import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";

const source = fs.readFileSync(path.join(import.meta.dirname, "workspace-search.user.js"), "utf8");

test("workspace search script provides project selection, preview and the original shortcut", () => {
  assert.match(source, /\/v1\/search\/start/);
  assert.match(source, /\/v1\/search\/preview/);
  assert.match(source, /\/v1\/search\/cancel/);
  assert.match(source, /Ctrl\+Shift\+F/);
  assert.match(source, /maxResults: 2000/);
  assert.match(source, /搜索项目/);
});

test("workspace search script calls only the independent bridge and never sends credentials", () => {
  assert.match(source, /__XUAN_BRIDGE_URL__/);
  assert.match(source, /workspace-search\/projects/);
  assert.match(source, /工作区全文搜索/);
  assert.doesNotMatch(source, /Authorization\s*:/i);
  assert.doesNotMatch(source, /bearer\s+\$?\{/i);
});
