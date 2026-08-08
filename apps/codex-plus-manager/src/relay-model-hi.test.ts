import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { runAllFetchedModelHiTests } from "./relay-model-hi.ts";

describe("batch model hi tests", () => {
  it("tests every unique non-empty model returned by the current fetch", async () => {
    const tested: string[] = [];

    const outcomes = await runAllFetchedModelHiTests(
      [" model-a ", "model-b", "", "model-a", "model-c"],
      async (model) => {
        tested.push(model);
        return `${model}-ok`;
      },
    );

    assert.deepEqual(tested, ["model-a", "model-b", "model-c"]);
    assert.deepEqual(
      outcomes.map((outcome) => outcome.model),
      tested,
    );
  });

  it("continues testing remaining fetched models after one request fails", async () => {
    const tested: string[] = [];

    const outcomes = await runAllFetchedModelHiTests(["model-a", "model-b", "model-c"], async (model) => {
      tested.push(model);
      if (model === "model-b") throw new Error("upstream failed");
      return model;
    });

    assert.deepEqual(tested, ["model-a", "model-b", "model-c"]);
    assert.deepEqual(
      outcomes.map((outcome) => outcome.status),
      ["fulfilled", "rejected", "fulfilled"],
    );
  });
});
