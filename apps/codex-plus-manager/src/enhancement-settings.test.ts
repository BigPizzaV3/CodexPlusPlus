import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import ts from "typescript";
import {
  enhancementCatalog,
  enhancementCatalogStrings,
  enhancementGroup,
  enhancementItem,
  isEnhancementItemVisible,
  managerBuiltinCards,
  managerStandaloneGroup,
  type EnhancementPlatform,
} from "./enhancement-catalog.ts";
import { EN_PLAIN } from "./i18n-en.ts";

const appSource = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
const app = ts.createSourceFile("App.tsx", appSource, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);

function descendants(node: ts.Node): ts.Node[] {
  const result: ts.Node[] = [];
  const visit = (child: ts.Node) => {
    result.push(child);
    ts.forEachChild(child, visit);
  };
  visit(node);
  return result;
}

function component(name: string): ts.FunctionDeclaration {
  const declaration = app.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === name);
  assert.ok(declaration && ts.isFunctionDeclaration(declaration), `missing ${name}`);
  return declaration;
}

function elements(node: ts.Node): Array<ts.JsxOpeningElement | ts.JsxSelfClosingElement> {
  return descendants(node).filter((child): child is ts.JsxOpeningElement | ts.JsxSelfClosingElement =>
    ts.isJsxOpeningElement(child) || ts.isJsxSelfClosingElement(child),
  );
}

function attribute(element: ts.JsxOpeningElement | ts.JsxSelfClosingElement, name: string): string | undefined {
  const property = element.attributes.properties.find((node) => ts.isJsxAttribute(node) && node.name.getText(app) === name);
  if (!property || !ts.isJsxAttribute(property)) return undefined;
  const initializer = property.initializer;
  if (initializer && ts.isJsxExpression(initializer)) return initializer.expression?.getText(app);
  return initializer && ts.isStringLiteral(initializer) ? initializer.text : undefined;
}

function fieldControl(node: ts.Node, expression: string) {
  const control = elements(node).find((element) =>
    attribute(element, "value") === expression || attribute(element, "checked") === expression,
  );
  assert.ok(control, `missing editable control for ${expression}`);
  assert.ok(attribute(control, "onChange"), `${expression} must remain editable`);
  return control;
}

function isConditional(node: ts.Node, boundary: ts.Node): boolean {
  for (let parent = node.parent; parent && parent !== boundary; parent = parent.parent) {
    if (ts.isConditionalExpression(parent) || (ts.isBinaryExpression(parent)
      && [ts.SyntaxKind.AmpersandAmpersandToken, ts.SyntaxKind.BarBarToken].includes(parent.operatorToken.kind))) return true;
  }
  return false;
}

function eventHandlerNodes(component: ts.Node, element: ts.JsxOpeningElement | ts.JsxSelfClosingElement, event: string): ts.Node[] {
  const property = element.attributes.properties.find((node) => ts.isJsxAttribute(node) && node.name.getText(app) === event);
  assert.ok(property && ts.isJsxAttribute(property) && property.initializer && ts.isJsxExpression(property.initializer));
  const expression = property.initializer.expression;
  assert.ok(expression, `missing ${event} handler`);
  const result = descendants(expression);
  const visited = new Set<string>();
  for (let index = 0; index < result.length; index += 1) {
    const node = result[index];
    if (!ts.isCallExpression(node) || !ts.isIdentifier(node.expression) || visited.has(node.expression.text)) continue;
    visited.add(node.expression.text);
    const declaration = descendants(component).find((candidate) => ts.isVariableDeclaration(candidate)
      && ts.isIdentifier(candidate.name) && candidate.name.text === node.expression.getText(app));
    if (declaration && ts.isVariableDeclaration(declaration) && declaration.initializer) result.push(...descendants(declaration.initializer));
  }
  return result;
}

