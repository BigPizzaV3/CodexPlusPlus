import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import ts from "typescript";
import {
  enhancementCatalog,
  enhancementCatalogStrings,
  enhancementGroup,
  enhancementGroupAnchorId,
  enhancementItem,
  managerGeneralGroups,
  managerGroupItems,
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

const ENHANCEMENT_COMPONENTS = ["EnhanceScreen", "StandaloneModeGroup", "StepwiseSettingsPanel", "EnhancementSettingControl"];

function settingControls(node: ts.Node) {
  return elements(node).filter((element) => element.tagName.getText(app) === "EnhancementSettingControl");
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
  for (const name of ["EnhanceScreen", "StandaloneModeGroup"]) {
    const placements = settingControls(component(name));
    assert.ok(placements.length > 0, `${name} renders catalog settings`);
    for (const placement of placements) assert.equal(attribute(placement, "masterEnabled"), "masterEnabled");
  }
  assert.equal(attribute(settingControls(component("StepwiseSettingsPanel"))[0], "masterEnabled"), "form.enhancementsEnabled");
  for (const item of enhancementCatalog.items) {
    const standalone = enhancementGroup(item.group).standalone === true;
    const unmanaged = ["enhancementsEnabled", "providerSyncEnabled"].includes(item.key);
    assert.equal(item.masterGated !== false, !standalone && !unmanaged, `${item.key} master gating`);
  }

  const petGroup = enhancementGroup("pet");
  assert.equal(petGroup.title, "桌宠与活动");
  assert.equal(petGroup.description, "调整 Codex 原生桌宠与活动进度条。可点按的 Agent 桌宠在「工具 → Agent 桌宠」里设置。");
  const visible = (platform: EnhancementPlatform) => managerGroupItems("pet", platform).map((item) => item.key);
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
  const suggestions = component("StepwiseSettingsPanel");
  const header = elements(suggestions).find((element) => element.tagName.getText(app) === "CardHead");
  assert.ok(header);
  assert.equal(attribute(header, "title"), "t(group.title)");
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
  assert.match(appSource, /<strong>\{t\(providerSyncItem\.title\)\}<\/strong>/);
  assert.match(appSource, /<strong>\{t\(masterItem\.title\)\}<\/strong>/);
});

test("every manager-visible catalog item has exactly one place in the manager", () => {
  for (const platform of ["windows", "macos", "linux"] as const) {
    const placed = [
      ...managerGeneralGroups().flatMap((group) => managerGroupItems(group.id, platform)),
      ...managerGroupItems(managerStandaloneGroup().id, platform),
      ...managerGroupItems("stepwise", platform),
      enhancementItem("enhancementsEnabled"),
      enhancementItem("providerSyncEnabled"),
    ].map((item) => item.key);
    const expected = enhancementCatalog.items
      .filter((item) => item.surfaces.includes("manager") && (!item.platforms || item.platforms.includes(platform)))
      .map((item) => item.key);
    assert.deepEqual([...placed].sort(), [...expected].sort(), `manager switches on ${platform} must match the catalog`);
    assert.equal(new Set(placed).size, placed.length);
  }
  assert.equal(enhancementGroup("stepwise").managerTab, "stepwise");
  // 左侧目录（PageSectionNav）的分组同样按清单生成，锚点 id 保持既有值，深链和滚动定位不受影响。
  assert.deepEqual(
    [...managerGeneralGroups(), managerStandaloneGroup()].map(enhancementGroupAnchorId),
    ["enhance-group-plugins", "enhance-group-chat", "enhance-group-pet", "enhance-group-standalone"],
  );
  assert.match(appSource, /const generalSections = \[\.\.\.managerGeneralGroups\(\), managerStandaloneGroup\(\)\]\s*\.map\(\(group\) => \(\{ id: enhancementGroupAnchorId\(group\), label: t\(group\.title\) \}\)\);/);
  for (const name of ["EnhanceScreen", "StandaloneModeGroup"]) {
    const groups = elements(component(name)).filter((element) => element.tagName.getText(app) === "FeatureGroup");
    assert.ok(groups.length > 0);
    for (const group of groups) assert.equal(attribute(group, "id"), "enhancementGroupAnchorId(group)", `${name} groups keep their anchor id`);
  }
  const enhance = component("EnhanceScreen");
  const generalGroups = descendants(enhance).find((node) => ts.isCallExpression(node) && node.expression.getText(app) === "managerGeneralGroups().map");
  assert.ok(generalGroups, "general tab groups are generated from the catalog");
  const stepwise = component("StepwiseSettingsPanel");
  assert.ok(descendants(stepwise).some((node) => ts.isCallExpression(node) && node.expression.getText(app) === "managerGroupItems(group.id, enhancementPlatform).map"));
});

