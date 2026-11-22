import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const app = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
const catalog = JSON.parse(readFileSync(new URL("../../../crates/codex-plus-core/assets/enhancement-settings.json", import.meta.url), "utf8")) as {
  groups: Array<{ id: string; title: string }>;
  items: Array<{ key: string; group: string }>;
};

test("Codex 用量挂件不再属于增强页", () => {
  // 原「Codex 用量挂件」（含余额配置）已移出增强页，改为独立的「Agent 桌宠」页。
  const pet = catalog.groups.find((group) => group.title === "桌宠与活动");
  assert.ok(pet);
  const petKeys = catalog.items.filter((item) => item.group === pet.id).map((item) => item.key);
  assert.ok(petKeys.includes("codexAppActivityOverlayRight"));
  assert.ok(!petKeys.includes("codexAppWhaleWidgetEnabled"), "Codex 用量挂件不再属于增强页");
  const start = app.indexOf("function EnhanceScreen(");
  assert.ok(start >= 0);
  const enhance = app.slice(start, app.indexOf("\n}\n", start));
  assert.doesNotMatch(enhance, /Codex 用量挂件|WhaleBalanceSettingsFields/);
});

test("the old enhancement tabs, service panels and built-in extension cards leave no styles behind", () => {
  // 这些界面已经搬进 Codex 的 Codex++ 设置页，管理器里不再渲染，对应样式一并删掉。
  const sheets = ["./styles.css", "./cursor-ui.css", "./cx-pages-agents.css"]
    .map((path) => readFileSync(new URL(path, import.meta.url), "utf8")).join("\n");
  for (const name of ["enhance-tabs", "enhance-tab-panel", "enhance-feature-groups", "enhance-service-panel",
    "stepwise-settings-block", "stepwise-advanced", "cx-enhance-pane", "builtin-extensions-title", "feature-switch-grid"]) {
    assert.doesNotMatch(sheets, new RegExp(`\\.${name}(?![\\w-])`), name);
  }
});
