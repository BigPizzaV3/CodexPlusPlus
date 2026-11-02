import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import createBackend from "../backend/main.mjs";
import requestHook from "../hooks/request.mjs";

const serviceName = "codexpp.riceMaidPet";

function startBackend() {
  const events = [];
  const services = new Map();
  const backend = createBackend({
    emit(event, payload) {
      events.push({ event, payload });
    },
    provide(name, service) {
      assert.equal(services.has(name), false);
      services.set(name, service);
    },
  });
  assert.deepEqual([...services.keys()], [serviceName]);
  return { backend, events, service: services.get(serviceName) };
}

async function check(name, run) {
  await run();
  console.log(`ok - ${name}`);
}

await check("backend starts empty and exposes metadata only", () => {
  const { backend, events } = startBackend();
  const initial = backend.getState();
  assert.deepEqual(Object.keys(initial).sort(), ["at", "count", "epoch"]);
  assert.match(initial.epoch, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i);
  assert.equal(initial.count, 0);
  assert.equal(initial.at, 0);
  assert.deepEqual(events, []);
});

await check("concurrent requests produce exactly one ordered bite each", async () => {
  const { backend, events, service } = startBackend();
  const initial = backend.getState();
  const startedAt = Date.now();
  const requests = 128;
  const results = await Promise.all(
    Array.from({ length: requests }, () => Promise.resolve().then(() => service.requestBite())),
  );
  assert.equal(events.length, requests);
  assert.equal(backend.getState().count, requests);
  for (let index = 0; index < requests; index += 1) {
    assert.equal(events[index].event, "bite");
    assert.equal(results[index].count, index + 1);
    assert.equal(results[index].epoch, initial.epoch);
    assert.ok(results[index].at >= startedAt);
    assert.ok(results[index].at <= Date.now());
    assert.deepEqual(Object.keys(results[index]).sort(), ["at", "count", "epoch"]);
    assert.deepEqual(events[index].payload, results[index]);
  }
  assert.deepEqual(backend.getState(), results.at(-1));
});

await check("snapshots and returned events cannot mutate backend state", () => {
  const { backend, events, service } = startBackend();
  const initial = backend.getState();
  const epoch = initial.epoch;
  initial.epoch = "changed";
  initial.count = 1000;
  initial.at = -1;
  assert.deepEqual(backend.getState(), { epoch, count: 0, at: 0 });

  const first = service.requestBite();
  const expected = { ...first };
  first.count = -100;
  first.epoch = "changed";
  events[0].payload.at = -1;
  assert.deepEqual(backend.getState(), expected);
  assert.equal(service.requestBite().count, 2);
});

await check("restarting the backend gives a new epoch and empty counter", () => {
  const first = startBackend();
  first.service.requestBite();
  const second = startBackend();
  assert.notEqual(first.backend.getState().epoch, second.backend.getState().epoch);
  assert.equal(first.backend.getState().count, 1);
  assert.equal(second.backend.getState().count, 0);
  assert.equal(second.backend.getState().at, 0);
});

await check("request hook ignores the payload and passes no request data", async () => {
  const forbiddenPayload = new Proxy({}, {
    get() {
      assert.fail("hook must not read request payload");
    },
    ownKeys() {
      assert.fail("hook must not enumerate request payload");
    },
  });
  let calls = 0;
  const result = await requestHook(forbiddenPayload, {
    service(name) {
      assert.equal(name, serviceName);
      return {
        requestBite(...args) {
          calls += 1;
          assert.deepEqual(args, []);
          return { count: calls };
        },
      };
    },
  });
  assert.equal(calls, 1);
  assert.equal(result, null);
});

await check("request hook waits for delivery and still returns no hook output", async () => {
  let resolveBite;
  let complete = false;
  const delivery = new Promise((resolve) => {
    resolveBite = resolve;
  });
  const result = requestHook(null, {
    service: () => ({ requestBite: () => delivery }),
  }).then((value) => {
    complete = true;
    return value;
  });
  await Promise.resolve();
  assert.equal(complete, false);
  resolveBite({ epoch: "test", count: 1, at: 1 });
  assert.equal(await result, null);
  assert.equal(complete, true);
});

