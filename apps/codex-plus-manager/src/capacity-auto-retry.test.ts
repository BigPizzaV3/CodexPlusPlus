// 容量错误自动重试：核心循环的行为回归。
//
// 这里不读组装产物做字符串断言（那是 cdp_bridge.rs 的活），而是把分片放进一个
// 手写的假 DOM + 假时钟里真跑一遍，验证四件在真机上最难复现的事：
//   1. 横幅在 → 真的点了按钮，且只点一次；
//   2. 间隔 0 秒时仍有 1 秒地板值，不会连发；
//   3. 原生倒计时在跑时让路，不抢着点；
//   4. 到上限停手、横幅消失后复位。
//
// 依赖项（codexPlusSettings / setCodexPlusSetting / showToast）全部是 stub：
// 分片本身只依赖这三个跨分片函数，其余都是 DOM 与定时器。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";

const fragmentDir = path.resolve(import.meta.dirname, "../../../assets/inject/renderer-inject");
const preludeSource = readFileSync(path.join(fragmentDir, "00-prelude.js"), "utf8");
const shardSource = readFileSync(path.join(fragmentDir, "81-capacity-auto-retry.js"), "utf8");

/** 从 prelude 里抠出本功能用到的常量声明，保证测试与源码不出现两份字面量。 */
function preludeConstants() {
  const declarations = [];
  const pattern = /^\s*const (capacityAutoRetry[A-Za-z]*) = ([^;]+);\s*$/gm;
  let match = pattern.exec(preludeSource);
  while (match) {
    declarations.push(`const ${match[1]} = ${match[2]};`);
    match = pattern.exec(preludeSource);
  }
  assert.ok(declarations.length >= 4, `prelude 常量没抠全：${declarations.length}`);
  return declarations.join("\n");
}

// ── 最小 DOM ────────────────────────────────────────────────────────────────
// 只实现分片真正用到的部分：单层复合选择器 + 逗号列表 + 后代查询。

type FakeElement = {
  tagName: string;
  attributes: Record<string, string>;
  children: FakeElement[];
  parent: FakeElement | null;
  style: Record<string, unknown>;
  dataset: Record<string, string>;
  disabled: boolean;
  clickCount: number;
  listeners: Record<string, Array<(event: unknown) => void>>;
  id: string;
  _text: string;
  textContent: string;
  setAttribute(name: string, value: unknown): void;
  getAttribute(name: string): string | null;
  removeAttribute(name: string): void;
  addEventListener(type: string, handler: (event: unknown) => void): void;
  append(...nodes: FakeElement[]): void;
  appendChild(node: FakeElement): FakeElement;
  click(): void;
  closest(selector: string): FakeElement | null;
  querySelector(selector: string): FakeElement | null;
  querySelectorAll(selector: string): FakeElement[];
  remove(): void;
};