test("every catalog string the manager shows has an English translation", () => {
  const missing = enhancementCatalogStrings().filter((text) => !EN_PLAIN[text]);
  assert.deepEqual(missing, []);
});

test("complete voice and Stepwise panels share the enhancement form and save action", () => {
  const enhance = component("EnhanceScreen");
  for (const name of ["DictationSettingsPanel", "StepwiseSettingsPanel"]) {
    const panel = elements(enhance).find((element) => element.tagName.getText(app) === name);
    assert.ok(panel, `${name} belongs in Codex enhancements`);
    assert.equal(attribute(panel, "form"), "form");
    assert.equal(attribute(panel, "onFormChange"), "onFormChange");
  }
  // 三个标签共用同一份表单，由同一个自动保存覆盖（语音输入、下一步建议的改动也会自动保存）。
  const calls = descendants(enhance).filter(ts.isCallExpression);
  assert.ok(
    calls.some((node) => node.expression.getText(app) === "useEnhanceAutoSave"
      && node.arguments.map((arg) => arg.getText(app)).join(", ") === "dirty, form, actions.autoSaveSettings"),
    "enhancements must save the shared form automatically",
  );

  for (const name of ["GlobalSettingsScreen", "AgentSettingsScreen"]) {
    const settings = component(name);
    assert.ok(!elements(settings).some((element) => ["DictationSettingsPanel", "StepwiseSettingsPanel"].includes(element.tagName.getText(app))));
    assert.ok(!descendants(settings).some((node) => ts.isPropertyAccessExpression(node)
      && (/^form\.dictation(?:\.|$)/.test(node.getText(app)) || /^form\.codexApp(?:Stepwise|AnswerOutline)/.test(node.getText(app)))));
  }
});

test("local speech download requires a configuration button click and mount only reads status", () => {
  const panel = component("DictationSettingsPanel");
  const button = elements(panel).find((element) => attribute(element, "onClick")?.includes("downloadLocalModel()"));
  assert.ok(button, "the voice settings must expose a manual model download action");
  assert.equal(attribute(button, "disabled"), "downloading");
  const calls = eventHandlerNodes(panel, button, "onClick").filter(ts.isCallExpression);
  assert.ok(calls.some((node) => node.expression.getText(app) === "invoke"
    && node.arguments[0]?.getText(app) === '"download_dictation_local_model"'));
  const effects = descendants(panel).filter(ts.isCallExpression)
    .filter((node) => node.expression.getText(app) === "useEffect");
  assert.ok(effects.some((effect) => effect.getText(app).includes('"dictation_local_model_status"')));
  for (const effect of effects) assert.ok(!effect.getText(app).includes("download_dictation_local_model"));
});

test("enhancement panels retain every service field, private key input and Stepwise connection test", () => {
  const dictation = component("DictationSettingsPanel");
  for (const field of ["enabled", "baseUrl", "apiKey", "apiKeyEnv", "model", "language", "timeoutSeconds"]) {
    fieldControl(dictation, `form.dictation.${field}`);
  }
  assert.equal(attribute(fieldControl(dictation, "form.dictation.apiKey"), "type"), "password");

  const stepwise = component("StepwiseSettingsPanel");
  // 三个开关按清单「下一步建议」组生成，其余连接参数仍是面板里的定制字段。
  assert.deepEqual(managerGroupItems("stepwise", "macos").map((item) => item.key),
    ["codexAppStepwiseEnabled", "codexAppAnswerOutlineEnabled", "codexAppStepwiseDirectSend"]);
  assert.ok(settingControls(stepwise).length > 0);
  for (const field of [
    "codexAppStepwiseProtocol", "codexAppStepwiseGenerationMode", "codexAppStepwiseBaseUrl",
    "codexAppStepwiseApiKey", "codexAppStepwiseApiKeyEnv", "codexAppStepwiseModel",
    "codexAppStepwiseMaxItems", "codexAppStepwiseMaxInputChars", "codexAppStepwiseMaxOutputTokens", "codexAppStepwiseTimeoutMs",
  ]) fieldControl(stepwise, `form.${field}`);
  assert.equal(attribute(fieldControl(stepwise, "form.codexAppStepwiseApiKey"), "type"), "password");
  const connectionTest = descendants(stepwise).find((node) => ts.isCallExpression(node)
    && node.expression.getText(app) === "actions.testStepwiseSettings");
  assert.ok(connectionTest && ts.isCallExpression(connectionTest));
  assert.equal(connectionTest.arguments[0]?.getText(app), "form", "connection test must use the complete current form");
});

