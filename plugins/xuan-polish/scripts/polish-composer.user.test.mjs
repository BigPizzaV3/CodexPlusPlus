import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";

const source = fs.readFileSync(path.join(import.meta.dirname, "polish-composer.user.js"), "utf8");

test("polish script is a thin feature-detected composer adapter", () => {
  assert.match(source, /MutationObserver/);
  assert.match(source, /data-xuan-polish-button/);
  assert.match(source, /aria-label='更改权限'/);
  assert.match(source, /__codexSessionDeleteBridge/);
  assert.match(source, /127\.0\.0\.1:57324/);
  assert.doesNotMatch(source, /api[_-]?key|authorization|bearer/i);
});

test("polish script installs once and replaces composer text", async () => {
  const events = [];
  const appended = [];
  const textarea = {
    value: "  rough draft  ",
    dispatchEvent(event) { events.push(event.type); }
  };
  const permission = {
    className: "permission-control",
    parentElement: {},
    nextElementSibling: null,
    insertAdjacentElement(position, node) {
      assert.equal(position, "afterend");
      appended.push(node);
      this.nextElementSibling = node;
      node.parentElement = this.parentElement;
    }
  };
  class FakeButton {
    constructor() {
      this.listeners = new Map();
      this.disabled = false;
      this.attributes = new Map();
      this.style = {};
    }
    setAttribute(name, value) { this.attributes.set(name, value); }
    addEventListener(name, listener) { this.listeners.set(name, listener); }
  }
  const bridgeCalls = [];
  const context = {
    window: {
      __XUAN_BRIDGE_URL__: "http://127.0.0.1:57324",
      __XUAN_BRIDGE_TOKEN__: "test-token",
      async __codexSessionDeleteBridge(path, payload) {
        bridgeCalls.push({ path, payload });
        return { text: "polished draft" };
      }
    },
    document: {
      documentElement: {},
      querySelector(selector) {
        if (selector === "[data-xuan-polish-button]") return appended[0] || null;
        if (selector.includes("button[aria-label='更改权限']")) return permission;
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
    fetch: async () => { throw new Error("native bridge should be preferred"); }
  };
  vm.runInNewContext(source, context);
  assert.equal(appended.length, 1);
  assert.equal(permission.nextElementSibling, appended[0]);
  assert.equal(appended[0].className, "permission-control");
  assert.equal(appended[0].attributes.get("aria-label"), "润色输入内容");
  await appended[0].listeners.get("click")();
  assert.equal(bridgeCalls.length, 1);
  assert.equal(bridgeCalls[0].path, "/v1/polish");
  assert.equal(bridgeCalls[0].payload.text, "rough draft");
  assert.equal(bridgeCalls[0].payload.style, "structured");
  assert.equal(textarea.value, "polished draft");
  assert.deepEqual(events, ["input"]);
  assert.equal(appended[0].disabled, false);
});
