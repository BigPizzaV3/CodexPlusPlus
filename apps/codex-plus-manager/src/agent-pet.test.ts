import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { agentPetSettingsIssue, agentPetSoundKey, defaultAgentPetSettings, isAgentPetImagePetActive, isAgentPetWhaleWidgetActive, normalizeAgentPetSettings } from "./agent-pet-settings.ts";
import { agentPetReleaseDelay } from "./agent-pet-sound.ts";

/**
 * Agent 桌宠：设置规范化、Codex 里的运行时（挂载 / 压扁回弹 / 取图 / 卸载）以及管理器页面接线。
 * 桌宠是官方拓展包（assets/extensions/agent-pet）：运行时用最小的假 DOM 与假 ctx 在 vm 沙箱里跑包里的 pet.js，不启动 Codex。
 */
const fragment = readFileSync(new URL("../../../assets/extensions/agent-pet/pet.js", import.meta.url), "utf8");
const app = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
const page = readFileSync(new URL("./AgentPetPage.tsx", import.meta.url), "utf8");

test("设置默认关闭，只认 Codex，数值夹在范围内", () => {
  assert.deepEqual(normalizeAgentPetSettings(undefined), defaultAgentPetSettings());
  assert.equal(defaultAgentPetSettings().enabled, false);
  const normalized = normalizeAgentPetSettings({
    enabled: true, agents: ["codex", "claude", "grok", "codex"], image: " fox ", size: 999, volume: 3, soundSet: " other ",
  });
  assert.deepEqual(normalized.agents, ["codex"]);
  assert.equal(normalized.image, "fox");
  assert.equal(normalized.size, 240);
  assert.equal(normalized.volume, 1);
  // 音效组是否存在由后端校验（下载目录在后端），前端只去空白。
  assert.equal(normalized.soundSet, "other");
  assert.equal(normalizeAgentPetSettings({ soundSet: 3 }).soundSet, "duck");
  assert.equal(normalizeAgentPetSettings({ size: Number.NaN, volume: Number.NaN }).size, 112);
});

test("音效键：自定义音效带文件名，换了文件键就变", () => {
  assert.equal(agentPetSoundKey(defaultAgentPetSettings()), "duck");
  assert.equal(agentPetSoundKey({ soundSet: "nya", customSoundPress: "x", customSoundRelease: "" }), "nya");
  assert.equal(
    agentPetSoundKey({ soundSet: "custom", customSoundPress: "press-0123456789abcdef.mp3", customSoundRelease: "" }),
    "custom:press-0123456789abcdef.mp3:",
  );
});

test("旧版用量挂件开着的用户升级后开启大肥鱼，已有桌宠设置时以它为准", () => {
  const legacy = normalizeAgentPetSettings(undefined, true);
  assert.equal(legacy.enabled, true);
  assert.equal(legacy.kind, "whaleWidget");
  assert.equal(isAgentPetWhaleWidgetActive(legacy), true);
  assert.equal(normalizeAgentPetSettings(undefined, false).kind, "image");
  assert.equal(normalizeAgentPetSettings({ enabled: false }, true).enabled, false);
  // 已有桌宠设置但没写 kind（升级到简单桌宠之后保存过）：保持现状，仍是简单桌宠。
  const existing = normalizeAgentPetSettings({ enabled: true, image: "fox" }, true);
  assert.equal(existing.kind, "image");
  assert.equal(existing.image, "fox");
});