await check("request hook failures never alter or reject the model request", async () => {
  const contexts = [
    { service: () => { throw new Error("service unavailable"); } },
    { service: () => ({ requestBite: () => { throw new Error("backend unavailable"); } }) },
    { service: () => ({ requestBite: async () => { throw new Error("delivery failed"); } }) },
  ];
  for (const context of contexts) {
    assert.equal(await requestHook(null, context), null);
  }
});

const rendererSource = readFileSync(new URL("../renderer.js", import.meta.url), "utf8");
const clone = (value) => value === undefined ? undefined : JSON.parse(JSON.stringify(value));
const flush = async () => {
  for (let index = 0; index < 6; index += 1) await Promise.resolve();
};

function rendererHarness(options = {}) {
  const created = [];
  const timers = new Map();
  const cleanups = [];
  const listeners = new Map();
  const scans = new Set();
  const backendListeners = new Map();
  const menus = [];
  const failures = [];
  const calls = [];
  const storage = new Map(Object.entries(options.storage ?? {}).map(([key, value]) => [key, clone(value)]));
  let now = 0;
  let nextTimer = 0;
  let extensionCalls = 0;

  class Element {
    constructor(tag, host = false) {
      this.tagName = tag.toUpperCase();
      this.host = host;
      this.children = [];
      this.parentNode = null;
      this.attributes = new Map();
      this.style = {};
      this.dataHistory = [];
      this.dataset = new Proxy({}, {
        set: (target, key, value) => {
          this.dataHistory.push({ key, value });
          target[key] = value;
          return true;
        },
      });
      this.captured = new Set();
    }

    get isConnected() {
      return this.host || !!this.parentNode?.isConnected;
    }

    setAttribute(key, value) {
      this.attributes.set(key, String(value));
    }

    getAttribute(key) {
      return this.attributes.get(key) ?? null;
    }

    append(...children) {
      for (const child of children) {
        assert.equal(child.getAttribute("data-codex-plus-ext"), "rice-maid-pet");
        child.remove();
        child.parentNode = this;
        this.children.push(child);
      }
    }

    remove() {
      if (!this.parentNode) return;
      const siblings = this.parentNode.children;
      siblings.splice(siblings.indexOf(this), 1);
      this.parentNode = null;
    }

    setPointerCapture(pointerId) {
      this.captured.add(pointerId);
    }

    hasPointerCapture(pointerId) {
      return this.captured.has(pointerId);
    }

    releasePointerCapture(pointerId) {
      this.captured.delete(pointerId);
    }
  }

  const document = {
    hidden: options.hidden ?? false,
    head: new Element("head", true),
    body: new Element("body", true),
    createElement(tag) {
      const element = new Element(tag);
      created.push(element);
      return element;
    },
  };
  const window = { innerWidth: options.width ?? 1200, innerHeight: options.height ?? 900 };
  const track = (cleanup) => {
    cleanups.push(cleanup);
    return cleanup;
  };
  const forbidden = () => assert.fail("renderer must not read sessions or make network requests");
  const ctx = {
    config: { size: options.size ?? 192 },
    storage: {
      get(key, fallback) {
        assert.ok(["visible", "position", "bites"].includes(key));
        return clone(storage.has(key) ? storage.get(key) : fallback);
      },
      set(key, value) {
        storage.set(key, clone(value));
      },
    },
    asset(path) {
      assert.equal(path, "assets/rice-maid-sequence.webp");
      return "data:image/png;base64,test";
    },
    effect(run) {
      return track(run());
    },
    listen(target, type, handler) {
      if (!listeners.has(target)) listeners.set(target, new Map());
      const targetListeners = listeners.get(target);
      if (!targetListeners.has(type)) targetListeners.set(type, new Set());
      targetListeners.get(type).add(handler);
      return track(() => targetListeners.get(type).delete(handler));
    },
    setTimeout(run, delay) {
      assert.ok(Number.isFinite(delay) && delay >= 0);
      const id = ++nextTimer;
      timers.set(id, { at: now + delay, run });
      return track(() => timers.delete(id));
    },
    onScan(run, scanOptions) {
      assert.equal(scanOptions.phase, "light");
      scans.add(run);
      return track(() => scans.delete(run));
    },
    registerMenuItem(definition) {
      menus.push(definition);
    },
    backend: {
      on(event, handler) {
        assert.equal(event, "bite");
        backendListeners.set(event, handler);
        return track(() => backendListeners.delete(event));
      },
      call(method) {
        assert.equal(method, "getState");
        calls.push(method);
        return Promise.resolve(options.getState ? options.getState() : { epoch: "first", count: 0, at: 0 });
      },
    },
    fail(error) {
      failures.push(error);
    },
    get sessionQuery() { return forbidden(); },
    get sessions() { return forbidden(); },
    call: forbidden,
  };
  vm.runInNewContext(rendererSource, {
    codexPlus: {
      extension(apply) {
        extensionCalls += 1;
        apply(ctx);
      },
    },
    document,
    window,
    fetch: forbidden,
    XMLHttpRequest: forbidden,
    MutationObserver: forbidden,
  }, { filename: "rice-maid-pet/renderer.js" });
  assert.equal(extensionCalls, 1);
  const root = created.find((element) => element.id === "codex-plus-ext-rice-maid-pet");
  const pet = created.find((element) => element.className === "rice-maid-pet-body");
  assert.ok(root && pet);
  return {
    root, pet, document, window, created, menus, storage, timers, failures, calls,
    bite(payload) {
      backendListeners.get("bite")?.(payload);
    },
    dispatch(target, type, values = {}) {
      const event = {
        pointerId: 1, button: 0, isPrimary: true, clientX: 0, clientY: 0,
        defaultPrevented: false,
        preventDefault() { this.defaultPrevented = true; },
        ...values,
      };
      for (const handler of listeners.get(target)?.get(type) ?? []) handler(event);
      return event;
    },
    scan() {
      for (const run of scans) run();
    },
    bounds() {
      return Object.fromEntries(["left", "top", "width", "height"].map((key) => [key, parseFloat(root.style[key])]));
    },
    async advance(ms) {
      const end = now + ms;
      let steps = 0;
      while (true) {
        const next = [...timers.entries()].filter(([, timer]) => timer.at <= end)
          .sort((left, right) => left[1].at - right[1].at)[0];
        if (!next) break;
        assert.ok(++steps < 1000, "timer loop must remain bounded");
        now = next[1].at;
        timers.delete(next[0]);
        next[1].run();
        await flush();
      }
      now = end;
      await flush();
    },
    dispose() {
      for (const cleanup of cleanups.splice(0).reverse()) cleanup();
    },
    listenerCount() {
      let count = 0;
      for (const types of listeners.values()) {
        for (const handlers of types.values()) count += handlers.size;
      }
      return count;
    },
  };
}

