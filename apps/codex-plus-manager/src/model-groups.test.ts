import assert from "node:assert/strict";
import test from "node:test";

import { filterModelGroups, groupModelIds, OTHER_MODEL_GROUP } from "./model-groups.ts";

test("groups known families in a fixed order and sorts within a group", () => {
  assert.deepEqual(groupModelIds(["claude-opus-5", "gpt-5.4", "gpt-5.2", "claude-haiku-4-5"]), [
    { label: "GPT", models: ["gpt-5.2", "gpt-5.4"] },
    { label: "Claude", models: ["claude-haiku-4-5", "claude-opus-5"] },
  ]);
});

test("puts unrecognized model ids in Other, last", () => {
  const groups = groupModelIds(["codex-auto-review", "gpt-5.4", "some-vendor-model"]);
  assert.deepEqual(groups.map((group) => group.label), ["GPT", "Codex", OTHER_MODEL_GROUP]);
  assert.deepEqual(groups[groups.length - 1].models, ["some-vendor-model"]);
});

test("drops blank entries and duplicates, keeping the first spelling", () => {
  assert.deepEqual(groupModelIds(["gpt-5.4", " ", "", "gpt-5.4", "  gpt-5.2  "]), [
    { label: "GPT", models: ["gpt-5.2", "gpt-5.4"] },
  ]);
});

test("matches families across separator styles and version suffixes", () => {
  assert.deepEqual(groupModelIds(["qwen3.5-max", "qwen_vl", "glm/4.6"]), [
    { label: "Qwen", models: ["qwen_vl", "qwen3.5-max"] },
    { label: "GLM", models: ["glm/4.6"] },
  ]);
  // 版本号直接贴在家族名后面也要认出来
  assert.deepEqual(groupModelIds(["o3-mini", "llama4", "glm4-plus"]).map((group) => group.label), [
    "OpenAI o-series",
    "GLM",
    "Llama",
  ]);
});

test("does not treat a family name appearing mid-id as that family", () => {
  assert.deepEqual(groupModelIds(["vendor-gpt-5"]), [
    { label: OTHER_MODEL_GROUP, models: ["vendor-gpt-5"] },
  ]);
});

test("returns no groups for an empty list", () => {
  assert.deepEqual(groupModelIds([]), []);
  assert.deepEqual(groupModelIds(["", "   "]), []);
});

test("filters by substring, case-insensitively, before grouping", () => {
  const models = ["gpt-5.4", "gpt-5.4-mini", "claude-opus-5"];
  assert.deepEqual(filterModelGroups(models, "MINI"), [{ label: "GPT", models: ["gpt-5.4-mini"] }]);
  assert.deepEqual(filterModelGroups(models, "opus"), [{ label: "Claude", models: ["claude-opus-5"] }]);
  assert.deepEqual(filterModelGroups(models, "nothing-matches"), []);
});

test("blank query filters nothing", () => {
  const models = ["gpt-5.4", "claude-opus-5"];
  assert.deepEqual(filterModelGroups(models, "   "), groupModelIds(models));
});