test("retired enhancements have no settings, navigation or command references in the manager UI", () => {
  const retired = new Set([
    "codexAppZedRemoteOpen", "zedRemoteOpenStrategy", "zedRemoteProjectRegistryEnabled", "zedRemoteSyncToZedSettings",
    "codexAppUpstreamWorktreeCreate", "codexAppNativeMenuLocalization",
    "codexAppFastStartup", "快速启动",
    "codexAppServiceTierControls", "Fast 按钮",
    "ZedRemoteScreen", "ZedRemoteProjectSection", "zedRemote", "list_zed_remote_projects", "open_zed_remote", "forget_zed_remote_project",
    "launchMode", "LaunchMode", "ModeSelector", "saveLaunchMode", "setLaunchMode", "patchMode",
    "repairPluginMarketplace", "refreshRemotePluginMarketplace", "repairRemotePluginMarketplace",
    "PluginMarketplaceRepairResult", "PluginMarketplaceStatusResult", "RemotePluginMarketplaceResult",
    "pluginMarketplaceProgress", "remotePluginMarketplace", "remotePluginMarketplaceProgress",
    "setPluginMarketplaceProgress", "setRemotePluginMarketplace", "setRemotePluginMarketplaceProgress",
    "repair_plugin_marketplace", "plugin_marketplace_status", "remote_plugin_marketplace_status", "repair_remote_plugin_marketplace",
    "修复插件市场", "官方远端插件缓存", "插件市场修复进度", "官方远端插件缓存进度", "释放并注册内置缓存",
  ]);
  const references = descendants(app)
    .filter((node) => ts.isIdentifier(node) || ts.isStringLiteral(node))
    .map((node) => (node as ts.Identifier | ts.StringLiteral).text)
    .filter((name) => retired.has(name));
  assert.deepEqual([...new Set(references)], []);
});

const ENHANCEMENT_COMPONENTS = ["EnhanceScreen", "EnhancementSettingControl"];
// 管理器里只留增强总开关、独立模式补丁（应急）和会话维护页的两个开关；其余都在 Codex 的 Codex++ 设置页里。
const MANAGER_KEYS = ["enhancementsEnabled", "sessionIndexRepairEnabled", "providerSyncEnabled", "codexAppStandaloneQuotaUnlock"];

const servicePage = await readFile(new URL("../../../assets/inject/renderer-inject/46-service-settings.js", import.meta.url), "utf8");
const catalogPage = await readFile(new URL("../../../assets/inject/renderer-inject/40-backend-settings.js", import.meta.url), "utf8");

function settingControls(node: ts.Node) {
  return elements(node).filter((element) => element.tagName.getText(app) === "EnhancementSettingControl");
}

function functionBody(source: string, name: string): string {
  const start = source.indexOf(`function ${name}(`);
  assert.ok(start >= 0, `missing ${name}`);
  const end = source.indexOf("\n  }\n", start);
  return source.slice(start, end);
}

