import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { agentPetSettingsIssue, defaultAgentPetSettings, normalizeAgentPetSettings } from "./agent-pet-settings.ts";
import { agentPetReleaseDelay } from "./agent-pet-sound.ts";

/**
 * Agent 桌宠：设置规范化、Codex 里的运行时（挂载 / 压扁回弹 / 取图 / 卸载）以及管理器页面接线。
 * 运行时用最小的假 DOM 在 vm 沙箱里跑分片本身，不启动 Codex。
 */

const fragment = readFileSync(new URL("../../../assets/inject/renderer-inject/97-agent-pet.js", import.meta.url), "utf8");
const app = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
const page = readFileSync(new URL("./AgentPetPage.tsx", import.meta.url), "utf8");

test("设置默认关闭，只认 Codex，数值夹在范围内", () => {
  assert.deepEqual(normalizeAgentPetSettings(undefined), defaultAgentPetSettings());
  assert.equal(defaultAgentPetSettings().enabled, false);
  const normalized = normalizeAgentPetSettings({
    enabled: true, agents: ["codex", "claude", "grok", "codex"], image: " fox ", size: 999, volume: 3, soundSet: "other",
  });
  assert.deepEqual(normalized.agents, ["codex"]);
  assert.equal(normalized.image, "fox");
  assert.equal(normalized.size, 240);
  assert.equal(normalized.volume, 1);
  assert.equal(normalized.soundSet, "duck");
  assert.equal(normalizeAgentPetSettings({ size: Number.NaN, volume: Number.NaN }).size, 112);
});

test("旧版用量挂件开着的用户升级后桌宠保持开启，已有桌宠设置时以它为准", () => {
  assert.equal(normalizeAgentPetSettings(undefined, true).enabled, true);
  assert.equal(normalizeAgentPetSettings({ enabled: false }, true).enabled, false);
});

test("自定义图片只接受不带账号信息的 https 地址", () => {
  const custom = (customImageUrl: string) => agentPetSettingsIssue({ ...defaultAgentPetSettings(), image: "custom", customImageUrl });
  assert.equal(custom("https://example.com/pet.png"), null);
  assert.equal(custom("http://example.com/pet.png"), "customUrl");
  assert.equal(custom("https://user:pass@example.com/pet.png"), "customUrl");
  assert.equal(custom("not a url"), "customUrl");
  assert.equal(agentPetSettingsIssue({ ...defaultAgentPetSettings(), image: "fox", customImageUrl: "bad" }), null);
});

type Listener = (event: any) => void;

function fixture(agentPet: Record<string, unknown> | null) {
  class Element {
    tagName: string; children: Element[] = []; parentNode: Element | null = null; attrs: Record<string, string> = {};
    style: Record<string, string> = {}; listeners = new Map<string, Listener[]>(); alt = ""; draggable = true; src = "";
    offsetWidth = 0;
    constructor(tag: string) { this.tagName = tag.toUpperCase(); }
    get isConnected(): boolean { let node: Element | null = this; while (node) { if (node === body) return true; node = node.parentNode; } return false; }
    appendChild(node: Element) { node.parentNode = this; this.children.push(node); return node; }
    remove() { if (this.parentNode) this.parentNode.children = this.parentNode.children.filter((child) => child !== this); this.parentNode = null; }
    setAttribute(name: string, value: string) { this.attrs[name] = value; }
    getAttribute(name: string) { return this.attrs[name]; }
    addEventListener(name: string, listener: Listener) { this.listeners.set(name, [...(this.listeners.get(name) || []), listener]); }
    emit(name: string, event: Record<string, unknown> = {}) { for (const listener of this.listeners.get(name) || []) listener({ preventDefault() {}, button: 0, pointerType: "mouse", pointerId: 1, clientX: 0, clientY: 0, ...event }); }
    setPointerCapture() {}
  }
  const body = new Element("body");
  const requests: string[] = [];
  const selectors: string[] = [];
  const storage = new Map<string, string>();
  const window: Record<string, any> = { innerWidth: 1200, innerHeight: 800, addEventListener() {}, removeEventListener() {} };
  const context = createContext({
    window,
    document: { body, createElement: (tag: string) => new Element(tag) },
    localStorage: { getItem: (key: string) => storage.get(key) ?? null, setItem: (key: string, value: string) => storage.set(key, value) },
    codexPlusBackendSettingsLoaded: true,
    codexPlusBackendSettings: { agentPet },
    registerCodexPlusExtensionSelector: (selector: string) => selectors.push(selector),
    postJson: async (path: string) => { requests.push(path); return { status: "ok", image: "data:image/png;base64,AAAA" }; },
    sendCodexPlusDiagnostic: async () => {},
    atob: (value: string) => Buffer.from(value, "base64").toString("binary"),
    Promise, JSON, Math, Number, Array, String, Map, Date, Uint8Array,
  });
  runInContext(fragment, context);
  return { context, body, requests, selectors, storage, window };
}

