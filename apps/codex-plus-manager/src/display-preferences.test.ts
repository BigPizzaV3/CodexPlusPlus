import assert from "node:assert/strict";
import test from "node:test";
import { normalizeDisplayPreferences, zoomOptions } from "./display-preferences.ts";

test("管理器显示偏好兼容旧安装与无效值", () => {
  for (const value of [null, {}, { zoom: 0, density: "other" }]) {
    assert.deepEqual(normalizeDisplayPreferences(value), { zoom: 100, density: "standard" });
  }
  for (const zoom of zoomOptions) for (const density of ["compact", "standard", "comfortable"]) {
    assert.deepEqual(normalizeDisplayPreferences({zoom, density}), {zoom, density});
  }
});
