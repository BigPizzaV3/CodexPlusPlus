import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { modelRoutable, pickerSections, type GatewayPayload } from "./gateway-model.ts";

function payload(): GatewayPayload {
  return {
    gateway: { version: 1, bindings: {}, keyPolicy: { mode: "sticky" }, restoreOnExit: true, autoModels: true },
    port: 1,
    helperRunning: true,
    pool: [
      { id: "chatgpt", name: "ChatGPT Pro · 2649340134@qq.com", baseUrl: "", protocol: "responses", official: true, models: ["gpt-6.1-sol", "gpt-6.1-terra"], keyCount: 1, enabled: true },
      { id: "jojo", name: "jojocodemax", baseUrl: "https://jojo.test/v1", protocol: "responses", official: false, models: ["gpt-6.1-sol", "gpt-image-2"], keyCount: 1, enabled: true },
    ],
    groups: [],
    autoGroups: [{ id: "auto:gpt-6.1-sol", name: "gpt-6.1-sol", mode: "inOrder", members: [{ model: "chatgpt/gpt-6.1-sol", weight: 1 }, { model: "jojo/gpt-6.1-sol", weight: 1 }], hidden: false, customized: false }],
    agents: [],
    health: { keys: [], providers: [] },
    codexProviders: [],
    claudeProviders: [],
    grokProfiles: [],
  };
}

test("模型选择器：多家都有的模型在每家名下固定发给那一家，另有一份「自动组」；只有一家的用裸名", () => {
  const sections = pickerSections(payload(), "", () => "");
  const byKey = Object.fromEntries(sections.map((section) => [section.key, section.items.map((item) => item.model)]));
  assert.deepEqual(byKey["provider:chatgpt"], ["chatgpt/gpt-6.1-sol", "gpt-6.1-terra"]);
  assert.deepEqual(byKey["provider:jojo"], ["jojo/gpt-6.1-sol", "gpt-image-2"]);
  assert.deepEqual(byKey.auto, ["gpt-6.1-sol"]);
  const auto = sections.find((section) => section.key === "auto")!;
  assert.equal(auto.items[0].sub, "ChatGPT Pro · 26****34@qq.com → jojocodemax", "列出按顺序尝试的几家，账号打码");
  for (const model of ["chatgpt/gpt-6.1-sol", "jojo/gpt-6.1-sol", "gpt-6.1-sol"]) assert.equal(modelRoutable(payload(), model), true, model);
});

test("自动组可隐藏自己的入口，隐藏成员保留自动组并隐藏供应商固定入口", () => {
  const off = payload();
  off.gateway.autoModels = false;
  assert.deepEqual(pickerSections(off, "", () => "").map((section) => section.key), ["provider:chatgpt", "provider:jojo"]);
  const hidden = payload();
  hidden.autoGroups![0].hidden = true;
  assert.deepEqual(pickerSections(hidden, "", () => "").map((section) => section.key), ["provider:chatgpt", "provider:jojo"]);
  const hideMembers = payload();
  hideMembers.autoGroups![0].hideMembers = true;
  const sections = pickerSections(hideMembers, "", () => "");
  assert.deepEqual(sections.find((section) => section.key === "provider:chatgpt")?.items.map((item) => item.model), ["gpt-6.1-terra"]);
  assert.equal(sections.find((section) => section.key === "provider:jojo")?.items.some((item) => item.model === "jojo/gpt-6.1-sol"), false);
  assert.deepEqual(sections.find((section) => section.key === "auto")?.items.map((item) => item.model), ["gpt-6.1-sol"]);
  const page = readFileSync(new URL("./GatewayPage.tsx", import.meta.url), "utf8");
  assert.match(page, /"save_gateway_auto_models", \{ enabled: !autoOn \}/, "自动建组开关在路由页");
});

test("自建组隐藏共享裸模型时保留供应商固定入口", () => {
  const data = payload();
  data.groups = [{
    id: "shared", name: "Shared", mode: "inOrder", hideMembers: true,
    members: [{ model: "GPT-6.1-SOL", weight: 1 }],
  }];
  const sections = pickerSections(data, "", () => "");
  assert.equal(sections.some(section => section.key === "auto"), false);
  for (const id of ["chatgpt", "jojo"]) {
    assert(sections.find(section => section.key === `provider:${id}`)?.items
      .some(item => item.model === `${id}/gpt-6.1-sol`));
  }
  assert.deepEqual(sections.find(section => section.key === "group")?.items.map(item => item.model), ["group/shared"]);
  assert.equal(pickerSections(data, "sol", () => "").some(section => section.key === "auto"), false);
});

test("旧的隐藏成员设置保留模型组，Codex 专用组隐藏不影响管理器的 Agent 选择器", () => {
  const data = payload();
  data.groups = [
    { id: "1", name: "1", mode: "inOrder", hideCodexGroup: true,
      members: [{ model: "group/coding", weight: 1 }, { model: "jojo/gpt-6.1-sol", weight: 1 }] },
    { id: "coding", name: "Coding", mode: "inOrder",
      members: [{ model: "jojo/gpt-6.1-sol", weight: 1 }] },
  ];
  const sections = pickerSections(data, "", () => "");
  assert(sections.find(section => section.key === "provider:jojo")?.items
    .some(item => item.model === "jojo/gpt-6.1-sol"));
  assert.deepEqual(sections.find(section => section.key === "group")?.items.map(item => item.model), ["group/1", "group/coding"]);
  assert(modelRoutable(data, "group/1"), "隐藏入口仍保留路由能力");
  data.groups[0].hideCodexGroup = false;
  data.groups[0].hideMembers = true;
  const legacy = pickerSections(data, "", () => "");
  assert.equal(legacy.find(section => section.key === "provider:jojo")?.items.some(item => item.model === "jojo/gpt-6.1-sol"), false);
  assert.deepEqual(legacy.find(section => section.key === "group")?.items.map(item => item.model), ["group/1", "group/coding"]);
});

test("模型按钮：自动切换的模型带「自动」标签与专用图标；Codex 目录侧同样拆开（后端测试覆盖）", () => {
  const ui = readFileSync(new URL("./gateway-ui.tsx", import.meta.url), "utf8");
  assert.match(ui, /export function ModelViewGlyph/);
  assert.match(ui, /export function AutoTag/);
  for (const file of ["./GatewayAgents.tsx", "./TrayPanel.tsx"]) {
    assert.match(readFileSync(new URL(file, import.meta.url), "utf8"), /<AutoTag view=\{display\} \/>/, file);
  }
});
