import assert from "node:assert/strict";
import test from "node:test";

import {
  codexGoalsFeatureState,
  codexGoalsFeatureValue,
  setCodexGoalsFeatureInConfig,
} from "./goals-config.ts";

test("reads explicit goals values only from the features table", () => {
  assert.equal(codexGoalsFeatureValue("[features]\ngoals = true\n"), true);
  assert.equal(codexGoalsFeatureValue("[features]\ngoals = false\n"), false);
  assert.equal(codexGoalsFeatureValue("[features.other]\ngoals = true\n"), undefined);
});

test("uses an explicit common goals value when the profile has no override", () => {
  assert.deepEqual(
    codexGoalsFeatureState("", "[features]\ngoals = true\n", true),
    { enabled: true, inherited: true },
  );
  assert.deepEqual(
    codexGoalsFeatureState("", "[features]\ngoals = false\n", true),
    { enabled: false, inherited: true },
  );
});

test("profile goals value overrides common config", () => {
  assert.deepEqual(
    codexGoalsFeatureState("[features]\ngoals = false\n", "[features]\ngoals = true\n", true),
    { enabled: false, inherited: false },
  );
  assert.deepEqual(
    codexGoalsFeatureState("[features]\ngoals = true\n", "[features]\ngoals = false\n", true),
    { enabled: true, inherited: false },
  );
});

test("uses the profile value when common config has no goals value", () => {
  assert.deepEqual(
    codexGoalsFeatureState("[features]\ngoals = true\n", "[features]\nfast_mode = true\n", true),
    { enabled: true, inherited: false },
  );
});

test("ignores common goals when common config is disabled", () => {
  assert.deepEqual(
    codexGoalsFeatureState("", "[features]\ngoals = true\n", false),
    { enabled: false, inherited: false },
  );
});

test("writes explicit true and false overrides without changing unrelated feature values", () => {
  const enabled = setCodexGoalsFeatureInConfig("[features]\nfast_mode = true\n", true);
  assert.equal(codexGoalsFeatureValue(enabled), true);
  assert.match(enabled, /fast_mode = true/);

  const disabled = setCodexGoalsFeatureInConfig(enabled, false);
  assert.equal(disabled, "[features]\ngoals = false\nfast_mode = true\n");
  assert.equal(codexGoalsFeatureValue(disabled), false);
});

test("creates a features table for an explicit false override", () => {
  assert.equal(setCodexGoalsFeatureInConfig('model = "gpt-5"\n', false), 'model = "gpt-5"\n\n[features]\ngoals = false\n');
});
