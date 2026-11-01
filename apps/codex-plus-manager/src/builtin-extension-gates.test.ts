import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";

// 改功能开关的内置拓展：停用 / 被顶替时功能侧读成关，设置页仍读用户自己存的值。
const read = (path: string) => readFileSync(new URL(`../../../assets/inject/${path}`, import.meta.url), "utf8");
const style = read("renderer-inject/10-style.js");
const extract = (name: string) => {
  const body = style.match(new RegExp(`^  function ${name}\\([^]*?^  \\}`, "m"))?.[0];
  assert.ok(body, name);
  return body;
};

function settingsContext() {
  const mapping = style.match(/^  const codexPlusBackendSettingMap = \{[^]*?^  \};/m)?.[0];
  assert.ok(mapping);
  const backend: Record<string, unknown> = {
    codexAppSessionDelete: true,
    codexAppMarkdownExport: true,
    codexAppThreadIdBadge: true,
    codexAppThreadScrollRestore: true,
    codexAppConversationView: true,
    codexAppActivityOverlayRight: true,
    codexAppPasteFix: true,
    codexAppStepwiseEnabled: true,
    codexAppAnswerOutlineEnabled: true,
    codexAppPetRealMouseLook: true,
  };
  const context = vm.createContext({
    codexPlusBackendSettings: backend,
    codexPlusSettingsKey: "settings",
    conversationViewDefaultWidth: 960,
    window: {},
    localStorage: { getItem: () => "{}" },
  });
  vm.runInContext([mapping, ...["defaultCodexPlusSettings", "backendCodexPlusSettings", "codexPlusSettings"].map(extract)].join("\n"), context);
  return context;
}

const gated = {
  sessionDelete: "session-delete",
  markdownExport: "markdown-export",
  threadIdBadge: "thread-id-badge",
  threadScrollRestore: "thread-scroll-restore",
  conversationView: "conversation-view",
  activityOverlayRight: "activity-overlay-right",
  pasteFix: "paste-fix",
  stepwise: "stepwise",
  answerOutline: "answer-outline",
  petRealMouseLook: "pet-real-mouse",
};

test("停用的内置拓展让对应设置项在功能侧读成关，其余不受影响", () => {
  const context = settingsContext();
  for (const key of Object.keys(gated)) assert.equal(context.codexPlusSettings()[key], true, key);

  for (const [key, id] of Object.entries(gated)) {
    context.__codexPlusBuiltinInactive = new Set([id]);
    const settings = context.codexPlusSettings();
    assert.equal(settings[key], false, `${id} 停用时 ${key} 应读成关`);
    for (const other of Object.keys(gated)) {
      if (other !== key) assert.equal(settings[other], true, `${id} 停用不该影响 ${other}`);
    }
    // 设置页显示 / 切换的是用户自己存的值。
    assert.equal(context.codexPlusSettings({ ignoreBuiltins: true })[key], true, key);
  }
});

test("设置页的开关读原始设置，不受内置拓展停用影响", () => {
  const menu = read("renderer-inject/20-menu.js");
  const page = read("renderer-inject/40-backend-settings.js");
  assert.match(menu, /function renderCodexPlusMenu\(\) \{\n    const settings = codexPlusSettings\(\{ ignoreBuiltins: true \}\);/);
  assert.match(page, /setCodexPlusSetting\(key, !codexPlusSettings\(\{ ignoreBuiltins: true \}\)\[key\]\)/);
});

test("每个改设置项的内置拓展都登记在内置拓展清单里", () => {
  const catalog = read("renderer-inject/96-builtin-extensions.js");
  for (const id of Object.values(gated)) assert.match(catalog, new RegExp(`id: "${id}"`), id);
});

test("悬浮球的下一步建议 / 回答大纲按停用集合现查", () => {
  const state = read("floating-panel/runtime/state.js");
  const sandbox = vm.createContext({ state: { settings: { enabled: true, answerOutlineEnabled: true } } });
  vm.runInContext(["stepwiseEnabled", "outlineEnabled", "runtimeEnabled"].map((name) => {
    const body = state.match(new RegExp(`^  function ${name}\\([^]*?^  \\}`, "m"))?.[0];
    assert.ok(body, name);
    return body;
  }).join("\n"), sandbox);
  assert.equal(sandbox.runtimeEnabled(), true);
  sandbox.__codexPlusBuiltinInactive = new Set(["stepwise"]);
  assert.equal(sandbox.stepwiseEnabled(), false);
  assert.equal(sandbox.outlineEnabled(), true);
  sandbox.__codexPlusBuiltinInactive = new Set(["stepwise", "answer-outline"]);
  assert.equal(sandbox.runtimeEnabled(), false);

});