function assertInside(harness) {
  const { left, top, width, height } = harness.bounds();
  for (const value of [left, top, width, height]) assert.ok(Number.isFinite(value));
  assert.ok(left >= 0 && top >= 0 && width > 0 && height > 0);
  assert.ok(left + width <= harness.window.innerWidth + 1e-8, "pet must fit viewport width");
  assert.ok(top + height <= harness.window.innerHeight + 1e-8, "pet must fit viewport height");
}

const frameDurations = [100, 100, 120, 100, 100, 180, 100, 100, 160, 100, 100];
const animationMs = frameDurations.reduce((total, duration) => total + duration, 0);
const cooldownMs = 100;
const cycleMs = animationMs + cooldownMs;
const cycleFrames = Array.from({ length: 11 }, (_, index) => String(index + 1)).concat("0");

await check("renderer counts every request but caps a burst at two twelve-frame animations", async () => {
  const harness = rendererHarness();
  await flush();
  assert.equal(harness.root.dataset.bites, "0");
  harness.bite({ epoch: "first", count: 1 });
  harness.bite({ epoch: "first", count: 3 });
  harness.bite({ epoch: "first", count: 3 });
  harness.bite({ epoch: "first", count: 2 });
  assert.equal(harness.root.dataset.bites, "3");
  assert.equal(harness.root.dataset.frame, "1");
  for (let index = 0; index < frameDurations.length; index += 1) {
    await harness.advance(frameDurations[index]);
    assert.equal(harness.root.dataset.frame, String(index === 10 ? 0 : index + 2));
  }
  assert.equal(harness.root.dataset.frame, "0");
  assert.equal(harness.root.dataset.phase, "idle");
  await harness.advance(cooldownMs - 1);
  assert.equal(harness.root.dataset.frame, "0");
  await harness.advance(1);
  assert.equal(harness.root.dataset.frame, "1");
  await harness.advance(cycleMs);
  assert.deepEqual(
    harness.root.dataHistory.filter(({ key }) => key === "frame").map(({ value }) => value),
    ["0", ...cycleFrames, ...cycleFrames],
  );
  assert.equal(harness.root.dataset.phase, "idle");
  assert.equal(harness.storage.get("bites"), 3);
  assert.match(harness.pet.getAttribute("aria-label"), /3/);
  harness.dispose();
});

