import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

/**
 * Agent 桌宠是官方拓展包（assets/extensions/agent-pet）：Codex 里的运行时（pet.js）、设置界面（settings.js，
 * Codex「拓展」详情页和管理器设置页共用）都在包里。运行时用最小的假 DOM 与假 ctx 在 vm 沙箱里跑，不启动 Codex。
 */
const packageFile = (name: string) => readFileSync(new URL(`../../../assets/extensions/agent-pet/${name}`, import.meta.url), "utf8");
const fragment = packageFile("pet.js");
const settingsSource = packageFile("settings.js");
const main = packageFile("main.js");
const manifest = JSON.parse(packageFile("codexpp.json"));
const app = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
const injectFragment = (name: string) => readFileSync(new URL(`../../../assets/inject/renderer-inject/${name}`, import.meta.url), "utf8");

type PetSettings = Record<string, any>;
// settings.js 的纯函数：在 vm 里跑一遍包里的脚本取出来。
const settingsApi = (() => {
  const context = createContext({ URL, Math, Number, String, JSON, Object, Array, window: {} });
  runInContext(`${settingsSource}\nglobalThis.api = { normalize: cppPetSettingsNormalize, issue: cppPetSettingsIssue, soundKey: cppPetSettingsSoundKey, releaseDelay: cppPetSettingsReleaseDelay };`, context);
  return context.api as { normalize(raw: unknown): PetSettings; issue(config: PetSettings): string; soundKey(config: PetSettings): string; releaseDelay(start: number, duration: number, now: number): number };
})();
const defaults = () => settingsApi.normalize({});

test("设置：默认值与清单 config 一致，数值夹在范围内，键都在清单里声明过", () => {
  const value = defaults();
  for (const [key, field] of Object.entries(manifest.config as Record<string, { default: unknown }>)) {
    assert.deepEqual(value[key], field.default, key);
  }
  assert.deepEqual(Object.keys(value).sort(), Object.keys(manifest.config).sort(), "设置界面存的键就是清单 config 的键");
  const normalized = settingsApi.normalize({ kind: " whaleWidget ", image: " fox ", size: 999, volume: 3, soundSet: " other ", codexAppWhaleBalanceCurrency: "cny" });
  assert.equal(normalized.kind, "whaleWidget");
  assert.equal(normalized.image, "fox");
  assert.equal(normalized.size, 240);
  assert.equal(normalized.volume, 1);
  assert.equal(normalized.soundSet, "other");
  assert.equal(normalized.codexAppWhaleBalanceCurrency, "CNY");
  for (const kind of ["dragon", "", 1, null]) assert.equal(settingsApi.normalize({ kind }).kind, "image", String(kind));
  assert.equal(settingsApi.normalize({ size: Number.NaN, volume: Number.NaN }).size, 112);
});

