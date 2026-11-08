import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import { isFavoriteModel, modelDisplay, ownPickerSection, pickerSections, type GatewayPayload, type OwnModels, type PickerSection } from "./gateway-model.ts";

const source = readFileSync(new URL("./GatewayAgents.tsx", import.meta.url), "utf8").replaceAll("\r\n", "\n");
const ts = createRequire(import.meta.url)("typescript");
const file = ts.createSourceFile("GatewayAgents.tsx", source, ts.ScriptTarget.ES2022, true, ts.ScriptKind.TSX);
const component = file.statements.find((node: any) => ts.isFunctionDeclaration(node) && node.name?.text === "ModelList");
const statement = component?.body?.statements.find((node: any) =>
  ts.isVariableStatement(node) && node.declarationList.declarations.some((entry: any) => entry.name.getText(file) === "sections"));
const declaration = statement?.declarationList.declarations.find((entry: any) => entry.name.getText(file) === "sections");
const callback = declaration?.initializer?.arguments?.[0];
assert.ok(callback && ts.isArrowFunction(callback) && ts.isBlock(callback.body));
const select = new Function(
  "payload", "query", "exclude", "skipGroup", "ownList", "sectionOnly", "favoritesOnly", "favorites",
  "withOwnSection", "pickerSections", "ownPickerSection", "avatarOf", "isFavoriteModel", "selectedModel", "pinProvider", "modelDisplay",
  callback.body.getText(file).slice(1, -1),
);
const ownStart = source.indexOf("function withOwnSection(");
const bodyStart = source.indexOf("{\n", ownStart);
const bodyEnd = source.indexOf("\n}", bodyStart);
assert.ok(ownStart >= 0 && bodyStart > ownStart && bodyEnd > bodyStart);
const withOwnSection = new Function("sections", "own", source.slice(bodyStart + 2, bodyEnd));

const payload = {
  gateway: { version: 1, bindings: {}, keyPolicy: { mode: "sticky" }, restoreOnExit: false },
  port: 57321,
  helperRunning: true,
  pool: ["a", "b"].map((id) => ({
    id, name: id, baseUrl: "", protocol: "responses", official: false,
    models: ["example-model"], keyCount: 1, enabled: true, source: "",
  })),
  groups: [], agents: [], health: { keys: [], providers: [] },
  codexProviders: [], claudeProviders: [], grokProfiles: [],
} as GatewayPayload;
const own: OwnModels = { vendor: "Direct", current: "example-model", models: [{ id: "example-model", label: "Example" }] };

test("自带同名模型不遮住按供应商池路由的当前选择", () => {
  const sections: PickerSection[] = select(
    payload, "", [], undefined, own, null, false, [], withOwnSection, pickerSections,
    ownPickerSection, (name: string) => name, isFavoriteModel, "example-model", true, modelDisplay,
  );
  assert.ok(sections.some((section) => section.key === "own"));
  assert.equal(sections.find((section) => section.key === "current")?.items[0].model, "example-model");
});