await check("rapid clicking and menu feeding never pile up manual bites", async () => {
  const harness = rendererHarness();
  await flush();
  const feedMenu = harness.menus.find((menu) => menu.label === "喂一口米饭");
  assert.ok(feedMenu);
  const phases = [0, 100, 520, animationMs, cycleMs - 1];
  let now = 0;
  let menuCloses = 0;
  for (const time of phases) {
    await harness.advance(time - now);
    now = time;
    for (let click = 0; click < 100; click += 1) {
      harness.dispatch(harness.pet, "click");
      feedMenu.onActivate({ close: () => { menuCloses += 1; } });
    }
    assert.equal(harness.storage.get("bites"), 1);
    assert.ok(harness.timers.size <= 2, "only one frame timer and one sync timer may be active");
  }
  assert.equal(menuCloses, 500);
  await harness.advance(cycleMs - now);
  assert.equal(harness.root.dataset.frame, "0");
  assert.equal(harness.root.dataset.phase, "idle");
  assert.equal(harness.timers.size, 1);
  harness.dispatch(harness.pet, "click");
  assert.equal(harness.root.dataset.bites, "2");
  assert.equal(harness.root.dataset.frame, "1");
  await harness.advance(cycleMs);
  assert.deepEqual(
    harness.root.dataHistory.filter(({ key }) => key === "frame").map(({ value }) => value),
    ["0", ...cycleFrames, ...cycleFrames],
  );
  harness.dispose();
});

await check("manual bites and request bursts coexist with one pending visual cycle", async () => {
  const harness = rendererHarness();
  await flush();
  harness.dispatch(harness.pet, "click");
  harness.bite({ epoch: "first", count: 1 });
  harness.bite({ epoch: "first", count: 3 });
  for (let index = 0; index < 40; index += 1) harness.dispatch(harness.pet, "click");
  assert.equal(harness.root.dataset.bites, "4");
  assert.equal(harness.root.dataset.requestCount, "3");
  await harness.advance(cycleMs);
  assert.equal(harness.root.dataset.frame, "1", "the pending request cycle starts after cooldown");
  harness.bite({ epoch: "first", count: 10 });
  for (let index = 0; index < 40; index += 1) harness.dispatch(harness.pet, "click");
  assert.equal(harness.root.dataset.bites, "11");
  await harness.advance(cycleMs);
  assert.equal(harness.root.dataset.frame, "1", "later requests coalesce into one final pending cycle");
  await harness.advance(cycleMs);
  assert.equal(harness.root.dataset.phase, "idle");
  assert.equal(harness.root.dataset.frame, "0");
  assert.equal(harness.timers.size, 1);
  assert.deepEqual(
    harness.root.dataHistory.filter(({ key }) => key === "frame").map(({ value }) => value),
    ["0", ...cycleFrames, ...cycleFrames, ...cycleFrames],
  );
  harness.dispose();
});

await check("cooldown is still busy and cannot be extended by rapid manual input", async () => {
  const harness = rendererHarness();
  await flush();
  harness.dispatch(harness.pet, "click");
  await harness.advance(animationMs);
  assert.equal(harness.root.dataset.phase, "idle");
  assert.equal(harness.root.dataset.frame, "0");
  for (let index = 0; index < 100; index += 1) harness.dispatch(harness.pet, "click");
  assert.equal(harness.root.dataset.bites, "1");
  await harness.advance(cooldownMs - 1);
  harness.dispatch(harness.pet, "click");
  assert.equal(harness.root.dataset.bites, "1");
  await harness.advance(1);
  assert.equal(harness.timers.size, 1);
  harness.dispatch(harness.pet, "click");
  assert.equal(harness.root.dataset.bites, "2");
  assert.equal(harness.root.dataset.frame, "1");
  harness.dispose();
});