test("设置：自定义图片只接受不带账号信息的 https 地址；选了大肥鱼时只校验自定义余额接口", () => {
  const custom = (customImageUrl: string) => settingsApi.issue({ ...defaults(), image: "custom", customImageUrl });
  assert.equal(custom("https://example.com/pet.png"), "");
  for (const bad of ["http://example.com/pet.png", "https://user:pass@example.com/pet.png", "not a url"]) assert.match(custom(bad), /https:\/\//, bad);
  assert.equal(settingsApi.issue({ ...defaults(), kind: "whaleWidget", image: "custom", customImageUrl: "bad" }), "");
  const whale = (patch: PetSettings) => settingsApi.issue({ ...defaults(), kind: "whaleWidget", codexAppWhaleBalanceProtocol: "custom", codexAppWhaleBalancePath: "/api/balance", codexAppWhaleBalanceField: "data.balance", ...patch });
  assert.equal(whale({}), "");
  assert.match(whale({ codexAppWhaleBalancePath: "https://x.com/b" }), /余额接口路径/);
  assert.match(whale({ codexAppWhaleBalanceField: "a b" }), /余额字段路径/);
  assert.match(whale({ codexAppWhaleBalanceCurrency: "US" }), /三位代码/);
  assert.match(whale({ codexAppWhaleBalanceScale: 0 }), /倍率/);
});

test("设置：音效键与试听衔接和 Codex 里的桌宠一致", () => {
  assert.equal(settingsApi.soundKey(defaults()), "duck");
  assert.equal(settingsApi.soundKey({ soundSet: "nya", customSoundPress: "x", customSoundRelease: "" }), "nya");
  assert.equal(settingsApi.soundKey({ soundSet: "custom", customSoundPress: "press-0123456789abcdef.mp3", customSoundRelease: "" }), "custom:press-0123456789abcdef.mp3:");
  assert.ok(Math.abs(settingsApi.releaseDelay(1, 0.3, 1.05) - 0.21) < 1e-9);
  assert.equal(settingsApi.releaseDelay(1, 0.3, 2), 0);
  assert.match(fragment, /const codexPlusAgentPetReleaseLeadMs = 40;/);
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
  const f = fixture({ ...defaults(), image: "fox", size: 96 });
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
  const settings = { ...defaults(), soundSet: "custom", customSoundPress: "press-0123456789abcdef.mp3" };
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
  const f = fixture({ ...defaults(), soundSet: "fx1" }, {}, { "assets/sounds/D1.mp3": "data:audio/mpeg;base64,QUFB", "assets/sounds/D2.mp3": "data:audio/mpeg;base64,QkJC" });
  const started = fakeAudio(f.window);
  f.sync();
  await flush();
  await flush();
  assert.deepEqual(f.requests, ["/agent-pet/image"]);
  f.body.children[0].emit("pointerdown");
  assert.deepEqual(started, ["AAA"]);
});

test("按下压扁、松开回弹，以底边为支点", () => {
  const f = fixture({ ...defaults(), soundEnabled: false });
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
  const f = fixture({ ...defaults(), soundEnabled: false });
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
  const whale = fixture({ ...defaults(), kind: "whaleWidget" });
  whale.sync();
  await flush();
  assert.equal(whale.body.children.length, 0);
  assert.deepEqual(whale.requests, [], "不取图、不取音效");
  const f = fixture({ ...defaults() });
  f.sync();
  assert.equal(f.body.children.length, 1);
  f.configure({ kind: "whaleWidget" });
  assert.equal(f.body.children.length, 0);
});

test("拓展卸下时桌宠撤掉，之后再同步也不挂回来", () => {
  const f = fixture({ ...defaults() });
  f.sync();
  assert.equal(f.body.children.length, 1);
  runInContext("window.pet.dispose()", f.context);
  assert.equal(f.body.children.length, 0);
  f.sync();
  assert.equal(f.body.children.length, 0);
});

test("设置界面在拓展包里：Codex 详情页经 ctx.registerSettingsPanel 挂它，管理器设置页（settings.page）内联同一份", () => {
  assert.deepEqual(manifest.renderer.slice(-2), ["settings.js", "main.js"], "settings.js 要在 main.js 之前加载");
  assert.deepEqual(manifest.settings, { page: "settings.html" });
  assert.match(main, /ctx\.registerSettingsPanel\(\{\s*render: \(\{ container, config, saveConfig \}\) => createCodexPlusAgentPetSettings\(container, \{/);
  for (const call of ["ctx.registerSlot(", "ctx.onScan(", "ctx.onConfigUpdate(", "ctx.listen(", "ctx.effect("]) assert.ok(main.includes(call), call);
  // 管理器的沙箱页面拿不到包里别的文件：设置界面脚本内联进 settings.html，不能自己带 </script>。
  assert.ok(!/<\/script/i.test(settingsSource));
  assert.match(packageFile("settings.html"), /\/\*__AGENT_PET_SETTINGS_SCRIPTS__\*\//);
  assert.match(packageFile("settings-page.js"), /createCodexPlusAgentPetSettings\(root, \{[\s\S]*?manager\.call\(route, payload\)/);
  // 设置界面调的路由都在清单里声明过，也都在拓展路由白名单里。
  const api = injectFragment("91-extension-api.js");
  const routes = [...new Set([...settingsSource.matchAll(/call\("(\/[^"]+)"/g)].map((match) => match[1]))];
  assert.deepEqual(routes.sort(), ["/agent-pet/catalog", "/agent-pet/image", "/agent-pet/import-sound", "/agent-pet/sound"]);
  for (const route of routes) {
    assert.ok(manifest.permissions.routes.includes(route), `${route} 没在清单里声明`);
    assert.ok(api.includes(`"${route}",`), `${route} 不在拓展路由白名单`);
  }
  // 写配置的路由只给 Codex++ 自己的详情页用，不开放给拓展。
  assert.ok(!api.includes('"/user-scripts/set-config"'));
});

test("Codex 「拓展」详情页挂拓展的设置面板：重画时搬回同一个节点，保存走 set-config 并原地生效", () => {
  const detail = injectFragment("40-backend-settings.js");
  // 设置面板挂在详情的「设置」页签里（41-extension-detail.js）。
  assert.match(injectFragment("41-extension-detail.js"), /if \(panel && local\.enabled\) \{\s*return `<div class="codex-plus-extensions-detail-settings-host" data-codex-plus-settings-panel=/);
  assert.match(detail, /detail\.innerHTML = renderCodexPlusExtensionsDetail\(\);[\s\S]{0,120}mountCodexPlusSettingsPanel\(detail\);/);
  assert.match(detail, /slot\.replaceWith\(mounted\.node\);/);
  assert.match(detail, /postJson\("\/user-scripts\/set-config", \{ key, values:/);
  assert.match(detail, /window\.__codexPlusUserScripts\?\.updateConfig\?\.\(key, config\)/);
  assert.match(detail, /if \(script\?\.source === "bundled"\) return "官方";/);
  assert.match(injectFragment("92-extension-views.js"), /function codexPlusRegisterSettingsPanel\(definition, options = \{\}\)/);
  assert.match(injectFragment("91-extension-runtime.js"), /registerSettingsPanel\(definition, itemOptions\) \{\s*return track\(codexPlusRegisterSettingsPanel/);
});

test("管理器里没有桌宠专属代码：「设置」按钮放在行上，有设置页的拓展打开它自己的设置页", () => {
  assert.doesNotMatch(app, /AgentPet|agentPet"|AGENT_PET/);
  assert.match(app, /aria-label=\{tf\("\{0\} 设置", \[displayName\]\)\}/);
  // 设置在行下面展开（包一层 cx-row-expand 放进同一张卡片）。
  assert.match(app, /\{configOpen && settingsPage \? \(\s*(?:<div className="cx-row-expand">)?<ExtensionManagerPanel\s*kind="settings"/);
  assert.match(app, /\{configOpen && !settingsPage && configSchema\.length \? \(\s*(?:<div className="cx-row-expand">)?<ExtensionConfigForm/);
  assert.doesNotMatch(app, /t\("Codex 用量挂件"\)|WhaleBalanceSettingsFields/);
});
