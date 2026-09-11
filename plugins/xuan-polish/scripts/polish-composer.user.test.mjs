import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";

const source = fs.readFileSync(path.join(import.meta.dirname, "polish-composer.user.js"), "utf8");

test("polish script is a thin feature-detected composer adapter", () => {
  assert.match(source, /MutationObserver/);
  assert.match(source, /data-xuan-polish-button/);
  assert.match(source, /127\.0\.0\.1:57324/);
  assert.doesNotMatch(source, /api[_-]?key|authorization|bearer/i);
});

test("polish script installs once and replaces composer text", async () => {
  const events = [];
  const appended = [];
  const textarea = {
    value: "  rough draft  ",
    parentElement: { append(node) { appended.push(node); } },
    dispatchEvent(event) { events.push(event.type); }
  };
  class FakeButton {
    constructor() {
      this.listeners = new Map();
      this.disabled = false;
      this.attributes = new Map();
    }
    setAttribute(name, value) { this.attributes.set(name, value); }
    addEventListener(name, listener) { this.listeners.set(name, listener); }
  }
  const fetchCalls = [];
  const context = {
    window: {
      __XUAN_BRIDGE_URL__: "http://127.0.0.1:57324",
      __XUAN_BRIDGE_TOKEN__: "test-token"
    },
    document: {
      documentElement: {},
      querySelector(selector) {
        if (selector === "[data-xuan-polish-button]") return appended[0] || null;
        return textarea;
      },
      createElement(tag) {
        assert.equal(tag, "button");
        return new FakeButton();
      }
    },
    MutationObserver: class {
      constructor(callback) { this.callback = callback; }
      observe() {}
    },
    Event: class { constructor(type) { this.type = type; } },
    fetch: async (url, options) => {
      fetchCalls.push({ url, options });
      return { json: async () => ({ text: "polished draft" }) };
    }
  };
  vm.runInNewContext(source, context);
  assert.equal(appended.length, 1);
  await appended[0].listeners.get("click")();
  assert.equal(fetchCalls[0].url, "http://127.0.0.1:57324/v1/polish");
  assert.equal(fetchCalls[0].options.headers["x-xuan-bridge-token"], "test-token");
  assert.equal(textarea.value, "polished draft");
  assert.deepEqual(events, ["input"]);
  assert.equal(appended[0].disabled, false);
});