await check("renderer does not replay initial history or older snapshots", async () => {
  let snapshot = { epoch: "first", count: 50, at: 0 };
  const harness = rendererHarness({ getState: () => snapshot });
  await flush();
  assert.equal(harness.root.dataset.bites, "0");
  assert.equal(harness.root.dataset.requestCount, "50");
  harness.bite({ epoch: "first", count: 51 });
  snapshot = { epoch: "first", count: 50, at: 0 };
  await harness.advance(15000);
  assert.equal(harness.root.dataset.bites, "1");
  assert.equal(harness.root.dataset.requestCount, "51");
  snapshot = { epoch: "first", count: 53, at: 0 };
  await harness.advance(15000);
  assert.equal(harness.root.dataset.bites, "3");
  harness.bite(snapshot);
  assert.equal(harness.root.dataset.bites, "3");
  harness.dispose();
});

await check("renderer survives epoch changes and ignores retired events", async () => {
  const harness = rendererHarness();
  await flush();
  harness.bite({ epoch: "first", count: 2 });
  harness.bite({ epoch: "second", count: 1 });
  harness.bite({ epoch: "first", count: 3 });
  harness.bite({ epoch: "second", count: 1 });
  assert.equal(harness.root.dataset.bites, "3");
  assert.equal(harness.root.dataset.requestCount, "1");
  harness.bite({ epoch: "second", count: 2 });
  assert.equal(harness.root.dataset.bites, "4");
  harness.dispose();
});

await check("late initial snapshot cannot replace a newer epoch", async () => {
  let resolveSnapshot;
  const harness = rendererHarness({
    getState: () => new Promise((resolve) => { resolveSnapshot = resolve; }),
  });
  harness.bite({ epoch: "new", count: 1 });
  resolveSnapshot({ epoch: "old", count: 100, at: 0 });
  await flush();
  assert.equal(harness.root.dataset.bites, "1");
  assert.equal(harness.root.dataset.requestCount, "1");
  harness.bite({ epoch: "new", count: 2 });
  assert.equal(harness.root.dataset.bites, "2");
  harness.dispose();
});

await check("a first bite before the initial snapshot does not replay historical counts", async () => {
  let resolveSnapshot;
  const harness = rendererHarness({
    getState: () => new Promise((resolve) => { resolveSnapshot = resolve; }),
  });
  harness.bite({ epoch: "existing", count: 51 });
  assert.equal(harness.root.dataset.bites, "1");
  resolveSnapshot({ epoch: "existing", count: 51, at: 0 });
  await flush();
  assert.equal(harness.root.dataset.bites, "1");
  assert.equal(harness.root.dataset.requestCount, "51");
  harness.bite({ epoch: "existing", count: 52 });
  assert.equal(harness.root.dataset.bites, "2");
  assert.equal(harness.root.dataset.requestCount, "52");
  harness.dispose();
});

await check("dragging persists position without accidentally feeding", async () => {
  const harness = rendererHarness();
  await flush();
  const before = harness.bounds();
  harness.dispatch(harness.pet, "pointerdown", { clientX: 100, clientY: 100 });
  harness.dispatch(harness.pet, "pointermove", { clientX: 40, clientY: 70 });
  assert.equal(harness.root.dataset.dragging, "true");
  harness.dispatch(harness.pet, "pointerup");
  const click = harness.dispatch(harness.pet, "click");
  assert.equal(click.defaultPrevented, true);
  assert.equal(harness.root.dataset.bites, "0");
  assert.deepEqual(harness.storage.get("position"), { x: before.left - 60, y: before.top - 30 });
  assert.equal(harness.pet.captured.size, 0);
  harness.dispatch(harness.pet, "click");
  assert.equal(harness.root.dataset.bites, "1");
  harness.dispatch(harness.pet, "keydown", { key: "ArrowLeft", shiftKey: true });
  assert.equal(harness.storage.get("position").x, before.left - 84);
  harness.dispose();
});