test("unified enhancements use the master switch and Chinese suggestion labels", () => {
  // 开关统一经 EnhancementSettingControl 渲染：受总开关控制的项在增强关闭时禁用。
  const control = component("EnhancementSettingControl");
  const disabled = descendants(control).find((node) => ts.isVariableDeclaration(node) && node.name.getText(app) === "disabled");
  assert.ok(disabled && ts.isVariableDeclaration(disabled));
  assert.equal(disabled.initializer?.getText(app), "isEnhancementItemMasterGated(item) && !masterEnabled");
  for (const element of elements(control).filter((candidate) => ["FeatureToggle", "AppSelect"].includes(candidate.tagName.getText(app)))) {
    assert.equal(attribute(element, "disabled"), "disabled");
  }
  const placements = settingControls(component("EnhanceScreen"));
  assert.ok(placements.length > 0, "EnhanceScreen renders the standalone catalog settings");
  for (const placement of placements) assert.equal(attribute(placement, "masterEnabled"), "masterEnabled");
  for (const item of enhancementCatalog.items) {
    const standalone = enhancementGroup(item.group).standalone === true;
    // 启动前的会话维护与拓展权限不是内置拓展，总开关管不着。
    const unmanaged = ["enhancementsEnabled", "providerSyncEnabled", "sessionIndexRepairEnabled", "usageHistoryEnabled"].includes(item.key);
    assert.equal(item.masterGated !== false, !standalone && !unmanaged, `${item.key} master gating`);
  }

  const petGroup = enhancementGroup("pet");
  assert.equal(petGroup.title, "桌宠与活动");
  assert.equal(petGroup.description, "调整 Codex 原生桌宠与活动进度条。可点按的 Agent 桌宠在「工具 → Agent 桌宠」里设置。");
  const visible = (platform: EnhancementPlatform) => enhancementCatalog.items
    .filter((item) => item.group === "pet" && isEnhancementItemVisible(item, "codex", platform)).map((item) => item.key);
  for (const platform of ["windows", "macos"] as const) {
    assert.ok(visible(platform).includes("codexAppPetRealMouseLook"), "native pet mouse tracking must be available on Windows and macOS");
  }
  assert.ok(!visible("linux").includes("codexAppPetRealMouseLook"));
  for (const platform of ["windows", "macos", "linux"] as const) {
    assert.ok(visible(platform).includes("codexAppActivityOverlayRight"), "the native activity overlay mirror belongs with the pet layout settings");
  }
  // Agent 桌宠是独立页面，原「Codex 用量挂件」也已移出增强页，清单里不能再出现。
  assert.ok(!component("EnhanceScreen").getText(app).includes("agentPet"), "Agent 桌宠是独立页面，不再放在 Codex 增强里");
  assert.ok(!enhancementCatalog.items.some((item) => /WhaleWidget|AgentPet/.test(item.key)));

  const visibleEnglish = descendants(app).filter((node) => ts.isStringLiteral(node)
    || ts.isNoSubstitutionTemplateLiteral(node) || ts.isJsxText(node)).filter((node) => /Stepwise/.test(node.getText(app)));
  assert.deepEqual(visibleEnglish.map((node) => node.getText(app)), [], "internal Stepwise fields stay compatible but visible names must use Chinese");
  for (const text of enhancementCatalogStrings()) assert.doesNotMatch(text, /Stepwise/);
  assert.match(servicePage, /<h2 class="codex-plus-settings-section-title">下一步建议服务<\/h2>/);
  // 页面上看得到的字符串（带空格或中文的字面量）不出现 Stepwise；字段名、函数名不算。
  const pageSource = ts.createSourceFile("46-service-settings.js", servicePage, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);
  const pageTexts = descendants(pageSource).filter((node) => ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)
    || ts.isTemplateHead(node) || ts.isTemplateMiddle(node) || ts.isTemplateTail(node)).map((node) => (node as ts.LiteralLikeNode).text);
  assert.ok(pageTexts.length > 50);
  assert.deepEqual(pageTexts.filter((text) => /[\s\u4e00-\u9fff]/.test(text) && /Stepwise/.test(text.replace(/data-codex-plus-stepwise-[\w-]+|codexAppStepwise\w+/g, ""))), []);
  assert.equal(enhancementGroup("stepwise").title, "下一步建议");
  assert.equal(enhancementItem("codexAppStepwiseDirectSend").title, "建议直接发送");
});

