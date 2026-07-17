import assert from "node:assert";
import { describe, it } from "node:test";
import { normalizeVlmProtocol } from "./vlm-protocol.ts";

describe("normalizeVlmProtocol", () => {
  it("缺省/非法值回落 chatCompletions", () => {
    assert.strictEqual(normalizeVlmProtocol(undefined), "chatCompletions");
    assert.strictEqual(normalizeVlmProtocol(""), "chatCompletions");
    assert.strictEqual(normalizeVlmProtocol("foo"), "chatCompletions");
  });
  it("显式 responses 保留", () => {
    assert.strictEqual(normalizeVlmProtocol("responses"), "responses");
  });
  it("chatCompletions 保留", () => {
    assert.strictEqual(normalizeVlmProtocol("chatCompletions"), "chatCompletions");
  });
});