test("manager deep links select their enhancement tab before displaying the page", () => {
  const navigation = descendants(app).find((node) => ts.isVariableDeclaration(node)
    && ts.isIdentifier(node.name) && node.name.text === "consumePendingManagerNavigation");
  assert.ok(navigation);
  const calls = descendants(navigation).filter(ts.isCallExpression);
  const selection = calls.find((node) => node.expression.getText(app) === "setEnhancementTab");
  const argument = selection?.arguments[0];
  assert.ok(argument && ts.isBinaryExpression(argument));
  assert.equal(argument.operatorToken.kind, ts.SyntaxKind.QuestionQuestionToken);
  assert.equal(argument.left.getText(app), "destination.section");
  assert.ok(ts.isStringLiteral(argument.right) && argument.right.text === "general");
  const route = calls.find((node) => node.expression.getText(app) === "setRoute");
  assert.ok(selection && route && selection.pos < route.pos);
});

test("all enhancement panes stay mounted while tab selection only changes visibility", () => {
  const enhance = component("EnhanceScreen");
  const panes = elements(enhance).filter((element) => attribute(element, "role") === "tabpanel");
  assert.equal(panes.length, 3);
  for (const tab of ["general", "dictation", "stepwise"]) {
    const pane = panes.find((element) => attribute(element, "hidden") === `activeTab !== "${tab}"`);
    assert.ok(pane, `missing visibility control for ${tab}`);
    assert.equal(isConditional(pane, enhance), false, `${tab} must retain local form UI state when hidden`);
    assert.ok(attribute(pane, "id"));
  }
  assert.equal(new Set(panes.map((pane) => attribute(pane, "id"))).size, 3);
  assert.ok(elements(enhance).some((element) => attribute(element, "role") === "tablist"));
  const tabs = elements(enhance).filter((element) => attribute(element, "role") === "tab");
  assert.ok(tabs.length > 0);
  for (const tab of tabs) {
    assert.ok(attribute(tab, "aria-selected")?.includes("activeTab"));
    assert.ok(attribute(tab, "aria-controls"));
    const click = eventHandlerNodes(enhance, tab, "onClick");
    const keyboard = eventHandlerNodes(enhance, tab, "onKeyDown");
    for (const handler of [click, keyboard]) {
      const calls = handler.filter(ts.isCallExpression).map((node) => node.expression.getText(app));
      assert.ok(calls.includes("onTabChange"));
      assert.ok(!calls.some((name) => /^actions\.saveSettings(?:Value)?$/.test(name)), "tab selection must not save business settings");
    }
    const keys = keyboard.filter(ts.isStringLiteral).map((node) => node.text);
    for (const key of ["ArrowLeft", "ArrowRight", "Home", "End"]) assert.ok(keys.includes(key));
  }
  const screen = elements(app).find((element) => element.tagName.getText(app) === "EnhanceScreen");
  assert.ok(screen);
  assert.equal(attribute(screen, "activeTab"), "enhancementTab");
  assert.equal(attribute(screen, "onTabChange"), "setEnhancementTab");
});