test("enhancement switches come only from the settings catalog", () => {
  // 管理器里不能再写死一份开关文案：清单中的标题/说明不得以 t("字面量") 出现在增强页组件里。
  const catalogStrings = new Set(enhancementCatalogStrings());
  for (const name of ENHANCEMENT_COMPONENTS) {
    const node = component(name);
    assert.deepEqual(elements(node).filter((element) => element.tagName.getText(app) === "FeatureToggle"
      && /^t\("/.test(attribute(element, "title") ?? "")).map((element) => attribute(element, "title")), [], `${name} has a hard-coded switch`);
    const literals = descendants(node).filter(ts.isCallExpression)
      .filter((call) => call.expression.getText(app) === "t" && call.arguments[0] && ts.isStringLiteral(call.arguments[0]))
      .map((call) => (call.arguments[0] as ts.StringLiteral).text)
      .filter((text) => catalogStrings.has(text));
    assert.deepEqual(literals, [], `${name} duplicates catalog text`);
  }
  for (const item of enhancementCatalog.items) {
    assert.ok(!appSource.includes(`t("${item.description}")`), `${item.key} description is duplicated in App.tsx`);
  }
  // 会话维护页的自动修复开关与 Codex 设置页同一条文案。
  assert.match(appSource, /const providerSyncItem = enhancementItem\("providerSyncEnabled"\);/);
  assert.match(appSource, /<SettingRow title=\{t\(providerSyncItem\.title\)\} description=\{t\(providerSyncItem\.description\)\}>/);
  assert.match(appSource, /<strong>\{t\(masterItem\.title\)\}<\/strong>/);
});

test("the manager keeps only the master switch, standalone patches and session maintenance; everything else is Codex-only", () => {
  const managerKeys = enhancementCatalog.items.filter((item) => item.surfaces.includes("manager")).map((item) => item.key);
  assert.deepEqual([...managerKeys].sort(), [...MANAGER_KEYS].sort());
  // 管理器留着的项在 Codex 里也能看到（总开关除外的几项都能在两边改）；其余只在 Codex 里。
  for (const item of enhancementCatalog.items) {
    assert.ok(item.surfaces.includes("codex"), `${item.key} must be editable in the Codex settings page`);
    if (!MANAGER_KEYS.includes(item.key)) assert.deepEqual(item.surfaces, ["codex"], `${item.key} is Codex-only`);
  }
  for (const platform of ["windows", "macos", "linux"] as const) {
    const cards = managerBuiltinCards(managerStandaloneGroup().id, platform);
    const placed = [
      ...cards.flatMap((card) => [...(card.item ? [card.item.key] : []), ...card.params.map((param) => param.key)]),
      "enhancementsEnabled",
      "providerSyncEnabled",
      "sessionIndexRepairEnabled",
    ];
    const expected = enhancementCatalog.items
      .filter((item) => isEnhancementItemVisible(item, "manager", platform))
      .map((item) => item.key);
    assert.deepEqual([...placed].sort(), [...expected].sort(), `manager switches on ${platform} must match the catalog`);
    assert.equal(new Set(placed).size, placed.length);
  }

  // 增强页：没有页签、没有内置拓展卡片，只有一处说明指向 Codex 和独立模式那一组。
  const enhance = component("EnhanceScreen");
  for (const role of ["tablist", "tab", "tabpanel"]) {
    assert.ok(!elements(enhance).some((element) => attribute(element, "role") === role), `EnhanceScreen has no ${role}`);
  }
  for (const name of ["BuiltinExtensionCardView", "StandaloneModeGroup", "DictationSettingsPanel", "StepwiseSettingsPanel"]) {
    assert.ok(!app.statements.some((node) => ts.isFunctionDeclaration(node) && node.name?.text === name), `${name} is gone`);
  }
  const groups = elements(enhance).filter((element) => element.tagName.getText(app) === "FeatureGroup");
  assert.deepEqual(groups.map((group) => attribute(group, "id")), ["enhance-group-standalone"]);
  const moved = elements(enhance).find((element) => attribute(element, "data-enhance-moved") === "codex");
  assert.ok(moved, "the enhancement page points to the Codex settings page");
  assert.equal(isConditional(moved, enhance), false);
  assert.ok(!appSource.includes("enhancementTab"), "the manager no longer remembers an enhancement tab");
});

test("every catalog string the manager shows has an English translation", () => {
  const missing = enhancementCatalogStrings().filter((text) => !EN_PLAIN[text]);
  assert.deepEqual(missing, []);
});

test("voice and Stepwise service settings live in the Codex settings page, not the manager", () => {
  for (const name of ["EnhanceScreen", "GlobalSettingsScreen", "AgentSettingsScreen"]) {
    const screen = component(name);
    assert.ok(!descendants(screen).some((node) => ts.isPropertyAccessExpression(node)
      && (/^form\.dictation(?:\.|$)/.test(node.getText(app)) || /^form\.codexApp(?:Stepwise|AnswerOutline)/.test(node.getText(app)))), name);
  }
  assert.ok(!appSource.includes("testStepwiseSettings"));
  // Codex 设置页里两节跟在对应清单分组后面。
  const afterGroup = functionBody(servicePage, "renderCodexPlusServiceSettingsAfterGroup");
  assert.match(afterGroup, /groupId === "input"[\s\S]*renderCodexPlusDictationServiceSection\(\)/);
  assert.match(afterGroup, /groupId === "stepwise"\) return renderCodexPlusStepwiseServiceSection\(\)/);
  assert.match(catalogPage, /typeof renderCodexPlusServiceSettingsAfterGroup === "function" \? renderCodexPlusServiceSettingsAfterGroup\(group\.id\) : ""/);
});

test("local speech download requires a button click and rendering only reads status", () => {
  assert.match(servicePage, /data-codex-plus-dictation-local-model-download="true"\$\{downloading \? " disabled" : ""\}/);
  const click = functionBody(servicePage, "handleCodexPlusServiceSettingsClick");
  assert.match(click, /\[data-codex-plus-dictation-local-model-download\][\s\S]*?startCodexPlusLocalModelDownload\(\)/);
  assert.equal(servicePage.match(/postJson\("\/dictation\/local-model\/download"/g)?.length, 1);
  assert.match(functionBody(servicePage, "startCodexPlusLocalModelDownload"), /\/dictation\/local-model\/download/);
  for (const name of ["codexPlusEnsureServiceSettingsLoaded", "refreshCodexPlusLocalModelStatus", "codexPlusSyncLocalModelPolling", "renderCodexPlusDictationServiceSection"]) {
    assert.doesNotMatch(functionBody(servicePage, name), /local-model\/download|startCodexPlusLocalModelDownload/, name);
  }
});

test("the Codex service sections keep every service field, private key input and the Stepwise connection test", () => {
  const dictation = functionBody(servicePage, "renderCodexPlusDictationServiceSection");
  for (const field of ["baseUrl", "apiKeyEnv", "model", "language", "timeoutSeconds"]) {
    assert.match(dictation, new RegExp(`renderCodexPlusServiceInput\\("dictation", "${field}"`), field);
  }
  assert.match(dictation, /renderCodexPlusServiceSecret\("dictation", "apiKey"/);
  // Key 不回页面：只看 apiKeyConfigured，输入框是 password 且不带 value。
  assert.match(functionBody(servicePage, "renderCodexPlusServiceSecret"), /type="password" value=""/);
  // 页面从不读 Key 本身（删本地副本那一处除外）。
  assert.doesNotMatch(servicePage, /(?<!delete )dictation\.apiKey(?![CE])/);
  assert.doesNotMatch(servicePage, /settings\.codexAppStepwiseApiKey(?![CE])|codexPlusBackendSettings\.codexAppStepwiseApiKey(?![CE])/);

  // 「下一步建议」「回答大纲」是 Codex 设置页里的两个内置拓展开关，「建议直接发送」是前者的子选项。
  assert.deepEqual(enhancementCatalog.items.filter((item) => item.group === "stepwise").map((item) => item.key),
    ["codexAppStepwiseEnabled", "codexAppAnswerOutlineEnabled", "codexAppStepwiseDirectSend"]);
  assert.equal(enhancementItem("codexAppStepwiseDirectSend").parent, "stepwise");
  const stepwise = functionBody(servicePage, "renderCodexPlusStepwiseServiceSection");
  for (const field of ["codexAppStepwiseBaseUrl", "codexAppStepwiseModel", "codexAppStepwiseApiKeyEnv"]) {
    assert.match(stepwise, new RegExp(`field\\("${field}"`), field);
  }
  for (const field of ["codexAppStepwiseMaxItems", "codexAppStepwiseMaxInputChars", "codexAppStepwiseMaxOutputTokens", "codexAppStepwiseTimeoutMs"]) {
    assert.match(stepwise, new RegExp(`number\\("${field}"`), field);
  }
  for (const field of ["codexAppStepwiseProtocol", "codexAppStepwiseGenerationMode"]) {
    assert.match(stepwise, new RegExp(`data-codex-plus-stepwise-select="${field}"`), field);
  }
  assert.match(stepwise, /renderCodexPlusServiceSecret\("stepwise", "codexAppStepwiseApiKey", settings\.codexAppStepwiseApiKeyConfigured === true/);
  assert.match(functionBody(servicePage, "testCodexPlusStepwiseConnection"), /postJson\("\/stepwise\/test", \{\}\)/);
});

test("manager deep links only pick the route", () => {
  const navigation = descendants(app).find((node) => ts.isVariableDeclaration(node)
    && ts.isIdentifier(node.name) && node.name.text === "consumePendingManagerNavigation");
  assert.ok(navigation);
  const calls = descendants(navigation).filter(ts.isCallExpression).map((node) => node.expression.getText(app));
  assert.ok(calls.includes("setRoute"));
  assert.ok(!calls.includes("setEnhancementTab"));
});

test("enhancement settings save automatically and show a permanent status at the top", () => {
  const enhance = component("EnhanceScreen");
  // 不再有手动保存按钮：改动由 useEnhanceAutoSave 防抖后自动保存。
  assert.equal(elements(enhance).filter((element) => attribute(element, "onClick")?.includes("actions.saveSettings(")).length, 0);
  const calls = descendants(enhance).filter(ts.isCallExpression);
  assert.ok(calls.some((node) => node.expression.getText(app) === "useEnhanceAutoSave"
    && node.arguments.map((arg) => arg.getText(app)).join(", ") === "dirty, form, actions.autoSaveSettings"));
  const status = elements(enhance).filter((element) => element.tagName.getText(app) === "AutoSaveStatus");
  assert.equal(status.length, 1);
  assert.equal(attribute(status[0], "state"), "autoSave");
  assert.equal(isConditional(status[0], enhance), false);
  const firstGroup = elements(enhance).find((element) => element.tagName.getText(app) === "FeatureGroup");
  assert.ok(firstGroup && status[0].pos < firstGroup.pos);

  // 保存不弹窗；保存期间的改动保留草稿并同步版本号。
  // 语音输入在 Codex 里改，配置不完整也不能挡住管理器里其他设置的保存。
  const autoSave = appSource.slice(appSource.indexOf("const autoSaveSettings = async"), appSource.indexOf("const beginWeixinQrLogin"));
  assert.doesNotMatch(autoSave, /showNotice\(|dictationSettingsIssue/);
  assert.doesNotMatch(appSource, /dictationSettingsIssue/);
  assert.match(autoSave, /backendSettingsEqual\(current, snapshot\) \? saved : \{ \.\.\.current, revision: saved\.revision \}/);
});

test("standalone quota unlock lives in its own group and stays editable when enhancements are off", () => {
  const enhance = component("EnhanceScreen");
  const standaloneGroup = managerStandaloneGroup();
  assert.equal(standaloneGroup.title, "独立模式");
  assert.equal(standaloneGroup.description, "增强总开关关闭后仍会生效的补丁。");
  assert.deepEqual(managerBuiltinCards(standaloneGroup.id, "macos").map((card) => card.item?.key), ["codexAppStandaloneQuotaUnlock"]);
  const standalone = enhancementItem("codexAppStandaloneQuotaUnlock");
  assert.equal(standalone.title, "官方额度解锁");
  assert.equal(standalone.control, "bool");
  assert.equal(standalone.masterGated, false, "独立模式补丁不能被增强总开关禁用");
  // 开关说明要区分增强开启/关闭两种状态。
  assert.equal(standalone.descriptionWhenEnhancementsOn, "增强开启时此能力已自动生效；此开关只在关闭增强后生效。");
  assert.match(standalone.description, /官方订阅额度耗尽后仍可发送，不区分供应商模式/);
  const control = component("EnhancementSettingControl");
  assert.match(control.getText(app), /masterEnabled && item\.descriptionWhenEnhancementsOn \? item\.descriptionWhenEnhancementsOn : item\.description/);
  assert.match(control.getText(app), /onToggle \?\? \(\(value\) => onFormChange\(\{ \.\.\.form, \[key\]: value \}\)\)/);

  const featureGroup = elements(enhance).find((element) => element.tagName.getText(app) === "FeatureGroup");
  assert.ok(featureGroup);
  assert.equal(attribute(featureGroup, "title"), "t(standaloneGroup.title)");
  assert.equal(attribute(featureGroup, "detail"), 't(standaloneGroup.description ?? "")');
  // 不管总开关开没开都显示（关掉增强后这里是唯一还能改的地方）。
  assert.equal(isConditional(featureGroup, enhance), false);
  assert.ok(descendants(enhance).some((node) => ts.isCallExpression(node)
    && node.expression.getText(app) === "managerBuiltinCards"
    && node.arguments[0]?.getText(app) === "managerStandaloneGroup().id"));
});

test("native Fast unlock is the only Fast enhancement toggle and keeps the provider support notice", () => {
  const nativeFast = enhancementItem("codexAppNativeFastUnlock");
  assert.equal(nativeFast.control, "bool");
  assert.equal(nativeFast.title, "原生 Fast 强制解锁");
  assert.notEqual(nativeFast.masterGated, false, "原生 Fast 受增强总开关控制");
  assert.match(nativeFast.description, /纯 API Key.*原生 Fast.*供应商决定/);
  assert.notEqual(enhancementGroup(nativeFast.group).standalone, true);
  assert.deepEqual(nativeFast.surfaces, ["codex"]);
  assert.deepEqual(enhancementCatalog.items.filter((item) => /Fast/.test(item.title)).map((item) => item.key), ["codexAppNativeFastUnlock"]);
  assert.ok(!enhancementCatalog.items.some((item) => item.key === "codexAppServiceTierControls"));
  assert.match(appSource, /codexAppNativeFastUnlock: false,/);
});
