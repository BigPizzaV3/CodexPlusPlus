import assert from "node:assert";
import { describe, it } from "node:test";
import { configHasFeature, setFeatureInConfig } from "./relay-features.ts";

describe("relay feature helpers", () => {
  it("writes realtime_conversation without overwriting other features", () => {
    const config = setFeatureInConfig(
      `model = "gpt-5.6-sol"\n\n[features]\nfast_mode = true\nimage_generation = true\n`,
      "realtime_conversation",
      true,
    );

    assert.match(config, /fast_mode = true/);
    assert.match(config, /image_generation = true/);
    assert.match(config, /realtime_conversation = true/);
    assert.ok(configHasFeature(config, "realtime_conversation"));
  });

  it("removes only realtime_conversation when disabled", () => {
    const config = setFeatureInConfig(
      `[features]\nfast_mode = true\nrealtime_conversation = true\n`,
      "realtime_conversation",
      false,
    );

    assert.match(config, /fast_mode = true/);
    assert.doesNotMatch(config, /realtime_conversation/);
    assert.ok(!configHasFeature(config, "realtime_conversation"));
  });

  it("creates a features table when the profile has none", () => {
    const config = setFeatureInConfig('model = "gpt-5.6-sol"\n', "realtime_conversation", true);

    assert.match(config, /\[features\]/);
    assert.match(config, /realtime_conversation = true/);
  });
});
