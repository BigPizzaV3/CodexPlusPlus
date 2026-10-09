const assert = require("node:assert/strict");
const { test } = require("node:test");
const fs = require("node:fs");
const vm = require("node:vm");

const source = fs.readFileSync(require.resolve("./site.js"), "utf8");
const primaryUrl = "https://raw.githubusercontent.com/BigPizzaV3/Ad-List/main/ads.json";
const fallbackUrl = "https://cdn.jsdelivr.net/gh/BigPizzaV3/Ad-List@main/ads.json";
const now = Date.parse("2026-10-09T00:00:00Z");
const sponsor = (title, url = `https://example.com/${encodeURIComponent(title)}`) => ({
  type: "sponsor",
  title,
  description: `Description for ${title}`,
  url,
});

class Element {
  constructor(tag) {
    this.tagName = tag.toUpperCase();
    this.children = [];
    this.attributes = new Map();
    this.listeners = new Map();
    this.text = "";
  }
  get textContent() { return this.text + this.children.map((child) => child.textContent).join(""); }
  set textContent(value) { this.text = String(value); this.replaceChildren(); }
  set innerHTML(value) { throw new Error("Sponsor content must not use innerHTML"); }
  setAttribute(name, value) { this.attributes.set(name, String(value)); }
  append(...children) {
    children.forEach((child) => { child.parent = this; this.children.push(child); });
  }
  replaceChildren(...children) {
    this.children.forEach((child) => { child.parent = null; });
    this.children = [];
    this.append(...children);
  }
  replaceWith(element) {
    const index = this.parent.children.indexOf(this);
    element.parent = this.parent;
    this.parent.children[index] = element;
    this.parent = null;
  }
  addEventListener(event, callback) { this.listeners.set(event, callback); }
  dispatch(event) { this.listeners.get(event)?.(); }
}

function fixture(respond = async () => ({ ok: true, json: async () => ({ ads: [] }) })) {
  const requests = [];
  const timers = new Map();
  const elements = new Map();
  let nextTimer = 0;
  const setTimer = (callback, delay) => {
    const id = ++nextTimer;
    timers.set(id, { callback, delay });
    return id;
  };
  const clearTimer = (id) => timers.delete(id);
  const context = vm.createContext({
    document: {
      documentElement: { dataset: { language: "zh" } },
      body: { classList: { contains: () => false } },
      createElement: (tag) => new Element(tag),
      createTreeWalker: () => ({ nextNode: () => null }),
      querySelector: (selector) => elements.get(selector) || null,
      querySelectorAll: () => [],
    },
    navigator: { platform: "Win32" },
    window: { addEventListener() {}, scrollY: 0, setTimeout: setTimer, clearTimeout: clearTimer },
    NodeFilter: { SHOW_TEXT: 4 },
    HTMLCanvasElement: class {},
    localStorage: { setItem() {} },
    Date: class extends Date { static now() { return now; } },
    URL,
    AbortController,
    setTimeout: setTimer,
    clearTimeout: clearTimer,
    console: { info() {}, warn() {}, error() {} },
    fetch: async (url, options) => {
      if (!String(url).includes("ads.json")) return { ok: false, status: 503 };
      requests.push({ url: String(url), options });
      return respond(String(url), options);
    },
  });
  vm.runInContext(`${source}\nglobalThis.sponsorTestApi = { normalizeSponsors, fetchSponsorList, createSponsorCard, updateSponsors, setupSponsors };`, context, { filename: "site.js" });
  const mountSponsors = () => {
    const nodes = Object.fromEntries(["featured", "grid", "status", "retry"].map((name) => {
      const element = new Element(name === "retry" ? "button" : "div");
      elements.set(`[data-sponsor-${name}]`, element);
      return [name, element];
    }));
    return nodes;
  };
  return { ...context.sponsorTestApi, requests, timers, mountSponsors };
}

const titles = (items) => Array.from(items, (item) => item.title);
function assertFreshRequest(request, expectedSource) {
  const requested = new URL(request.url);
  const expected = new URL(expectedSource);
  assert.equal(requested.origin + requested.pathname, expected.origin + expected.pathname);
  assert.equal(requested.searchParams.get("v"), String(now));
  assert.equal(request.options.cache, "no-store");
  assert.equal(request.options.credentials, "omit");
  assert.ok(request.options.signal);
}

test("normalization keeps complete sponsor text without requiring an image", () => {
  const description = "A complete introduction with <em>literal text</em>.\n" + "Details, prices, and contact information. ".repeat(30).trimEnd();
  const input = { ...sponsor("Text-only sponsor"), description };
  const result = fixture().normalizeSponsors({ ads: [input] }, primaryUrl, now);
  assert.equal(result.featured.length, 0);
  assert.equal(result.ads.length, 1);
  assert.equal(result.ads[0].title, input.title);
  assert.equal(result.ads[0].description, description);
  assert.equal(result.ads[0].url, input.url);
  assert.ok(!result.ads[0].image);
});

