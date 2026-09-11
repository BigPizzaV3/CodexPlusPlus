import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";

const source = fs.readFileSync(path.join(import.meta.dirname, "polish-composer.user.js"), "utf8");

test("polish script is a thin feature-detected composer adapter", () => {
  assert.match(source, /MutationObserver/);
  assert.match(source, /data-xuan-polish-button/);
  assert.match(source, /127\.0\.0\.1:57324/);
  assert.doesNotMatch(source, /api[_-]?key|authorization|bearer/i);
});