const flush = () => new Promise((resolve) => setImmediate(resolve));

test("开启且选了 Codex 时挂载桌宠，节点都带拓展归属标记", async () => {
  const { context, body, requests, selectors } = fixture({ ...defaultAgentPetSettings(), enabled: true, size: 96 });
  runInContext("syncCodexPlusAgentPet()", context);
  const root = body.children[0];
  assert.ok(root, "应挂载桌宠");
  assert.equal(root.getAttribute("data-codex-plus-ext"), "agent-pet");
  assert.equal(root.children[0].getAttribute("data-codex-plus-ext"), "agent-pet");
  assert.equal(root.style.width, "96px");
  assert.deepEqual(selectors, ["[data-codex-plus-ext=agent-pet]"]);
  await flush();
  assert.deepEqual(requests, ["/agent-pet/image"]);
  assert.equal(root.children[0].children[0].src, "data:image/png;base64,AAAA");
  assert.equal(root.style.display, "block");
  // 设置没变时重复同步不会再次取图或重复挂载。
  runInContext("syncCodexPlusAgentPet()", context);
  await flush();
  assert.equal(body.children.length, 1);
  assert.deepEqual(requests, ["/agent-pet/image"]);
});

test("按下压扁、松开回弹，以底边为支点", () => {
  const { context, body } = fixture({ ...defaultAgentPetSettings(), enabled: true, soundEnabled: false });
  runInContext("syncCodexPlusAgentPet()", context);
  const root = body.children[0];
  const figure = root.children[0];
  assert.match(figure.style.cssText, /transform-origin:50% 100%/);
  root.emit("pointerdown");
  assert.equal(figure.style.transform, "scaleY(0.88) scaleX(1.05)");
  root.emit("pointerup");
  assert.equal(figure.style.transform, "");
});

test("拖动后记住位置，单纯点按不改位置", () => {
  const { context, body, storage } = fixture({ ...defaultAgentPetSettings(), enabled: true, soundEnabled: false });
  runInContext("syncCodexPlusAgentPet()", context);
  const root = body.children[0];
  root.offsetWidth = 112;
  root.emit("pointerdown", { clientX: 500, clientY: 500 });
  root.emit("pointerup", { clientX: 501, clientY: 501 });
  assert.equal(storage.size, 0);
  root.emit("pointerdown", { clientX: 500, clientY: 500 });
  root.emit("pointermove", { clientX: 400, clientY: 450 });
  root.emit("pointerup", { clientX: 400, clientY: 450 });
  assert.deepEqual(JSON.parse(storage.get("codexPlus.agentPet.position.v1")!), { right: 128, bottom: 182 });
});

test("关闭、未选 Codex 或还没读到设置时不挂载；关闭后卸载", () => {
  assert.equal(fixture(null).body.children.length, 0);
  for (const agentPet of [{ ...defaultAgentPetSettings() }, { ...defaultAgentPetSettings(), enabled: true, agents: [] }]) {
    const { context, body } = fixture(agentPet);
    runInContext("syncCodexPlusAgentPet()", context);
    assert.equal(body.children.length, 0);
  }
  const { context, body } = fixture({ ...defaultAgentPetSettings(), enabled: true });
  runInContext("syncCodexPlusAgentPet()", context);
  assert.equal(body.children.length, 1);
  runInContext("codexPlusBackendSettings = { agentPet: { ...codexPlusBackendSettings.agentPet, enabled: false } }; syncCodexPlusAgentPet()", context);
  assert.equal(body.children.length, 0);
});

test("管理器：Agent 桌宠是「工具」里的独立页面，图库与图片都走后端命令", () => {
  assert.match(app, /const toolNavigationRoutes: Route\[\] = \[[^\]]*"agentPet"/);
  assert.match(app, /route === "agentPet" \? \(\s*<AgentPetPage/);
  assert.match(page, /invoke<CatalogEntry\[\]>\("agent_pet_catalog"\)/);
  assert.match(page, /invoke<ImageResult>\("agent_pet_image", \{ image, customImageUrl \}\)/);
  assert.doesNotMatch(app, /WhaleBalance|whale-settings|t\("Codex 用量挂件"\)/);
});

test("试听与 Codex 同一套衔接：点按时松开音排到按压音结束前 40ms，按住超过时长则立即播放", () => {
  assert.ok(Math.abs(agentPetReleaseDelay(1, 0.3, 1.05) - 0.21) < 1e-9);
  assert.equal(agentPetReleaseDelay(1, 0.3, 2), 0);
  assert.match(fragment, /const codexPlusAgentPetReleaseLeadMs = 40;/);
});

test("管理器：音效由后端提供，预览按住会出声，并有试听按钮", () => {
  assert.match(page, /invoke<AgentPetSoundSources>\("agent_pet_sounds"\)/);
  assert.match(page, /<PetPreview [^>]*onPress=\{pressSound\} onRelease=\{releaseSound\}/);
  assert.match(page, /player\.preview\(settings\.soundSet, settings\.volume\)/);
});