test("normalization excludes non-sponsors, invalid required fields, and expired sponsors", () => {
  const entries = [
    { ...sponsor("Other type"), type: "recommendation" },
    { ...sponsor("Missing type"), type: undefined },
    { ...sponsor("Missing title"), title: " " },
    { ...sponsor("Missing description"), description: " " },
    { ...sponsor("Missing URL"), url: "" },
    { ...sponsor("Relative URL"), url: "/advertiser" },
    { ...sponsor("Invalid protocol"), url: "javascript:alert(1)" },
    { ...sponsor("Expired"), expires_at: "2026-10-08T23:59:59Z" },
    { ...sponsor("Future"), expires_at: "2026-10-10T00:00:00Z" },
    { ...sponsor("Equal"), expires_at: "2026-10-09T00:00:00Z" },
    { ...sponsor("Invalid expiry"), expires_at: "not a date" },
  ];
  const result = fixture().normalizeSponsors({ top_ad: sponsor("Featured"), ads: entries }, primaryUrl, now);
  assert.deepEqual(titles(result.featured), ["Featured"]);
  assert.deepEqual(titles(result.ads), ["Future", "Equal", "Invalid expiry"]);
});

test("featured aliases accept objects and arrays", () => {
  const api = fixture();
  for (const [key, value] of [
    ["top_ad", sponsor("First")],
    ["top_ad", [sponsor("First"), sponsor("Second")]],
    ["topAd", sponsor("First")],
    ["topAd", [sponsor("First"), sponsor("Second")]],
    ["topAds", [sponsor("First"), sponsor("Second")]],
  ]) {
    const result = api.normalizeSponsors({ [key]: value, ads: [] }, primaryUrl, now);
    assert.deepEqual(titles(result.featured), Array.isArray(value) ? ["First", "Second"] : ["First"]);
  }
});

test("raw featured entries take priority, then nonempty topAds, then topAd", () => {
  const api = fixture();
  const fields = { top_ad: sponsor("Raw"), topAds: [sponsor("Normalized")], topAd: sponsor("Legacy") };
  assert.deepEqual(titles(api.normalizeSponsors(fields, primaryUrl, now).featured), ["Raw"]);
  assert.deepEqual(titles(api.normalizeSponsors({ ...fields, top_ad: undefined }, primaryUrl, now).featured), ["Normalized"]);
  assert.deepEqual(titles(api.normalizeSponsors({ ...fields, top_ad: undefined, topAds: [] }, primaryUrl, now).featured), ["Legacy"]);
});

test("featured sponsors take priority and URL deduplication preserves the remaining order", () => {
  const result = fixture().normalizeSponsors({
    topAds: [
      sponsor("Featured first", "https://EXAMPLE.com/offer/?aff=top#banner"),
      sponsor("Featured duplicate", "http://example.com/offer?aff=other"),
      sponsor("Featured second", "https://example.com/second"),
    ],
    ads: [
      sponsor("Ordinary featured duplicate", "https://example.com/offer?aff=list"),
      sponsor("Ordinary first", "https://example.com/regular/?campaign=one"),
      sponsor("Ordinary duplicate", "https://example.com/regular#campaign-two"),
      sponsor("Ordinary second", "https://example.com/final"),
    ],
  }, primaryUrl, now);
  assert.deepEqual(titles(result.featured), ["Featured first", "Featured second"]);
  assert.deepEqual(titles(result.ads), ["Ordinary first", "Ordinary second"]);
});

test("primary fetch uses fresh credential-free requests and returns the live payload", async () => {
  const payload = { ads: [sponsor("Live sponsor")] };
  const api = fixture(async () => ({ ok: true, json: async () => payload }));
  const result = await api.fetchSponsorList();
  assert.equal(result.payload, payload);
  assert.equal(new URL(result.sourceUrl).origin + new URL(result.sourceUrl).pathname, primaryUrl);
  assert.equal(api.requests.length, 1);
  assertFreshRequest(api.requests[0], primaryUrl);
  assert.equal(api.timers.size, 0);
});

test("a failed primary source falls back to jsDelivr", async () => {
  const payload = { ads: [sponsor("Fallback sponsor")] };
  const api = fixture(async (url) => url.startsWith(primaryUrl)
    ? { ok: false, status: 503 }
    : { ok: true, json: async () => payload });
  const result = await api.fetchSponsorList();
  assert.equal(result.payload, payload);
  assert.equal(new URL(result.sourceUrl).origin + new URL(result.sourceUrl).pathname, fallbackUrl);
  assert.equal(api.requests.length, 2);
  assertFreshRequest(api.requests[0], primaryUrl);
  assertFreshRequest(api.requests[1], fallbackUrl);
  assert.equal(api.timers.size, 0);
});