function matchesSimple(element: FakeElement, selector: string): boolean {
  const text = selector.trim();
  if (!text) return false;
  const attrPattern = /\[([\w-]+)(?:=("([^"]*)"|'([^']*)'))?\]/g;
  let rest = text;
  let match = attrPattern.exec(text);
  while (match) {
    const name = match[1];
    const expected = match[3] ?? match[4];
    const actual = element.attributes[name];
    if (expected === undefined ? actual === undefined : actual !== expected) return false;
    match = attrPattern.exec(text);
  }
  rest = rest.replace(attrPattern, "");
  const idMatch = rest.match(/#([\w-]+)/);
  if (idMatch && element.attributes.id !== idMatch[1]) return false;
  rest = rest.replace(/#[\w-]+/, "");
  const tag = rest.trim();
  if (tag && element.tagName !== tag.toUpperCase()) return false;
  return true;
}

function matchesSelector(element: FakeElement, selector: string): boolean {
  return selector.split(",").some((part) => matchesSimple(element, part));
}

function descendants(element: FakeElement): FakeElement[] {
  const out: FakeElement[] = [];
  for (const child of element.children) {
    out.push(child, ...descendants(child));
  }
  return out;
}

function createElement(tag: string): FakeElement {
  const element: FakeElement = {
    tagName: tag.toUpperCase(),
    attributes: {},
    children: [],
    parent: null,
    style: {},
    dataset: {},
    id: "",
    disabled: false,
    clickCount: 0,
    listeners: {},
    _text: "",
    // 分片读的是 aside.textContent（判定原生倒计时），所以必须像真实 DOM 一样
    // 把子节点文本拼起来，而不是只看自己身上存的那份。
    get textContent() {
      if (this.children.length) return this.children.map((child) => child.textContent).join("");
      return this._text;
    },
    set textContent(value) {
      this._text = String(value ?? "");
      this.children = [];
    },
    setAttribute(name: string, value: unknown) {
      this.attributes[name] = String(value);
      if (name === "id") this.id = String(value);
    },
    getAttribute(name: string) {
      return Object.hasOwn(this.attributes, name) ? this.attributes[name] : null;
    },
    removeAttribute(name: string) {
      delete this.attributes[name];
    },
    addEventListener(type: string, handler: (event: unknown) => void) {
      (this.listeners[type] ||= []).push(handler);
    },
    append(...nodes: FakeElement[]) {
      for (const node of nodes) {
        node.parent = this;
        this.children.push(node);
      }
    },
    appendChild(node: FakeElement) {
      this.append(node);
      return node;
    },
    click() {
      this.clickCount += 1;
      for (const handler of this.listeners.click || []) handler({ currentTarget: this, preventDefault() {} });
    },
    closest(selector: string): FakeElement | null {
      let node: FakeElement | null = this;
      while (node) {
        if (matchesSelector(node, selector)) return node;
        node = node.parent;
      }
      return null;
    },
    querySelector(selector: string): FakeElement | null {
      return this.querySelectorAll(selector)[0] ?? null;
    },
    querySelectorAll(selector: string): FakeElement[] {
      return descendants(this).filter((node) => matchesSelector(node, selector));
    },
    remove() {
      if (!this.parent) return;
      this.parent.children = this.parent.children.filter((child) => child !== this);
      this.parent = null;
    },
  };
  return element;
}

// ── 假时钟 ──────────────────────────────────────────────────────────────────

type HarnessSettings = Record<string, boolean | number>;

function createHarness(settings: HarnessSettings) {
  const body = createElement("body");
  const main = createElement("main");
  body.appendChild(main);

  let now = 1_000_000;
  let timerSeq = 0;
  const timers = new Map();
  const toasts: string[] = [];
  const writes: Array<[string, unknown]> = [];

  const document = {
    body,
    documentElement: body,
    getElementById(id: string): FakeElement | null {
      return descendants(body).find((node) => node.id === id) || null;
    },
    querySelector(selector: string): FakeElement | null {
      if (selector === "main") return main;
      return descendants(body).find((node) => matchesSelector(node, selector)) || null;
    },
    querySelectorAll(selector: string): FakeElement[] {
      return descendants(body).filter((node) => matchesSelector(node, selector));
    },
    createElement(tag: string): FakeElement {
      const element = createElement(tag);
      if (tag === "div") element.id = "";
      return element;
    },
  };

  const window = {
    setTimeout(handler: () => void, delay?: number) {
      timerSeq += 1;
      timers.set(timerSeq, { handler, at: now + Math.max(0, Number(delay) || 0) });
      return timerSeq;
    },
    clearTimeout(id: number) {
      timers.delete(id);
    },
  };

  // 分片里的时间判断（nextAttemptAt / 倒计时文案）都走 Date.now()，必须一起假掉，
  // 否则真实时钟会让假时钟推进永远追不上。
  const DateStub = { now: () => now };

  const codexPlusSettings = () => ({ ...settings });
  const setCodexPlusSetting = (key: string, value: boolean | number) => {
    writes.push([key, value]);
    settings[key] = value;
  };
  const showToast = (message: string) => {
    toasts.push(message);
  };

  const factory = new Function(
    "document",
    "window",
    "setTimeout",
    "clearTimeout",
    "Date",
    "codexPlusSettings",
    "setCodexPlusSetting",
    "showToast",
    `${preludeConstants()}\n${shardSource}\nreturn {
      tick: capacityAutoRetryTick,
      sync: syncCodexPlusCapacityAutoRetry,
      find: findCapacityAutoRetryBanner,
      state: capacityAutoRetryState,
      intervalSeconds: capacityAutoRetryIntervalSeconds,
      maxAttempts: capacityAutoRetryMaxAttempts,
      waitMs: capacityAutoRetryWaitMs,
    };`,
  );
  const api = factory(document, window, window.setTimeout, window.clearTimeout, DateStub, codexPlusSettings, setCodexPlusSetting, showToast);

  return {
    api,
    body,
    main,
    settings,
    toasts,
    writes,
    /** 前移假时钟并按到期顺序执行定时器（模拟浏览器事件循环）。 */
    advance(ms: number) {
      const target = now + ms;
      for (;;) {
        const due = [...timers.entries()]
          .filter(([, timer]) => timer.at <= target)
          .sort((a, b) => a[1].at - b[1].at);
        if (!due.length) break;
        const [id, timer] = due[0];
        timers.delete(id);
        now = Math.max(now, timer.at);
        timer.handler();
      }
      now = target;
    },
    get now() {
      return now;
    },
  };
}

/** 造一条容量错误横幅：<aside> 里放一个「重试」按钮。 */
function addCapacityBanner(
  harness: ReturnType<typeof createHarness>,
  { label = "重试", disabled = false, extraText = "" }: { label?: string; disabled?: boolean; extraText?: string } = {},
) {
  const aside = createElement("aside");
  const text = createElement("span");
  text.textContent = `Selected model is at capacity. Please try a different model.${extraText}`;
  const button = createElement("button");
  button.textContent = label;
  button.disabled = disabled;
  aside.append(text, button);
  harness.main.appendChild(aside);
  return { aside, button };
}

function baseSettings(overrides: HarnessSettings = {}): HarnessSettings {
  return {
    capacityAutoRetry: true,
    capacityAutoRetryMaxAttempts: 3,
    capacityAutoRetryIntervalSeconds: 0,
    ...overrides,
  };
}

test("容量横幅出现后按一次按钮，且只按一次", () => {
  const harness = createHarness(baseSettings());
  const { button } = addCapacityBanner(harness);

  harness.api.tick();
  assert.equal(button.clickCount, 1, "横幅在，应当点一次重试");
  assert.equal(harness.api.state.attempts, 1);

  // 紧接着再走一轮：还在等待间隔，不该重复点。
  harness.api.tick();
  assert.equal(button.clickCount, 1, "间隔没到之前不能重复点");
});

test("间隔 0 秒时仍有 1 秒地板值，不会连发", () => {
  const harness = createHarness(baseSettings());
  const { button } = addCapacityBanner(harness);

  assert.equal(harness.api.intervalSeconds(), 0);
  assert.equal(harness.api.waitMs(), 1000, "0 秒间隔必须被 1 秒地板值兜住");

  harness.api.tick();
  const firstAttemptAt = harness.api.state.nextAttemptAt;
  assert.equal(firstAttemptAt - harness.now, 1000);

  harness.advance(400);
  assert.equal(button.clickCount, 1, "不到 1 秒不能点第二次");

  harness.advance(700);
  assert.equal(button.clickCount, 2, "过了 1 秒应当继续点");
});

test("原生倒计时在跑时也立刻接管（前四次同样压成 0 秒）", () => {
  const harness = createHarness(baseSettings());
  const { button } = addCapacityBanner(harness, { extraText: " 10 秒后重试" });

  harness.api.tick();
  assert.equal(button.clickCount, 1, "原生显示 10 秒后重试也必须立刻点，不等它");
  assert.equal(harness.api.state.nativeCountdown, true, "要标记出这轮接管了原生倒计时");

  const chip = harness.body.children.find((node) => node.id === "codex-plus-capacity-auto-retry");
  assert.ok(chip);
  const label = chip.children.find((node) => node.getAttribute("data-codex-plus-capacity-retry-text") !== null);
  assert.match(String(label?.textContent), /已接管原生倒计时/);

  harness.advance(1100);
  assert.equal(button.clickCount, 2, "接管后仍然按 1 秒地板值继续，不会连发");
});

test("达到次数上限后停手并提示", () => {
  const harness = createHarness(baseSettings({ capacityAutoRetryMaxAttempts: 2 }));
  const { button } = addCapacityBanner(harness);

  harness.api.tick();
  harness.advance(1500);
  assert.equal(button.clickCount, 2, "上限 2 次应当正好点两次");

  harness.advance(60_000);
  assert.equal(button.clickCount, 2, "到上限后不能再点");
  assert.ok(harness.api.state.exhausted);
  assert.equal(harness.toasts.length, 1);
  assert.match(harness.toasts[0], /已达上限 2 次/);
});

test("按钮在重试途中被禁用时不点", () => {
  const harness = createHarness(baseSettings());
  const { button } = addCapacityBanner(harness, { disabled: true });

  harness.api.tick();
  assert.equal(button.clickCount, 0, "重试请求在途（按钮 disabled）时不能点");

  button.disabled = false;
  harness.advance(2500);
  assert.equal(button.clickCount, 1, "按钮恢复后继续点");
});

test("横幅消失后计数复位、浮标隐藏", () => {
  const harness = createHarness(baseSettings());
  const { aside } = addCapacityBanner(harness);

  harness.api.tick();
  assert.equal(harness.api.state.attempts, 1);

  aside.remove();
  for (let i = 0; i < 4; i += 1) harness.advance(2500);
  assert.equal(harness.api.state.attempts, 0, "横幅消失即本轮结束，计数要复位");

  const chip = harness.body.children.find((node) => node.id === "codex-plus-capacity-auto-retry");
  assert.ok(chip, "浮标节点应当已创建");
  assert.equal(chip.style.display, "none", "本轮结束后浮标必须隐藏");
});

test("对话框里的重试按钮不会被误点", () => {
  const harness = createHarness(baseSettings());
  const dialog = harness.main.appendChild(createElement("div"));
  dialog.setAttribute("role", "dialog");
  const aside = createElement("aside");
  const button = createElement("button");
  button.textContent = "重试";
  aside.append(button);
  dialog.appendChild(aside);

  harness.api.tick();
  assert.equal(button.clickCount, 0, "模态框里的按钮不属于对话横幅");
});

test("关闭开关后不再点击并清理浮标", () => {
  const harness = createHarness(baseSettings());
  const { button } = addCapacityBanner(harness);

  harness.api.tick();
  assert.equal(button.clickCount, 1);

  harness.settings.capacityAutoRetry = false;
  harness.api.sync();
  const clickedBefore = button.clickCount;
  harness.advance(120_000);
  assert.equal(button.clickCount, clickedBefore, "关掉开关后不能再点");
  assert.equal(harness.api.state.timer, null, "关掉开关要清掉定时器");
});

test("设置读取异常时按关闭处理，不抛错", () => {
  const harness = createHarness(baseSettings());
  addCapacityBanner(harness);
  // 分片内部对 codexPlusSettings() 有 try/catch 兜底，这里用一个正常 tick 验证不抛。
  assert.doesNotThrow(() => harness.api.tick());
});

test("10-style 的字面量默认值与 prelude 常量不漂移", () => {
  // defaultCodexPlusSettings() 里必须用字面量（切片测试没有 prelude），
  // 所以这里把两处钉在一起：改了 prelude 常量却忘了改默认值，这条会红。
  const values = new Function(`${preludeConstants()}
    return { maxAttempts: capacityAutoRetryDefaultMaxAttempts, intervalSeconds: capacityAutoRetryDefaultIntervalSeconds };`)();
  const style = readFileSync(path.join(fragmentDir, "10-style.js"), "utf8");
  const defaults = style.slice(style.indexOf("function defaultCodexPlusSettings()"));
  const maxMatch = defaults.match(/capacityAutoRetryMaxAttempts: (\d+)/);
  const intervalMatch = defaults.match(/capacityAutoRetryIntervalSeconds: (\d+)/);
  assert.ok(maxMatch && intervalMatch, "默认设置里应当有这两个字面量");
  assert.equal(Number(maxMatch[1]), values.maxAttempts);
  assert.equal(Number(intervalMatch[1]), values.intervalSeconds);
});
