import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import ts from "typescript";

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
    "codexAppUpstreamWorktreeCreate", "codexAppForceChineseLocale", "codexAppNativeMenuLocalization",
    "codexAppFastStartup", "快速启动",
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

test("unified enhancements use the master switch and Chinese suggestion labels", () => {
  const enhance = component("EnhanceScreen");
  const toggles = elements(enhance).filter((element) => element.tagName.getText(app) === "FeatureToggle");
  const share = fieldControl(enhance, "form.codexAppSessionShare");
  assert.equal(attribute(share, "onChange"), '(value) => setEnhanceFlag("codexAppSessionShare", value)');
  assert.ok(toggles.length > 0);
  for (const toggle of toggles) assert.equal(attribute(toggle, "disabled"), "!masterEnabled");
  assert.equal(attribute(fieldControl(enhance, "form.codexAppPluginMarketplaceUnlock"), "disabled"), "!masterEnabled");
  const petGroup = elements(enhance).find((element) => element.tagName.getText(app) === "FeatureGroup"
    && attribute(element, "title") === 't("挂件与桌宠")');
  assert.ok(petGroup);
  assert.equal(attribute(petGroup, "detail"), 't("在 Codex 中查看用量，设置自己的角色和互动方式。")');
  const petLook = fieldControl(petGroup.parent, "form.codexAppPetRealMouseLook");
  assert.ok(descendants(enhance).some((node) => ts.isConditionalExpression(node)
    && node.condition.getText(app) === "isWindowsPlatform" && descendants(node.whenTrue).includes(petLook)), "native pet mouse tracking must remain Windows-only");
  const whaleWidget = fieldControl(petGroup.parent, "form.codexAppWhaleWidgetEnabled");
  assert.ok(!descendants(enhance).some((node) => ts.isConditionalExpression(node)
    && node.condition.getText(app) === "isWindowsPlatform" && descendants(node.whenTrue).includes(whaleWidget)), "the Codex usage widget must be available on every platform");

  const visibleEnglish = descendants(app).filter((node) => ts.isStringLiteral(node)
    || ts.isNoSubstitutionTemplateLiteral(node) || ts.isJsxText(node)).filter((node) => /Stepwise/.test(node.getText(app)));
  assert.deepEqual(visibleEnglish.map((node) => node.getText(app)), [], "internal Stepwise fields stay compatible but visible names must use Chinese");
  const suggestions = component("StepwiseSettingsPanel");
  const header = elements(suggestions).find((element) => element.tagName.getText(app) === "CardHead");
  assert.ok(header);
  assert.equal(attribute(header, "title"), 't("下一步建议")');
  assert.equal(attribute(fieldControl(suggestions, "form.codexAppStepwiseDirectSend"), "title"), 't("建议直接发送")');
});

test("complete voice and Stepwise panels share the enhancement form and save action", () => {
  const enhance = component("EnhanceScreen");
  for (const name of ["DictationSettingsPanel", "StepwiseSettingsPanel"]) {
    const panel = elements(enhance).find((element) => element.tagName.getText(app) === name);
    assert.ok(panel, `${name} belongs in Codex enhancements`);
    assert.equal(attribute(panel, "form"), "form");
    assert.equal(attribute(panel, "onFormChange"), "onFormChange");
  }
  const calls = descendants(enhance).filter(ts.isCallExpression);
  assert.ok(calls.some((node) => node.expression.getText(app) === "actions.saveSettings"), "enhancements must retain their save action");

  const settings = component("SettingsScreen");
  assert.ok(!elements(settings).some((element) => ["DictationSettingsPanel", "StepwiseSettingsPanel"].includes(element.tagName.getText(app))));
  assert.ok(!descendants(settings).some((node) => ts.isPropertyAccessExpression(node)
    && (/^form\.dictation(?:\.|$)/.test(node.getText(app)) || /^form\.codexApp(?:Stepwise|AnswerOutline)/.test(node.getText(app)))));
});

test("enhancement panels retain every service field, private key input and Stepwise connection test", () => {
  const dictation = component("DictationSettingsPanel");
  for (const field of ["enabled", "baseUrl", "apiKey", "apiKeyEnv", "model", "language", "timeoutSeconds"]) {
    fieldControl(dictation, `form.dictation.${field}`);
  }
  assert.equal(attribute(fieldControl(dictation, "form.dictation.apiKey"), "type"), "password");

  const stepwise = component("StepwiseSettingsPanel");
  for (const field of [
    "codexAppStepwiseEnabled", "codexAppAnswerOutlineEnabled", "codexAppStepwiseDirectSend",
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

test("enhancement saving is an explicit permanent top action", () => {
  const enhance = component("EnhanceScreen");
  const saves = elements(enhance).filter((element) => attribute(element, "onClick")?.includes("actions.saveSettings("));
  assert.equal(saves.length, 1);
  assert.equal(attribute(saves[0], "disabled"), "!dirty");
  assert.equal(isConditional(saves[0], enhance), false);
  const firstPane = elements(enhance).find((element) => attribute(element, "role") === "tabpanel");
  assert.ok(firstPane && saves[0].pos < firstPane.pos);
});
