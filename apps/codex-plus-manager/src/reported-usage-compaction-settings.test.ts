import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";

const read = (path: string) => readFileSync(new URL(path, import.meta.url), "utf8");
const source = read("../../../assets/inject/renderer-inject/10-style.js");
const extract = (name: string) => {
  const body = source.match(new RegExp(`^  function ${name}\\([^]*?^  \\}`, "m"))?.[0];
  assert.ok(body, name);
  return body;
};

test("reported usage compaction defaults off and follows backend, master and builtin gates", () => {
  const mapping = source.match(/^  const codexPlusBackendSettingMap = \{[^]*?^  \};/m)?.[0];
  assert.ok(mapping);
  const backend: Record<string, boolean> = {};
  const stored: Record<string, boolean> = {};
  const context = vm.createContext({
    codexPlusBackendSettings: backend,
    codexPlusSettingsKey: "settings",
    conversationViewDefaultWidth: 960,
    window: {},
    localStorage: { getItem: () => JSON.stringify(stored) },
  });
  vm.runInContext([mapping, ...["defaultCodexPlusSettings", "backendCodexPlusSettings", "codexPlusSettings"].map(extract)].join("\n"), context);
  assert.equal(context.codexPlusSettings().reportedUsageCompaction, false);
  backend.codexAppReportedUsageCompaction = true;
  assert.equal(context.codexPlusSettings().reportedUsageCompaction, true);
  context.__codexPlusBuiltinInactive = new Set(["reported-usage-compaction"]);
  assert.equal(context.codexPlusSettings().reportedUsageCompaction, false);
  assert.equal(context.codexPlusSettings({ ignoreBuiltins: true }).reportedUsageCompaction, true);
  context.__codexPlusBuiltinInactive = new Set();
  backend.enhancementsEnabled = false;
  assert.equal(context.codexPlusSettings().reportedUsageCompaction, false);
  backend.enhancementsEnabled = true;
  backend.codexAppReportedUsageCompaction = false;
  stored.reportedUsageCompaction = true;
  assert.equal(context.codexPlusSettings().reportedUsageCompaction, false);
});

test("reported usage compaction has one default-off App and CLI builtin setting with its recovery limit", () => {
  const catalog = JSON.parse(read("../../../crates/codex-plus-core/assets/enhancement-settings.json"));
  const items = catalog.items.filter((item: { key: string }) => item.key === "codexAppReportedUsageCompaction");
  assert.equal(items.length, 1);
  const item = items[0];
  assert.equal(item.builtin, "reported-usage-compaction");
  assert.equal(item.control, "bool");
  assert.equal(item.group, "chat");
  assert.notEqual(item.masterGated, false);
  assert.deepEqual(item.surfaces, ["manager", "codex"]);
  assert.equal(item.title, "按上报用量判断压缩（实验）");
  assert.match(item.description, /仅对 Codex App 和 CLI.*本地代理/);
  assert.match(item.description, /不改 usage.*不自动启用代理/);
  assert.match(item.description, /旧会话首次响应前.*原估算/);
  const builtins = read("../../../assets/inject/renderer-inject/96-builtin-extensions.js");
  assert.match(builtins, /\{ id: "reported-usage-compaction", name: "按上报用量判断压缩（实验）" \}/);
  const app = read("./App.tsx");
  assert.match(app, /codexAppReportedUsageCompaction: boolean;/);
  assert.match(app, /codexAppReportedUsageCompaction: false,/);
});
