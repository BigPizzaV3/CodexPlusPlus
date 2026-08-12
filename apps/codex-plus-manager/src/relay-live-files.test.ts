import assert from "node:assert";
import { describe, it } from "node:test";
import { relayAuthForLiveDraft } from "./relay-live-files.ts";

describe("relayAuthForLiveDraft", () => {
  it("preserves the complete pure API provider auth snapshot", () => {
    assert.strictEqual(relayAuthForLiveDraft({
      relayMode: "pureApi",
      authContents: '{"OPENAI_API_KEY":"provider-key","vendor":"stored"}',
    }, '{"OPENAI_API_KEY":"login-key","tokens":"live"}'),
    '{"OPENAI_API_KEY":"provider-key","vendor":"stored"}');
  });

  it("keeps the current official auth state for mixed API mode", () => {
    assert.strictEqual(relayAuthForLiveDraft({
      relayMode: "official",
      authContents: '{"tokens":"stored"}',
    }, '{"tokens":"live"}'), '{"tokens":"live"}');
  });
});