test("a primary timeout aborts the request and continues with the fallback", async () => {
  const payload = { ads: [sponsor("After timeout")] };
  const api = fixture(async (url, options) => {
    if (!url.startsWith(primaryUrl)) return { ok: true, json: async () => payload };
    return new Promise((resolve, reject) => options.signal.addEventListener("abort", () => reject(new Error("Aborted")), { once: true }));
  });
  const pending = api.fetchSponsorList();
  const timer = api.timers.values().next().value;
  assert.ok(timer && Number.isFinite(timer.delay) && timer.delay > 0);
  timer.callback();
  const result = await pending;
  assert.equal(api.requests[0].options.signal.aborted, true);
  assert.equal(result.payload, payload);
  assertFreshRequest(api.requests[1], fallbackUrl);
  assert.equal(api.timers.size, 0);
});

test("failure of every live source rejects instead of returning static advertisers", async () => {
  const api = fixture(async () => { throw new Error("Unavailable"); });
  await assert.rejects(api.fetchSponsorList());
  assert.equal(api.requests.length, 2);
  assertFreshRequest(api.requests[0], primaryUrl);
  assertFreshRequest(api.requests[1], fallbackUrl);
  assert.equal(api.timers.size, 0);
});

test("invalid source structure tries the fallback while an empty ads array is valid", async () => {
  const payload = { ads: [] };
  const api = fixture(async (url) => ({ ok: true, json: async () => url.startsWith(primaryUrl) ? { unrelated: [] } : payload }));
  const result = await api.fetchSponsorList();
  assert.equal(result.payload, payload);
  assert.equal(api.requests.length, 2);
  assertFreshRequest(api.requests[1], fallbackUrl);
});

test("cards render HTML-like title, description, and highlights as complete text", () => {
  const api = fixture();
  const input = {
    ...sponsor("<img src=x onerror=alert(1)> Sponsor", "https://example.com/safe"),
    description: "<script>alert(1)</script> & complete description\n" + "Unabridged details. ".repeat(20).trimEnd(),
    highlights: ["<strong>Literal highlight</strong>", "Second & complete highlight"],
  };
  const normalized = api.normalizeSponsors({ ads: [input] }, primaryUrl, now).ads[0];
  const card = api.createSponsorCard(normalized, true);
  assert.deepEqual(card.children.map((child) => child.tagName), ["SPAN", "DIV"]);
  const copy = card.children[1];
  assert.deepEqual(copy.children.map((child) => child.tagName), ["H4", "P", "UL"]);
  assert.equal(copy.children[0].textContent, `${input.title} ↗`);
  assert.equal(copy.children[1].textContent, input.description);
  assert.deepEqual(copy.children[2].children.map((child) => child.textContent), input.highlights);
  assert.equal(card.children[0].textContent, "<");
});

test("relative sponsor images resolve from the live source and failures get a text placeholder", () => {
  const api = fixture();
  const ad = api.normalizeSponsors({ ads: [{ ...sponsor("Image sponsor"), image: "./logos/logo.svg" }] }, primaryUrl, now).ads[0];
  const card = api.createSponsorCard(ad, false);
  const image = card.children[0];
  assert.equal(image.tagName, "IMG");
  assert.equal(image.src, "https://raw.githubusercontent.com/BigPizzaV3/Ad-List/main/logos/logo.svg");
  image.dispatch("error");
  assert.equal(card.children[0].tagName, "SPAN");
  assert.equal(card.children[0].textContent, "I");
  assert.equal(card.children[1].children[1].textContent, ad.description);
});

test("empty live data clears existing cards and shows the empty state without using the fallback", async () => {
  const api = fixture();
  const nodes = api.mountSponsors();
  nodes.featured.append(new Element("a"));
  nodes.grid.append(new Element("a"));
  await api.updateSponsors();
  assert.equal(nodes.featured.children.length, 0);
  assert.equal(nodes.grid.children.length, 0);
  assert.equal(nodes.status.textContent, "暂无有效赞助商。");
  assert.equal(nodes.status.hidden, false);
  assert.equal(nodes.retry.hidden, true);
  assert.equal(api.requests.length, 1);
});

test("all-source failure clears cards and retry can load the live list", async () => {
  let attempts = 0;
  const api = fixture(async () => {
    if (++attempts <= 2) throw new Error("Unavailable");
    return { ok: true, json: async () => ({ ads: [sponsor("Recovered sponsor")] }) };
  });
  const nodes = api.mountSponsors();
  nodes.grid.append(new Element("a"));
  api.setupSponsors();
  await new Promise(setImmediate);
  assert.equal(nodes.featured.children.length, 0);
  assert.equal(nodes.grid.children.length, 0);
  assert.equal(nodes.status.textContent, "赞助商列表暂时无法加载，请稍后重试或查看来源。");
  assert.equal(nodes.status.hidden, false);
  assert.equal(nodes.retry.hidden, false);
  nodes.retry.dispatch("click");
  await new Promise(setImmediate);
  assert.equal(nodes.grid.children.length, 1);
  assert.match(nodes.grid.children[0].textContent, /Recovered sponsor/);
  assert.equal(nodes.status.hidden, true);
  assert.equal(nodes.retry.hidden, true);
  assert.equal(api.requests.length, 3);
});
