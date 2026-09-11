import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";

const source = fs.readFileSync(path.join(import.meta.dirname, "workspace-search.user.js"), "utf8");

test("workspace search script mounts beside Help and calls bounded bridge routes", () => {
  assert.match(source, /button\[aria-label='帮助'\]/);
  assert.match(source, /insertAdjacentElement\("beforebegin", button\)/);
  assert.match(source, /\/v1\/search\/start/);
  assert.match(source, /\/v1\/search\/preview/);
  assert.match(source, /__codexSessionDeleteBridge/);
  assert.match(source, /maxResults: 200/);
  assert.match(source, /data-app-action-sidebar-thread-active/);
  assert.match(source, /envTooltip/);
});

test("workspace search script keeps credentials out of the UI contract", () => {
  assert.doesNotMatch(source, /authorization|bearer|api[_-]?key/i);
  assert.match(source, /暂时无法搜索当前工作区/);
  assert.doesNotMatch(source, /JSON\.stringify\(body/);
});
