import assert from "node:assert/strict";
import test from "node:test";
import { maskAccountName } from "./gateway-model.ts";

test("订阅账号名里的用户名、邮箱打码，普通供应商名不变", () => {
  assert.equal(maskAccountName("ChatGPT Pro · 2649340134@qq.com"), "ChatGPT Pro · 26****34@qq.com");
  assert.equal(maskAccountName("Copilot · BigPizzaV3"), "Copilot · Bi****V3");
  assert.equal(maskAccountName("Copilot · octo"), "Copilot · o***");
  assert.equal(maskAccountName("SuperGrok · g@x.ai"), "SuperGrok · g***@x.ai");
  assert.equal(maskAccountName("ChatGPT API Plus · me@example.com"), "ChatGPT API Plus · m***@example.com");
  assert.equal(maskAccountName("DeepSeek"), "DeepSeek");
  assert.equal(maskAccountName("哈基米 国模 kimi"), "哈基米 国模 kimi");
  assert.equal(maskAccountName("我的中转 · 主力"), "我的中转 · 主力");
});

test("打码可以重复调用：已打码的名字再过一遍不变（头像悬停提示收到的可能是打过码的名字）", () => {
  for (const name of ["ChatGPT Pro · 2649340134@qq.com", "Copilot · BigPizzaV3", "Copilot · octo", "SuperGrok · g@x.ai", "DeepSeek"]) {
    const once = maskAccountName(name);
    assert.equal(maskAccountName(once), once, name);
  }
});
