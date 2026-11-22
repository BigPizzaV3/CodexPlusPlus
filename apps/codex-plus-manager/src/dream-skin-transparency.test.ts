import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";

// #3233：Codex 皮肤页的「壁纸透出」滑块。值存在 Codex++ 设置里，经 /dream-skin 的 transparency 动作提交。
const page = readFileSync(new URL("../../../assets/inject/renderer-inject/96-dream-skin-page.js", import.meta.url), "utf8");

type Request = { path: string; payload: Record<string, unknown> };

function harness(overview: Record<string, unknown>) {
  const requests: Request[] = [];
  const pending: Array<() => void> = [];
  let hold = false;
  let current = { status: "ok", message: "", enabled: true, paused: false, themes: [], ...overview } as Record<string, unknown>;
  const respond = (payload: Record<string, unknown>) => {
    if (payload.action === "transparency") {
      current = { ...current, userTransparency: payload.value ?? undefined };
    }
    return { ...current, status: "ok", message: "" };
  };
  const root = {
    isConnected: true,
    innerHTML: "",
    renders: 0,
    listeners: {} as Record<string, (event: unknown) => unknown>,
    closest: () => null,
    querySelectorAll: () => [],
    querySelector: () => null,
    addEventListener(type: string, listener: (event: unknown) => unknown) { this.listeners[type] = listener; },
    removeEventListener() {},
  };
  // 每次整页重绘都数一下：拖动中不能重绘。
  let html = "";
  Object.defineProperty(root, "innerHTML", { get: () => html, set: (value: string) => { html = value; root.renders += 1; } });
  class Element {}
  const context = vm.createContext({
    Element,
    CSS: { escape: (value: string) => value },
    FileReader: class {},
    codexPlusDreamSkinPlatform: "macos",
    codexPlusAsk: async () => null,
    escapeHtml: (value: unknown) => String(value).replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;").replaceAll('"', "&quot;").replaceAll("'", "&#39;"),
    postJson: (path: string, payload: Record<string, unknown>) => {
      requests.push({ path, payload });
      const result = respond(payload);
      if (!hold || payload.action !== "transparency") return Promise.resolve(result);
      return new Promise((resolve) => pending.push(() => resolve(result)));
    },
  });
  vm.runInContext(`${page}\nthis.api = { render: codexPlusDreamSkinPageRender, set: codexPlusSkinSetTransparency };`, context);
  const api = context.api as { render: (input: { container: unknown }) => () => void; set: (value: number | null) => Promise<void> };
  return {
    root,
    requests,
    api,
    transparencyRequests: () => requests.filter((request) => request.payload.action === "transparency").map((request) => request.payload.value),
    hold: (value: boolean) => { hold = value; },
    release: () => { while (pending.length) pending.shift()!(); },
    Element,
  };
}

const settle = () => new Promise((resolve) => setImmediate(resolve));

test("skin page shows an unset transparency as follow-theme and submits a custom value on release", async () => {
  const env = harness({ transparencySupported: true });
  env.api.render({ container: env.root });
  await settle();
  // 没设：滑块停在上游默认的 30%、淡显，标着「跟随主题」，没有多余的「跟随主题」按钮。
  assert.match(env.root.innerHTML, /<input type="range"[^>]*value="30"[^>]*data-skin-transparency data-follow="true"/);
  assert.match(env.root.innerHTML, /data-skin-transparency-value>跟随主题</);
  assert.doesNotMatch(env.root.innerHTML, /data-skin-action="transparency-follow"/);
  assert.match(env.root.innerHTML, /不写进主题包/);

  // 拖动只改旁边的数字，不提交也不重绘。
  const label = { textContent: "" };
  const slider = {
    value: "55",
    dataset: { skinTransparency: "", follow: "true" } as Record<string, string>,
    attributes: {} as Record<string, string>,
    setAttribute(name: string, value: string) { this.attributes[name] = value; },
    parentElement: { querySelector: (selector: string) => (selector === "[data-skin-transparency-value]" ? label : null) },
  };
  const renders = env.root.renders;
  env.root.listeners.input({ target: slider });
  assert.equal(label.textContent, "55%");
  assert.equal(slider.dataset.follow, "false");
  assert.equal(slider.attributes["aria-valuetext"], "55%");
  assert.equal(env.root.renders, renders);
  assert.deepEqual(env.transparencyRequests(), []);

  // 松手（change）才提交；回来后按设置里的值重绘，出现「跟随主题」按钮。
  await env.root.listeners.change({ target: slider });
  await settle();
  // 页面对象来自 vm 的另一个 realm，按 JSON 比。
  assert.equal(JSON.stringify(env.requests.at(-1)), JSON.stringify({ path: "/dream-skin", payload: { action: "transparency", value: 55 } }));
  assert.match(env.root.innerHTML, /<input type="range"[^>]*value="55"[^>]*data-follow="false"/);
  assert.match(env.root.innerHTML, /data-skin-transparency-value>55%</);
  assert.match(env.root.innerHTML, /data-skin-action="transparency-follow"/);

  // 点「跟随主题」提交 null，回到未设状态。
  const button = { disabled: false, dataset: { skinAction: "transparency-follow" } };
  const target = Object.assign(new env.Element(), { closest: (selector: string) => (selector === "[data-skin-action]" ? button : null) });
  await env.root.listeners.click({ target });
  await settle();
  assert.deepEqual(env.transparencyRequests(), [55, null]);
  assert.match(env.root.innerHTML, /data-follow="true"/);
  assert.doesNotMatch(env.root.innerHTML, /data-skin-action="transparency-follow"/);
});

test("skin page coalesces transparency changes made while a submit is in flight", async () => {
  const env = harness({ transparencySupported: true, userTransparency: 40 });
  env.api.render({ container: env.root });
  await settle();
  assert.match(env.root.innerHTML, /data-skin-transparency-value>40%</);
  env.hold(true);
  const first = env.api.set(10);
  void env.api.set(20);
  void env.api.set(30);
  assert.deepEqual(env.transparencyRequests(), [10]);
  env.release();
  await settle();
  env.release();
  await first;
  // 中间的 20 被后来的 30 盖掉：只发第一次和最后一次。
  assert.deepEqual(env.transparencyRequests(), [10, 30]);
  assert.match(env.root.innerHTML, /data-skin-transparency-value>30%</);
});

test("skin page disables transparency when the active theme engine ignores it", async () => {
  const env = harness({ transparencySupported: false, userTransparency: 40 });
  env.api.render({ container: env.root });
  await settle();
  assert.match(env.root.innerHTML, /<input type="range"[^>]*disabled>/);
  assert.match(env.root.innerHTML, /不支持调节壁纸透出/);
  assert.doesNotMatch(env.root.innerHTML, /data-skin-action="transparency-follow"/);
});
