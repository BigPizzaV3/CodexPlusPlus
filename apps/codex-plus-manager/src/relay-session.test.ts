import assert from "node:assert";
import { describe, it } from "node:test";
import { openAiSessionBlocksModelSave, sessionProviderForProtocol } from "./relay-session.ts";

describe("supplier session identity follows the selected protocol", () => {
  it("returns OpenAI session to Custom when switching to Chat Completions", () => {
    const sessionProvider = sessionProviderForProtocol("openai", "chatCompletions");

    assert.equal(sessionProvider, "custom");
    assert.equal(openAiSessionBlocksModelSave(sessionProvider, "chatCompletions"), false);
  });

  it("keeps OpenAI session on Responses and still allows save", () => {
    const sessionProvider = sessionProviderForProtocol("openai", "responses");

    assert.equal(sessionProvider, "openai");
    assert.equal(openAiSessionBlocksModelSave(sessionProvider, "responses"), false);
  });

  it("leaves Custom unchanged for either protocol", () => {
    assert.equal(sessionProviderForProtocol("custom", "chatCompletions"), "custom");
    assert.equal(sessionProviderForProtocol("custom", "responses"), "custom");
    assert.equal(openAiSessionBlocksModelSave("custom", "chatCompletions"), false);
  });

  it("keeps the save control disabled only for the unsupported combination", () => {
    assert.equal(openAiSessionBlocksModelSave("openai", "chatCompletions"), true);
  });
});