await check("stored positions and resized tiny viewports remain bounded", async () => {
  for (const position of [
    { x: -1000, y: 1e8 }, { x: 1e8, y: -1000 }, { x: "broken", y: null }, null,
  ]) {
    const harness = rendererHarness({ storage: { position } });
    await flush();
    assertInside(harness);
    for (const [width, height] of [[320, 480], [112, 120], [16, 56], [8, 20], [1, 1]]) {
      harness.window.innerWidth = width;
      harness.window.innerHeight = height;
      harness.dispatch(harness.window, "resize");
      assertInside(harness);
      const { left, top } = harness.bounds();
      assert.deepEqual(harness.storage.get("position"), { x: left, y: top });
    }
    harness.dispose();
  }
});

await check("hiding through menu stops animation without a replay backlog", async () => {
  const harness = rendererHarness();
  await flush();
  harness.bite({ epoch: "first", count: 2 });
  harness.menus.find((menu) => menu.toggleValue).onChange(false);
  assert.equal(harness.root.hidden, true);
  assert.equal(harness.root.dataset.phase, "idle");
  harness.bite({ epoch: "first", count: 5 });
  await harness.advance(5000);
  harness.menus.find((menu) => menu.toggleValue).onChange(true);
  assert.equal(harness.root.hidden, false);
  assert.equal(harness.root.dataset.bites, "5");
  assert.equal(harness.root.dataset.frame, "0");
  await harness.advance(5000);
  assert.equal(harness.root.dataset.phase, "idle");
  harness.bite({ epoch: "first", count: 6 });
  assert.equal(harness.root.dataset.frame, "1");
  harness.dispose();
});

await check("background documents stop motion and do not replay hidden requests", async () => {
  const harness = rendererHarness();
  await flush();
  harness.bite({ epoch: "first", count: 1 });
  harness.document.hidden = true;
  harness.dispatch(harness.document, "visibilitychange");
  assert.equal(harness.root.dataset.phase, "idle");
  harness.bite({ epoch: "first", count: 4 });
  await harness.advance(5000);
  harness.document.hidden = false;
  harness.dispatch(harness.document, "visibilitychange");
  assert.equal(harness.root.dataset.bites, "4");
  assert.equal(harness.root.dataset.frame, "0");
  await harness.advance(5000);
  assert.equal(harness.root.dataset.phase, "idle");
  harness.dispose();
});

await check("owned DOM remounts without duplicate nodes and unload clears everything", async () => {
  const harness = rendererHarness();
  await flush();
  for (const element of harness.created) {
    assert.equal(element.getAttribute("data-codex-plus-ext"), "rice-maid-pet");
  }
  assert.equal(harness.document.body.children.length, 1);
  assert.equal(harness.document.head.children.length, 1);
  harness.root.remove();
  harness.scan();
  harness.scan();
  assert.equal(harness.document.body.children.length, 1);
  harness.bite({ epoch: "first", count: 1 });
  assert.ok(harness.timers.size > 0);
  harness.dispose();
  assert.equal(harness.document.body.children.length, 0);
  assert.equal(harness.document.head.children.length, 0);
  assert.equal(harness.timers.size, 0);
  assert.equal(harness.listenerCount(), 0);
  harness.scan();
  harness.bite({ epoch: "first", count: 2 });
  await harness.advance(20000);
  assert.equal(harness.document.body.children.length, 0);
  assert.deepEqual(harness.failures, []);
});

await check("late backend resolution after disposal cannot remount or restart timers", async () => {
  let resolveSnapshot;
  const harness = rendererHarness({
    getState: () => new Promise((resolve) => { resolveSnapshot = resolve; }),
  });
  harness.dispose();
  resolveSnapshot({ epoch: "late", count: 99, at: 0 });
  await flush();
  harness.scan();
  assert.equal(harness.document.body.children.length, 0);
  assert.equal(harness.document.head.children.length, 0);
  assert.equal(harness.timers.size, 0);
  assert.equal(harness.root.dataset.bites, "0");
});