test("桌宠类型：未知值回落到简单桌宠，两种类型互斥", () => {
  assert.equal(defaultAgentPetSettings().kind, "image");
  for (const kind of ["dragon", "", 1, null]) assert.equal(normalizeAgentPetSettings({ kind }).kind, "image", String(kind));
  assert.equal(normalizeAgentPetSettings({ kind: " whaleWidget " }).kind, "whaleWidget");
  const cases: Array<[Partial<ReturnType<typeof defaultAgentPetSettings>>, boolean, boolean]> = [
    [{ enabled: true, kind: "image" }, true, false],
    [{ enabled: true, kind: "whaleWidget" }, false, true],
    [{ enabled: false, kind: "whaleWidget" }, false, false],
    [{ enabled: true, kind: "whaleWidget", agents: [] }, false, false],
  ];
  for (const [patch, image, whale] of cases) {
    const settings = { ...defaultAgentPetSettings(), ...patch };
    assert.equal(isAgentPetImagePetActive(settings), image, JSON.stringify(patch));
    assert.equal(isAgentPetWhaleWidgetActive(settings), whale, JSON.stringify(patch));
  }
  // 选了大肥鱼时不再校验简单桌宠没填好的自定义图片，避免拦住保存。
  assert.equal(agentPetSettingsIssue({ ...defaultAgentPetSettings(), kind: "whaleWidget", image: "custom", customImageUrl: "bad" }), null);
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

function fixture(config: Record<string, unknown>, responses: Record<string, unknown> = {}, assets: Record<string, string> = {}) {
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
  const payloads: unknown[] = [];
  const storage = new Map<string, string>();
  const window: Record<string, any> = { innerWidth: 1200, innerHeight: 800, addEventListener() {}, removeEventListener() {} };
  // 拓展 ctx 的最小替身：config 是拓展配置，asset 取包内资源，call 走清单声明的路由。
  const state = { config };
  const ctx = {
    get config() { return state.config; },
    asset: (path: string) => assets[path] || "",
    call: async (path: string, payload: unknown) => {
      if (path === "/diagnostics/log") return { status: "ok" };
      requests.push(path); payloads.push(JSON.parse(JSON.stringify(payload)));
      return responses[path] ?? { status: "ok", image: "data:image/png;base64,AAAA" };
    },
  };
  const context = createContext({
    window,
    ctx,
    document: { body, createElement: (tag: string) => new Element(tag) },
    localStorage: { getItem: (key: string) => storage.get(key) ?? null, setItem: (key: string, value: string) => storage.set(key, value) },
    atob: (value: string) => Buffer.from(value, "base64").toString("binary"),
    Promise, JSON, Math, Number, Array, String, Map, Date, Uint8Array,
  });
  runInContext(`${fragment}\nwindow.pet = createCodexPlusAgentPet(ctx);`, context);
  return {
    context, body, requests, payloads, storage, window,
    sync() { window.pet.sync(); },
    configure(next: Record<string, unknown>) { state.config = { ...state.config, ...next }; window.pet.sync(); },
  };
}

const flush = () => new Promise((resolve) => setImmediate(resolve));

test("拓展装上后挂载桌宠，节点都带拓展归属标记，图片按拓展配置取", async () => {
  const f = fixture({ ...defaultAgentPetSettings(), image: "fox", size: 96 });
  f.sync();
  const root = f.body.children[0];
  assert.ok(root, "应挂载桌宠");
  assert.equal(root.getAttribute("data-codex-plus-ext"), "agent-pet");
  assert.equal(root.children[0].getAttribute("data-codex-plus-ext"), "agent-pet");
  assert.equal(root.style.width, "96px");
  await flush();
  assert.deepEqual(f.requests, ["/agent-pet/image"]);
  assert.deepEqual(f.payloads[0], { image: "fox", customImageUrl: "" });
  assert.equal(root.children[0].children[0].src, "data:image/png;base64,AAAA");
  assert.equal(root.style.display, "block");
  // 设置没变时重复同步不会再次取图或重复挂载。
  f.sync();
  await flush();
  assert.equal(f.body.children.length, 1);
  assert.deepEqual(f.requests, ["/agent-pet/image"]);
});

// 最小的假 AudioContext：解码直接成功，记录每次起播用的是哪段音频。
function fakeAudio(window: Record<string, any>) {
  const started: string[] = [];
  window.AudioContext = class {
    state = "running"; currentTime = 0; destination = {};
    decodeAudioData(bytes: ArrayBuffer) { return Promise.resolve({ duration: 0.2, label: Buffer.from(bytes).toString() }); }
    createGain() { return { gain: { value: 1 }, connect: (next: unknown) => next }; }
    createBufferSource() {
      const source: Record<string, any> = { connect: (next: unknown) => next, start() { started.push(source.buffer.label); }, stop() {} };
      return source;
    }
    close() { return Promise.resolve(); }
  };
  return started;
}

test("下载 / 自定义音效经桥接取一次，按音效键缓存，换了文件才重新取", async () => {
  const sound = (key: string) => ({ status: "ok", key, press: `data:audio/mpeg;base64,${Buffer.from(`${key}:press`).toString("base64")}` });
  const settings = { ...defaultAgentPetSettings(), soundSet: "custom", customSoundPress: "press-0123456789abcdef.mp3" };
  const responses: Record<string, unknown> = { "/agent-pet/sound": sound("custom:press-0123456789abcdef.mp3:") };
  const f = fixture(settings, responses);
  const { body, requests, window } = f;
  const started = fakeAudio(window);
  f.sync();
  await flush();
  await flush();
  f.sync();
  await flush();
  assert.deepEqual(requests.filter((path) => path === "/agent-pet/sound"), ["/agent-pet/sound"]);
  assert.deepEqual(f.payloads[requests.indexOf("/agent-pet/sound")], { soundSet: "custom", customSoundPress: "press-0123456789abcdef.mp3", customSoundRelease: "" });
  const root = body.children[0];
  root.emit("pointerdown");
  root.emit("pointerup");
  assert.deepEqual(started, ["custom:press-0123456789abcdef.mp3::press"]);

  responses["/agent-pet/sound"] = sound("custom:press-fedcba9876543210.mp3:");
  f.configure({ customSoundPress: "press-fedcba9876543210.mp3" });
  await flush();
  await flush();
  assert.equal(requests.filter((path) => path === "/agent-pet/sound").length, 2);
  root.emit("pointerdown");
  assert.equal(started.at(-1), "custom:press-fedcba9876543210.mp3::press");
});

test("内嵌音效组取拓展包的 assets，不走路由", async () => {
  const f = fixture({ ...defaultAgentPetSettings(), soundSet: "fx1" }, {}, { "assets/sounds/D1.mp3": "data:audio/mpeg;base64,QUFB", "assets/sounds/D2.mp3": "data:audio/mpeg;base64,QkJC" });
  const started = fakeAudio(f.window);
  f.sync();
  await flush();
  await flush();
  assert.deepEqual(f.requests, ["/agent-pet/image"]);
  f.body.children[0].emit("pointerdown");
  assert.deepEqual(started, ["AAA"]);
});

test("按下压扁、松开回弹，以底边为支点", () => {
  const f = fixture({ ...defaultAgentPetSettings(), soundEnabled: false });
  f.sync();
  const root = f.body.children[0];
  const figure = root.children[0];
  assert.match(figure.style.cssText, /transform-origin:50% 100%/);
  root.emit("pointerdown");
  assert.equal(figure.style.transform, "scaleY(0.88) scaleX(1.05)");
  root.emit("pointerup");
  assert.equal(figure.style.transform, "");
});

test("拖动后记住位置，单纯点按不改位置", () => {
  const f = fixture({ ...defaultAgentPetSettings(), soundEnabled: false });
  const { body, storage } = f;
  f.sync();
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

test("选了大肥鱼时图片桌宠不挂载，切过去时已挂载的会卸载（两者互斥）", async () => {
  const whale = fixture({ ...defaultAgentPetSettings(), kind: "whaleWidget" });
  whale.sync();
  await flush();
  assert.equal(whale.body.children.length, 0);
  assert.deepEqual(whale.requests, [], "不取图、不取音效");
  const f = fixture({ ...defaultAgentPetSettings() });
  f.sync();
  assert.equal(f.body.children.length, 1);
  f.configure({ kind: "whaleWidget" });
  assert.equal(f.body.children.length, 0);
});

test("拓展卸下时桌宠撤掉，之后再同步也不挂回来", () => {
  const f = fixture({ ...defaultAgentPetSettings() });
  f.sync();
  assert.equal(f.body.children.length, 1);
  runInContext("window.pet.dispose()", f.context);
  assert.equal(f.body.children.length, 0);
  f.sync();
  assert.equal(f.body.children.length, 0);
});

test("管理器：Agent 桌宠是官方拓展，设置页从本地拓展那一行打开，不再单占侧栏入口；设置写进桌宠拓展的配置", () => {
  assert.doesNotMatch(app, /const toolNavigationRoutes: Route\[\] = \[[^\]]*"agentPet"/);
  assert.match(app, /const navParentRoute: Partial<Record<Route, Route>> = \{ agentPet: "userScripts" \}/);
  assert.match(app, /onOpenSettings=\{script\.key === AGENT_PET_EXTENSION_KEY \? onOpenAgentPet : undefined\}/);
  assert.match(app, /route === "agentPet" \? <AgentPetExtensionScreen /);
  assert.match(app, /setExtensionConfig\(AGENT_PET_EXTENSION_KEY, agentPetExtensionConfig\(draft\), \{ silent: true \}\)/);
  assert.match(page, /invoke<CatalogEntry\[\]>\("agent_pet_catalog"\)/);
  assert.match(page, /invoke<ImageResult>\("agent_pet_image", \{ image, customImageUrl \}\)/);
  // 旧版增强页里的「Codex 用量挂件」开关已并入桌宠类型，余额配置改由 Agent 桌宠页提供。
  assert.doesNotMatch(app, /t\("Codex 用量挂件"\)|WhaleBalanceSettingsFields/);
  assert.match(app, /<AgentPetPage[\s\S]*?whaleBalance=\{draft\.whale\}[\s\S]*?onWhaleBalanceChange=/);
});

test("管理器：桌宠类型二选一，选大肥鱼时显示余额配置与素材来源", () => {
  assert.match(page, /role="radiogroup" aria-label=\{t\("桌宠类型"\)\}/);
  assert.match(page, /\[AGENT_PET_KIND_IMAGE,[\s\S]*?\[AGENT_PET_KIND_WHALE_WIDGET,[\s\S]*?t\("大肥鱼（DeepSeek 鲸鱼）"\)/);
  assert.match(page, /\{isWhale \? \(\r?\n\s*<WhaleWidgetSection /);
  assert.match(page, /invoke<string>\("agent_pet_whale_image"\)/);
  for (const value of ["auto", "off", "custom"]) assert.match(page, new RegExp(`\\{ value: "${value}", label:`));
  for (const field of ["codexAppWhaleBalancePath", "codexAppWhaleBalanceField", "codexAppWhaleBalanceCurrency", "codexAppWhaleBalanceScale"]) {
    assert.match(page, new RegExp(`onChange\\(\\{ ${field}: `), field);
  }
  assert.match(page, /MeteorNOX\/DeepSeek-Balance-Whale-Widget/);
  // 保存前按同一规则校验：选了大肥鱼才校验自定义余额接口。
  assert.match(app, /whaleBalanceSettingsIssue\(draft\.whale, isAgentPetWhaleWidgetActive\(draft\.pet\)\)/);
});

test("试听与 Codex 同一套衔接：点按时松开音排到按压音结束前 40ms，按住超过时长则立即播放", () => {
  assert.ok(Math.abs(agentPetReleaseDelay(1, 0.3, 1.05) - 0.21) < 1e-9);
  assert.equal(agentPetReleaseDelay(1, 0.3, 2), 0);
  assert.match(fragment, /const codexPlusAgentPetReleaseLeadMs = 40;/);
});

test("管理器：音效由后端提供，预览按住会出声，并有试听按钮", () => {
  assert.match(page, /invoke<AgentPetSoundSources>\("agent_pet_sounds"\)/);
  assert.match(page, /<PetPreview [^>]*onPress=\{pressSound\} onRelease=\{releaseSound\}/);
  assert.match(page, /player\.preview\(soundKey, settings\.volume\)/);
  assert.match(page, /invoke<SoundEntry\[\]>\("agent_pet_sound_catalog"\)/);
  assert.match(page, /invoke<SoundResult>\("agent_pet_sound", \{/);
  assert.match(page, /invoke<ImportResult>\("agent_pet_import_sound", \{ slot, data \}\)/);
});