test("enhancement settings save automatically and show a permanent status at the top", () => {
  const enhance = component("EnhanceScreen");
  // 不再有手动保存按钮：改动由 useEnhanceAutoSave 防抖后自动保存。
  assert.equal(elements(enhance).filter((element) => attribute(element, "onClick")?.includes("actions.saveSettings(")).length, 0);
  const status = elements(enhance).filter((element) => element.tagName.getText(app) === "AutoSaveStatus");
  assert.equal(status.length, 1);
  assert.equal(attribute(status[0], "state"), "autoSave");
  assert.equal(isConditional(status[0], enhance), false);
  const firstPane = elements(enhance).find((element) => attribute(element, "role") === "tabpanel");
  assert.ok(firstPane && status[0].pos < firstPane.pos);

  // 校验不过时不保存也不弹窗，交给页面显示；保存期间的改动保留草稿并同步版本号。
  const autoSave = appSource.slice(appSource.indexOf("const autoSaveSettings = async"), appSource.indexOf("const beginWeixinQrLogin"));
  assert.match(autoSave, /if \(issue\) return \{ status: "invalid", message: issue \};/);
  assert.doesNotMatch(autoSave, /showNotice\(/);
  assert.match(autoSave, /backendSettingsEqual\(current, snapshot\) \? saved : \{ \.\.\.current, revision: saved\.revision \}/);
});

test("standalone quota unlock lives in its own group inside the general tab and stays editable when enhancements are off", () => {
  const enhance = component("EnhanceScreen");
  const group = component("StandaloneModeGroup");
  const standaloneGroup = managerStandaloneGroup();
  assert.equal(standaloneGroup.title, "独立模式");
  assert.equal(standaloneGroup.description, "增强总开关关闭后仍会生效的补丁。");
  assert.deepEqual(managerGroupItems(standaloneGroup.id, "macos").map((item) => item.key), ["codexAppStandaloneQuotaUnlock"]);
  const standalone = enhancementItem("codexAppStandaloneQuotaUnlock");
  assert.equal(standalone.title, "官方额度解锁");
  assert.equal(standalone.control, "bool");
  assert.equal(standalone.masterGated, false, "独立模式补丁不能被增强总开关禁用");
  // 开关说明要区分增强开启/关闭两种状态。
  assert.equal(standalone.descriptionWhenEnhancementsOn, "增强开启时此能力已自动生效；此开关只在关闭增强后生效。");
  assert.match(standalone.description, /官方登录 \+ API 供应商在官方订阅额度耗尽后仍可发送；只改客户端发送状态，不改变真实用量与计费。/);
  const control = component("EnhancementSettingControl");
  assert.match(control.getText(app), /masterEnabled && item\.descriptionWhenEnhancementsOn \? item\.descriptionWhenEnhancementsOn : item\.description/);
  assert.match(control.getText(app), /onToggle \?\? \(\(value\) => onFormChange\(\{ \.\.\.form, \[key\]: value \}\)\)/);

  const featureGroup = elements(group).find((element) => element.tagName.getText(app) === "FeatureGroup");
  assert.ok(featureGroup);
  assert.equal(attribute(featureGroup, "title"), "t(group.title)");
  assert.equal(attribute(featureGroup, "detail"), 't(group.description ?? "")');
  assert.ok(settingControls(group).length === 1);

  // EnhanceScreen 只负责按总开关状态换位置：关闭增强时置顶，开启增强时置底。
  const placements = elements(enhance).filter((element) => element.tagName.getText(app) === "StandaloneModeGroup");
  assert.equal(placements.length, 2, "独立模式分组要有置顶/置底两个挂载点");
  const conditions = placements.map((element) => {
    let parent: ts.Node | undefined = element.parent;
    while (parent && !ts.isConditionalExpression(parent)) parent = parent.parent;
    assert.ok(parent && ts.isConditionalExpression(parent));
    return parent;
  });
  for (const condition of conditions) assert.equal(condition.condition.getText(app), "masterEnabled");
  assert.equal(conditions[0].whenTrue.getText(app), "null", "关闭增强时第一个挂载点生效");
  assert.equal(conditions[1].whenFalse.getText(app), "null", "开启增强时第二个挂载点生效");
  const firstFeatureGroup = elements(enhance).find((element) => element.tagName.getText(app) === "FeatureGroup");
  assert.ok(firstFeatureGroup);
  assert.ok(placements[0].pos < firstFeatureGroup.pos, "关闭增强时独立模式分组排在最前");
  const featureGroups = elements(enhance).filter((element) => element.tagName.getText(app) === "FeatureGroup");
  const lastFeatureGroup = featureGroups[featureGroups.length - 1];
  assert.ok(lastFeatureGroup && placements[1].pos > lastFeatureGroup.pos, "开启增强时独立模式分组排在最后");
  const generalPane = elements(enhance).find((element) => attribute(element, "role") === "tabpanel"
    && attribute(element, "id") === "enhance-pane-general");
  const dictationPane = elements(enhance).find((element) => attribute(element, "role") === "tabpanel"
    && attribute(element, "id") === "enhance-pane-dictation");
  assert.ok(generalPane && dictationPane);
  for (const placement of placements) assert.ok(placement.pos > generalPane.pos && placement.pos < dictationPane.pos);
});

test("native Fast unlock is the only Fast enhancement toggle and keeps the provider support notice", () => {
  const nativeFast = enhancementItem("codexAppNativeFastUnlock");
  assert.equal(nativeFast.control, "bool");
  assert.equal(nativeFast.title, "原生 Fast 强制解锁");
  assert.notEqual(nativeFast.masterGated, false, "原生 Fast 受增强总开关控制");
  assert.match(nativeFast.description, /纯 API Key.*原生 Fast.*供应商决定/);
  assert.ok(managerGeneralGroups().some((group) => group.id === nativeFast.group));
  assert.ok(nativeFast.surfaces.includes("manager") && nativeFast.surfaces.includes("codex"));
  assert.deepEqual(enhancementCatalog.items.filter((item) => /Fast/.test(item.title)).map((item) => item.key), ["codexAppNativeFastUnlock"]);
  assert.ok(!enhancementCatalog.items.some((item) => item.key === "codexAppServiceTierControls"));
  assert.match(appSource, /codexAppNativeFastUnlock: false,/);
});