await check("renderer never observes native DOM or reads conversation and network APIs", () => {
  assert.doesNotMatch(rendererSource, /\bMutationObserver\b|\bsessionQuery\b|\bsessions\b|\bfetch\s*\(|\bXMLHttpRequest\b/);
});

await check("twelve-frame RGBA atlas is bounded and retains the original source", () => {
  const atlas = readFileSync(new URL("../assets/rice-maid-sequence.webp", import.meta.url));
  assert.equal(atlas.toString("ascii", 0, 4), "RIFF");
  assert.equal(atlas.readUInt32LE(4) + 8, atlas.length);
  assert.equal(atlas.toString("ascii", 8, 16), "WEBPVP8X");
  assert.ok(atlas[20] & 0x10, "atlas must preserve transparency");
  assert.equal(atlas.readUIntLE(24, 3) + 1, 1024);
  assert.equal(atlas.readUIntLE(27, 3) + 1, 852);
  assert.ok(atlas.length < 1024 * 1024, "an extension asset must fit under 1 MiB");
  const source = readFileSync(new URL("../assets/rice-maid-sheet.png", import.meta.url));
  assert.deepEqual([...source.subarray(0, 8)], [137, 80, 78, 71, 13, 10, 26, 10]);
  assert.equal(source.readUInt32BE(16), 1024);
  assert.equal(source.readUInt32BE(20), 572);
  assert.ok(atlas.length + source.length < 4 * 1024 * 1024);
});

await check("one twelve-frame atlas advances through a stable 4x3 grid without ghosted crossfades", async () => {
  const harness = rendererHarness();
  const images = harness.created.filter((element) => element.tagName === "IMG");
  assert.equal(images.length, 1);
  const image = images[0];
  assert.equal(image.parentNode?.className, "rice-maid-pet-frame");
  const frame = image.parentNode;
  assert.equal(frame.parentNode?.className, "rice-maid-pet-motion");
  const css = harness.created.find((element) => element.tagName === "STYLE").textContent;
  const sheet = css.match(/\.rice-maid-pet-sheet\s*\{([^}]+)\}/)?.[1];
  assert.ok(sheet);
  assert.match(sheet, /width:\s*400%;\s*height:\s*300%/);
  assert.match(sheet, /left:\s*0;\s*top:\s*0/);
  assert.match(sheet, /will-change:\s*transform/);
  assert.doesNotMatch(sheet, /(?:opacity|transition):/);
  assert.match(css, /\.rice-maid-pet-motion\s*\{[^}]*animation:\s*rice-maid-pet-breathe 3\.8s ease-in-out infinite/s);
  assert.match(css, /\[data-phase="eating"\] \.rice-maid-pet-frame\s*\{[^}]*animation:\s*rice-maid-pet-eat 1\.26s ease-in-out both/s);
  assert.match(css, /\.rice-maid-pet-bubble\s*\{[^}]*transition:\s*opacity 120ms ease-in-out/s);
  assert.doesNotMatch(css, /rice-maid-pet-munch|rice-maid-pet-eat [^;]*infinite/);
  assert.match(css, /@media \(prefers-reduced-motion: reduce\)\s*\{[\s\S]*?rice-maid-pet-motion,[\s\S]*?rice-maid-pet-frame\s*\{\s*animation:\s*none !important;[\s\S]*?rice-maid-pet-bubble\s*\{\s*transition:\s*none !important;/);
  const assertGridPosition = (index) => {
    assert.equal(harness.root.dataset.frame, String(index));
    const match = /^translate3d\((-?[\d.]+)%, (-?[\d.]+)%, 0\)$/.exec(image.style.transform);
    assert.ok(match);
    assert.ok(Math.abs(Number(match[1]) + (index % 4) * 25) < 1e-8);
    assert.ok(Math.abs(Number(match[2]) + Math.floor(index / 4) * 100 / 3) < 1e-8);
  };
  harness.dispatch(harness.pet, "click");
  for (let index = 1; index <= 11; index += 1) {
    assertGridPosition(index);
    await harness.advance(frameDurations[index - 1]);
  }
  assertGridPosition(0);
  assert.equal(harness.root.dataset.phase, "idle");
  harness.dispose();
});
